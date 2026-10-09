//! Durable workflow operation binding.

use super::{
    ActivityRunError, ChildWait, DurableBranch, DurableChild, DurableClock, DurableClockError,
    DurableDeferred, DurableParallel, DurableParallelError, DurableRace, DurableRaceError,
    DurableRetry, DurableRetryError, DurableRetryPolicy, DurableSignalError, DurableSignalWait,
    HasWorkflowEngine, RecordedDeadline, WorkflowActivity, WorkflowEngine, WorkflowRaceId,
    WorkflowRunId,
};
use crate::{Effect, HasTaskSpawner, HasWallClock, WallClock};
use std::num::{NonZeroU32, NonZeroUsize};

/// Type alias for engine error in workflow durable.
///
/// # Type parameters
/// - `R`: Environment type.
type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;
/// Type alias for wall clock error in workflow durable.
///
/// # Type parameters
/// - `R`: Environment type.
type WallClockError<R> = <<R as HasWallClock>::Clock as WallClock>::Error;
/// Type alias for branch results in workflow durable.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type BranchResults<A, E> = Vec<Result<A, E>>;

/// Deterministic workflow coordination description. Its constructor is
/// private: ordinary Effects belong inside named activity bodies, not as
/// directly retryable workflow coordination steps.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
/// - `R`: Environment type. Bound or type: `Send + Sync + 'static`.
pub struct Durable<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    effect: Effect<A, E, R>,
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for Durable<A, E, R> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            effect: self.effect.clone(),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Durable<A, E, R> {
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `effect` (`Effect<A, E, R>`): The effect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn from_effect(effect: Effect<A, E, R>) -> Self {
        Self { effect }
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `Effect<A, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub(crate) fn into_effect(self) -> Effect<A, E, R> {
        self.effect
    }

    /// Reconstruct a restricted durable description on each evaluation, including pre-bind locals.
    /// The factory must return Durable; this does not admit external Effects into replay.
    pub fn defer(factory: impl Fn() -> Self + Send + Sync + 'static) -> Self {
        Self::from_effect(Effect::defer(move || factory().effect))
    }

    /// Construct a durable body whose Rust locals are fresh per evaluation.
    ///
    /// The supplied context accepts only Durable operations. Ordinary Effects remain
    /// confined to recorded activity bodies; this is not an unrestricted lift operation.
    #[doc(hidden)]
    pub fn from_body<F>(factory: impl Fn(DurableContext<E, R>) -> F + Send + Sync + 'static) -> Self
    where
        F: std::future::Future<Output = Result<A, crate::Exit<A, E>>> + Send + 'static,
    {
        let factory = std::sync::Arc::new(factory);
        Self::from_effect(crate::internal::workflow::reusable(move || {
            let factory = std::sync::Arc::clone(&factory);
            crate::internal::macro_future::async_workflow(move |context| async move {
                match factory(DurableContext { context }).await {
                    Ok(value) => crate::Exit::Success(value),
                    Err(failure) => failure,
                }
            })
        }))
    }

    /// Construct one callback invocation's durable body with owned, non-Clone inputs.
    /// Workflow replay reconstructs the callback; executing this same description twice
    /// reports the existing consumed-workflow defect instead of copying its input.
    #[doc(hidden)]
    pub fn from_body_once<F>(
        factory: impl FnOnce(DurableContext<E, R>) -> F + Send + 'static,
    ) -> Self
    where
        F: std::future::Future<Output = Result<A, crate::Exit<A, E>>> + Send + 'static,
    {
        Self::from_effect(crate::internal::workflow::once(move || {
            crate::internal::macro_future::async_workflow(move |context| async move {
                match factory(DurableContext { context }).await {
                    Ok(value) => crate::Exit::Success(value),
                    Err(failure) => failure,
                }
            })
        }))
    }

    /// Produce a fresh successful value when this description is executed.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`impl Fn() -> A + Send + Sync + 'static`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn succeed_with(value: impl Fn() -> A + Send + Sync + 'static) -> Self {
        Self::from_effect(Effect::sync(value))
    }

    /// Consume an owned result produced within one workflow evaluation.
    /// A replay rebuilds the workflow body and therefore obtains fresh values.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`impl FnOnce() -> A + Send + 'static`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn succeed_once(value: impl FnOnce() -> A + Send + 'static) -> Self {
        Self::from_effect(Effect::sync_once(value))
    }

    /// Transform successful values while preserving other outcome channels.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `map` (`impl Fn(A) -> B + Send + Sync + 'static`): The map supplied to this operation.
    ///
    /// # Returns
    /// Type: `Durable<B, E, R>`.
    pub fn map<B: Send + 'static>(
        self,
        map: impl Fn(A) -> B + Send + Sync + 'static,
    ) -> Durable<B, E, R> {
        Durable::from_effect(self.effect.map(map))
    }

    /// Transform the expected error channel without changing other workflow outcomes.
    ///
    /// # Type parameters
    /// - `F`: Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `map` (`impl Fn(E) -> F + Send + Sync + 'static`): The map supplied to this operation.
    ///
    /// # Returns
    /// Type: `Durable<A, F, R>`.
    pub fn map_error<F: Send + 'static>(
        self,
        map: impl Fn(E) -> F + Send + Sync + 'static,
    ) -> Durable<A, F, R> {
        Durable::from_effect(self.effect.map_error(map))
    }

    /// Build a dependent operation from each successful value.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `next` (`impl Fn(A) -> Durable<B, E, R> + Send + Sync + 'static`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Durable<B, E, R>`.
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
    ///
    /// # Type parameters
    /// - `Items`: Input collection or iterator type. Additional bound: `IntoIterator + 'static`. Additional bound: `:Item: Send + 'static`. Additional bound: `:IntoIter: Send + 'static`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `items` (`impl Fn() -> Items + Send + Sync + 'static`): The items supplied to this operation.
    /// - `body` (`impl Fn(Items::Item) -> Durable<(), E, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Self`.
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

    /// Select the named activity registered with this workflow.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `DomainError`: Bounds: `Send + 'static`.
    /// - `BodyError`: Bounds: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `description` (`WorkflowActivity<A, DomainError>`): The description supplied to this operation.
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `body` (`impl Fn() -> Effect<Result<A, DomainError>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Durable<Result<A, DomainError>, ActivityRunError<EngineError<R>, BodyError>, R>`.
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
    /// Record or await a named signal so replay observes the same value.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `token` (`DurableDeferred<A>`): Fencing token that identifies the current claim or owner.
    ///
    /// # Returns
    /// Type: `Durable<DurableSignalWait<A>, DurableSignalError<EngineError<R>>, R>`.
    pub fn signal<A: Send + 'static>(
        token: DurableDeferred<A>,
    ) -> Durable<DurableSignalWait<A>, DurableSignalError<EngineError<R>>, R>
    where
        R: HasWorkflowEngine,
    {
        Durable::from_effect(token.wait())
    }

    #[must_use]
    /// Wait on a recorded deadline rather than a transient timer alone.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `deadline` (`RecordedDeadline`): The deadline for the pending operation.
    ///
    /// # Returns
    /// Type: `Durable<super::DeadlineWait, DurableClockError<EngineError<R>, WallClockError<R>>, R>`.
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
    /// Await a child workflow through a recorded parent/child relationship.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `parent` (`WorkflowRunId`): Parent scope, span, or workflow identity.
    /// - `child` (`WorkflowRunId`): Child workflow or scope-owned task being observed.
    ///
    /// # Returns
    /// Type: `Durable<ChildWait, EngineError<R>, R>`.
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
    /// Describe another attempt under the supplied retry policy.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `DomainError`: Bounds: `Send + 'static`.
    /// - `BodyError`: Bounds: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `branch` (`DurableBranch<A, DomainError, BodyError, R>`): The branch supplied to this operation.
    /// - `policy` (`DurableRetryPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Durable<Result<A, DomainError>, DurableRetryError<EngineError<R>, BodyError>, R>`.
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
    /// Coordinate multiple durable branches and record their results for replay.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `DomainError`: Bounds: `Send + 'static`.
    /// - `BodyError`: Bounds: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `concurrency` (`NonZeroUsize`): Positive bound on independently admitted work.
    /// - `branches` (`Vec<DurableBranch<A, DomainError, BodyError, R>>`): The branches supplied to this operation.
    ///
    /// # Returns
    /// Type: `Durable<BranchResults<A, DomainError>, DurableParallelError<EngineError<R>, BodyError>, R>`.
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
    /// Run durable branches with recorded retry decisions.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `DomainError`: Bounds: `Send + 'static`.
    /// - `BodyError`: Bounds: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `concurrency` (`NonZeroUsize`): Positive bound on independently admitted work.
    /// - `policy` (`DurableRetryPolicy`): The caller-supplied behavior policy.
    /// - `branches` (`Vec<DurableBranch<A, DomainError, BodyError, R>>`): The branches supplied to this operation.
    ///
    /// # Returns
    /// Type: `Durable<BranchResults<A, DomainError>, DurableRetryError<EngineError<R>, BodyError>, R>`.
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
    /// Record which durable branch wins so replay chooses the same result.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `DomainError`: Bounds: `Send + 'static`.
    /// - `BodyError`: Bounds: `Send + 'static`.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `race` (`WorkflowRaceId`): Stable identity or definition of the durable race.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `left` (`DurableBranch<A, DomainError, BodyError, R>`): The left supplied to this operation.
    /// - `right` (`DurableBranch<A, DomainError, BodyError, R>`): Right-hand operation composed with this one.
    ///
    /// # Returns
    /// Type: `Durable<Result<A, DomainError>, DurableRaceError<EngineError<R>, BodyError>, R>`.
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

/// Select the named activity registered with this workflow.
///
/// # Type parameters
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `DomainError`: Expected domain-error type. Additional bound: `Send + 'static`.
/// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
///
/// # Parameters
/// - `description` (`WorkflowActivity<A, DomainError>`): The description supplied to this operation.
/// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
/// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
/// - `body` (`impl Fn() -> Effect<Result<A, DomainError>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
///
/// # Returns
/// Type: `Durable<Result<A, DomainError>, ActivityRunError<EngineError<R>, BodyError>, R>`.
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

/// Evaluation-local generated-code bridge enforcing the durable bind boundary.
#[doc(hidden)]
pub struct DurableContext<E: Send + 'static, R: Send + Sync + 'static> {
    /// Shares the existing Effect runtime context; never a second interpreter.
    context: crate::internal::macro_future::MacroContext<E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> DurableContext<E, R> {
    /// Evaluate a supported durable description under the current runtime and tracing context.
    pub async fn bind<A: Send + 'static, B: Send + 'static>(
        &self,
        program: Durable<A, E, R>,
    ) -> Result<A, crate::Exit<B, E>> {
        self.context.bind(program.effect).await
    }
}
