//! Distinct readiness/health/liveness with explicit deadlines and bounded parallelism.
//!
//! Probe provider errors become safe availability states; their details are never
//! serialized into public responses. Defect/interruption/cleanup keep their original
//! Effect outcomes. Completed observations survive an overall collection timeout.

use crate::WebError;
use crate::{
    effect_all_concurrent, effect_die, effect_future, effect_map, effect_map_error, effect_sync,
    effect_try_sync,
};
use effect::{Effect, Exit, HasClock, HasTaskSpawner};
use effect::{effect, effect_timeout};
use effect_server::{endpoint, registered, reply, router};
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
/// Private collection carrier retaining backend failures until classification.
enum ProbeFailure<E> {
    /// Original expected provider error, not exposed through public report fields.
    Expected(E),
    /// Named explicit deadline failure.
    Deadline(Arc<str>),
}
/// Inert complete operational policy; nonempty readiness is mandatory.
#[must_use]
pub struct Availability<E: Send + 'static, R: Send + Sync + 'static> {
    /// Prepared service identity shown by safe writers/application renderers.
    service: Arc<str>,
    /// All selected signals with distinct responsibilities.
    probes: Arc<[Probe<E, R>]>,
    /// Caller-supplied elapsed per-signal/overall bounds.
    deadlines: ProbeDeadlines,
    /// Positive bounded concurrent probe count.
    parallelism: NonZeroUsize,
    /// Optional explicit startup/drain readiness capability.
    traffic_ready: Option<Effect<bool, E, R>>,
    /// Application error mapping for own deadline/configuration paths.
    errors: Arc<dyn Fn(AvailabilityError) -> E + Send + Sync>,
    /// Explicitly authorized operator registrations, absent unless selected.
    dashboard: Option<crate::dashboard::ProtectedDashboard<E, R>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for Availability<E, R> {
    /// Share immutable definitions/callbacks without copying errors or spawning work.
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            probes: self.probes.clone(),
            deadlines: self.deadlines,
            parallelism: self.parallelism,
            traffic_ready: self.traffic_ready.clone(),
            errors: self.errors.clone(),
            dashboard: self.dashboard.clone(),
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
        Ok(Self {
            service,
            probes: probes.into(),
            deadlines: deadlines.validate()?,
            parallelism,
            traffic_ready,
            errors: Arc::new(errors),
            dashboard: None,
        })
    }
    /// Attach prepared operator-authorized routes. No dashboard renderers/access
    /// policy or visible labels are manufactured by availability collection.
    pub fn with_dashboard(mut self, dashboard: crate::dashboard::ProtectedDashboard<E, R>) -> Self {
        self.dashboard = Some(dashboard);
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
    pub fn collect(&self, role: ProbeRole) -> Effect<ProbeReport, E, R>
    where
        R: HasClock + HasTaskSpawner,
    {
        let definition = self.clone();
        effect_future! {
                   exit_with_context; body: move |env: Arc<R>, context| {
                   let definition = definition.clone();
                   async move {
                       let mut selected = definition
                           .probes
                           .iter()
                           .filter(|probe| probe.role == role)
                           .cloned()
                           .collect::<Vec<_>>();
                       if role == ProbeRole::Readiness
                           && let Some(traffic) = &definition.traffic_ready
                       {
                           selected.push(Probe {
                               name: Arc::from("traffic readiness"),
                               role,
                               required: true,
                               sample: traffic.clone(),
                           });
                       }
                       let reports = Arc::new(Mutex::new(
                           selected
                               .iter()
                               .map(|probe| SignalReport {
                                   name: probe.name.clone(),
                                   required: probe.required,
                                   state: SignalState::TimedOut,
                               })
                               .collect::<Vec<_>>(),
                       ));
                       let mut programs = Vec::new();
                       for (index, probe) in selected.into_iter().enumerate() {
                           let signal_reports = reports.clone();
                           let name = probe.name.clone();
                           let deadline_name = name.clone();
                           let errors = definition.errors.clone();
                           let program = effect_timeout! {
                               program: effect_map_error! { program: probe.sample, map: ProbeFailure::Expected, },
                               duration: definition.deadlines.per_probe,
                               on_timeout: move || {
                                   ProbeFailure::Deadline(deadline_name.clone())
                               },
                           };
                           programs.push(
                               effect! {
        bind;
        program: effect_map_error! {
            program: program
                                   .materialize_exit(), map: |never| match never {}, },
        body: move |exit| {
                                       let state = match exit {
                                           Exit::Success(true) => SignalState::Available,
                                           Exit::Success(false)
                                           | Exit::Fail(ProbeFailure::Expected(_)) => {
                                               SignalState::Unavailable
                                           }
                                           Exit::Fail(ProbeFailure::Deadline(_)) => SignalState::TimedOut,
                                           Exit::Die(defect) => return effect_die! { defect: defect, },
                                           Exit::Interrupted => {
                                               return crate::restore_exit(Exit::Interrupted);
                                           }
                                           Exit::FinalizationFailed(failure) => {
                                               let errors = errors.clone();
                                               return crate::restore_exit(
                                                   Exit::<(), _>::FinalizationFailed(failure).map_error(
                                                       move |error| match error {
                                                           ProbeFailure::Expected(error) => error,
                                                           ProbeFailure::Deadline(name) => {
                                                               errors(AvailabilityError::ProbeDeadline(
                                                                   name.to_string(),
                                                               ))
                                                           }
                                                       },
                                                   ),
                                               );
                                           }
                                       };
                                       if let Some(report) = signal_reports
                                           .lock()
                                           .unwrap_or_else(std::sync::PoisonError::into_inner)
                                           .get_mut(index)
                                       {
                                           report.state = state;
                                       }
                                       effect_sync! { body { () }, }
                                   },
        },
                           );
                       }
                       let errors = definition.errors.clone();
                       let collection = effect_timeout! {
                           program: effect_map! { program: effect_all_concurrent! { concurrency: definition.parallelism, items: programs, }, map: |_| (), },
                           duration: definition.deadlines.overall,
                           on_timeout: move || {
                               errors(AvailabilityError::CollectionDeadline)
                           },
                       };
                       match context.run(collection, env.clone()).await {
                           Exit::Success(()) | Exit::Fail(_) => {}
                           Exit::Die(defect) => return Exit::Die(defect),
                           Exit::Interrupted => return Exit::Interrupted,
                           Exit::FinalizationFailed(failure) => return Exit::FinalizationFailed(failure),
                       }
                       let signals = reports
                           .lock()
                           .unwrap_or_else(std::sync::PoisonError::into_inner)
                           .clone();
                       let accepting_traffic = signals
                           .iter()
                           .all(|signal| !signal.required || signal.state == SignalState::Available);
                       let degraded = signals
                           .iter()
                           .any(|signal| signal.state != SignalState::Available);
                       Exit::Success(ProbeReport {
                           service: definition.service.clone(),
                           role,
                           accepting_traffic,
                           degraded,
                           signals,
                       })
                   }
               }, }
    }
    /// Native finite reply uses a direct safe JSON writer. Only prepared identities,
    /// roles/states/booleans enter the response; no serde serialization of E occurs.
    pub fn reply(&self, role: ProbeRole) -> Effect<effect_server::Reply<E, R>, E, R>
    where
        R: HasClock + HasTaskSpawner,
    {
        let errors = self.errors.clone();
        effect! {
        bind;
        program: self.collect(role),
        body: move |report| {
                   let mut output = Vec::new();
                   // Writing to Vec cannot fail except allocation; string escaping uses serde's native writer.
                   let write = (|| -> Result<(), serde_json::Error> {
                       output.extend_from_slice(b"{\"service\":");
                       serde_json::to_writer(&mut output, report.service.as_ref())?;
                       output.extend_from_slice(b",\"role\":");
                       serde_json::to_writer(
                           &mut output,
                           match report.role {
                               ProbeRole::Health => "health",
                               ProbeRole::Readiness => "readiness",
                               ProbeRole::Liveness => "liveness",
                           },
                       )?;
                       output.extend_from_slice(b",\"acceptingTraffic\":");
                       output.extend_from_slice(if report.accepting_traffic {
                           b"true"
                       } else {
                           b"false"
                       });
                       output.extend_from_slice(b",\"degraded\":");
                       output.extend_from_slice(if report.degraded { b"true" } else { b"false" });
                       output.extend_from_slice(b",\"signals\":[");
                       let mut first = true;
                       for signal in &report.signals {
                           if !first {
                               output.push(b',');
                           }
                           first = false;
                           output.extend_from_slice(b"{\"name\":");
                           serde_json::to_writer(&mut output, signal.name.as_ref())?;
                           output.extend_from_slice(b",\"required\":");
                           output.extend_from_slice(if signal.required { b"true" } else { b"false" });
                           output.extend_from_slice(b",\"state\":");
                           serde_json::to_writer(
                               &mut output,
                               match signal.state {
                                   SignalState::Available => "available",
                                   SignalState::Unavailable => "unavailable",
                                   SignalState::TimedOut => "timedOut",
                               },
                           )?;
                           output.push(b'}');
                       }
                       output.extend_from_slice(b"]}");
                       Ok(())
                   })();
                   match write {
                       Ok(()) => {
                           let reply = reply! {
                               bytes(output.into(), http::HeaderValue::from_static("application/json")),
                           };
                           let status = match effect_server::BodyStatus::new(if report.accepting_traffic {
                               http::StatusCode::OK
                           } else {
                               http::StatusCode::SERVICE_UNAVAILABLE
                           }) {
                               Ok(status) => status,
                               Err(_) => {
                                   return effect_try_sync! {
                                       once; body: {
                                       let __dsl_result = Err(errors(
                                       AvailabilityError::Output,
                                   )); move || __dsl_result }, on_error: ::core::convert::identity, };
                               }
                           };
                           effect_try_sync! {
                               once; body: {
                               let __dsl_result = Ok(reply! {
                               status(status),
                               header(
                                   http::header::CACHE_CONTROL,
                                   http::HeaderValue::from_static("no-store"),
                               ),
                               source(reply),
                           }); move || __dsl_result }, on_error: ::core::convert::identity, }
                       }
                       Err(_) => effect_try_sync! { once; body: { let __dsl_result = Err(errors(AvailabilityError::Output)); move || __dsl_result }, on_error: ::core::convert::identity, },
                   }
               },
        }
    }
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
