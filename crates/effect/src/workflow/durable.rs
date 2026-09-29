use super::{
    ActivityRunError, ChildWait, DurableBranch, DurableChild, DurableClock, DurableClockError,
    DurableDeferred, DurableParallel, DurableParallelError, DurableRace, DurableRaceError,
    DurableRetry, DurableRetryError, DurableRetryPolicy, DurableSignalError, DurableSignalWait,
    HasWorkflowEngine, RecordedDeadline, WorkflowActivity, WorkflowEngine, WorkflowRaceId,
    WorkflowRunId,
};
use crate::{Effect, HasTaskSpawner, HasWallClock, WallClock};
use std::num::{NonZeroU32, NonZeroUsize};

type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;
type WallClockError<R> = <<R as HasWallClock>::Clock as WallClock>::Error;
type BranchResults<A, E> = Vec<Result<A, E>>;

/// Deterministic workflow coordination description. Its constructor is
/// private: ordinary Effects belong inside named activity bodies, not as
/// directly retryable workflow coordination steps.
pub struct Durable<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    effect: Effect<A, E, R>,
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for Durable<A, E, R> {
    fn clone(&self) -> Self {
        Self {
            effect: self.effect.clone(),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Durable<A, E, R> {
    pub(crate) fn from_effect(effect: Effect<A, E, R>) -> Self {
        Self { effect }
    }

    pub(crate) fn into_effect(self) -> Effect<A, E, R> {
        self.effect
    }

    pub fn succeed_with(value: impl Fn() -> A + Send + Sync + 'static) -> Self {
        Self::from_effect(Effect::sync(value))
    }

    /// Consume an owned result produced within one workflow evaluation.
    /// A replay rebuilds the workflow body and therefore obtains fresh values.
    pub fn succeed_once(value: impl FnOnce() -> A + Send + 'static) -> Self {
        Self::from_effect(Effect::sync_once(value))
    }

    pub fn map<B: Send + 'static>(
        self,
        map: impl Fn(A) -> B + Send + Sync + 'static,
    ) -> Durable<B, E, R> {
        Durable::from_effect(self.effect.map(map))
    }

    pub fn map_error<F: Send + 'static>(
        self,
        map: impl Fn(E) -> F + Send + Sync + 'static,
    ) -> Durable<A, F, R> {
        Durable::from_effect(self.effect.map_error(map))
    }

    pub fn flat_map<B: Send + 'static>(
        self,
        next: impl Fn(A) -> Durable<B, E, R> + Send + Sync + 'static,
    ) -> Durable<B, E, R> {
        Durable::from_effect(self.effect.flat_map(move |value| next(value).effect))
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Durable<(), E, R> {
    /// Rebuild the iterator on each evaluation. The caller supplies a
    /// deterministic source and stable activity identities inside `body`.
    pub fn for_each<Items>(
        items: impl Fn() -> Items + Send + Sync + 'static,
        body: impl Fn(Items::Item) -> Durable<(), E, R> + Send + Sync + 'static,
    ) -> Self
    where
        Items: IntoIterator + 'static,
        Items::Item: Send + 'static,
        Items::IntoIter: Send + 'static,
    {
        Durable::from_effect(Effect::for_each_sequential(items, move |item| {
            body(item).effect
        }))
    }

    pub fn activity<A: Send + 'static, DomainError: Send + 'static, BodyError: Send + 'static>(
        description: WorkflowActivity<A, DomainError>,
        run: WorkflowRunId,
        attempt: NonZeroU32,
        body: impl Fn() -> Effect<Result<A, DomainError>, BodyError, R> + Send + Sync + 'static,
    ) -> Durable<Result<A, DomainError>, ActivityRunError<EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine,
    {
        Durable::from_effect(description.run(run, attempt, body))
    }

    #[must_use]
    pub fn signal<A: Send + 'static>(
        token: DurableDeferred<A>,
    ) -> Durable<DurableSignalWait<A>, DurableSignalError<EngineError<R>>, R>
    where
        R: HasWorkflowEngine,
    {
        Durable::from_effect(token.wait())
    }

    #[must_use]
    pub fn deadline(
        run: WorkflowRunId,
        deadline: RecordedDeadline,
    ) -> Durable<super::DeadlineWait, DurableClockError<EngineError<R>, WallClockError<R>>, R>
    where
        R: HasWorkflowEngine + HasWallClock,
    {
        Durable::from_effect(DurableClock::wait_until(run, deadline))
    }

    #[must_use]
    pub fn child_wait(
        parent: WorkflowRunId,
        child: WorkflowRunId,
    ) -> Durable<ChildWait, EngineError<R>, R>
    where
        R: HasWorkflowEngine,
    {
        DurableChild::wait(parent, child)
    }

    #[must_use]
    pub fn retry<A: Send + 'static, DomainError: Send + 'static, BodyError: Send + 'static>(
        run: WorkflowRunId,
        branch: DurableBranch<A, DomainError, BodyError, R>,
        policy: DurableRetryPolicy,
    ) -> Durable<Result<A, DomainError>, DurableRetryError<EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine,
    {
        Durable::from_effect(DurableRetry::run(run, branch, policy))
    }

    #[must_use]
    pub fn parallel<A: Send + 'static, DomainError: Send + 'static, BodyError: Send + 'static>(
        run: WorkflowRunId,
        attempt: NonZeroU32,
        concurrency: NonZeroUsize,
        branches: Vec<DurableBranch<A, DomainError, BodyError, R>>,
    ) -> Durable<BranchResults<A, DomainError>, DurableParallelError<EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine + HasTaskSpawner,
    {
        Durable::from_effect(DurableParallel::all(run, attempt, concurrency, branches))
    }

    #[must_use]
    pub fn parallel_retry<
        A: Send + 'static,
        DomainError: Send + 'static,
        BodyError: Send + 'static,
    >(
        run: WorkflowRunId,
        concurrency: NonZeroUsize,
        policy: DurableRetryPolicy,
        branches: Vec<DurableBranch<A, DomainError, BodyError, R>>,
    ) -> Durable<BranchResults<A, DomainError>, DurableRetryError<EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine + HasTaskSpawner,
    {
        Durable::from_effect(DurableParallel::all_retry(
            run,
            concurrency,
            policy,
            branches,
        ))
    }

    #[must_use]
    pub fn race<A: Send + 'static, DomainError: Send + 'static, BodyError: Send + 'static>(
        run: WorkflowRunId,
        race: WorkflowRaceId,
        attempt: NonZeroU32,
        left: DurableBranch<A, DomainError, BodyError, R>,
        right: DurableBranch<A, DomainError, BodyError, R>,
    ) -> Durable<Result<A, DomainError>, DurableRaceError<EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine + HasTaskSpawner,
    {
        Durable::from_effect(DurableRace::first(run, race, attempt, left, right))
    }
}

pub fn activity<A, DomainError, BodyError, R>(
    description: WorkflowActivity<A, DomainError>,
    run: WorkflowRunId,
    attempt: NonZeroU32,
    body: impl Fn() -> Effect<Result<A, DomainError>, BodyError, R> + Send + Sync + 'static,
) -> Durable<Result<A, DomainError>, ActivityRunError<EngineError<R>, BodyError>, R>
where
    A: Send + 'static,
    DomainError: Send + 'static,
    BodyError: Send + 'static,
    R: HasWorkflowEngine + Send + Sync + 'static,
{
    Durable::from_effect(description.run(run, attempt, body))
}
