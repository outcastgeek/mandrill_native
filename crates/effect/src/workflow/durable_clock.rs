use super::{
    DeadlineClear, DeadlineWait, DeadlineWake, HasWorkflowEngine, RecordedDeadline, WorkflowEngine,
    WorkflowRunId, WorkflowSnapshot,
};
use crate::{Effect, HasWallClock, WallClock};

#[derive(Debug)]
pub enum DurableClockError<EngineError, ClockError> {
    Engine(EngineError),
    Clock(ClockError),
}

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
    pub fn wait_until<R>(
        run: WorkflowRunId,
        deadline: RecordedDeadline,
    ) -> DurableClockEffect<DeadlineWait, R>
    where
        R: HasWorkflowEngine + HasWallClock + Send + Sync + 'static,
    {
        Effect::wall_now()
            .map_error(DurableClockError::Clock)
            .flat_map(move |now| {
                Effect::service(|env: &R| env.workflow_engine()).flat_map({
                    let run = run.clone();
                    let deadline = deadline.clone();
                    move |engine| {
                        engine
                            .wait_deadline(run.clone(), deadline.clone(), now)
                            .map_error(DurableClockError::Engine)
                            .provide_some(|_: &R| ())
                    }
                })
            })
    }

    pub fn schedule<R>(
        run: WorkflowRunId,
        deadline: RecordedDeadline,
    ) -> Effect<WorkflowSnapshot, <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
            engine
                .set_deadline(run.clone(), deadline.clone())
                .provide_some(|_: &R| ())
        })
    }

    pub fn clear<R>(
        run: WorkflowRunId,
        expected: RecordedDeadline,
    ) -> Effect<DeadlineClear, <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
            engine
                .clear_deadline(run.clone(), expected.clone())
                .provide_some(|_: &R| ())
        })
    }

    pub fn wake_due<R>() -> DurableClockEffect<Vec<WorkflowRunId>, R>
    where
        R: HasWorkflowEngine + HasWallClock + Send + Sync + 'static,
    {
        Effect::wall_now()
            .map_error(DurableClockError::Clock)
            .flat_map(|now| {
                Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
                    engine
                        .wake_due(now)
                        .map_error(DurableClockError::Engine)
                        .provide_some(|_: &R| ())
                })
            })
    }

    /// Wake one recorded deadline only if it has not been rescheduled.
    pub fn wake_one<R>(
        run: WorkflowRunId,
        expected: RecordedDeadline,
    ) -> DurableClockEffect<DeadlineWake, R>
    where
        R: HasWorkflowEngine + HasWallClock + Send + Sync + 'static,
    {
        Effect::wall_now()
            .map_error(DurableClockError::Clock)
            .flat_map(move |now| {
                Effect::service(|env: &R| env.workflow_engine()).flat_map({
                    let run = run.clone();
                    let expected = expected.clone();
                    move |engine| {
                        engine
                            .wake_deadline(run.clone(), expected.clone(), now)
                            .map_error(DurableClockError::Engine)
                            .provide_some(|_: &R| ())
                    }
                })
            })
    }
}
