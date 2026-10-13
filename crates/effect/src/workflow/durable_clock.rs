//! Named recoverable workflow deadlines.

use super::{
    DeadlineClear, DeadlineWait, DeadlineWake, HasWorkflowEngine, RecordedDeadline, WorkflowEngine,
    WorkflowRunId, WorkflowSnapshot,
};
use crate::{Effect, HasWallClock, WallClock};
use crate::{effect, effect_map_error, effect_provide_some, service};

#[derive(Debug)]
/// An engine or time-provider failure while waiting on a recorded deadline.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
/// - `ClockError`: Error returned by the clock capability.
pub enum DurableClockError<EngineError, ClockError> {
    /// The workflow engine rejected or could not complete the operation. Retains the `EngineError` detail.
    Engine(EngineError),
    /// The selected clock could not schedule or observe the requested time. Retains the `ClockError` detail.
    Clock(ClockError),
}

/// Type alias for durable clock effect in workflow durable clock.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `R`: Environment type.
pub type DurableClockEffect<A, R> = Effect<
    A,
    DurableClockError<
        <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error,
        <<R as HasWallClock>::Clock as WallClock>::Error,
    >,
    R,
>;

/// Deadline coordination is expressed through the environment's wall clock
/// and workflow engine. Local sleeps may use any runtime, but cannot be the
/// authority for recoverable workflow deadlines.
pub struct DurableClock;

impl DurableClock {
    /// Record and wait for a durable deadline using the workflow engine and clock.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + HasWallClock + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `deadline` (`RecordedDeadline`): The deadline for the pending operation.
    ///
    /// # Returns
    /// Type: `DurableClockEffect<DeadlineWait, R>`.
    pub fn wait_until<R>(
        run: WorkflowRunId,
        deadline: RecordedDeadline,
    ) -> DurableClockEffect<DeadlineWait, R>
    where
        R: HasWorkflowEngine + HasWallClock + Send + Sync + 'static,
    {
        {
            let __dsl_program =
                effect_map_error! { program: Effect::wall_now(), map: DurableClockError::Clock, };
            effect! { let deadline = deadline.clone(); let run = run.clone(); let now = __dsl_program.clone()?; {
                            { let __dsl_program = service! { env: |env: &R| env.workflow_engine(), }; let run = run.clone();
            let deadline = deadline.clone(); effect! { let __dsl_program = __dsl_program.clone(); let deadline = deadline.clone(); let run = run.clone(); let engine = __dsl_program?; {
                                    effect_provide_some! { program: effect_map_error! { program: engine
                                        .wait_deadline(run.clone(), deadline.clone(), now), map: DurableClockError::Engine, }, project: |_: &R| (), }
                                }? } }
                        }? }
        }
    }

    /// Record a deadline so replay can recover the same scheduling decision.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `deadline` (`RecordedDeadline`): The deadline for the pending operation.
    ///
    /// # Returns
    /// Type: `Effect<WorkflowSnapshot, <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn schedule<R>(
        run: WorkflowRunId,
        deadline: RecordedDeadline,
    ) -> Effect<WorkflowSnapshot, <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        {
            let __dsl_program = service! { env: |env: &R| env.workflow_engine(), };
            effect! { let deadline = deadline.clone(); let run = run.clone(); let engine = __dsl_program.clone()?; {
                effect_provide_some! { program: engine
                    .set_deadline(run.clone(), deadline.clone()), project: |_: &R| (), }
            }? }
        }
    }

    /// Remove a recorded deadline after its wait is no longer needed.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `expected` (`RecordedDeadline`): Value or revision required for the proposed state transition.
    ///
    /// # Returns
    /// Type: `Effect<DeadlineClear, <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn clear<R>(
        run: WorkflowRunId,
        expected: RecordedDeadline,
    ) -> Effect<DeadlineClear, <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        {
            let __dsl_program = service! { env: |env: &R| env.workflow_engine(), };
            effect! { let expected = expected.clone(); let run = run.clone(); let engine = __dsl_program.clone()?; {
                effect_provide_some! { program: engine
                    .clear_deadline(run.clone(), expected.clone()), project: |_: &R| (), }
            }? }
        }
    }

    /// Resume recorded waits whose deadlines have become due.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + HasWallClock + Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `DurableClockEffect<Vec<WorkflowRunId>, R>`.
    pub fn wake_due<R>() -> DurableClockEffect<Vec<WorkflowRunId>, R>
    where
        R: HasWorkflowEngine + HasWallClock + Send + Sync + 'static,
    {
        {
            let __dsl_program =
                effect_map_error! { program: Effect::wall_now(), map: DurableClockError::Clock, };
            effect! { let now = __dsl_program.clone()?; {
                { let __dsl_program = service! { env: |env: &R| env.workflow_engine(), }; effect! { let engine = __dsl_program.clone()?; {
                    effect_provide_some! { program: effect_map_error! { program: engine
                        .wake_due(now), map: DurableClockError::Engine, }, project: |_: &R| (), }
                }? } }
            }? }
        }
    }

    /// Wake one recorded deadline only if it has not been rescheduled.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + HasWallClock + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `expected` (`RecordedDeadline`): Value or revision required for the proposed state transition.
    ///
    /// # Returns
    /// Type: `DurableClockEffect<DeadlineWake, R>`.
    pub fn wake_one<R>(
        run: WorkflowRunId,
        expected: RecordedDeadline,
    ) -> DurableClockEffect<DeadlineWake, R>
    where
        R: HasWorkflowEngine + HasWallClock + Send + Sync + 'static,
    {
        {
            let __dsl_program =
                effect_map_error! { program: Effect::wall_now(), map: DurableClockError::Clock, };
            effect! { let expected = expected.clone(); let run = run.clone(); let now = __dsl_program.clone()?; {
                            { let __dsl_program = service! { env: |env: &R| env.workflow_engine(), }; let run = run.clone();
            let expected = expected.clone(); effect! { let __dsl_program = __dsl_program.clone(); let expected = expected.clone(); let run = run.clone(); let engine = __dsl_program?; {
                                    effect_provide_some! { program: effect_map_error! { program: engine
                                        .wake_deadline(run.clone(), expected.clone(), now), map: DurableClockError::Engine, }, project: |_: &R| (), }
                                }? } }
                        }? }
        }
    }
}
