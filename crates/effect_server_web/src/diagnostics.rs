//! Supported process measurements and host-scoped latest-value diagnostics.
//!
//! POSIX resource usage is read through the application's blocking provider. CPU
//! durations are native process user/system times; peak resident bytes are labelled
//! as peak, not current working-set size. .NET-specific counters remain unavailable.

use crate::{
    effect_die, effect_finalizer, effect_future, effect_map, effect_map_error, effect_resource,
    effect_try_blocking, effect_try_sync, effect_using,
};
use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool};
use effect::stream;
use effect::{
    AdmissionCapacity, Effect, Exit, HasBlockingExecutor, HasClock, HasTaskSpawner,
    PreparedSharedValuePool, PreparedWatchPool, PreparedWatchReceiver, ProviderAdmission,
    PubSubError, PubSubOverflow, Scope, SharedValue, Stream, SubscriptionRef, effect,
};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
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
    /// Selected operational workspace could not retain a diagnostic version or subscriber.
    Memory(MemoryError),
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
        effect_using! {
            acquire: effect_try_sync! { body { owner.admit() }, on_error: on_error, },
            body: move |_permit| program.clone(),
        }
    }
    /// Register tracking in the real request/body scope for long-lived operations.
    pub fn scoped<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        scope: Scope<E>,
        on_error: impl Fn(effect::AdmissionError) -> E + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        let owner = self.owner.clone();
        effect! {
        bind;
        program: effect_try_sync! { body: move || owner.admit(), on_error: on_error, },
        body: move |permit| {
                   let permit = Arc::new(std::sync::Mutex::new(Some(permit)));
                   effect_finalizer! {
                       release;
                       scope: scope,
                       release: move || {
                       let permit = permit
                           .lock()
                           .unwrap_or_else(std::sync::PoisonError::into_inner)
                           .take();
                           effect_resource! { release_sync; body: move || drop(permit) }
                       },
                   }
               },
        }
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
    effect_try_blocking! { body: move |_| native_process(&tracker), on_error: |error| error, }
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
struct PreparedRuntimeVersion<E: Send + 'static> {
    sample: RuntimeSnapshot,
    failure: Option<SharedValue<E>>,
}
type RuntimeReceiver<E> = PreparedWatchReceiver<Option<PreparedRuntimeVersion<E>>>;

/// Independent latest-version and observer capacity for runtime diagnostics.
pub struct PreparedDiagnostics<E: Send + 'static> {
    watch: PreparedWatchPool<Option<PreparedRuntimeVersion<E>>>,
    errors: PreparedSharedValuePool<E>,
    cursors: SlotPool<'static, CursorSlot<E>>,
}

impl<E: Send + 'static> PreparedDiagnostics<E> {
    /// Region layout for a host sampler, including one internal reader.
    pub fn storage_bytes(
        subscribers: NonZeroUsize,
        versions: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        if versions.get() < 2 {
            return Err(MemoryError::NoSlot);
        }
        let listeners = subscribers
            .get()
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        PreparedWatchPool::<Option<PreparedRuntimeVersion<E>>>::storage_bytes(
            1,
            listeners,
            versions.get(),
        )?
        .checked_add(PreparedSharedValuePool::<E>::storage_bytes(versions.get())?)?
        .checked_add(SlotPool::<CursorSlot<E>>::storage_bytes(subscribers.get())?)
    }

    /// Provision version and observer banks before invoking the native source.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        subscribers: NonZeroUsize,
        versions: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(subscribers, versions)?;
        let listeners = subscribers
            .get()
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok(Self {
            watch: PreparedWatchPool::prepare(memory, region, 1, listeners, versions.get())?,
            errors: PreparedSharedValuePool::prepare(memory, region, versions.get())?,
            cursors: memory.slots(region, subscribers.get(), |_| Ok(Mutex::new(None)))?,
        })
    }
}

enum RuntimeSource<E: Send + 'static> {
    Ordinary(SubscriptionRef<RuntimeStatus<E>>),
    Prepared {
        receiver: Arc<Mutex<RuntimeReceiver<E>>>,
        storage: Arc<PreparedDiagnostics<E>>,
    },
}
impl<E: Send + 'static> Clone for RuntimeSource<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(state) => Self::Ordinary(state.clone()),
            Self::Prepared { receiver, storage } => Self::Prepared {
                receiver: receiver.clone(),
                storage: storage.clone(),
            },
        }
    }
}

/// Host-scoped real sampler and latest-value bounded subscriptions.
#[must_use]
pub struct RuntimeSampler<E: Send + 'static> {
    /// Existing atomic snapshot/subscription owner.
    state: RuntimeSource<E>,
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
            return effect_try_sync! { body: { let __dsl_error = || DiagnosticsError::Definition; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
        }
        let source = Arc::new(source);
        effect! {
                    let source = source.clone();
                    let scope = scope.clone();
                    let initial_source = source.clone();
                    let sample = effect_map_error! { program: { let __dsl_factory = move || initial_source(); effect! { __dsl_factory()? } }, map: DiagnosticsError::Source, }?;
                    let state = SubscriptionRef::new(
                        RuntimeStatus {
                            sample: Arc::new(sample),
                            failure: None,
                        },
                        subscription_capacity,
                        PubSubOverflow::DropOldest,
                    );
                    let sampler = Self { state: RuntimeSource::Ordinary(state.clone()), interval };
                    let worker_state = state.clone();
                    let worker_source = source.clone();
                    let worker = effect_future! {
                        exit_with_context; body: move |env: Arc<R>, context| {
                        let state = worker_state.clone();
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
                                            .run(state.read(), env.clone())
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
                                        effect_map_error! {
                                            program: state.set(status), map: DiagnosticsError::Subscription, },
                                        env.clone(),
                                    )
                                    .await;
                                if !matches!(published, Exit::Success(_)) {
                                    return discard_value(published);
                                }
                            }
                        }
                    }, };
                    effect_finalizer! {
        scope: scope, body: effect_map! { program: effect_map_error! { program: state.shutdown(effect::PubSubShutdown::Discard), map: DiagnosticsError::Subscription, }, map: |_| (), },
        }?;
                    scope.fork(worker)?;
                    sampler
                }
    }
    /// Acquire a latest-value sampler with independent physical publication slots.
    pub fn scoped_prepared<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<DiagnosticsError<E>>,
        interval: Duration,
        storage: Arc<PreparedDiagnostics<E>>,
        source: impl Fn() -> Effect<RuntimeSnapshot, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, DiagnosticsError<E>, R> {
        if interval.is_zero() {
            return effect_try_sync! { body: || Err(DiagnosticsError::Definition), on_error: ::core::convert::identity, };
        }
        let source = Arc::new(source);
        effect_future! {
            exit_with_context; body: move |env: Arc<R>, context| {
                let scope = scope.clone();
                let storage = storage.clone();
                let source = source.clone();
                async move {
                    let (sender, receiver) = match storage.watch.try_watch(None) {
                        Ok(state) => state,
                        Err((error, _)) => return Exit::Fail(DiagnosticsError::Memory(error)),
                    };
                    let publication = match sender.reserve_publication() {
                        Ok(publication) => publication,
                        Err(error) => return Exit::Fail(DiagnosticsError::Memory(error)),
                    };
                    let sample = match crate::split_exit(context.run(source(), env.clone()).await) {
                        Ok(sample) => sample,
                        Err(failure) => return failure.map_error(DiagnosticsError::Source),
                    };
                    publication.publish(Some(PreparedRuntimeVersion { sample: sample.clone(), failure: None }));
                    let sender = Arc::new(sender);
                    let worker_storage = storage.clone();
                    let worker = effect_future! {
                        exit_with_context; body: move |env: Arc<R>, context| {
                            let sender = sender.clone();
                            let storage = worker_storage.clone();
                            let source = source.clone();
                            let mut sample = sample.clone();
                            async move {
                                loop {
                                    let sleep = context.run(Effect::<(), DiagnosticsError<E>, R>::sleep(interval), env.clone()).await;
                                    if !matches!(sleep, Exit::Success(())) {
                                        return sleep;
                                    }
                                    let publication = match sender.reserve_publication() {
                                        Ok(publication) => publication,
                                        Err(error) => return Exit::Fail(DiagnosticsError::Memory(error)),
                                    };
                                    let shared_error = match storage.errors.reserve() {
                                        Ok(shared_error) => shared_error,
                                        Err(error) => return Exit::Fail(DiagnosticsError::Memory(error)),
                                    };
                                    let failure = match context.run(source(), env.clone()).await {
                                        Exit::Success(next) => { sample = next; None },
                                        Exit::Fail(error) => Some(shared_error.publish(error)),
                                        failure @ (Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
                                            return discard_value(failure.map_error(DiagnosticsError::Source));
                                        }
                                    };
                                    publication.publish(Some(PreparedRuntimeVersion { sample: sample.clone(), failure }));
                                }
                            }
                        },
                    };
                    match crate::split_exit(context.run(scope.fork(worker), env).await) {
                        Ok(_) => Exit::Success(Self { state: RuntimeSource::Prepared { receiver: Arc::new(Mutex::new(receiver)), storage }, interval }),
                        Err(failure) => failure,
                    }
                }
            },
        }
    }

    /// Read the same latest immutable status used by initial views and subscriptions.
    pub fn status<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<Arc<RuntimeStatus<E>>, DiagnosticsError<E>, R> {
        match &self.state {
            RuntimeSource::Ordinary(state) => {
                effect_map_error! { program: state.read(), map: DiagnosticsError::Subscription, }
            }
            RuntimeSource::Prepared { receiver, .. } => {
                let state = receiver.clone();
                effect_try_sync! {
                    body: move || {
                        let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        latest(&mut state)
                    },
                    on_error: DiagnosticsError::Memory,
                }
            }
        }
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
        let state = match &self.state {
            RuntimeSource::Ordinary(state) => state,
            RuntimeSource::Prepared { receiver, storage } => {
                return prepared_changes(receiver.clone(), storage.clone(), Arc::new(inject));
            }
        };
        let inject = Arc::new(inject);
        let acquire_inject = inject.clone();
        let pull_inject = inject.clone();
        let release_inject = inject;
        stream! {
            pull: effect_map! {
                program: effect_map_error! {
                program: state.subscribe(), map: move |error| acquire_inject(DiagnosticsError::Subscription(error)), }, map: |snapshot| Cursor {
                    initial: std::sync::Mutex::new(Some(snapshot.value)),
                    subscription: snapshot.subscription,
                }, },
            read: move |resource| {
                let inject = pull_inject.clone();
                match resource.with(|cursor| {
                    match cursor
                        .initial
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                    {
                        Some(initial) => {
                            effect_try_sync! { once; body: { let __dsl_result = Ok(effect::Chunk::Values(vec![initial])); move || __dsl_result }, on_error: ::core::convert::identity, }
                        }
                        None => effect_map_error! {
                            program: cursor
                            .subscription
                            .pull_chunk(), map: move |error| inject(DiagnosticsError::Subscription(error)), },
                    }
                }) {
                    Ok(pull) => pull,
                    Err(defect) => effect_die! { defect: defect, },
                }
            },
            release: move |cursor| {
                let inject = release_inject.clone();
                effect_map! {
                    program: effect_map_error! {
                    program: cursor
                        .subscription
                        .unsubscribe(), map: move |error| inject(DiagnosticsError::Subscription(error)), }, map: |_| (), }
            },
        }
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

fn latest<E: Send + 'static>(
    receiver: &mut RuntimeReceiver<E>,
) -> Result<Arc<RuntimeStatus<E>>, MemoryError> {
    let view = receiver.borrow_and_update()?.ok_or(MemoryError::Closed)?;
    let value = view.borrow().ok_or(MemoryError::Closed)?;
    let version = value
        .get()
        .and_then(Option::as_ref)
        .ok_or(MemoryError::Closed)?;
    Ok(Arc::new(RuntimeStatus {
        sample: Arc::new(version.sample.clone()),
        failure: version.failure.clone(),
    }))
}

struct PreparedCursor<E: Send + 'static> {
    first: bool,
    receiver: RuntimeReceiver<E>,
}

type CursorSlot<E> = Mutex<Option<PreparedCursor<E>>>;

struct CursorOwner<E: Send + 'static>(Option<SharedSlot<'static, CursorSlot<E>>>);
impl<E: Send + 'static> Clone for CursorOwner<E> {
    #[allow(clippy::expect_used)] // A live cursor retains its slot; only the owner-count bound can fail.
    fn clone(&self) -> Self {
        Self(Some(
            self.shared()
                .try_clone()
                .expect("diagnostic cursor owner limit"),
        ))
    }
}
impl<E: Send + 'static> CursorOwner<E> {
    #[allow(clippy::expect_used)] // Retirement consumes the live owner.
    fn shared(&self) -> &SharedSlot<'static, CursorSlot<E>> {
        self.0.as_ref().expect("live diagnostic cursor")
    }
}
impl<E: Send + 'static> Drop for CursorOwner<E> {
    fn drop(&mut self) {
        if let Some(mut slot) = self.0.take().and_then(SharedSlot::into_last) {
            drop(
                slot.get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take(),
            );
        }
    }
}

fn prepared_changes<E: Send + 'static, F: Send + 'static, R: Send + Sync + 'static>(
    root: Arc<Mutex<RuntimeReceiver<E>>>,
    storage: Arc<PreparedDiagnostics<E>>,
    inject: Arc<impl Fn(DiagnosticsError<E>) -> F + Send + Sync + 'static>,
) -> Stream<Arc<RuntimeStatus<E>>, F, R> {
    let acquire_inject = inject.clone();
    let acquire = effect_try_sync! {
        body: move || {
            let mut slot = storage.cursors.try_checkout()?;
            let receiver = root.lock().unwrap_or_else(std::sync::PoisonError::into_inner).try_clone()?;
            *slot.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(PreparedCursor { first: true, receiver });
            Ok(CursorOwner(Some(slot.share())))
        },
        on_error: move |error| acquire_inject(DiagnosticsError::Memory(error)),
    };
    stream! {
        pull: acquire,
        read: move |resource| {
            let state = match resource.with(CursorOwner::clone) {
                Ok(state) => state,
                Err(defect) => return effect_die! { defect: defect, },
            };
            let inject = inject.clone();
            effect_future! {
                exit_with_context; body: move |_, _| {
                    let state = state.clone();
                    let inject = inject.clone();
                    async move {
                        let cursor = state.shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                        let Some(cursor) = cursor else {
                            return Exit::Fail(inject(DiagnosticsError::Memory(MemoryError::Busy)));
                        };
                        let mut cursor = std::mem::DropGuard::new(cursor, |cursor| {
                            *state.shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cursor);
                        });
                        if !cursor.first && cursor.receiver.changed().await.is_err() {
                            return Exit::Success(effect::Chunk::End);
                        }
                        cursor.first = false;
                        match latest(&mut cursor.receiver) {
                            Ok(value) => Exit::Success(effect::Chunk::Values(vec![value])),
                            Err(error) => Exit::Fail(inject(DiagnosticsError::Memory(error))),
                        }
                    }
                },
            }
        },
        release: |_cursor| crate::effect_sync! { body { () }, },
    }
}
