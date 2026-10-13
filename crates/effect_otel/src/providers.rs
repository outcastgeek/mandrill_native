//! SDK ownership and joined shutdown using core scopes and selected blocking work.

use crate::NativeError;
use crate::{
    effect_future, effect_map_error, effect_scope_acquire, effect_try_blocking,
    effect_uninterruptible,
};
use effect::{Deferred, Effect, Exit, HasBlockingExecutor, ResourceValue, Scope};
use opentelemetry_sdk::logs::{LoggerProviderBuilder, SdkLoggerProvider};
use opentelemetry_sdk::metrics::{MeterProviderBuilder, SdkMeterProvider};
use opentelemetry_sdk::trace::{SdkTracerProvider, TracerProviderBuilder};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Native signal identity used by lifecycle diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Official trace SDK and its processors/exporters.
    Traces,
    /// Official meter SDK and its readers/exporters.
    Metrics,
    /// Official logging SDK and its processors/exporters.
    Logs,
}

/// The exact lifecycle operation that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecyclePhase {
    /// Native builder/configuration/acquisition failed.
    Acquire,
    /// Force-flush failed before provider shutdown.
    Flush,
    /// Native provider shutdown failed.
    Shutdown,
}

/// Original typed cause; SDK failures retain their native error variants.
#[derive(Clone, Debug)]
pub enum LifecycleCause {
    /// Official SDK error shared without flattening to absence/success.
    Sdk(Arc<opentelemetry_sdk::error::OTelSdkError>),
    /// Checked native configuration callback failure or caught panic.
    Native(NativeError),
    /// Caller-supplied shutdown budget elapsed before this native operation.
    BudgetExpired,
}

/// One primary/acquisition/flush/shutdown failure in observation order.
#[derive(Clone, Debug)]
pub struct LifecycleFailure {
    /// Provider whose acknowledgment failed.
    pub signal: Signal,
    /// The actual failing lifecycle boundary.
    pub phase: LifecyclePhase,
    /// Retained native cause, without application payload serialization.
    pub cause: LifecycleCause,
}

/// Ordered nonempty lifecycle failures shared by all concurrent close observers.
#[derive(Clone, Debug)]
pub struct LifecycleError {
    /// First failure is the primary native failure, followed by cleanup failures.
    failures: Arc<[LifecycleFailure]>,
}
impl LifecycleError {
    /// Borrow the ordered failures; shared errors copy no business payload.
    #[must_use]
    pub fn failures(&self) -> &[LifecycleFailure] {
        &self.failures
    }
    /// Construct only after a failing native boundary has produced evidence.
    fn from_failures(failures: Vec<LifecycleFailure>) -> Self {
        Self {
            failures: failures.into(),
        }
    }
}
impl std::fmt::Display for LifecycleError {
    /// Summarize phases/count without dumping provider configuration or attributes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(first) = self.failures.first() {
            write!(
                f,
                "telemetry lifecycle failed at {:?}/{:?}; {} retained failures",
                first.signal,
                first.phase,
                self.failures.len()
            )
        } else {
            f.write_str("telemetry lifecycle failure record is empty")
        }
    }
}
impl std::error::Error for LifecycleError {}

/// Explicit SDK shutdown policy with a caller-supplied total time budget.
#[derive(Clone, Copy, Debug)]
pub struct ShutdownPolicy {
    /// Remaining budget is passed into each official `shutdown_with_timeout`.
    budget: Duration,
    /// Caller chooses whether force-flush is attempted before shutdown.
    flush: bool,
}
impl ShutdownPolicy {
    /// Require a positive native shutdown budget; no arbitrary library timeout.
    ///
    /// The SDK's `force_flush` API has no timeout argument, and SDK 0.32.1's meter
    /// provider ignores its shutdown timeout. The caller must select processors/
    /// readers with bounded behavior. Each shutdown receives the remaining budget
    /// (zero after expiry) and release is attempted for every acquired signal.
    /// It cannot forcibly interrupt a running synchronous SDK callback.
    pub fn new(budget: Duration, flush_before_shutdown: bool) -> Result<Self, NativeError> {
        if budget.is_zero() {
            return Err(NativeError::Native(effect::Defect::new(
                "SDK shutdown budget must be positive",
            )));
        }
        Ok(Self {
            budget,
            flush: flush_before_shutdown,
        })
    }
    /// Borrow the shutdown budget in duration units, with no epoch interpretation.
    #[must_use]
    pub fn budget(&self) -> Duration {
        self.budget
    }
}

/// Retained host-owned SDK handles; no extension shutdown occurs on this value.
///
/// The host must retain its original SDK owners until all borrowed work settles.
/// The SDK itself shuts down on its last owner drop. Cloning this adapter merely
/// shares provider handles; it is not acquisition or a new application runtime.
#[derive(Clone)]
pub struct BorrowedProviders {
    /// Native SDK trace handle; no exporter is constructed here.
    traces: SdkTracerProvider,
    /// Native SDK meter handle and its selected reader integration.
    metrics: SdkMeterProvider,
    /// Native SDK logging handle used by the official tracing appender.
    logs: SdkLoggerProvider,
    /// Owned-mode admission closes atomically before shutdown begins.
    open: Option<Arc<AtomicBool>>,
    /// Core admission joins in-flight checked SDK operations before owned shutdown.
    admission: effect::ProviderAdmission,
}
impl BorrowedProviders {
    /// Borrow providers by sharing their handles; retain the originals in the host.
    #[must_use]
    pub fn new(
        traces: &SdkTracerProvider,
        metrics: &SdkMeterProvider,
        logs: &SdkLoggerProvider,
    ) -> Self {
        Self {
            traces: traces.clone(),
            metrics: metrics.clone(),
            logs: logs.clone(),
            open: None,
            admission: effect::ProviderAdmission::new(effect::AdmissionCapacity::Unbounded),
        }
    }
    /// Reject new extension native operations once owned shutdown starts.
    /// Host-borrowed shutdown remains the host's responsibility.
    pub fn ensure_open(&self) -> Result<(), NativeError> {
        if self
            .open
            .as_ref()
            .is_some_and(|open| !open.load(Ordering::Acquire))
        {
            Err(NativeError::ProviderClosed)
        } else {
            Ok(())
        }
    }
    /// Reserve a native invocation atomically with owned close admission.
    /// The core permit is retained until synchronous callback/export observation ends.
    pub(crate) fn admit(&self) -> Result<effect::AdmissionPermit, NativeError> {
        self.ensure_open()?;
        self.admission.admit().map_err(|error| match error {
            effect::AdmissionError::Closed => NativeError::ProviderClosed,
            effect::AdmissionError::Full | effect::AdmissionError::Overflow => {
                NativeError::Native(effect::Defect::from(error))
            }
        })
    }
    /// Borrow native trace configuration; caller explicitly owns native escape use.
    #[must_use]
    pub fn tracer(&self) -> &SdkTracerProvider {
        &self.traces
    }
    /// Borrow native meter/provider for observable instruments and custom scopes.
    #[must_use]
    pub fn meter(&self) -> &SdkMeterProvider {
        &self.metrics
    }
    /// Borrow native log provider for official appender/subscriber composition.
    #[must_use]
    pub fn logger(&self) -> &SdkLoggerProvider {
        &self.logs
    }

    /// Check and force-flush host-owned signals without claiming shutdown ownership.
    /// All three signals are attempted and typed failures remain ordered.
    pub fn flush<R>(&self) -> Effect<(), LifecycleError, R>
    where
        R: HasBlockingExecutor + Send + Sync + 'static,
    {
        let providers = self.clone();
        effect_try_blocking! {
        body: move |_| {
            let _permit = providers.admit().map_err(|error| {
                LifecycleError::from_failures(vec![LifecycleFailure {
                    signal: Signal::Traces,
                    phase: LifecyclePhase::Flush,
                    cause: LifecycleCause::Native(error),
                }])
            })?;
            let mut failures = Vec::new();
            flush_all(&providers, &mut failures);
            result(failures)
        }, on_error: std::convert::identity, }
    }
}

/// Native factory callbacks are reusable configuration, not reusable live owners.
#[derive(Clone)]
pub struct ProviderFactories {
    /// Caller configures resources/sampling/processors/exporters on the official builder.
    trace:
        Arc<dyn Fn(TracerProviderBuilder) -> Result<SdkTracerProvider, NativeError> + Send + Sync>,
    /// Caller selects native readers, aggregation/views and exporter runtime contract.
    meter: Arc<dyn Fn(MeterProviderBuilder) -> Result<SdkMeterProvider, NativeError> + Send + Sync>,
    /// Caller selects log processors/exporters without any implicit collector.
    logger:
        Arc<dyn Fn(LoggerProviderBuilder) -> Result<SdkLoggerProvider, NativeError> + Send + Sync>,
}
impl ProviderFactories {
    /// Supply all native SDK hooks; factories execute only on Effect acquisition.
    #[must_use]
    pub fn new(
        trace: impl Fn(TracerProviderBuilder) -> Result<SdkTracerProvider, NativeError>
        + Send
        + Sync
        + 'static,
        meter: impl Fn(MeterProviderBuilder) -> Result<SdkMeterProvider, NativeError>
        + Send
        + Sync
        + 'static,
        logger: impl Fn(LoggerProviderBuilder) -> Result<SdkLoggerProvider, NativeError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            trace: Arc::new(trace),
            meter: Arc::new(meter),
            logger: Arc::new(logger),
        }
    }
}

/// One owned SDK acquisition with one canonical close result.
#[derive(Clone)]
pub struct OwnedProviders {
    /// Complete retained set; access checks its owned admission flag.
    providers: BorrowedProviders,
    /// Core deferred joins concurrent close without a new lifecycle interpreter.
    closing: Arc<Mutex<Option<Deferred<(), LifecycleError>>>>,
    /// Every observer joins the policy chosen at acquisition, not its own timeout.
    policy: ShutdownPolicy,
}
impl OwnedProviders {
    /// Acquire fresh providers on the blocking capability selected by `R`.
    ///
    /// Partial acquisition is already owned before the next factory runs. Failure
    /// explicitly shuts down acquired providers and retains primary plus cleanup
    /// errors. Callers of this low-level entry must arrange `close` through a scope.
    pub fn start<R>(
        factories: ProviderFactories,
        policy: ShutdownPolicy,
    ) -> Effect<Self, LifecycleError, R>
    where
        R: HasBlockingExecutor + Send + Sync + 'static,
    {
        effect_uninterruptible! { program: effect_try_blocking! { body: move |_| build(&factories, policy), on_error: std::convert::identity, }, }
    }

    /// Acquire into an existing core scope; release is registered before acquisition.
    /// Body success, error, defect and interruption retain ordered shutdown failure.
    pub fn acquire<R>(
        scope: Scope<LifecycleError>,
        factories: ProviderFactories,
        policy: ShutdownPolicy,
    ) -> Effect<ResourceValue<Self>, LifecycleError, R>
    where
        R: HasBlockingExecutor + Send + Sync + 'static,
    {
        effect_scope_acquire! {
        scope: scope, acquire: Self::start(factories, policy), release: |providers| {
                    providers.close()
                },
        }
    }

    /// Acquire in an application's existing typed scope with explicit error mapping.
    /// Both acquisition and ordered release failures use the same selected mapping;
    /// native lifecycle causes remain inspectable before the application wraps them.
    pub fn acquire_with_error<E, R>(
        scope: Scope<E>,
        factories: ProviderFactories,
        policy: ShutdownPolicy,
        on_error: impl Fn(LifecycleError) -> E + Send + Sync + 'static,
    ) -> Effect<ResourceValue<Self>, E, R>
    where
        E: Send + 'static,
        R: HasBlockingExecutor + Send + Sync + 'static,
    {
        let on_error = Arc::new(on_error);
        let acquire_error = Arc::clone(&on_error);
        let acquisition = effect_map_error! { program: Self::start(factories, policy), map: move |error| acquire_error(error), };
        effect_scope_acquire! {
        scope: scope, acquire: acquisition, release: move |providers| {
                    let on_error = Arc::clone(&on_error);
                    effect_map_error! { program: providers.close(), map: move |error| on_error(error), }
                },
        }
    }

    /// Share checked native handles with a scoped application environment.
    /// These handles do not transfer the shutdown owner; close revokes admission.
    #[must_use]
    pub fn borrowed(&self) -> BorrowedProviders {
        self.providers.clone()
    }

    /// Close exactly once and join the canonical complete Effect exit concurrently.
    ///
    /// Protected blocking cleanup runs through `R`. A physically abandoned driver
    /// does not acknowledge shutdown: retain the Effect scope/run owner through
    /// settlement. Only the small lifecycle error record is shared/copied when
    /// observers reconstitute its typed exit; application payloads are untouched.
    pub fn close<R>(&self) -> Effect<(), LifecycleError, R>
    where
        R: HasBlockingExecutor + Send + Sync + 'static,
    {
        let owned = self.clone();
        effect_uninterruptible! {
            program: effect_future! {
            exit_with_context; body: move |env: Arc<R>, context| {
            let owned = owned.clone();
            async move {
                let (completion, first) = {
                    let mut closing = owned
                        .closing
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    match closing.as_ref() {
                        Some(completion) => (completion.clone(), false),
                        None => {
                            let completion = Deferred::new();
                            *closing = Some(completion.clone());
                            if let Some(open) = &owned.providers.open {
                                open.store(false, Ordering::Release);
                            }
                            (completion, true)
                        }
                    }
                };
                if first {
                    owned.providers.admission.stop().wait().await;
                    let providers = owned.providers.clone();
                    let policy = owned.policy;
                    let close = effect_try_blocking! {
                        body: move |_| {
                            let mut failures = Vec::new();
                            let started = Instant::now();
                            if policy.flush {
                                flush_all(&providers, &mut failures);
                            }
                            shutdown_all(&providers, started, policy, &mut failures);
                            result(failures)
                        }, on_error: std::convert::identity, };
                    let outcome = context.run(close, Arc::clone(&env)).await;
                    // Publish even a provider-admission defect so every waiter joins it.
                    let published = context
                        .run(completion.complete(outcome), Arc::clone(&env))
                        .await;
                    if let Exit::Die(defect) = published {
                        return Exit::Die(defect);
                    }
                }
                match context.run(completion.await_shared(), env).await {
                    Exit::Success(shared) => shared.with(Clone::clone).unwrap_or_else(Exit::Die),
                    Exit::Fail(error) => Exit::Fail(error),
                    Exit::Die(defect) => Exit::Die(defect),
                    Exit::Interrupted => Exit::Interrupted,
                    Exit::FinalizationFailed(errors) => Exit::FinalizationFailed(errors),
                }
            }
        }, }, }
    }
}

/// Catch configuration panics at acquisition without losing existing owned handles.
fn factory<T>(
    signal: Signal,
    action: impl FnOnce() -> Result<T, NativeError>,
) -> Result<T, LifecycleFailure> {
    let value = catch_unwind(AssertUnwindSafe(action))
        .unwrap_or_else(|panic| Err(NativeError::Native(effect::Defect::from_panic(panic))));
    value.map_err(|error| LifecycleFailure {
        signal,
        phase: LifecyclePhase::Acquire,
        cause: LifecycleCause::Native(error),
    })
}

/// Incremental construction with explicit partial rollback and no collector defaults.
fn build(
    factories: &ProviderFactories,
    policy: ShutdownPolicy,
) -> Result<OwnedProviders, LifecycleError> {
    let traces = factory(Signal::Traces, || {
        (factories.trace)(SdkTracerProvider::builder())
    })
    .map_err(|failure| LifecycleError::from_failures(vec![failure]))?;
    let metrics = match factory(Signal::Metrics, || {
        (factories.meter)(SdkMeterProvider::builder())
    }) {
        Ok(metrics) => metrics,
        Err(failure) => {
            let mut failures = vec![failure];
            sdk_call(
                Signal::Traces,
                LifecyclePhase::Shutdown,
                &mut failures,
                || traces.shutdown_with_timeout(policy.budget),
            );
            return Err(LifecycleError::from_failures(failures));
        }
    };
    let logs = match factory(Signal::Logs, || {
        (factories.logger)(SdkLoggerProvider::builder())
    }) {
        Ok(logs) => logs,
        Err(failure) => {
            let mut failures = vec![failure];
            let started = Instant::now();
            shutdown_one(Signal::Traces, started, policy, &mut failures, |budget| {
                traces.shutdown_with_timeout(budget)
            });
            shutdown_one(Signal::Metrics, started, policy, &mut failures, |budget| {
                metrics.shutdown_with_timeout(budget)
            });
            return Err(LifecycleError::from_failures(failures));
        }
    };
    Ok(OwnedProviders {
        providers: BorrowedProviders {
            traces,
            metrics,
            logs,
            open: Some(Arc::new(AtomicBool::new(true))),
            admission: effect::ProviderAdmission::new(effect::AdmissionCapacity::Unbounded),
        },
        closing: Arc::new(Mutex::new(None)),
        policy,
    })
}

/// Attempt every native flush so a first failure never suppresses later signals.
fn flush_all(providers: &BorrowedProviders, failures: &mut Vec<LifecycleFailure>) {
    sdk_call(Signal::Logs, LifecyclePhase::Flush, failures, || {
        providers.logs.force_flush()
    });
    sdk_call(Signal::Traces, LifecyclePhase::Flush, failures, || {
        providers.traces.force_flush()
    });
    sdk_call(Signal::Metrics, LifecyclePhase::Flush, failures, || {
        providers.metrics.force_flush()
    });
}
/// Release all provider owners in declared log/trace/meter order.
fn shutdown_all(
    providers: &BorrowedProviders,
    started: Instant,
    policy: ShutdownPolicy,
    failures: &mut Vec<LifecycleFailure>,
) {
    shutdown_one(Signal::Logs, started, policy, failures, |budget| {
        providers.logs.shutdown_with_timeout(budget)
    });
    shutdown_one(Signal::Traces, started, policy, failures, |budget| {
        providers.traces.shutdown_with_timeout(budget)
    });
    shutdown_one(Signal::Metrics, started, policy, failures, |budget| {
        providers.metrics.shutdown_with_timeout(budget)
    });
}
/// Pass the remaining duration and attempt release even after the budget expires.
fn shutdown_one(
    signal: Signal,
    started: Instant,
    policy: ShutdownPolicy,
    failures: &mut Vec<LifecycleFailure>,
    action: impl FnOnce(Duration) -> opentelemetry_sdk::error::OTelSdkResult,
) {
    let remaining = policy.budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        failures.push(LifecycleFailure {
            signal,
            phase: LifecyclePhase::Shutdown,
            cause: LifecycleCause::BudgetExpired,
        });
    }
    sdk_call(signal, LifecyclePhase::Shutdown, failures, || {
        action(remaining)
    });
}
/// SDK errors and callback panics remain typed, ordered, and independently retained.
fn sdk_call(
    signal: Signal,
    phase: LifecyclePhase,
    failures: &mut Vec<LifecycleFailure>,
    action: impl FnOnce() -> opentelemetry_sdk::error::OTelSdkResult,
) {
    let cause = match catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(())) => return,
        Ok(Err(error)) => LifecycleCause::Sdk(Arc::new(error)),
        Err(panic) => {
            LifecycleCause::Native(NativeError::Native(effect::Defect::from_panic(panic)))
        }
    };
    failures.push(LifecycleFailure {
        signal,
        phase,
        cause,
    });
}
/// Complete success or preserve all actual failures as one structured error.
fn result(failures: Vec<LifecycleFailure>) -> Result<(), LifecycleError> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(LifecycleError::from_failures(failures))
    }
}
