use super::{
    Durable, HasWorkflowEngine, WorkflowCodecError, WorkflowDefinition, WorkflowEngine,
    WorkflowHandle, WorkflowRunId, WorkflowStatus,
};
use crate::{Effect, PrimaryOutcomeRef, Release};
use std::sync::{Arc, Mutex};

type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;

pub enum WorkflowRunResult<A, E> {
    Completed(Result<A, E>),
    Busy,
    Ready,
    Suspended,
    Cancelled,
    Abandoned,
}

pub enum WorkflowStep<A, E> {
    Completed(Result<A, E>),
    Suspended,
}

/// A registered definition together with its durable coordination body.
/// The input is moved into execution; replay decodes a fresh value from the
/// engine's recorded bytes, so callers never need to clone application data.
type TypedWorkflowBody<I, A, E, BodyError, R> =
    dyn Fn(I) -> Durable<Result<A, E>, BodyError, R> + Send + Sync;

pub struct TypedWorkflowDefinition<I, A, E, BodyError, R>
where
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: HasWorkflowEngine + Send + Sync + 'static,
{
    pub definition: WorkflowDefinition<I, A, E>,
    body: Arc<TypedWorkflowBody<I, A, E, BodyError, R>>,
}

impl<I, A, E, BodyError, R> TypedWorkflowDefinition<I, A, E, BodyError, R>
where
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: HasWorkflowEngine + Send + Sync + 'static,
{
    pub fn new(
        definition: WorkflowDefinition<I, A, E>,
        body: impl Fn(I) -> Durable<Result<A, E>, BodyError, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            definition,
            body: Arc::new(body),
        }
    }

    pub fn run(
        &self,
        run: WorkflowRunId,
        input: I,
    ) -> Effect<WorkflowRunResult<A, E>, WorkflowRunError<EngineError<R>, BodyError>, R> {
        let body = Arc::clone(&self.body);
        self.definition
            .execute(run, input, move |value| body(value))
    }
}

#[derive(Debug)]
pub enum WorkflowRunError<EngineError, BodyError> {
    Engine(EngineError),
    Body(BodyError),
    Codec(WorkflowCodecError),
    Recovery {
        body: Option<BodyError>,
        source: EngineError,
    },
    RecoveryTransitionMismatch {
        body: Option<BodyError>,
    },
    UncertainCompletion {
        run: WorkflowRunId,
        source: EngineError,
    },
    StartedRunMismatch,
    RunningTransitionMismatch,
    RecordedCompletionMismatch,
    InputConsumed,
    UnregisteredSuspension,
}

impl<I: Send + 'static, A: Send + 'static, E: Send + 'static> WorkflowDefinition<I, A, E> {
    /// Register, start or resume, and execute this definition without a
    /// Cluster dependency. The engine is the sole execution-record authority.
    /// Activities invoked by `body` consult that same engine for replay.
    pub fn execute<R, BodyError>(
        &self,
        run: WorkflowRunId,
        input: I,
        body: impl Fn(I) -> Durable<Result<A, E>, BodyError, R> + Send + Sync + 'static,
    ) -> Effect<WorkflowRunResult<A, E>, WorkflowRunError<EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
        BodyError: Send + 'static,
    {
        self.execute_resumable(run, input, move |value| {
            body(value).map(WorkflowStep::Completed)
        })
    }

    /// A suspended step leaves its activities and signals recorded without
    /// writing a terminal result. A later signal or deadline can mark it
    /// ready for deterministic coordination replay.
    pub fn execute_resumable<R, BodyError>(
        &self,
        run: WorkflowRunId,
        input: I,
        body: impl Fn(I) -> Durable<WorkflowStep<A, E>, BodyError, R> + Send + Sync + 'static,
    ) -> Effect<WorkflowRunResult<A, E>, WorkflowRunError<EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
        BodyError: Send + 'static,
    {
        let definition = self.clone();
        let run_definition = self.clone();
        let input = Mutex::new(Some(input));
        let body = Arc::new(body);
        let handle = WorkflowHandle {
            run: run.clone(),
            definition: definition.id.clone(),
            version: definition.version,
        };
        Effect::try_sync(move || {
            let input = input.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take().ok_or(WorkflowRunError::InputConsumed)?;
            definition.encode_input_bytes(&input).map_err(WorkflowRunError::Codec)
        }, std::convert::identity)
        .flat_map(move |encoded_input| {
            let definition = run_definition.clone();
            let body = Arc::clone(&body);
            let handle = handle.clone();
            Effect::service(|env: &R| env.workflow_engine())
                .flat_map(move |engine| {
                    let definition = definition.clone();
                    let body = Arc::clone(&body);
                    let handle = handle.clone();
                    let encoded_input = Arc::clone(&encoded_input);
                    engine.register(definition.registration())
                        .map_error(WorkflowRunError::Engine)
                        .provide_some(|_: &R| ())
                        .flat_map(move |_| {
                            let definition = definition.clone();
                            let body = Arc::clone(&body);
                            let engine = Arc::clone(&engine);
                            let handle_for_run = handle.clone();
                            let expected_input = Arc::clone(&encoded_input);
                            engine.start(handle.clone(), Arc::clone(&encoded_input))
                                .map_error(WorkflowRunError::Engine)
                                .provide_some(|_: &R| ())
                                .flat_map(move |snapshot| {
                                    if snapshot.handle != handle_for_run
                                        || snapshot.compatibility != definition.compatibility
                                        || snapshot.encoded_input != expected_input {
                                        return Effect::from_result_once(Err(
                                            WorkflowRunError::StartedRunMismatch
                                        ));
                                    }
                                    match snapshot.status {
                                    WorkflowStatus::Completed(ref outcome) => {
                                        Effect::from_result_once(
                                            definition.decode_outcome(outcome)
                                                .map(WorkflowRunResult::Completed)
                                                .map_err(WorkflowRunError::Codec),
                                        )
                                    }
                                    WorkflowStatus::Running => {
                                        Effect::sync(|| WorkflowRunResult::Busy)
                                    }
                                    WorkflowStatus::Suspended => {
                                        Effect::sync(|| WorkflowRunResult::Suspended)
                                    }
                                    WorkflowStatus::Cancelled => {
                                        Effect::sync(|| WorkflowRunResult::Cancelled)
                                    }
                                    WorkflowStatus::Abandoned => {
                                        Effect::sync(|| WorkflowRunResult::Abandoned)
                                    }
                                    WorkflowStatus::Ready => {
                                        let definition = definition.clone();
                                        let body = Arc::clone(&body);
                                        let engine = Arc::clone(&engine);
                                        let run = handle_for_run.run.clone();
                                        let input = match definition
                                            .decode_input_bytes(&snapshot.encoded_input) {
                                            Ok(value) => value,
                                            Err(error) => return Effect::from_result_once(
                                                Err(WorkflowRunError::Codec(error))
                                            ),
                                        };
                                        let input = Arc::new(Mutex::new(Some(input)));
                                        engine.transition(
                                            run.clone(), WorkflowStatus::Ready,
                                            WorkflowStatus::Running,
                                        ).map_error(WorkflowRunError::Engine)
                                            .provide_some(|_: &R| ())
                                            .flat_map(move |transitioned| {
                                                if transitioned.handle.run != run
                                                    || transitioned.status != WorkflowStatus::Running {
                                                    return Effect::from_result_once(Err(
                                                        WorkflowRunError::RunningTransitionMismatch
                                                    ));
                                                }
                                                let input = match input.lock()
                                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                    .take() {
                                                    Some(value) => value,
                                                    None => return Effect::from_result_once(
                                                        Err(WorkflowRunError::InputConsumed)
                                                    ),
                                                };
                                                let engine_for_failure = Arc::clone(&engine);
                                                let failure_run = run.clone();
                                                let engine_for_interruption = Arc::clone(&engine);
                                                let interrupted_run = run.clone();
                                                let definition = definition.clone();
                                                let engine_for_completion = Arc::clone(&engine);
                                                let run_for_result = run.clone();
                                                body(input).into_effect()
                                                    .map_error(WorkflowRunError::Body)
                                                    .ensuring_with(move |outcome| match outcome {
                                                        PrimaryOutcomeRef::Defected(_)
                                                        | PrimaryOutcomeRef::WasInterrupted => {
                                                            let expected_run = interrupted_run.clone();
                                                            Release::from_effect(
                                                                engine_for_interruption.transition(
                                                                    interrupted_run.clone(),
                                                                    WorkflowStatus::Running,
                                                                    WorkflowStatus::Ready,
                                                                )
                                                                .map_error(WorkflowRunError::Engine)
                                                                .provide_some(|_: &R| ())
                                                                .flat_map(move |snapshot| {
                                                                    if snapshot.handle.run != expected_run
                                                                        || snapshot.status != WorkflowStatus::Ready {
                                                                        Effect::from_result_once(Err(
                                                                            WorkflowRunError::RecoveryTransitionMismatch {
                                                                                body: None,
                                                                            },
                                                                        ))
                                                                    } else {
                                                                        Effect::sync(|| ())
                                                                    }
                                                                }),
                                                            )
                                                        }
                                                        PrimaryOutcomeRef::Succeeded
                                                        | PrimaryOutcomeRef::Failed(_) => Release::sync(|| ()),
                                                    })
                                                    .catch_all(move |failure| {
                                                        let WorkflowRunError::Body(body) = failure
                                                            else { return Effect::from_result_once(
                                                                Err(failure)
                                                            ); };
                                                        let body = Arc::new(Mutex::new(Some(body)));
                                                        let body_for_error = Arc::clone(&body);
                                                        let expected_run = failure_run.clone();
                                                        engine_for_failure.transition(
                                                            failure_run.clone(),
                                                            WorkflowStatus::Running,
                                                            WorkflowStatus::Ready,
                                                        ).map_error(move |source| {
                                                            let body = body_for_error.lock()
                                                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                                .take();
                                                            WorkflowRunError::Recovery { body, source }
                                                        }).provide_some(|_: &R| ())
                                                            .flat_map(move |snapshot| {
                                                                let body = body.lock()
                                                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                                    .take();
                                                                if snapshot.handle.run != expected_run
                                                                    || snapshot.status != WorkflowStatus::Ready {
                                                                    return Effect::from_result_once(Err(
                                                                        WorkflowRunError::RecoveryTransitionMismatch { body }
                                                                    ));
                                                                }
                                                                let body = body.ok_or(WorkflowRunError::InputConsumed);
                                                                Effect::from_result_once(Err(
                                                                    match body {
                                                                        Ok(body) => WorkflowRunError::Body(body),
                                                                        Err(error) => error,
                                                                    }
                                                                ))
                                                            })
                                                    })
                                                    .flat_map(move |step| match step {
                                                        WorkflowStep::Suspended => {
                                                            engine_for_completion.inspect(run_for_result.clone())
                                                                .map_error(WorkflowRunError::Engine)
                                                                .provide_some(|_: &R| ())
                                                                .flat_map(|snapshot| match snapshot
                                                                    .map(|value| value.status) {
                                                                    Some(WorkflowStatus::Suspended) => {
                                                                        Effect::sync(|| WorkflowRunResult::Suspended)
                                                                    }
                                                                    Some(WorkflowStatus::Ready) => {
                                                                        Effect::sync(|| WorkflowRunResult::Ready)
                                                                    }
                                                                    _ => Effect::from_result_once(Err(
                                                                        WorkflowRunError::UnregisteredSuspension
                                                                    )),
                                                                })
                                                        }
                                                        WorkflowStep::Completed(outcome) => {
                                                        let encoded = match definition
                                                            .encode_outcome(&outcome) {
                                                            Ok(value) => value,
                                                            Err(error) => return Effect::from_result_once(
                                                                Err(WorkflowRunError::Codec(error))
                                                            ),
                                                        };
                                                        let outcome = Arc::new(Mutex::new(Some(outcome)));
                                                        let failed_run = run_for_result.clone();
                                                        let expected_run = run_for_result.clone();
                                                        let expected = encoded.clone();
                                                        engine_for_completion.complete(
                                                            run_for_result.clone(), encoded,
                                                        ).map_error(move |source| {
                                                            WorkflowRunError::UncertainCompletion {
                                                                run: failed_run.clone(), source,
                                                            }
                                                        }).provide_some(|_: &R| ())
                                                            .flat_map(move |snapshot| {
                                                                if snapshot.handle.run != expected_run
                                                                    || !matches!(snapshot.status,
                                                                        WorkflowStatus::Completed(ref recorded)
                                                                            if recorded == &expected) {
                                                                    return Effect::from_result_once(Err(
                                                                        WorkflowRunError::RecordedCompletionMismatch
                                                                    ));
                                                                }
                                                                let result = outcome.lock()
                                                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                                    .take()
                                                                    .map(WorkflowRunResult::Completed)
                                                                    .ok_or(WorkflowRunError::InputConsumed);
                                                                Effect::from_result_once(result)
                                                            })
                                                        }
                                                    })
                                            })
                                    }
                                    }
                                })
                        })
                })
        })
    }
}
