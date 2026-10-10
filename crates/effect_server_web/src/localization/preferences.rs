//! Best-effort bounded locale preference persistence through one core queue/worker.

use super::{CultureId, LocalizationError};
use effect::{
    AdmissionCapacity, AdmissionError, AsyncQueue, Effect, Exit, HasClock, HasTaskSpawner,
    ProviderAdmission, QueueCapacity, QueueError, QueueShutdown, Scope, SharedValue,
};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
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
/// Shared scoped queue/status/admission handle; exactly one worker is acquired.
#[must_use]
pub struct LocalePreferences<U: Send + Sync + 'static, W: Send + 'static> {
    /// Core queue retaining only immutable jobs.
    queue: AsyncQueue<Arc<PreferenceJob<U>>>,
    /// Core bound for suspended offers, separate from buffered job capacity.
    offers: ProviderAdmission,
    /// Protected last acknowledgment/failure; short synchronous updates only.
    status: Arc<Mutex<PreferenceStatus<U, W>>>,
    /// Deliberate local admission/shutdown policy.
    policy: PreferencePolicy,
}
impl<U: Send + Sync + 'static, W: Send + 'static> Clone for LocalePreferences<U, W> {
    /// Share the same queue and worker, never acquire another background task.
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            offers: self.offers.clone(),
            status: self.status.clone(),
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
        let same_user = Arc::new(same_user);
        let writer = Arc::new(writer);
        let inject = Arc::new(inject);
        let validation_inject = inject.clone();
        Effect::try_sync(
            move || policy.validate(),
            move |error| validation_inject(PreferenceError::Definition(error)),
        )
        .flat_map(move |policy| {
            let preferences = Self {
                queue: AsyncQueue::new(QueueCapacity::Bounded(policy.buffered)),
                offers: ProviderAdmission::new(AdmissionCapacity::Bounded(policy.maximum_offers)),
                status: Arc::new(Mutex::new(PreferenceStatus {
                    persisted: None,
                    failure: None,
                })),
                policy,
            };
            let queue = preferences.queue.clone();
            let shutdown_inject = inject.clone();
            let close = scope.add_finalizer(
                queue
                    .shutdown(match policy.shutdown {
                        PreferenceShutdown::DiscardPending => QueueShutdown::Discard,
                    })
                    .map_error(move |error| shutdown_inject(PreferenceError::Queue(error))),
            );
            let worker_preferences = preferences.clone();
            let writer = writer.clone();
            let same_user = same_user.clone();
            let worker_inject = inject.clone();
            let worker = Effect::from_exit_future_with_context(move |env: Arc<R>, context| {
                let preferences = worker_preferences.clone();
                let writer = writer.clone();
                let same_user = same_user.clone();
                let inject = worker_inject.clone();
                async move {
                    loop {
                        let job = match context.run(preferences.queue.take(), env.clone()).await {
                            Exit::Success(job) => job,
                            Exit::Fail(QueueError::Shutdown) => return Exit::Success(()),
                            Exit::Die(defect) => return Exit::Die(defect),
                            Exit::Interrupted => return Exit::Interrupted,
                            Exit::FinalizationFailed(failure) => {
                                return Exit::<(), _>::FinalizationFailed(failure)
                                    .map_error(|error| inject(PreferenceError::Queue(error)));
                            }
                        };
                        let duplicate = {
                            let status = preferences
                                .status
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            status.failure.is_none()
                                && status.persisted.as_ref().is_some_and(|previous| {
                                    previous.culture == job.culture
                                        && same_user(&previous.user, &job.user)
                                })
                        };
                        if duplicate {
                            continue;
                        }
                        match context
                            .run(writer(job.user.clone(), job.culture.clone()), env.clone())
                            .await
                        {
                            Exit::Success(()) => {
                                let mut status = preferences
                                    .status
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                status.persisted = Some(job);
                                status.failure = None;
                            }
                            Exit::Fail(error) => {
                                preferences
                                    .status
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .failure = Some(SharedValue::new(error))
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
            });
            let scope = scope.clone();
            close.flat_map(move |()| {
                let preferences = preferences.clone();
                scope.fork(worker.clone()).map(move |_| preferences.clone())
            })
        })
    }
    /// Move one immutable job through bounded immediate/queue admission. Busy is
    /// never presented as a persisted preference; queue shutdown stays typed.
    pub fn submit<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        &self,
        user: Arc<U>,
        culture: CultureId,
    ) -> Effect<PreferenceSubmission, PreferenceError<W>, R> {
        let queue = self.queue.clone();
        let job = Arc::new(PreferenceJob { user, culture });
        let deadline = self.policy.offer_deadline;
        crate::admission::independent(
            self.offers.clone(),
            move || {
                queue
                    .offer_with({
                        let job = job.clone();
                        move || job.clone()
                    })
                    .map_error(PreferenceError::Queue)
                    .timeout(deadline, || PreferenceError::TimedOut)
                    .map(|()| PreferenceSubmission::Accepted)
            },
            PreferenceError::Admission,
        )
        .catch_all(|error| match error {
            PreferenceError::TimedOut | PreferenceError::Admission(AdmissionError::Full) => {
                Effect::succeed(PreferenceSubmission::Busy)
            }
            error @ (PreferenceError::Writer(_)
            | PreferenceError::Queue(_)
            | PreferenceError::Admission(
                AdmissionError::Closed | AdmissionError::Overflow,
            )
            | PreferenceError::Definition(_)) => Effect::from_result_once(Err(error)),
        })
    }
    /// Read protected immutable status without retaining request-scoped capabilities.
    pub fn status(&self) -> PreferenceStatus<U, W> {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}
