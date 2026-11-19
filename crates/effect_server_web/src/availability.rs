//! Distinct readiness/health/liveness with explicit deadlines and bounded parallelism.
//!
//! Probe provider errors become safe availability states; their details are never
//! serialized into public responses. Defect/interruption/cleanup keep their original
//! Effect outcomes. Completed observations survive an overall collection timeout.

use crate::WebError;
use crate::{effect_die, effect_future, effect_map, effect_map_error, effect_sync};
use effect::memory::{
    Buffer, BufferLease, BufferView, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId,
    SharedSlot, SlotPool,
};
use effect::{Defect, Effect, Exit, HasClock, HasTaskSpawner, LibraryDefect, PreparedUnitCohorts};
use effect::{effect, effect_for_each_concurrent, effect_timeout};
use effect_server::{endpoint, registered, reply, router};
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Typed availability configuration/collection failures mapped into application E.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum AvailabilityError {
    /// Malformed/duplicate identity or missing readiness selection.
    Definition(String),
    /// Explicit per-probe deadline expired for this safe identity.
    ProbeDeadline(String),
    /// Explicit whole-collection deadline expired.
    CollectionDeadline,
    /// The selected native safe response writer rejected output.
    Output,
    /// The selected operational output storage refused admission or writing.
    Memory(MemoryError),
}
/// Probe responsibility; external dependencies belong in readiness, not shallow liveness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum ProbeRole {
    /// Safe operational health summary.
    Health,
    /// Whether this instance can accept new application traffic.
    Readiness,
    /// Whether the selected application loop is viable.
    Liveness,
}
/// Public safe probe classification, never a provider exception/body/credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum SignalState {
    /// Provider acknowledged the required condition.
    Available,
    /// Provider acknowledged false or returned an expected provider failure.
    Unavailable,
    /// Explicit probe/collection deadline elapsed before settlement.
    TimedOut,
}
/// One inert typed probe definition.
#[must_use]
pub struct Probe<E: Send + 'static, R: Send + Sync + 'static> {
    /// Safe prepared identity selected by the application.
    pub name: Arc<str>,
    /// Distinct selected probe responsibility.
    pub role: ProbeRole,
    /// Required failures reject readiness; optional failures only mark degradation.
    pub required: bool,
    /// Reusable provider Effect; execution is lazy in the selected R.
    pub sample: Effect<bool, E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for Probe<E, R> {
    /// Share descriptions and identity metadata, never provider errors/results.
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            role: self.role,
            required: self.required,
            sample: self.sample.clone(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Probe<E, R> {
    /// Prepare a safe probe name without invoking its provider.
    pub fn new(
        name: impl Into<Arc<str>>,
        role: ProbeRole,
        required: bool,
        sample: Effect<bool, E, R>,
    ) -> Result<Self, AvailabilityError> {
        let name = name.into();
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(AvailabilityError::Definition("probe identity".into()));
        }
        Ok(Self {
            name,
            role,
            required,
            sample,
        })
    }
    /// Explicit shallow process viability. This named path makes no claim about
    /// databases/dependencies; callers select their real application loop Effect.
    pub fn process_liveness(
        name: impl Into<Arc<str>>,
        viable: Effect<bool, E, R>,
    ) -> Result<Self, AvailabilityError> {
        Self::new(name, ProbeRole::Liveness, true, viable)
    }
}
/// Explicit elapsed collection bounds, independent of runtime timer quanta.
#[derive(Clone, Copy)]
#[must_use]
pub struct ProbeDeadlines {
    /// Deadline for each started signal including owned cancellation/cleanup.
    pub per_probe: Duration,
    /// Deadline for the complete collection, preserving completed observations.
    pub overall: Duration,
}
impl ProbeDeadlines {
    /// Reject zero durations before any probe starts.
    pub fn validate(self) -> Result<Self, AvailabilityError> {
        if self.per_probe.is_zero() || self.overall.is_zero() {
            return Err(AvailabilityError::Definition("probe deadlines".into()));
        }
        Ok(self)
    }
}
/// Public finite safe observation metadata.
#[derive(Clone, Debug)]
#[must_use]
pub struct SignalReport {
    /// Safe prepared probe identity.
    pub name: Arc<str>,
    /// Whether a failed signal is required for this collection's success.
    pub required: bool,
    /// Safe classification with provider details removed.
    pub state: SignalState,
}
/// Finite role-specific collection result; external service exceptions are excluded.
#[derive(Clone, Debug)]
#[must_use]
pub struct ProbeReport {
    /// Prepared service identity.
    pub service: Arc<str>,
    /// Distinct collection responsibility.
    pub role: ProbeRole,
    /// Required signals and explicit traffic-ready state permit new traffic.
    pub accepting_traffic: bool,
    /// At least one optional/required signal did not acknowledge availability.
    pub degraded: bool,
    /// Completed and timed-out safe observations in declaration order.
    pub signals: Vec<SignalReport>,
}

type ReportSlot = Mutex<FixedVector<'static, SignalReport>>;
type OutputSlots = SlotPool<'static, Buffer<'static>>;

/// Independent operational headroom for report assembly and encoded replies.
pub struct PreparedAvailability<E: Send + 'static> {
    reports: SlotPool<'static, ReportSlot>,
    output: Arc<OutputSlots>,
    cohorts: Arc<PreparedUnitCohorts<CollectionFailure<E>>>,
}

impl<E: Send + 'static> PreparedAvailability<E> {
    /// Layout bytes for the two physical pools, excluding allocator overhead.
    pub fn storage_bytes(
        calls: NonZeroUsize,
        probes: NonZeroUsize,
        output_bytes: Bytes,
    ) -> Result<Bytes, MemoryError> {
        let reports = std::alloc::Layout::array::<SignalReport>(probes.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<ReportSlot>::storage_bytes(calls.get())?
            .checked_add(Bytes::new(reports.size()).checked_mul(calls.get())?)?
            .checked_add(OutputSlots::storage_bytes(calls.get())?)?
            .checked_add(output_bytes.checked_mul(calls.get())?)?
            .checked_add(PreparedUnitCohorts::<CollectionFailure<E>>::storage_bytes(
                calls, probes, probes,
            )?)
    }

    /// Provision independent report and output slots before accepting traffic.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        calls: NonZeroUsize,
        probes: NonZeroUsize,
        output_bytes: Bytes,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(calls, probes, output_bytes)?;
        Ok(Self {
            reports: memory.slots(region, calls.get(), |_| {
                FixedVector::prepare(memory.allocator(region)?, probes.get()).map(Mutex::new)
            })?,
            output: Arc::new(memory.buffers(region, calls.get(), output_bytes)?),
            cohorts: Arc::new(PreparedUnitCohorts::prepare(
                memory, region, calls, probes, probes,
            )?),
        })
    }
}

struct PreparedSignals(Option<SharedSlot<'static, ReportSlot>>);
impl Clone for PreparedSignals {
    #[allow(clippy::expect_used)] // Live handles share one bounded slot with Arc's representational limit.
    fn clone(&self) -> Self {
        Self(Some(
            self.slot()
                .try_clone()
                .expect("availability report owner limit"),
        ))
    }
}
impl PreparedSignals {
    #[allow(clippy::expect_used)] // The slot is present until its owning handle drops.
    fn slot(&self) -> &SharedSlot<'static, ReportSlot> {
        self.0.as_ref().expect("live availability report")
    }
}
impl Drop for PreparedSignals {
    fn drop(&mut self) {
        if let Some(mut slot) = self.0.take().and_then(SharedSlot::into_last) {
            slot.get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
        }
    }
}

#[derive(Clone)]
enum ReportSignals {
    Ordinary(Arc<Mutex<Vec<SignalReport>>>),
    Prepared(PreparedSignals),
}
impl ReportSignals {
    fn with<T>(&self, use_signals: impl FnOnce(&[SignalReport]) -> T) -> T {
        match self {
            Self::Ordinary(signals) => use_signals(
                &signals
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
            Self::Prepared(signals) => use_signals(
                signals
                    .slot()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_slice(),
            ),
        }
    }

    fn record(&self, name: &str, state: SignalState) {
        let update = |signals: &mut [SignalReport]| {
            if let Some(report) = signals
                .iter_mut()
                .find(|report| report.name.as_ref() == name)
            {
                report.state = state;
            }
        };
        match self {
            Self::Ordinary(signals) => update(
                &mut signals
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
            Self::Prepared(signals) => update(
                signals
                    .slot()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_mut_slice(),
            ),
        }
    }

    fn into_ordinary(self) -> Vec<SignalReport> {
        match self {
            Self::Ordinary(signals) => std::mem::take(
                &mut *signals
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
            prepared @ Self::Prepared(_) => prepared.with(<[SignalReport]>::to_vec),
        }
    }
}

struct CollectedReport {
    service: Arc<str>,
    role: ProbeRole,
    accepting_traffic: bool,
    degraded: bool,
    signals: ReportSignals,
}
/// Private collection carrier retaining backend failures until classification.
enum ProbeFailure<E> {
    /// Original expected provider error, not exposed through public report fields.
    Expected(E),
    /// Named explicit deadline failure.
    Deadline(Arc<str>),
}
enum CollectionFailure<E> {
    Domain(E),
    Deadline(E),
    Memory(MemoryError),
}

impl<E> CollectionFailure<E> {
    fn into_error(self, errors: &dyn Fn(AvailabilityError) -> E) -> E {
        match self {
            Self::Domain(error) | Self::Deadline(error) => error,
            Self::Memory(error) => errors(AvailabilityError::Memory(error)),
        }
    }
}

/// Inert complete operational policy; nonempty readiness is mandatory.
#[must_use]
pub struct Availability<E: Send + 'static, R: Send + Sync + 'static> {
    /// Prepared service identity shown by safe writers/application renderers.
    service: Arc<str>,
    /// Startup partitions keep the declaration order for each responsibility.
    probes: [Arc<[Probe<E, R>]>; 3],
    /// Caller-supplied elapsed per-signal/overall bounds.
    deadlines: ProbeDeadlines,
    /// Positive bounded concurrent probe count.
    parallelism: NonZeroUsize,
    /// Application error mapping for own deadline/configuration paths.
    errors: Arc<dyn Fn(AvailabilityError) -> E + Send + Sync>,
    /// Explicitly authorized operator registrations, absent unless selected.
    dashboard: Option<crate::dashboard::ProtectedDashboard<E, R>>,
    prepared_output: Option<Arc<SlotPool<'static, Buffer<'static>>>>,
    workspace: Option<Arc<PreparedAvailability<E>>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for Availability<E, R> {
    /// Share immutable definitions/callbacks without copying errors or spawning work.
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            probes: self.probes.clone(),
            deadlines: self.deadlines,
            parallelism: self.parallelism,
            errors: self.errors.clone(),
            dashboard: self.dashboard.clone(),
            prepared_output: self.prepared_output.clone(),
            workspace: self.workspace.clone(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Availability<E, R> {
    /// Validate service/probe identities and nonempty readiness before startup.
    pub fn new(
        service: impl Into<Arc<str>>,
        errors: impl Fn(AvailabilityError) -> E + Send + Sync + 'static,
        probes: Vec<Probe<E, R>>,
        deadlines: ProbeDeadlines,
        parallelism: NonZeroUsize,
        traffic_ready: Option<Effect<bool, E, R>>,
    ) -> Result<Self, AvailabilityError> {
        let service = service.into();
        if service.is_empty()
            || service.chars().any(char::is_control)
            || !probes
                .iter()
                .any(|probe| probe.role == ProbeRole::Readiness)
        {
            return Err(AvailabilityError::Definition(
                "service/nonempty readiness".into(),
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for probe in &probes {
            if probe.name.is_empty() || probe.name.chars().any(char::is_control) {
                return Err(AvailabilityError::Definition("probe identity".into()));
            }
            if !names.insert((probe.role as u8, probe.name.clone())) {
                return Err(AvailabilityError::Definition(
                    "duplicate probe identity".into(),
                ));
            }
        }
        if traffic_ready.is_some()
            && probes.iter().any(|probe| {
                probe.role == ProbeRole::Readiness && probe.name.as_ref() == "traffic readiness"
            })
        {
            return Err(AvailabilityError::Definition(
                "traffic readiness probe identity is reserved when selected".into(),
            ));
        }
        let deadlines = deadlines.validate()?;
        let (mut health, mut readiness, mut liveness) = (Vec::new(), Vec::new(), Vec::new());
        for probe in probes {
            match probe.role {
                ProbeRole::Health => health.push(probe),
                ProbeRole::Readiness => readiness.push(probe),
                ProbeRole::Liveness => liveness.push(probe),
            }
        }
        if let Some(sample) = traffic_ready {
            readiness.push(Probe {
                name: Arc::from("traffic readiness"),
                role: ProbeRole::Readiness,
                required: true,
                sample,
            });
        }
        Ok(Self {
            service,
            probes: [health.into(), readiness.into(), liveness.into()],
            deadlines,
            parallelism,
            errors: Arc::new(errors),
            dashboard: None,
            prepared_output: None,
            workspace: None,
        })
    }
    /// Attach prepared operator-authorized routes. No dashboard renderers/access
    /// policy or visible labels are manufactured by availability collection.
    pub fn with_dashboard(mut self, dashboard: crate::dashboard::ProtectedDashboard<E, R>) -> Self {
        self.dashboard = Some(dashboard);
        self
    }
    /// Select independent finite response storage for all availability aliases.
    /// The actual buffer is admitted before any probe executes.
    pub fn with_prepared_output(mut self, output: Arc<SlotPool<'static, Buffer<'static>>>) -> Self {
        self.prepared_output = Some(output);
        self
    }
    /// Select independent operational report and output ownership for the mounted aliases.
    pub fn with_prepared_workspace(mut self, workspace: Arc<PreparedAvailability<E>>) -> Self {
        self.prepared_output = Some(workspace.output.clone());
        self.workspace = Some(workspace);
        self
    }
    /// Mount the six reference aliases using existing Server registration. These
    /// paths are protocol identities from effect.server_web, not operational quotas.
    pub fn routes(&self) -> Result<effect_server::Router<E, R>, effect_server::ServerError>
    where
        R: HasClock + HasTaskSpawner + effect_server::HasRequest,
    {
        let methods = effect_server::HttpMethodSet::new(vec![http::Method::GET])?;
        let mut router = router! {}?;
        for (path, role) in [
            ("/health", ProbeRole::Health),
            ("/healthz", ProbeRole::Health),
            ("/ready", ProbeRole::Readiness),
            ("/readyz", ProbeRole::Readiness),
            ("/live", ProbeRole::Liveness),
            ("/livez", ProbeRole::Liveness),
        ] {
            router = router! {
                source: router,
                methods(methods.clone(), path, registered! {
                    endpoint(endpoint! { effect(self.reply(role)) }),
                }),
            }?;
        }
        if let Some(dashboard) = &self.dashboard {
            router = router! {
                source: router,
                include(dashboard.clone().routes()),
            }?;
        }
        Ok(router)
    }
    /// Collect one responsibility. Each signal runs on the selected core provider;
    /// completed observations survive overall timeout after cancelled losers join.
    /// This compatibility method returns an owned Vec. The mounted prepared
    /// reply keeps the selected report rows until encoding is complete.
    pub fn collect(&self, role: ProbeRole) -> Effect<ProbeReport, E, R>
    where
        R: HasClock + HasTaskSpawner,
    {
        effect_map! {
            program: self.collect_report(role),
            map: |report: CollectedReport| ProbeReport {
                service: report.service,
                role: report.role,
                accepting_traffic: report.accepting_traffic,
                degraded: report.degraded,
                signals: report.signals.into_ordinary(),
            },
        }
    }

    fn collect_report(&self, role: ProbeRole) -> Effect<CollectedReport, E, R>
    where
        R: HasClock + HasTaskSpawner,
    {
        let definition = self.clone();
        effect_future! {
            exit_with_context; body: move |env: Arc<R>, context| {
                let definition = definition.clone();
                async move {
                    let [health, readiness, liveness] = &definition.probes;
                    let selected = match role {
                        ProbeRole::Health => health,
                        ProbeRole::Readiness => readiness,
                        ProbeRole::Liveness => liveness,
                    };
                    let reports = match definition.reserve_reports(selected) {
                        Ok(reports) => reports,
                        Err(error) => return Exit::Fail((definition.errors)(AvailabilityError::Memory(error))),
                    };
                    let shared_reports = reports.clone();
                    let observer = definition.clone();
                    let probes = selected.clone();
                    let errors = definition.errors.clone();
                    let program = if let Some(workspace) = &definition.workspace {
                        workspace.cohorts.run(
                            probes.len(),
                            definition.parallelism,
                            CollectionFailure::Memory,
                            move |index| {
                                let Some(probe) = probes.get(index) else {
                                    return effect_die! { defect: Defect::from(LibraryDefect::ParallelResultMissing), };
                                };
                                effect_map_error! {
                                    program: observer.observe_probe(probe, shared_reports.clone()),
                                    map: CollectionFailure::Domain,
                                }
                            },
                        )
                    } else {
                        effect_map! {
                            program: effect_for_each_concurrent! {
                                shared;
                                concurrency: definition.parallelism,
                                values: probes,
                                body: move |probe| effect_map_error! {
                                    program: observer.observe_probe(probe, shared_reports.clone()),
                                    map: CollectionFailure::Domain,
                                },
                            },
                            map: |_| (),
                        }
                    };
                    let collection = effect_timeout! {
                        program: program,
                        duration: definition.deadlines.overall,
                        on_timeout: move || CollectionFailure::Deadline(errors(AvailabilityError::CollectionDeadline)),
                    };
                    match effect::effect! { run_in_context; context: context, program: collection, environment: env, }.await {
                        Exit::Success(()) | Exit::Fail(CollectionFailure::Deadline(_)) => {}
                        Exit::Fail(error) => return Exit::Fail(error.into_error(&*definition.errors)),
                        Exit::Die(defect) => return Exit::Die(defect),
                        Exit::Interrupted => return Exit::Interrupted,
                        Exit::FinalizationFailed(failure) => {
                            return Exit::<CollectedReport, _>::FinalizationFailed(failure)
                                .map_error(|error| error.into_error(&*definition.errors));
                        }
                    }
                    let (accepting_traffic, degraded) = reports.with(|signals| {
                        (
                            signals.iter().all(|signal| !signal.required || signal.state == SignalState::Available),
                            signals.iter().any(|signal| signal.state != SignalState::Available),
                        )
                    });
                    Exit::Success(CollectedReport {
                        service: definition.service.clone(), role, accepting_traffic, degraded, signals: reports,
                    })
                }
            },
        }
    }

    fn reserve_reports(&self, probes: &[Probe<E, R>]) -> Result<ReportSignals, MemoryError> {
        let reports = probes.iter().map(|probe| SignalReport {
            name: probe.name.clone(),
            required: probe.required,
            state: SignalState::TimedOut,
        });
        let Some(workspace) = &self.workspace else {
            return Ok(ReportSignals::Ordinary(Arc::new(Mutex::new(
                reports.collect(),
            ))));
        };
        let mut slot = workspace.reports.try_checkout()?;
        let table = slot
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if probes.len() > table.capacity() {
            return Err(MemoryError::NoSlot);
        }
        for report in reports {
            if table.push(report).is_err() {
                table.clear();
                return Err(MemoryError::NoSlot);
            }
        }
        Ok(ReportSignals::Prepared(PreparedSignals(Some(slot.share()))))
    }

    fn observe_probe(&self, probe: &Probe<E, R>, reports: ReportSignals) -> Effect<(), E, R>
    where
        R: HasClock + HasTaskSpawner,
    {
        let name = probe.name.clone();
        let deadline_name = name.clone();
        let errors = self.errors.clone();
        let program = effect_timeout! {
            program: effect_map_error! { program: probe.sample.clone(), map: ProbeFailure::Expected, },
            duration: self.deadlines.per_probe,
            on_timeout: move || ProbeFailure::Deadline(deadline_name.clone()),
        };
        effect! {
            bind;
            program: effect_map_error! { program: program.materialize_exit(), map: |never| match never {}, },
            body: move |exit| {
                let state = match exit {
                    Exit::Success(true) => SignalState::Available,
                    Exit::Success(false) | Exit::Fail(ProbeFailure::Expected(_)) => SignalState::Unavailable,
                    Exit::Fail(ProbeFailure::Deadline(_)) => SignalState::TimedOut,
                    Exit::Die(defect) => return effect_die! { defect: defect, },
                    Exit::Interrupted => return crate::restore_exit(Exit::Interrupted),
                    Exit::FinalizationFailed(failure) => {
                        let errors = errors.clone();
                        return crate::restore_exit(
                            Exit::<(), _>::FinalizationFailed(failure).map_error(move |error| match error {
                                ProbeFailure::Expected(error) => error,
                                ProbeFailure::Deadline(name) => errors(AvailabilityError::ProbeDeadline(name.to_string())),
                            }),
                        );
                    }
                };
                reports.record(&name, state);
                effect_sync! { body { () }, }
            },
        }
    }

    /// Native finite reply uses a direct safe JSON writer. Only prepared identities,
    /// roles/states/booleans enter the response; no serde serialization of E occurs.
    pub fn reply(&self, role: ProbeRole) -> Effect<effect_server::Reply<E, R>, E, R>
    where
        R: HasClock + HasTaskSpawner,
    {
        let errors = self.errors.clone();
        let output = self.prepared_output.clone();
        let collection = self.collect_report(role);
        effect_future! {
            exit_with_context; body: move |env, context| {
                let errors = errors.clone();
                let output = output.clone();
                let collection = collection.clone();
                async move {
                    let output = match output {
                        Some(pool) => match pool.try_checkout() {
                            Ok(mut buffer) => {
                                buffer.clear();
                                ProbeOutput::Prepared { buffer, failure: None }
                            }
                            Err(error) => return Exit::Fail(errors(AvailabilityError::Memory(error))),
                        },
                        None => ProbeOutput::Ordinary(Vec::new()),
                    };
                    match effect::effect! { run_in_context; context: context, program: collection, environment: env, }.await {
                        Exit::Success(report) => match format_report(&report, output) {
                            Ok(reply) => Exit::Success(reply),
                            Err(error) => Exit::Fail(errors(error)),
                        },
                        Exit::Fail(error) => Exit::Fail(error),
                        Exit::Die(defect) => Exit::Die(defect),
                        Exit::Interrupted => Exit::Interrupted,
                        Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
                    }
                }
            },
        }
    }
}

enum ProbeOutput {
    Ordinary(Vec<u8>),
    Prepared {
        buffer: BufferLease<'static>,
        failure: Option<MemoryError>,
    },
}

impl Write for ProbeOutput {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        match self {
            Self::Ordinary(output) => output.extend_from_slice(input),
            Self::Prepared { buffer, failure } => {
                if let Err(error) = buffer.extend_from_slice(input) {
                    *failure = Some(error);
                    return Err(io::ErrorKind::WriteZero.into());
                }
            }
        }
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn format_report<E: Send + 'static, R: Send + Sync + 'static>(
    report: &CollectedReport,
    mut output: ProbeOutput,
) -> Result<effect_server::Reply<E, R>, AvailabilityError> {
    if report
        .signals
        .with(|signals| write_report(report, signals, &mut output))
        .is_err()
    {
        return Err(match output {
            ProbeOutput::Prepared {
                failure: Some(error),
                ..
            } => AvailabilityError::Memory(error),
            ProbeOutput::Prepared { failure: None, .. } | ProbeOutput::Ordinary(_) => {
                AvailabilityError::Output
            }
        });
    }
    let media = http::HeaderValue::from_static("application/json");
    let reply = match output {
        ProbeOutput::Prepared { buffer, .. } => {
            reply! { lease(BufferView::new(buffer.share()), media) }
        }
        ProbeOutput::Ordinary(bytes) => reply! { bytes(bytes.into(), media) },
    };
    let status = effect_server::BodyStatus::new(if report.accepting_traffic {
        http::StatusCode::OK
    } else {
        http::StatusCode::SERVICE_UNAVAILABLE
    })
    .map_err(|_| AvailabilityError::Output)?;
    Ok(reply! {
        status(status),
        header(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-store")),
        source(reply),
    })
}

fn write_report(
    report: &CollectedReport,
    signals: &[SignalReport],
    output: &mut impl Write,
) -> io::Result<()> {
    output.write_all(b"{\"service\":")?;
    serde_json::to_writer(&mut *output, report.service.as_ref()).map_err(io::Error::other)?;
    output.write_all(b",\"role\":")?;
    let role = match report.role {
        ProbeRole::Health => "health",
        ProbeRole::Readiness => "readiness",
        ProbeRole::Liveness => "liveness",
    };
    serde_json::to_writer(&mut *output, role).map_err(io::Error::other)?;
    output.write_all(b",\"acceptingTraffic\":")?;
    output.write_all(if report.accepting_traffic {
        b"true"
    } else {
        b"false"
    })?;
    output.write_all(b",\"degraded\":")?;
    output.write_all(if report.degraded { b"true" } else { b"false" })?;
    output.write_all(b",\"signals\":[")?;
    for (index, signal) in signals.iter().enumerate() {
        if index != 0 {
            output.write_all(b",")?;
        }
        output.write_all(b"{\"name\":")?;
        serde_json::to_writer(&mut *output, signal.name.as_ref()).map_err(io::Error::other)?;
        output.write_all(b",\"required\":")?;
        output.write_all(if signal.required { b"true" } else { b"false" })?;
        output.write_all(b",\"state\":")?;
        let state = match signal.state {
            SignalState::Available => "available",
            SignalState::Unavailable => "unavailable",
            SignalState::TimedOut => "timedOut",
        };
        serde_json::to_writer(&mut *output, state).map_err(io::Error::other)?;
        output.write_all(b"}")?;
    }
    output.write_all(b"]}")
}

/// Map static web configuration failures without selecting another error/runtime engine.
impl From<WebError> for AvailabilityError {
    /// Preserve the policy identity as a safe definition error.
    fn from(error: WebError) -> Self {
        Self::Definition(error.to_string())
    }
}

/// Safe category display excludes all provider errors and observation payloads.
impl std::fmt::Display for AvailabilityError {
    /// Render only configuration identity and named elapsed deadline categories.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Definition(name) => write!(f, "invalid availability definition: {name}"),
            Self::ProbeDeadline(name) => write!(f, "probe deadline: {name}"),
            Self::CollectionDeadline => f.write_str("availability collection deadline"),
            Self::Output => f.write_str("availability output"),
            Self::Memory(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for AvailabilityError {}
/// Prepared and checked nested operational descriptions retain their typed failure.
pub trait IntoAvailability<E: Send + 'static, R: Send + Sync + 'static> {
    /// Resolve inert configuration before host startup, without sampling probes.
    fn into_availability(self) -> Result<Availability<E, R>, AvailabilityError>;
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoAvailability<E, R> for Availability<E, R> {
    /// A previously checked description needs no provider work.
    fn into_availability(self) -> Result<Availability<E, R>, AvailabilityError> {
        Ok(self)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoAvailability<E, R>
    for Result<Availability<E, R>, AvailabilityError>
{
    /// Nested macro configuration errors remain typed startup failures.
    fn into_availability(self) -> Result<Availability<E, R>, AvailabilityError> {
        self
    }
}
