//! Recorded bounded parallelism and races.

use super::{
    ActivityRunError, HasWorkflowEngine, RecordedRace, WorkflowActivity, WorkflowActivityId,
    WorkflowEngine, WorkflowRaceId, WorkflowRunId,
};
use crate::{Effect, HasTaskSpawner};
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::{Arc, Mutex};

/// Type alias for engine error in workflow durable parallel.
///
/// # Type parameters
/// - `R`: Environment type.
type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;
/// Type alias for branch body in workflow durable parallel.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `BodyError`: Error produced by the registered workflow body.
/// - `R`: Environment type.
type BranchBody<A, E, BodyError, R> = dyn Fn() -> Effect<Result<A, E>, BodyError, R> + Send + Sync;
/// Type alias for branch results in workflow durable parallel.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type BranchResults<A, E> = Vec<Result<A, E>>;

/// One replayable branch in a bounded fan-out. The activity identity is
/// stable across reconstruction; the body is registered code, not state.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `BodyError`: Error produced by the registered workflow body.
/// - `R`: Environment type.
pub struct DurableBranch<A, E, BodyError, R>
where
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: Send + Sync + 'static,
{
    activity: WorkflowActivity<A, E>,
    body: Arc<BranchBody<A, E, BodyError, R>>,
}

impl<A, E, BodyError, R> Clone for DurableBranch<A, E, BodyError, R>
where
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            activity: self.activity.clone(),
            body: Arc::clone(&self.body),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The recorded delay and attempt limits for a replayable retry.
pub struct DurableRetryPolicy {
    /// The maximum permitted attempts. Type: `NonZeroU32`.
    pub max_attempts: NonZeroU32,
    /// A fixed decision is replayable without evaluating arbitrary ambient
    /// state between attempts.
    pub retry_domain_errors: bool,
}

#[derive(Debug)]
/// An attempt, schedule, or engine failure in a durable retry.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
/// - `BodyError`: Error produced by the registered workflow body.
pub enum DurableRetryError<EngineError, BodyError> {
    /// A branch failed during parallel or race coordination.
    Branch {
        /// Identity of the registered workflow activity. Type: `WorkflowActivityId`.
        activity: WorkflowActivityId,
        /// The underlying failure or provider cause. Type: `ActivityRunError<EngineError, BodyError>`.
        source: ActivityRunError<EngineError, BodyError>,
    },
    /// No further attempt can be allocated without exceeding its range.
    AttemptExhausted,
}

/// A replayable retry description whose decisions are recorded by the workflow engine.
pub struct DurableRetry;

impl DurableRetry {
    /// Execute or describe one run using the supplied environment and inputs.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `branch` (`DurableBranch<A, E, BodyError, R>`): The branch supplied to this operation.
    /// - `policy` (`DurableRetryPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Effect<Result<A, E>, DurableRetryError<EngineError<R>, BodyError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn run<A, E, BodyError, R>(
        run: WorkflowRunId,
        branch: DurableBranch<A, E, BodyError, R>,
        policy: DurableRetryPolicy,
    ) -> Effect<Result<A, E>, DurableRetryError<EngineError<R>, BodyError>, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        BodyError: Send + 'static,
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        retry_step(run, branch, policy, NonZeroU32::MIN)
    }
}

///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
///
/// # Parameters
/// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
/// - `branch` (`DurableBranch<A, E, BodyError, R>`): The branch supplied to this operation.
/// - `policy` (`DurableRetryPolicy`): The caller-supplied behavior policy.
/// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
///
/// # Returns
/// Type: `Effect<Result<A, E>, DurableRetryError<EngineError<R>, BodyError>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn retry_step<A, E, BodyError, R>(
    run: WorkflowRunId,
    branch: DurableBranch<A, E, BodyError, R>,
    policy: DurableRetryPolicy,
    attempt: NonZeroU32,
) -> Effect<Result<A, E>, DurableRetryError<EngineError<R>, BodyError>, R>
where
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: HasWorkflowEngine + Send + Sync + 'static,
{
    Effect::defer(move || {
        let run = run.clone();
        let branch = branch.clone();
        let next_branch = branch.clone();
        let activity = branch.id().clone();
        branch
            .into_effect(run.clone(), attempt)
            .map_error(move |source| DurableRetryError::Branch {
                activity: activity.clone(),
                source,
            })
            .flat_map(move |result| match result {
                Err(_) if policy.retry_domain_errors && attempt < policy.max_attempts => {
                    let next = attempt.get().checked_add(1).and_then(NonZeroU32::new);
                    match next {
                        Some(next) => retry_step(run.clone(), next_branch.clone(), policy, next),
                        None => Effect::from_result_once(Err(DurableRetryError::AttemptExhausted)),
                    }
                }
                other => Effect::sync_once(move || other),
            })
    })
}

impl<A, E, BodyError, R> DurableBranch<A, E, BodyError, R>
where
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    /// Construct a `DurableBranch` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `BodyError`: Error produced by the registered workflow body.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `activity` (`WorkflowActivity<A, E>`): The activity supplied to this operation.
    /// - `body` (`impl Fn() -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        activity: WorkflowActivity<A, E>,
        body: impl Fn() -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            activity,
            body: Arc::new(body),
        }
    }

    #[must_use]
    /// Return the stable identity associated with this value.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `BodyError`: Error produced by the registered workflow body.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `&WorkflowActivityId`.
    pub fn id(&self) -> &WorkflowActivityId {
        &self.activity.id
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    ///
    /// # Returns
    /// Type: `Effect<Result<A, E>, ActivityRunError<EngineError<R>, BodyError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn into_effect(
        self,
        run: WorkflowRunId,
        attempt: NonZeroU32,
    ) -> Effect<Result<A, E>, ActivityRunError<EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine,
    {
        let body = Arc::clone(&self.body);
        self.activity.run(run, attempt, move || body())
    }
}

/// A replayable composition that runs durable branches under an explicit bound.
pub struct DurableParallel;

#[derive(Debug)]
/// A branch or coordination failure in durable parallel execution.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
/// - `BodyError`: Error produced by the registered workflow body.
pub enum DurableParallelError<EngineError, BodyError> {
    /// A branch failed during parallel or race coordination.
    Branch {
        /// Identity of the registered workflow activity. Type: `WorkflowActivityId`.
        activity: WorkflowActivityId,
        /// The underlying failure or provider cause. Type: `ActivityRunError<EngineError, BodyError>`.
        source: ActivityRunError<EngineError, BodyError>,
    },
}

impl DurableParallel {
    /// Execute every durable branch under the supplied retry policy.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + HasTaskSpawner + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `concurrency` (`NonZeroUsize`): Positive bound on independently admitted work.
    /// - `policy` (`DurableRetryPolicy`): The caller-supplied behavior policy.
    /// - `branches` (`Vec<DurableBranch<A, E, BodyError, R>>`): The branches supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<BranchResults<A, E>, DurableRetryError<EngineError<R>, BodyError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn all_retry<A, E, BodyError, R>(
        run: WorkflowRunId,
        concurrency: NonZeroUsize,
        policy: DurableRetryPolicy,
        branches: Vec<DurableBranch<A, E, BodyError, R>>,
    ) -> Effect<BranchResults<A, E>, DurableRetryError<EngineError<R>, BodyError>, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        BodyError: Send + 'static,
        R: HasWorkflowEngine + HasTaskSpawner + Send + Sync + 'static,
    {
        let programs = branches
            .into_iter()
            .map(|branch| DurableRetry::run(run.clone(), branch, policy))
            .collect();
        Effect::all_concurrent(concurrency, programs)
    }

    /// Run bounded branches through the user's task-spawning capability.
    /// Each terminal domain outcome is recorded by its activity before it is
    /// included in the aggregate. Failures of the engine/body remain typed.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + HasTaskSpawner + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `concurrency` (`NonZeroUsize`): Positive bound on independently admitted work.
    /// - `branches` (`Vec<DurableBranch<A, E, BodyError, R>>`): The branches supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<BranchResults<A, E>, DurableParallelError<EngineError<R>, BodyError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn all<A, E, BodyError, R>(
        run: WorkflowRunId,
        attempt: NonZeroU32,
        concurrency: NonZeroUsize,
        branches: Vec<DurableBranch<A, E, BodyError, R>>,
    ) -> Effect<BranchResults<A, E>, DurableParallelError<EngineError<R>, BodyError>, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        BodyError: Send + 'static,
        R: HasWorkflowEngine + HasTaskSpawner + Send + Sync + 'static,
    {
        let programs = branches
            .into_iter()
            .map(|branch| {
                let activity = branch.id().clone();
                branch
                    .into_effect(run.clone(), attempt)
                    .map_error(move |source| DurableParallelError::Branch {
                        activity: activity.clone(),
                        source,
                    })
            })
            .collect();
        Effect::all_concurrent(concurrency, programs)
    }
}

#[derive(Debug)]
/// A branch or coordination failure in a durable race.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
/// - `BodyError`: Error produced by the registered workflow body.
pub enum DurableRaceError<EngineError, BodyError> {
    /// A branch failed during parallel or race coordination.
    Branch {
        /// Identity of the registered workflow activity. Type: `WorkflowActivityId`.
        activity: WorkflowActivityId,
        /// The underlying failure or provider cause. Type: `ActivityRunError<EngineError, BodyError>`.
        source: ActivityRunError<EngineError, BodyError>,
    },
    /// The workflow engine rejected or could not complete the operation. Retains the `EngineError` detail.
    Engine(EngineError),
    /// The winning branch may have committed; its record could not be confirmed.
    UncertainWinner {
        /// Stable identity of the recorded workflow race. Type: `WorkflowRaceId`.
        race: WorkflowRaceId,
        /// The underlying failure or provider cause. Type: `EngineError`.
        source: EngineError,
    },
    /// The recorded winner differs from the value produced during replay.
    RecordedWinnerMismatch,
    /// The recorded race winner is absent from this definition.
    RecordedWinnerUnknown,
    /// The recorded attempt differs from the value produced during replay.
    RecordedAttemptMismatch,
    /// An owned input was already consumed.
    InputConsumed,
}

/// A replayable race whose winning branch is recorded for later reconstruction.
pub struct DurableRace;

impl DurableRace {
    /// Replays a recorded winner without restarting the losing branch. For a
    /// fresh race, each branch records its own result before selection and the
    /// winner is committed before it is reported to the caller.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + HasTaskSpawner + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `race` (`WorkflowRaceId`): Stable identity or definition of the durable race.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `left` (`DurableBranch<A, E, BodyError, R>`): The left supplied to this operation.
    /// - `right` (`DurableBranch<A, E, BodyError, R>`): Right-hand operation composed with this one.
    ///
    /// # Returns
    /// Type: `Effect<Result<A, E>, DurableRaceError<EngineError<R>, BodyError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn first<A, E, BodyError, R>(
        run: WorkflowRunId,
        race: WorkflowRaceId,
        attempt: NonZeroU32,
        left: DurableBranch<A, E, BodyError, R>,
        right: DurableBranch<A, E, BodyError, R>,
    ) -> Effect<Result<A, E>, DurableRaceError<EngineError<R>, BodyError>, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        BodyError: Send + 'static,
        R: HasWorkflowEngine + HasTaskSpawner + Send + Sync + 'static,
    {
        let left_id = left.id().clone();
        let right_id = right.id().clone();
        let left_program = left
            .into_effect(run.clone(), attempt)
            .map_error({
                let id = left_id.clone();
                move |source| DurableRaceError::Branch {
                    activity: id.clone(),
                    source,
                }
            })
            .map({
                let id = left_id.clone();
                move |result| (id.clone(), result)
            });
        let right_program = right
            .into_effect(run.clone(), attempt)
            .map_error({
                let id = right_id.clone();
                move |source| DurableRaceError::Branch {
                    activity: id.clone(),
                    source,
                }
            })
            .map({
                let id = right_id.clone();
                move |result| (id.clone(), result)
            });
        Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
            let left = left_program.clone();
            let right = right_program.clone();
            let run = run.clone();
            let race = race.clone();
            let left_id = left_id.clone();
            let right_id = right_id.clone();
            engine
                .inspect_race(run.clone(), race.clone())
                .map_error(DurableRaceError::Engine)
                .provide_some(|_: &R| ())
                .flat_map(move |recorded| match recorded {
                    Some(recorded) if recorded.attempt != attempt => {
                        Effect::from_result_once(Err(DurableRaceError::RecordedAttemptMismatch))
                    }
                    Some(recorded) if recorded.winner == left_id => {
                        left.clone().map(|(_, result)| result)
                    }
                    Some(recorded) if recorded.winner == right_id => {
                        right.clone().map(|(_, result)| result)
                    }
                    Some(_) => {
                        Effect::from_result_once(Err(DurableRaceError::RecordedWinnerUnknown))
                    }
                    None => {
                        let engine = Arc::clone(&engine);
                        let run = run.clone();
                        let race = race.clone();
                        left.clone()
                            .race(right.clone())
                            .flat_map(move |(winner, result)| {
                                let record = RecordedRace {
                                    identity: race.clone(),
                                    winner,
                                    attempt,
                                };
                                let result = Arc::new(Mutex::new(Some(result)));
                                let race_on_error = race.clone();
                                let expected = record.clone();
                                engine
                                    .record_race(run.clone(), record)
                                    .map_error(move |source| DurableRaceError::UncertainWinner {
                                        race: race_on_error.clone(),
                                        source,
                                    })
                                    .provide_some(|_: &R| ())
                                    .flat_map(move |stored| {
                                        if stored != expected {
                                            return Effect::from_result_once(Err(
                                                DurableRaceError::RecordedWinnerMismatch,
                                            ));
                                        }
                                        let result = result
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .take()
                                            .ok_or(DurableRaceError::InputConsumed);
                                        Effect::from_result_once(result)
                                    })
                            })
                    }
                })
        })
    }
}
