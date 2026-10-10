//! Supported process measurements and host-scoped latest-value diagnostics.
//!
//! POSIX resource usage is read through the application's blocking provider. CPU
//! durations are native process user/system times; peak resident bytes are labelled
//! as peak, not current working-set size. .NET-specific counters remain unavailable.

use effect::{
    AdmissionCapacity, Effect, Exit, HasBlockingExecutor, HasClock, HasTaskSpawner,
    ProviderAdmission, PubSubError, PubSubOverflow, Release, ResourceBinding, Scope, SharedValue,
    Stream, SubscriptionRef,
};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// A supported native measurement or a precisely documented unavailable family.
#[derive(Clone, Debug)]
#[must_use]
pub enum Measurement<A> {
    /// Actual supported process/provider observation with explicit units.
    Available(A),
    /// This process runtime/provider does not expose the named family.
    Unavailable(UnavailableMeasurement),
}
/// Truthful missing-family reasons, never fabricated zero measurements.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub enum UnavailableMeasurement {
    /// This is a .NET GC/thread-pool counter family, absent from native Rust.
    ManagedRuntimeSpecific,
    /// Native process API reports only peak residency, not current working set.
    CurrentWorkingSet,
    /// Selected OS/runtime does not support this POSIX measurement API.
    NativeProvider,
}
/// Finite safe process/provider diagnostic value. Application labels/views remain external.
#[derive(Clone, Debug)]
#[must_use]
pub struct RuntimeSnapshot {
    /// Native wall-clock acquisition instant; no monotonic-to-epoch conversion occurs.
    pub at: SystemTime,
    /// Actual native process identifier.
    pub process_id: u32,
    /// Accumulated native process CPU time, expressed as elapsed duration.
    pub cpu: Measurement<Duration>,
    /// Actual peak native resident bytes, with platform units converted explicitly.
    pub peak_resident_bytes: Measurement<u64>,
    /// Actual current working set if a selected provider supplies it.
    pub working_set_bytes: Measurement<u64>,
    /// Deliberately tracked Effect operations held through nested cleanup.
    pub active_operations: usize,
    /// Managed GC collection counts are explicitly unavailable in native Rust.
    pub gc_collections: Measurement<u64>,
    /// Managed thread-pool worker counters are explicitly unavailable in native Rust.
    pub managed_thread_pool: Measurement<u64>,
}
/// Source/clock/publication failures retain typed ownership without entering public views.
#[must_use]
pub enum DiagnosticsError<E: Send + 'static> {
    /// Application-selected sampler/source failed before an initial snapshot existed.
    Source(E),
    /// Core bounded publication/subscription state failed.
    Subscription(PubSubError),
    /// Native process syscall or unit conversion failed.
    Native(std::io::Error),
    /// Invalid explicit interval/admission configuration.
    Definition,
}
/// Real tracked operation count using the existing core reservation owner.
#[derive(Clone)]
#[must_use]
pub struct OperationTracker {
    /// Core active-operation counter; no second semaphore.
    owner: ProviderAdmission,
}
impl OperationTracker {
    /// Construct the explicit tracked-operation capability; it imposes no admission
    /// quota because this handle measures operations rather than rejecting them.
    pub fn new() -> Self {
        Self {
            owner: ProviderAdmission::new(AdmissionCapacity::Unbounded),
        }
    }
    /// Count operations whose Effect lifetime/cleanup has not settled.
    #[must_use]
    pub fn current(&self) -> usize {
        self.owner.active()
    }
    /// Retain a reservation through the entire program and its ordered cleanup.
    pub fn track<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        program: Effect<A, E, R>,
        on_error: impl Fn(effect::AdmissionError) -> E + Send + Sync + 'static,
    ) -> Effect<A, E, R> {
        let owner = self.owner.clone();
        ResourceBinding::new(
            Effect::try_sync(move || owner.admit(), on_error),
            |permit, _| Release::sync(move || drop(permit)),
        )
        .use_with(move |_| program.clone())
    }
    /// Register tracking in the real request/body scope for long-lived operations.
    pub fn scoped<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        scope: Scope<E>,
        on_error: impl Fn(effect::AdmissionError) -> E + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        let owner = self.owner.clone();
        Effect::try_sync(move || owner.admit(), on_error).flat_map(move |permit| {
            let permit = Arc::new(std::sync::Mutex::new(Some(permit)));
            scope.add_release(move || {
                let permit = permit
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                Release::sync(move || drop(permit))
            })
        })
    }
}
impl Default for OperationTracker {
    /// Construct the same explicit measurement capability.
    fn default() -> Self {
        Self::new()
    }
}
/// Collect actual native process data on R's blocking provider. Timestamp comes
/// from the native observation clock, whose origin is SystemTime, not core Duration.
pub fn process<R: HasBlockingExecutor + Send + Sync + 'static>(
    tracker: OperationTracker,
) -> Effect<RuntimeSnapshot, std::io::Error, R> {
    Effect::try_blocking(move |_| native_process(&tracker), |error| error)
}
/// Native POSIX getrusage observes the current process; unsafe pointers remain
/// confined to initialized libc storage and checked syscall acknowledgment.
#[cfg(unix)]
fn native_process(tracker: &OperationTracker) -> Result<RuntimeSnapshot, std::io::Error> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage writes the complete rusage structure on zero acknowledgment.
    let acknowledged = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if acknowledged != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the successful native call initialized every field read below.
    let usage = unsafe { usage.assume_init() };
    let duration = |value: libc::timeval| -> Result<Duration, std::io::Error> {
        let seconds = u64::try_from(value.tv_sec)
            .map_err(|_| std::io::Error::other("negative CPU seconds"))?;
        let microseconds = u32::try_from(value.tv_usec)
            .map_err(|_| std::io::Error::other("negative CPU microseconds"))?;
        // SI units, not an operational policy: one second has one million microseconds.
        if microseconds >= 1_000_000 {
            return Err(std::io::Error::other("invalid CPU microseconds"));
        }
        Ok(Duration::new(seconds, microseconds * 1_000))
    };
    let cpu = duration(usage.ru_utime)?
        .checked_add(duration(usage.ru_stime)?)
        .ok_or_else(|| std::io::Error::other("CPU duration overflow"))?;
    let resident = u64::try_from(usage.ru_maxrss)
        .map_err(|_| std::io::Error::other("negative resident measurement"))?;
    // Darwin getrusage reports bytes. Linux/BSD ru_maxrss reports kibibytes;
    // explicit 1024 conversion is the native measurement unit, not a quota.
    #[cfg(not(target_os = "macos"))]
    let resident = resident
        .checked_mul(1024)
        .ok_or_else(|| std::io::Error::other("resident byte conversion overflow"))?;
    Ok(RuntimeSnapshot {
        at: SystemTime::now(),
        process_id: std::process::id(),
        cpu: Measurement::Available(cpu),
        peak_resident_bytes: Measurement::Available(resident),
        working_set_bytes: Measurement::Unavailable(UnavailableMeasurement::CurrentWorkingSet),
        active_operations: tracker.current(),
        gc_collections: Measurement::Unavailable(UnavailableMeasurement::ManagedRuntimeSpecific),
        managed_thread_pool: Measurement::Unavailable(
            UnavailableMeasurement::ManagedRuntimeSpecific,
        ),
    })
}
/// Other selected OS providers still expose real identity/time/operation counts;
/// unsupported POSIX fields are honest unavailable values rather than fake zeros.
#[cfg(not(unix))]
fn native_process(tracker: &OperationTracker) -> Result<RuntimeSnapshot, std::io::Error> {
    Ok(RuntimeSnapshot {
        at: SystemTime::now(),
        process_id: std::process::id(),
        cpu: Measurement::Unavailable(UnavailableMeasurement::NativeProvider),
        peak_resident_bytes: Measurement::Unavailable(UnavailableMeasurement::NativeProvider),
        working_set_bytes: Measurement::Unavailable(UnavailableMeasurement::CurrentWorkingSet),
        active_operations: tracker.current(),
        gc_collections: Measurement::Unavailable(UnavailableMeasurement::ManagedRuntimeSpecific),
        managed_thread_pool: Measurement::Unavailable(
            UnavailableMeasurement::ManagedRuntimeSpecific,
        ),
    })
}
/// Last successful finite sample and protected typed source failure.
#[must_use]
pub struct RuntimeStatus<E: Send + 'static> {
    /// Last acknowledged sample/timestamp, retained after refresh failure.
    pub sample: Arc<RuntimeSnapshot>,
    /// Protected source failure, never automatically serialized in public JSON/HTML.
    pub failure: Option<SharedValue<E>>,
}
impl<E: Send + 'static> Clone for RuntimeStatus<E> {
    /// Share sample/error owners without E Clone.
    fn clone(&self) -> Self {
        Self {
            sample: self.sample.clone(),
            failure: self.failure.clone(),
        }
    }
}
/// Host-scoped real sampler and latest-value bounded subscriptions.
#[must_use]
pub struct RuntimeSampler<E: Send + 'static> {
    /// Existing atomic snapshot/subscription owner.
    state: SubscriptionRef<RuntimeStatus<E>>,
    /// Explicit elapsed collection period exposed to application renderers.
    pub interval: Duration,
}
impl<E: Send + 'static> Clone for RuntimeSampler<E> {
    /// Share one worker and bounded hub.
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            interval: self.interval,
        }
    }
}
impl<E: Send + 'static> RuntimeSampler<E> {
    /// Acquire an initial real sample, then own one periodic worker in the supplied
    /// scope. Latest-value eviction is the declared diagnostic subscription contract.
    pub fn scoped<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<DiagnosticsError<E>>,
        interval: Duration,
        subscription_capacity: NonZeroUsize,
        source: impl Fn() -> Effect<RuntimeSnapshot, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, DiagnosticsError<E>, R> {
        if interval.is_zero() {
            return Effect::fail_with(|| DiagnosticsError::Definition);
        }
        let source = Arc::new(source);
        let initial_source = source.clone();
        Effect::defer(move || initial_source())
            .map_error(DiagnosticsError::Source)
            .flat_map(move |sample| {
                let sampler = Self {
                    state: SubscriptionRef::new(
                        RuntimeStatus {
                            sample: Arc::new(sample),
                            failure: None,
                        },
                        subscription_capacity,
                        PubSubOverflow::DropOldest,
                    ),
                    interval,
                };
                let worker_sampler = sampler.clone();
                let worker_source = source.clone();
                let worker = Effect::from_exit_future_with_context(move |env: Arc<R>, context| {
                    let sampler = worker_sampler.clone();
                    let source = worker_source.clone();
                    async move {
                        loop {
                            let sleep = context
                                .run(
                                    Effect::<(), DiagnosticsError<E>, R>::sleep(interval),
                                    env.clone(),
                                )
                                .await;
                            if !matches!(sleep, Exit::Success(())) {
                                return sleep;
                            }
                            let sampled = context.run(source(), env.clone()).await;
                            let status = match sampled {
                                Exit::Success(sample) => RuntimeStatus {
                                    sample: Arc::new(sample),
                                    failure: None,
                                },
                                Exit::Fail(error) => {
                                    let old = match context
                                        .run(sampler.state.read(), env.clone())
                                        .await
                                    {
                                        Exit::Success(old) => old,
                                        failure @ (Exit::Fail(_)
                                        | Exit::Die(_)
                                        | Exit::Interrupted
                                        | Exit::FinalizationFailed(_)) => {
                                            return discard_value(
                                                failure.map_error(DiagnosticsError::Subscription),
                                            );
                                        }
                                    };
                                    RuntimeStatus {
                                        sample: old.sample.clone(),
                                        failure: Some(SharedValue::new(error)),
                                    }
                                }
                                failure @ (Exit::Die(_)
                                | Exit::Interrupted
                                | Exit::FinalizationFailed(_)) => {
                                    return discard_value(
                                        failure.map_error(DiagnosticsError::Source),
                                    );
                                }
                            };
                            let published = context
                                .run(
                                    sampler
                                        .state
                                        .set(status)
                                        .map_error(DiagnosticsError::Subscription),
                                    env.clone(),
                                )
                                .await;
                            if !matches!(published, Exit::Success(_)) {
                                return discard_value(published);
                            }
                        }
                    }
                });
                let worker_scope = scope.clone();
                scope
                    .add_finalizer(
                        sampler
                            .state
                            .shutdown(effect::PubSubShutdown::Discard)
                            .map_error(DiagnosticsError::Subscription)
                            .map(|_| ()),
                    )
                    .flat_map(move |()| {
                        let sampler = sampler.clone();
                        worker_scope
                            .fork(worker.clone())
                            .map(move |_| sampler.clone())
                    })
            })
    }
    /// Read the same latest immutable status used by initial views and subscriptions.
    pub fn status<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<Arc<RuntimeStatus<E>>, DiagnosticsError<E>, R> {
        self.state.read().map_error(DiagnosticsError::Subscription)
    }
    /// Existing core stream pins an atomic current sample, then bounded changes.
    /// Lag/backpressure/termination follow the selected core subscription owner.
    #[must_use]
    pub fn changes<R: Send + Sync + 'static>(
        &self,
    ) -> Stream<Arc<RuntimeStatus<E>>, DiagnosticsError<E>, R> {
        self.changes_with(|error| error)
    }
    /// Map publication errors at the explicit domain boundary while preserving the
    /// same core atomic snapshot/subscription resource and native request scope.
    pub fn changes_with<F: Send + 'static, R: Send + Sync + 'static>(
        &self,
        inject: impl Fn(DiagnosticsError<E>) -> F + Send + Sync + 'static,
    ) -> Stream<Arc<RuntimeStatus<E>>, F, R> {
        let inject = Arc::new(inject);
        let acquire_inject = inject.clone();
        let pull_inject = inject.clone();
        let release_inject = inject;
        Stream::from_pull(
            self.state
                .subscribe()
                .map_error(move |error| acquire_inject(DiagnosticsError::Subscription(error)))
                .map(|snapshot| Cursor {
                    initial: std::sync::Mutex::new(Some(snapshot.value)),
                    subscription: snapshot.subscription,
                }),
            move |resource| {
                let inject = pull_inject.clone();
                match resource.with(|cursor| {
                    match cursor
                        .initial
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                    {
                        Some(initial) => {
                            Effect::from_result_once(Ok(effect::Chunk::Values(vec![initial])))
                        }
                        None => cursor
                            .subscription
                            .pull_chunk()
                            .map_error(move |error| inject(DiagnosticsError::Subscription(error))),
                    }
                }) {
                    Ok(pull) => pull,
                    Err(defect) => Effect::die(defect),
                }
            },
            move |cursor| {
                let inject = release_inject.clone();
                Release::from_effect(
                    cursor
                        .subscription
                        .unsubscribe()
                        .map_error(move |error| inject(DiagnosticsError::Subscription(error)))
                        .map(|_| ()),
                )
            },
        )
    }
}

/// Discard a settled success value while retaining every owned error, defect,
/// interruption and ordered finalization failure from the original worker outcome.
fn discard_value<A, E>(exit: Exit<A, E>) -> Exit<(), E> {
    match exit {
        Exit::Success(_) => Exit::Success(()),
        Exit::Fail(error) => Exit::Fail(error),
        Exit::Die(defect) => Exit::Die(defect),
        Exit::Interrupted => Exit::Interrupted,
        Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
    }
}

/// One owned subscription cursor with its atomic initial snapshot. The existing
/// core hub supplies later chunks, lag errors, capacity and close acknowledgment.
struct Cursor<E: Send + 'static> {
    /// Initial current value consumed exactly once before subsequent publications.
    initial: std::sync::Mutex<Option<Arc<RuntimeStatus<E>>>>,
    /// Core bounded subscription retained by the request/body scope.
    subscription: effect::Subscription<RuntimeStatus<E>>,
}
