//! Best-effort bounded locale preference persistence through one core queue/worker.

use super::{CultureId, LocalizationError};
use crate::{
    effect_finalizer, effect_future, effect_map, effect_map_error, effect_recover, effect_sync,
    effect_try_sync,
};
use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use effect::{
    AdmissionCapacity, AdmissionError, AsyncQueue, Effect, Exit, HasClock, HasTaskSpawner,
    PreparedCleanupConfig, PreparedPanicPool, PreparedQueue, PreparedQueueCapacity,
    PreparedQueueItem, PreparedSharedValuePool, ProviderAdmission, QueueCapacity, QueueError,
    QueueShutdown, Scope, SharedValue,
};
use effect::{effect, effect_timeout};
use std::future::{Future, poll_fn};
use std::num::NonZeroUsize;
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

/// Deliberate local persistence shutdown semantics. Pending writes are not durable;
/// shutdown/process loss may discard accepted but not yet acknowledged jobs.
#[derive(Clone, Copy)]
#[must_use]
pub enum PreferenceShutdown {
    /// Discard buffered jobs after joining the owned worker's current cleanup.
    DiscardPending,
}
/// Explicit queue/admission/deadline policy; no unbounded hidden submit waiters.
#[derive(Clone, Copy)]
#[must_use]
pub struct PreferencePolicy {
    /// Maximum immutable jobs buffered in the existing core queue.
    pub buffered: NonZeroUsize,
    /// Maximum simultaneous bounded offers admitted before queue suspension.
    pub maximum_offers: NonZeroUsize,
    /// Positive elapsed admission deadline for one queue offer.
    pub offer_deadline: Duration,
    /// Explicit best-effort shutdown loss policy.
    pub shutdown: PreferenceShutdown,
}
impl PreferencePolicy {
    /// Validate a positive admission duration before acquiring any worker.
    pub fn validate(self) -> Result<Self, LocalizationError> {
        if self.offer_deadline.is_zero() {
            return Err(LocalizationError::Definition(
                "preference offer deadline".into(),
            ));
        }
        Ok(self)
    }
}
/// One immutable owned persistence job. No request/context/service references exist.
#[must_use]
pub struct PreferenceJob<U: Send + Sync + 'static> {
    /// Shared immutable user identity selected by the application.
    pub user: Arc<U>,
    /// Validated culture value moved from submission.
    pub culture: CultureId,
}
/// Protected expected preference error; writer failures retain typed source ownership.
#[must_use]
pub enum PreferenceError<W: Send + 'static> {
    /// Selected writer failed; details are never automatically sent to HTTP.
    Writer(SharedValue<W>),
    /// Core queue closed or rejected ownership.
    Queue(QueueError),
    /// Bounded offer deadline expired.
    TimedOut,
    /// Immediate core offer admission failed.
    Admission(AdmissionError),
    /// Policy was rejected before worker acquisition.
    Definition(LocalizationError),
    /// The selected prepared preference workspace declined the operation.
    Memory(MemoryError),
}
/// Local submission acknowledgment, not a persistence durability claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum PreferenceSubmission {
    /// Job committed to the local queue; the writer may still fail or be lost at shutdown.
    Accepted,
    /// Immediate admission or bounded queue-offer deadline prevented acceptance.
    Busy,
}
/// Last acknowledged job and retained typed failure. Adjacent successful duplicate
/// suppression compares owned identities/cultures without copying user payloads.
#[must_use]
pub struct PreferenceStatus<U: Send + Sync + 'static, W: Send + 'static> {
    /// Last job whose writer Effect succeeded including its cleanup.
    pub persisted: Option<Arc<PreferenceJob<U>>>,
    /// Last expected writer failure retained as a shared typed owner.
    pub failure: Option<SharedValue<W>>,
}
impl<U: Send + Sync + 'static, W: Send + 'static> Clone for PreferenceStatus<U, W> {
    /// Share immutable jobs/failures without requiring U/W Clone.
    fn clone(&self) -> Self {
        Self {
            persisted: self.persisted.clone(),
            failure: self.failure.clone(),
        }
    }
}

/// One independently prepared locale preference queue and its retained writer errors.
#[must_use]
pub struct PreparedLocalePreferences<U: Send + Sync + 'static, W: Send + 'static> {
    queue: Mutex<Option<PreparedJobQueue<U>>>,
    failures: PreparedSharedValuePool<W>,
    policy: PreferencePolicy,
}

type PreparedJobQueue<U> = Arc<PreparedQueue<PreferenceJob<U>>>;

impl<U: Send + Sync + 'static, W: Send + 'static> PreparedLocalePreferences<U, W> {
    /// Region demand for jobs, waiters and typed failure payloads.
    ///
    /// In addition to `retained_failures`, reserve slots for the current status
    /// and the next write. Cleanup and panic storage are supplied independently.
    pub fn storage_bytes(
        policy: PreferencePolicy,
        retained_failures: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        PreparedQueue::<PreferenceJob<U>>::demand(
            QueueCapacity::Bounded(policy.buffered),
            Self::capacity(policy)?,
        )?
        .region_bytes
        .checked_add(PreparedSharedValuePool::<W>::storage_bytes(
            Self::failure_slots(retained_failures)?,
        )?)
    }

    /// Prepare the physical jobs and offer/take controls for one hosted worker.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        policy: PreferencePolicy,
        retained_failures: NonZeroUsize,
        cleanup: &PreparedCleanupConfig<QueueError>,
        panics: &PreparedPanicPool,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            queue: Mutex::new(Some(Arc::new(PreparedQueue::prepare(
                memory,
                region,
                QueueCapacity::Bounded(policy.buffered),
                Self::capacity(policy)?,
                cleanup,
                panics,
            )?))),
            failures: PreparedSharedValuePool::prepare(
                memory,
                region,
                Self::failure_slots(retained_failures)?,
            )?,
            policy,
        })
    }

    fn capacity(policy: PreferencePolicy) -> Result<PreparedQueueCapacity, MemoryError> {
        let producers = policy.maximum_offers.get();
        // Besides buffered/suspending offers, one item can be in the writer
        // and one remains in last-success status until a replacement commits.
        let payloads = policy
            .buffered
            .get()
            .checked_add(producers)
            .and_then(|payloads| payloads.checked_add(2))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Ok(PreparedQueueCapacity {
            payloads,
            producers,
            takers: 1,
        })
    }

    fn failure_slots(retained_failures: NonZeroUsize) -> Result<usize, MemoryError> {
        retained_failures
            .get()
            .checked_add(2)
            .ok_or(MemoryError::ArithmeticOverflow)
    }
}

enum OwnedJob<U: Send + Sync + 'static> {
    Ordinary(Arc<PreferenceJob<U>>),
    Prepared(PreparedQueueItem<PreferenceJob<U>>),
}
impl<U: Send + Sync + 'static> Deref for OwnedJob<U> {
    type Target = PreferenceJob<U>;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(job) => job,
            Self::Prepared(job) => job,
        }
    }
}

struct State<U: Send + Sync + 'static, W: Send + 'static> {
    persisted: Option<OwnedJob<U>>,
    failure: Option<SharedValue<W>>,
}

enum JobQueue<U: Send + Sync + 'static> {
    Ordinary(AsyncQueue<Arc<PreferenceJob<U>>>, ProviderAdmission),
    Prepared(PreparedJobQueue<U>),
}
impl<U: Send + Sync + 'static> Clone for JobQueue<U> {
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(queue, permit) => Self::Ordinary(queue.clone(), permit.clone()),
            Self::Prepared(queue) => Self::Prepared(queue.clone()),
        }
    }
}

impl<U: Send + Sync + 'static> JobQueue<U> {
    fn take<W: Send + 'static, R: Send + Sync + 'static>(
        &self,
    ) -> Effect<OwnedJob<U>, PreferenceError<W>, R> {
        match self {
            Self::Ordinary(queue, _) => effect_map! {
                program: effect_map_error! { program: effect! { queue_take; queue: queue, }, map: PreferenceError::Queue, },
                map: OwnedJob::Ordinary,
            },
            Self::Prepared(queue) => {
                let queue = queue.clone();
                effect_future! {
                    exit_with_context; body: move |_: Arc<R>, context| {
                        let queue = queue.clone();
                        async move {
                            let cancelled = match context.try_cancelled() {
                                Ok(cancelled) => cancelled,
                                Err(error) => return Exit::Fail(PreferenceError::Memory(error)),
                            };
                            let take = match queue.take_native() {
                                Ok(take) => take,
                                Err(error) => return Exit::Fail(PreferenceError::Memory(error)),
                            };
                            let outcome = await_queue(cancelled, take).await.map_error(PreferenceError::Queue);
                            match outcome.into_failure() {
                                Ok(item) => Exit::Success(OwnedJob::Prepared(item)),
                                Err(failure) => failure,
                            }
                        }
                    },
                }
            }
        }
    }

    fn close<W: Send + 'static, R: Send + Sync + 'static>(
        &self,
        shutdown: PreferenceShutdown,
    ) -> Effect<(), PreferenceError<W>, R> {
        let mode = match shutdown {
            PreferenceShutdown::DiscardPending => QueueShutdown::Discard,
        };
        match self {
            Self::Ordinary(queue, _) => effect_map_error! {
                program: effect! { queue_shutdown; queue: queue, mode: mode, }, map: PreferenceError::Queue,
            },
            Self::Prepared(queue) => {
                let queue = queue.clone();
                effect_future! {
                    exit_with_context; body: move |_: Arc<R>, _| {
                        let queue = queue.clone();
                        async move {
                            match queue.shutdown_now(mode) {
                                Ok(exit) => exit.map_error(PreferenceError::Queue),
                                Err(error) => Exit::Fail(PreferenceError::Memory(error)),
                            }
                        }
                    },
                }
            }
        }
    }
}

async fn await_queue<A>(
    cancelled: impl Future<Output = ()>,
    queue: impl Future<Output = Exit<A, QueueError>>,
) -> Exit<A, QueueError> {
    let mut cancelled = std::pin::pin!(cancelled);
    let mut queue = std::pin::pin!(queue);
    poll_fn(|context| {
        if cancelled.as_mut().poll(context).is_ready() {
            return Poll::Ready(Exit::Interrupted);
        }
        queue.as_mut().poll(context)
    })
    .await
}

/// Shared scoped queue/status/admission handle; exactly one worker is acquired.
#[must_use]
pub struct LocalePreferences<U: Send + Sync + 'static, W: Send + 'static> {
    /// Core queue retaining only immutable jobs.
    queue: JobQueue<U>,
    /// Protected last acknowledgment/failure; short synchronous updates only.
    status: Arc<Mutex<State<U, W>>>,
    prepared: Option<Arc<PreparedLocalePreferences<U, W>>>,
    /// Deliberate local admission/shutdown policy.
    policy: PreferencePolicy,
}
impl<U: Send + Sync + 'static, W: Send + 'static> Clone for LocalePreferences<U, W> {
    /// Share the same queue and worker, never acquire another background task.
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            status: self.status.clone(),
            prepared: self.prepared.clone(),
            policy: self.policy,
        }
    }
}
impl<U: Send + Sync + 'static, W: Send + 'static> LocalePreferences<U, W> {
    /// Each execution validates policy and acquires a fresh bounded queue/worker
    /// in the supplied host scope. The writer receives owned immutable jobs; errors
    /// move through the selected injection without requiring payload Clone.
    pub fn scoped<E, R>(
        scope: Scope<E>,
        policy: PreferencePolicy,
        same_user: impl Fn(&U, &U) -> bool + Send + Sync + 'static,
        writer: impl Fn(Arc<U>, CultureId) -> Effect<(), W, R> + Send + Sync + 'static,
        inject: impl Fn(PreferenceError<W>) -> E + Send + Sync + 'static,
    ) -> Effect<Self, E, R>
    where
        E: Send + 'static,
        R: HasTaskSpawner + Send + Sync + 'static,
    {
        Self::scoped_with(scope, policy, None, same_user, writer, inject)
    }

    /// Run the existing preference policy with queue and error cells from a prepared region.
    pub fn scoped_prepared<E, R>(
        scope: Scope<E>,
        storage: PreparedLocalePreferences<U, W>,
        same_user: impl Fn(&U, &U) -> bool + Send + Sync + 'static,
        writer: impl Fn(Arc<U>, CultureId) -> Effect<(), W, R> + Send + Sync + 'static,
        inject: impl Fn(PreferenceError<W>) -> E + Send + Sync + 'static,
    ) -> Effect<Self, E, R>
    where
        E: Send + 'static,
        R: HasTaskSpawner + Send + Sync + 'static,
    {
        Self::scoped_with(
            scope,
            storage.policy,
            Some(Arc::new(storage)),
            same_user,
            writer,
            inject,
        )
    }

    fn scoped_with<E, R>(
        scope: Scope<E>,
        policy: PreferencePolicy,
        prepared: Option<Arc<PreparedLocalePreferences<U, W>>>,
        same_user: impl Fn(&U, &U) -> bool + Send + Sync + 'static,
        writer: impl Fn(Arc<U>, CultureId) -> Effect<(), W, R> + Send + Sync + 'static,
        inject: impl Fn(PreferenceError<W>) -> E + Send + Sync + 'static,
    ) -> Effect<Self, E, R>
    where
        E: Send + 'static,
        R: HasTaskSpawner + Send + Sync + 'static,
    {
        let same_user = Arc::new(same_user);
        let writer = Arc::new(writer);
        let inject = Arc::new(inject);
        let initial_inject = inject.clone();
        let open = effect_try_sync! {
            body: move || {
                let policy = policy.validate().map_err(PreferenceError::Definition)?;
                let queue = match &prepared {
                    None => JobQueue::Ordinary(
                        AsyncQueue::new(QueueCapacity::Bounded(policy.buffered)),
                        ProviderAdmission::new(AdmissionCapacity::Bounded(policy.maximum_offers)),
                    ),
                    Some(storage) => {
                        let mut queue = storage.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        let queue = queue.take().ok_or(PreferenceError::Memory(MemoryError::Busy))?;
                        JobQueue::Prepared(queue)
                    }
                };
                Ok(Self {
                    queue,
                    status: Arc::new(Mutex::new(State { persisted: None, failure: None })),
                    prepared: prepared.clone(),
                    policy,
                })
            },
            on_error: move |error| initial_inject(error),
        };
        effect! {
        bind;
        program: open,
        body: move |preferences: Self| {
                    let queue = preferences.queue.clone();
                    let shutdown_inject = inject.clone();
                    let close = effect_finalizer! {
        scope: scope, body: effect_map_error! {
            program: queue.close(policy.shutdown), map: move |error| shutdown_inject(error), },
        };
                    let worker_preferences = preferences.clone();
                    let writer = writer.clone();
                    let same_user = same_user.clone();
                    let worker_inject = inject.clone();
                    let worker = effect_future! {
                        exit_with_context; body: move |env: Arc<R>, context| {
                        let preferences = worker_preferences.clone();
                        let writer = writer.clone();
                        let same_user = same_user.clone();
                        let inject = worker_inject.clone();
                        async move {
                            loop {
                                let job = match effect::effect! {
                                    run_in_context;
                                    context: context,
                                    program: effect! { queue_take; queue: preferences.queue, },
                                    environment: env.clone(),
                                }.await {
                                    Exit::Success(job) => job,
                                    Exit::Fail(PreferenceError::Queue(QueueError::Shutdown)) => return Exit::Success(()),
                                    Exit::Fail(error) => return Exit::Fail(inject(error)),
                                    Exit::Die(defect) => return Exit::Die(defect),
                                    Exit::Interrupted => return Exit::Interrupted,
                                    Exit::FinalizationFailed(failure) => {
                                        return Exit::<(), _>::FinalizationFailed(failure)
                                            .map_error(|error| inject(error));
                                    }
                                };
                                let previous_user = {
                                    let status = preferences
                                        .status
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                                    status.persisted.as_ref()
                                        .filter(|previous| status.failure.is_none() && previous.culture == job.culture)
                                        .map(|previous| previous.user.clone())
                                };
                                if previous_user.is_some_and(|previous| same_user(&previous, &job.user)) {
                                    continue;
                                }
                                let error_cell = match &preferences.prepared {
                                    Some(storage) => match storage.failures.reserve() {
                                        Ok(cell) => Some(cell),
                                        Err(error) => return Exit::Fail(inject(PreferenceError::Memory(error))),
                                    },
                                    None => None,
                                };
                                match effect::effect! {
                                    run_in_context;
                                    context: context,
                                    program: writer(job.user.clone(), job.culture.clone()),
                                    environment: env.clone(),
                                }
                                    .await
                                {
                                    Exit::Success(()) => {
                                        let retired = {
                                            let mut status = preferences
                                            .status
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                                            (status.persisted.replace(job), status.failure.take())
                                        };
                                        drop(retired);
                                    }
                                    Exit::Fail(error) => {
                                        let error = match error_cell {
                                            Some(cell) => cell.publish(error),
                                            None => SharedValue::new(error),
                                        };
                                        let retired = preferences
                                            .status
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .failure.replace(error);
                                        drop(retired);
                                    }
                                    Exit::Die(defect) => return Exit::Die(defect),
                                    Exit::Interrupted => return Exit::Interrupted,
                                    Exit::FinalizationFailed(failure) => {
                                        return Exit::<(), _>::FinalizationFailed(failure).map_error(
                                            |error| {
                                                inject(PreferenceError::Writer(SharedValue::new(error)))
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }, };
                    let scope = scope.clone();
                    effect! {
         bind;
         program: close,
         body: move |()| {
                        let preferences = preferences.clone();
                        effect_map! { program: effect::effect_scope! { fork; scope: scope, program: worker.clone(), }, map: move |_| preferences.clone(), }
                    },
         }
            },
         }
    }
    /// Move one immutable job through bounded immediate/queue admission. Busy is
    /// never presented as a persisted preference; queue shutdown stays typed.
    pub fn submit<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        &self,
        user: Arc<U>,
        culture: CultureId,
    ) -> Effect<PreferenceSubmission, PreferenceError<W>, R> {
        match &self.queue {
            JobQueue::Ordinary(queue, offers) => {
                self.submit_ordinary(queue.clone(), offers.clone(), user, culture)
            }
            JobQueue::Prepared(queue) => {
                prepared_offer(queue.clone(), user, culture, self.policy.offer_deadline)
            }
        }
    }

    fn submit_ordinary<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        &self,
        queue: AsyncQueue<Arc<PreferenceJob<U>>>,
        offers: ProviderAdmission,
        user: Arc<U>,
        culture: CultureId,
    ) -> Effect<PreferenceSubmission, PreferenceError<W>, R> {
        let job = Arc::new(PreferenceJob { user, culture });
        let deadline = self.policy.offer_deadline;
        effect_recover! {
            program: crate::admission::independent(
            offers,
            move || {
                effect_map! {
                    program: effect_timeout! {
                    program: effect_map_error! {
                    program: effect! {
                        queue_offer_with;
                        queue: queue,
                        factory: {
                            let job = job.clone();
                            move || job.clone()
                        },
                    }, map: PreferenceError::Queue, },
                    duration: deadline,
                    on_timeout: || PreferenceError::TimedOut,
                    }, map: |()| PreferenceSubmission::Accepted, }
            },
            PreferenceError::Admission,
        ), recover: |error| match error {
            PreferenceError::TimedOut | PreferenceError::Admission(AdmissionError::Full) => {
                effect_sync! { body { PreferenceSubmission::Busy }, }
            }
            error @ (PreferenceError::Writer(_)
            | PreferenceError::Queue(_)
            | PreferenceError::Admission(
                AdmissionError::Closed | AdmissionError::Overflow,
            )
            | PreferenceError::Definition(_) | PreferenceError::Memory(_)) => effect::effect_from_result! { once; result: Err(error), },
        }, }
    }
    /// Read protected immutable status without retaining request-scoped capabilities.
    pub fn status(&self) -> PreferenceStatus<U, W> {
        let status = self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let persisted = status.persisted.as_ref().map(|job| match job {
            OwnedJob::Ordinary(job) => job.clone(),
            OwnedJob::Prepared(job) => Arc::new(PreferenceJob {
                user: job.user.clone(),
                culture: job.culture.clone(),
            }),
        });
        PreferenceStatus {
            persisted,
            failure: status.failure.clone(),
        }
    }
}

fn prepared_offer<U, W, R>(
    queue: PreparedJobQueue<U>,
    user: Arc<U>,
    culture: CultureId,
    deadline: Duration,
) -> Effect<PreferenceSubmission, PreferenceError<W>, R>
where
    U: Send + Sync + 'static,
    W: Send + 'static,
    R: HasClock + HasTaskSpawner + Send + Sync + 'static,
{
    let submit = effect_future! {
        exit_with_context; body: move |_: Arc<R>, context| {
            let queue = queue.clone();
            let user = user.clone();
            let culture = culture.clone();
            async move {
                let cancelled = match context.try_cancelled() {
                    Ok(cancelled) => cancelled,
                    Err(error) => return Exit::Fail(PreferenceError::Memory(error)),
                };
                let offer = match queue.reserve_native_offer() {
                    Ok(reservation) => reservation.start(move || PreferenceJob { user, culture }),
                    Err(MemoryError::NoSlot | MemoryError::Busy) => return Exit::Success(PreferenceSubmission::Busy),
                    Err(error) => return Exit::Fail(PreferenceError::Memory(error)),
                };
                let outcome = await_queue(cancelled, offer).await.map_error(PreferenceError::Queue);
                match outcome.into_failure() {
                    Ok(()) => Exit::Success(PreferenceSubmission::Accepted),
                    Err(failure) => failure,
                }
            }
        },
    };
    effect_recover! {
        program: effect_timeout! {
            program: submit,
            duration: deadline,
            on_timeout: || PreferenceError::TimedOut,
        },
        recover: |error| match error {
            PreferenceError::TimedOut => effect_sync! { body { PreferenceSubmission::Busy }, },
            error @ (PreferenceError::Writer(_) | PreferenceError::Queue(_) | PreferenceError::Admission(_)
                | PreferenceError::Definition(_) | PreferenceError::Memory(_)) => effect::effect_from_result! { once; result: Err(error), },
        },
    }
}
