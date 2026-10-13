//! Workflow registration, replay, and typed handles.

use super::{
    Durable, HasWorkflowEngine, WorkflowCheckpoint, WorkflowCodecError, WorkflowDefinition,
    WorkflowEngine, WorkflowHandle, WorkflowRunId, WorkflowSnapshot, WorkflowStatus,
};
use crate::{Effect, PrimaryOutcomeRef, Release};
use crate::{
    effect, effect_map_error, effect_provide_some, effect_recover, effect_resource, effect_sync,
    effect_try_sync, service,
};
use std::sync::{Arc, Mutex};

/// The selected engine's error type remains part of the environment contract.
///
/// # Type parameters
/// - `R`: Environment type.
type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;

/// The typed status or terminal value returned when executing a workflow definition.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub enum WorkflowRunResult<A, E> {
    /// The operation completed and its outcome is available. Carries `Result<A, E>`.
    Completed(Result<A, E>),
    /// The requested work is already in progress.
    Busy,
    /// The operation is ready to continue.
    Ready,
    /// Execution is suspended until a durable wakeup.
    Suspended,
    /// Execution was cancelled.
    Cancelled,
    /// The owner was lost before the operation settled.
    Abandoned,
}

/// One replayable coordination operation in a durable workflow.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub enum WorkflowStep<A, E> {
    /// The operation completed and its outcome is available. Carries `Result<A, E>`.
    Completed(Result<A, E>),
    /// Execution is suspended until a durable wakeup.
    Suspended,
}

/// A registered definition together with its durable coordination body.
/// The input is moved into execution; replay decodes a fresh value from the
/// engine's recorded bytes, so callers never need to clone application data.
/// The body is dispatched through a local Rust vtable. Replay restores data
/// and re-enters registered code; it never persists the closure or vtable.
///
/// # Type parameters
/// - `I`: Input type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `BodyError`: Error produced by the registered workflow body.
/// - `R`: Environment type.
type TypedWorkflowBody<I, A, E, BodyError, R> =
    dyn Fn(I) -> Durable<Result<A, E>, BodyError, R> + Send + Sync;

/// Binds a typed workflow definition to its durable execution body.
///
/// # Type parameters
/// - `I`: Input type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `BodyError`: Error type produced by the workflow body.
/// - `R`: Environment type.
pub struct TypedWorkflowDefinition<I, A, E, BodyError, R>
where
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: HasWorkflowEngine + Send + Sync + 'static,
{
    /// Registered workflow identity and input/output codecs. Type: `WorkflowDefinition<I, A, E>`.
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
    /// Construct a `TypedWorkflowDefinition` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `BodyError`: Error produced by the registered workflow body.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `I`: Input type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `definition` (`WorkflowDefinition<I, A, E>`): The typed definition used by this operation.
    /// - `body` (`impl Fn(I) -> Durable<Result<A, E>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        definition: WorkflowDefinition<I, A, E>,
        body: impl Fn(I) -> Durable<Result<A, E>, BodyError, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            definition,
            body: Arc::new(body),
        }
    }

    /// Create a reusable execution recipe for this typed definition and run ID.
    ///
    /// Encoded input is retained after the first execution and fresh owned input is
    /// reconstructed per body attempt. Failed reset recovery keeps its exact checkpoint
    /// for retry; completed runs return the recorded outcome without invoking the body.
    /// The engine in R owns replay history and determines storage durability.
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
/// An engine, body, codec, or replay mismatch failure for a workflow run.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
/// - `BodyError`: Error produced by the registered workflow body.
pub enum WorkflowRunError<EngineError, BodyError> {
    /// The workflow engine failed while recording or replaying the run.
    Engine(EngineError),
    /// The registered workflow body failed.
    Body(BodyError),
    /// Encoding or decoding workflow data failed.
    Codec(WorkflowCodecError),
    /// Reset to Ready failed; the retained run/checkpoint identify the exact execution eligible for retry.
    Recovery {
        /// Original body failure when present; protected interruption recovery may have no body error.
        body: Option<BodyError>,
        /// The underlying failure or provider cause. Type: `EngineError`.
        source: EngineError,
        /// Exact execution ownership retained for retry.
        checkpoint: WorkflowCheckpoint,
        /// Run whose reset failed.
        run: WorkflowRunId,
    },
    /// The recovery transition differs from the required or recorded value.
    RecoveryTransitionMismatch {
        /// Application body invoked by this definition. Type: `Option<BodyError>`.
        body: Option<BodyError>,
    },
    /// Completion may have committed; the recorded outcome could not be confirmed.
    UncertainCompletion {
        /// Stable identity of the workflow run. Type: `WorkflowRunId`.
        run: WorkflowRunId,
        /// The underlying failure or provider cause. Type: `EngineError`.
        source: EngineError,
    },
    /// The started run differs from the required or recorded value.
    StartedRunMismatch,
    /// The running transition differs from the required or recorded value.
    RunningTransitionMismatch,
    /// The recorded completion differs from the value produced during replay.
    RecordedCompletionMismatch,
    /// An owned input was already consumed.
    InputConsumed,
    /// A workflow suspended on a step absent from its registered definition.
    UnregisteredSuspension,
}

impl<I: Send + 'static, A: Send + 'static, E: Send + 'static> WorkflowDefinition<I, A, E> {
    /// Lazily register, start or replay this definition using R's workflow engine.
    ///
    /// The returned run effect caches encoded input and retains exact failed-reset
    /// ownership for retry. Each body attempt decodes a fresh owned input; completed
    /// runs return their recorded outcome without rerunning the body. Activities use
    /// the same engine replay history. No Clone payload or Cluster runtime is required.
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

    /// Execute a body that may suspend on a registered workflow wait, preserving replay ownership.
    ///
    /// Input is encoded once per reusable run recipe and reconstructed for retries.
    /// Body failure or interruption resets only its exact execution checkpoint;
    /// failed reset ownership is retained for later Effect::retry. Unrelated Running
    /// work remains busy, and a completed run is acknowledged from recorded state.
    /// Physical persistence depends on the selected engine.
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
        let input = Mutex::new((Some(input), None::<Arc<[u8]>>));
        let pending_reset = Arc::new(Mutex::new(None::<WorkflowCheckpoint>));
        let body = Arc::new(body);
        let handle = WorkflowHandle {
            run: run.clone(),
            definition: definition.id.clone(),
            version: definition.version,
        };
        {
            let __dsl_program = effect_try_sync! {
                body: move || {
                let mut state = input.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(bytes) = &state.1 { return Ok(Arc::clone(bytes)); }
                let value = state.0.take().ok_or(WorkflowRunError::InputConsumed)?;
                drop(state);
                let bytes = definition.encode_input_bytes(&value).map_err(WorkflowRunError::Codec)?;
                input.lock().unwrap_or_else(std::sync::PoisonError::into_inner).1 = Some(Arc::clone(&bytes));
                Ok(bytes)
            }, on_error: std::convert::identity, };
            effect! {
                        let __dsl_program = __dsl_program.clone(); let body = body.clone(); let handle = handle.clone(); let pending_reset = pending_reset.clone(); let run_definition = run_definition.clone(); let encoded_input = __dsl_program?; {
                        let definition = run_definition.clone();
                        let body = Arc::clone(&body);
                        let pending_reset = Arc::clone(&pending_reset);
                        let handle = handle.clone();
                        {
                            let __dsl_program = service! { env: |env: &R| env.workflow_engine(), }; effect! {
                            let __dsl_program = __dsl_program.clone(); let body = body.clone(); let definition = definition.clone(); let encoded_input = encoded_input.clone(); let handle = handle.clone(); let pending_reset = pending_reset.clone(); let engine = __dsl_program?; {
                                let definition = definition.clone();
                                let body = Arc::clone(&body);
                        let pending_reset = Arc::clone(&pending_reset);
                                let handle = handle.clone();
                                let encoded_input = Arc::clone(&encoded_input);
                                {
                                    let __dsl_program = effect_provide_some! { program: effect_map_error! { program: engine.register(definition.registration()), map: WorkflowRunError::Engine, }, project: |_: &R| (), }; effect! {
                                    let __dsl_program = __dsl_program.clone(); let body = body.clone(); let definition = definition.clone(); let encoded_input = encoded_input.clone(); let engine = engine.clone(); let handle = handle.clone(); let pending_reset = pending_reset.clone(); let _ = __dsl_program?; {
                                        let definition = definition.clone();
                                        let body = Arc::clone(&body);
                        let pending_reset = Arc::clone(&pending_reset);
                                        let engine = Arc::clone(&engine);
                                        let handle_for_run = handle.clone();
                                        let expected_input = Arc::clone(&encoded_input);
                                        {
                                            let __dsl_program = {
                                            let __dsl_program = effect_provide_some! { program: effect_map_error! { program: engine.start(handle.clone(), Arc::clone(&encoded_input)), map: WorkflowRunError::Engine, }, project: |_: &R| (), }; let engine=Arc::clone(&engine);
            let pending=Arc::clone(&pending_reset);
            let expected_handle=handle_for_run.clone();
            let compatibility=Arc::clone(&definition.compatibility);
            let input=Arc::clone(&expected_input); effect! { let __dsl_program = __dsl_program.clone(); let compatibility = compatibility.clone(); let engine = engine.clone(); let expected_handle = expected_handle.clone(); let input = input.clone(); let pending = pending.clone(); let snapshot = __dsl_program?; (move || {if snapshot.handle!=expected_handle||snapshot.compatibility!=compatibility||snapshot.encoded_input!=input{return effect_try_sync! { once; body: { let __dsl_result = Err(WorkflowRunError::StartedRunMismatch); move || __dsl_result }, on_error: ::core::convert::identity, };}recover_pending::<R,BodyError>(Arc::clone(&engine),snapshot,Arc::clone(&pending))})()? } }; effect! {
                let __dsl_program = __dsl_program.clone(); let body = body.clone(); let definition = definition.clone(); let engine = engine.clone(); let expected_input = expected_input.clone(); let handle_for_run = handle_for_run.clone(); let pending_reset = pending_reset.clone(); let snapshot = __dsl_program?; (move || {
                                                if snapshot.handle != handle_for_run
                                                    || snapshot.compatibility != definition.compatibility
                                                    || snapshot.encoded_input != expected_input {
                                                    return effect_try_sync! {
                                                        once; body: {
                                                        let __dsl_result = Err(
                                                        WorkflowRunError::StartedRunMismatch
                                                    ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                                }
                                                match snapshot.status {
                                                WorkflowStatus::Completed(ref outcome) => {
                                                    effect_try_sync! {
                                                        once; body: {
                                                        let __dsl_result = definition.decode_outcome(outcome)
                                                            .map(WorkflowRunResult::Completed)
                                                            .map_err(WorkflowRunError::Codec); move || __dsl_result }, on_error: ::core::convert::identity, }
                                                }
                                                WorkflowStatus::Running => {
                                                    effect_sync! { body: || WorkflowRunResult::Busy, }
                                                }
                                                WorkflowStatus::Suspended => {
                                                    effect_sync! { body: || WorkflowRunResult::Suspended, }
                                                }
                                                WorkflowStatus::Cancelled => {
                                                    effect_sync! { body: || WorkflowRunResult::Cancelled, }
                                                }
                                                WorkflowStatus::Abandoned => {
                                                    effect_sync! { body: || WorkflowRunResult::Abandoned, }
                                                }
                                                WorkflowStatus::Ready => {
                                                    let definition = definition.clone();
                                                    let body = Arc::clone(&body);
                        let pending_reset = Arc::clone(&pending_reset);
                                                    let engine = Arc::clone(&engine);
                                                    let run = handle_for_run.run.clone();
                                                    let input = match definition
                                                        .decode_input_bytes(&snapshot.encoded_input) {
                                                        Ok(value) => value,
                                                        Err(error) => return effect_try_sync! { once; body: { let __dsl_result = Err(WorkflowRunError::Codec(error)); move || __dsl_result }, on_error: ::core::convert::identity, },
                                                    };
                                                    let input = Arc::new(Mutex::new(Some(input)));
                                                    {
                                                        let __dsl_program = effect_provide_some! {
                                                        program: effect_map_error! {
                                                        program: engine.transition(
                                                        run.clone(), snapshot.checkpoint(),
                                                        WorkflowStatus::Running,
                                                    ), map: WorkflowRunError::Engine, }, project: |_: &R| (), }; effect! {
                                                        let __dsl_program = __dsl_program.clone(); let body = body.clone(); let definition = definition.clone(); let engine = engine.clone(); let input = input.clone(); let pending_reset = pending_reset.clone(); let run = run.clone(); let transitioned = __dsl_program.clone()?; (move || {
                                                            if transitioned.handle.run != run
                                                                || transitioned.status != WorkflowStatus::Running {
                                                                return effect_try_sync! {
                                                                    once; body: {
                                                                    let __dsl_result = Err(
                                                                    WorkflowRunError::RunningTransitionMismatch
                                                                ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                                            }
                                        let owned_input = input.lock()
                                                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .take();
                                        let input = match owned_input {
                                                                Some(value) => value,
                                                                None => return effect_try_sync! { once; body: { let __dsl_result = Err(WorkflowRunError::InputConsumed); move || __dsl_result }, on_error: ::core::convert::identity, },
                                                            };
                                                            let failure_checkpoint=transitioned.checkpoint();let interruption_checkpoint=failure_checkpoint.clone();let expected_execution=transitioned.execution;
                                                            let pending_failure=Arc::clone(&pending_reset);let pending_interruption=Arc::clone(&pending_reset);
                                                            let engine_for_failure = Arc::clone(&engine);
                                                            let failure_run = run.clone();
                                                            let engine_for_interruption = Arc::clone(&engine);
                                                            let interrupted_run = run.clone();
                                                            let definition = definition.clone();
                                                            let engine_for_completion = Arc::clone(&engine);
                                                            let run_for_result = run.clone();
                                                            let body=Arc::clone(&body);let input=Arc::new(Mutex::new(Some(input)));
                                                            {
                                                                let __dsl_program = effect_recover! {
                                                                program: {
                                                                let __dsl_program = { let __dsl_factory = move ||{let input=input.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();match input{Some(input)=>effect_map_error! { program: body(input).into_effect(), map: WorkflowRunError::Body, },None=>effect_try_sync! { once; body: { let __dsl_result = Err(WorkflowRunError::InputConsumed); move || __dsl_result }, on_error: ::core::convert::identity, }}}; effect! { __dsl_factory()? } }; effect_resource! {
                                                                acquire: effect_sync! { body: || (), }, release: move |(), outcome| match outcome {
                                                                    PrimaryOutcomeRef::Defected(_)
                                                                    | PrimaryOutcomeRef::WasInterrupted => {
                                                                        let expected_run = interrupted_run.clone();
                                                                        *pending_interruption.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=Some(interruption_checkpoint.clone());
                                                                        let pending=Arc::clone(&pending_interruption);let failed_run=interrupted_run.clone();let checkpoint=interruption_checkpoint.clone();
                                                                        Release::from_effect(
                                                                            {
                                                                                let __dsl_program = effect_provide_some! {
                                                                                program: effect_map_error! {
                                                                                program: engine_for_interruption.transition(
                                                                                interrupted_run.clone(),
                                                                                interruption_checkpoint.clone(),
                                                                                WorkflowStatus::Ready,
                                                                            ), map: move |source| WorkflowRunError::Recovery {body:None,source,run:failed_run.clone(),checkpoint:checkpoint.clone()}, }, project: |_: &R| (), }; effect! {
                                                                                let expected_run = expected_run.clone(); let pending = pending.clone(); let snapshot = __dsl_program.clone()?; {
                                                                                if snapshot.handle.run != expected_run
                                                                                    || snapshot.status != WorkflowStatus::Ready || snapshot.execution!=expected_execution {
                                                                                    effect_try_sync! {
                                                                                        once; body: {
                                                                                        let __dsl_result = Err(
                                                                                        WorkflowRunError::RecoveryTransitionMismatch {
                                                                                            body: None,
                                                                                        },
                                                                                    ); move || __dsl_result }, on_error: ::core::convert::identity, }
                                                                                } else {
                                                                                    *pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=None;
                                                                                    effect_sync! { body: || (), }
                                                                                }
                                                                            }? } },
                                                                        )
                                                                    }
                                                                    PrimaryOutcomeRef::Succeeded
                                                                    | PrimaryOutcomeRef::Failed(_) => Release::sync(|| ()),
                                                                }, body: move |_| __dsl_program.clone(), } }, recover: move |failure| {
                                                                    let WorkflowRunError::Body(body) = failure
                                                                        else { return effect_try_sync! { once; body: { let __dsl_result = Err(failure); move || __dsl_result }, on_error: ::core::convert::identity, }; };
                                                                    *pending_failure.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=Some(failure_checkpoint.clone());
                                                                    let pending=Arc::clone(&pending_failure);let failed_run=failure_run.clone();let checkpoint=failure_checkpoint.clone();
                                                                    let body = Arc::new(Mutex::new(Some(body)));
                                                                    let body_for_error = Arc::clone(&body);
                                                                    let expected_run = failure_run.clone();
                                                                    {
                                                                        let __dsl_program = effect_provide_some! {
                                                                        program: effect_map_error! {
                                                                        program: engine_for_failure.transition(
                                                                        failure_run.clone(),
                                                                        failure_checkpoint.clone(),
                                                                        WorkflowStatus::Ready,
                                                                    ), map: move |source| {
                                                                        let body = body_for_error.lock()
                                                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                                            .take();
                                                                        WorkflowRunError::Recovery { body, source, run:failed_run.clone(), checkpoint:checkpoint.clone() }
                                                                    }, }, project: |_: &R| (), }; effect! {
                                                                        let __dsl_program = __dsl_program.clone(); let body = body.clone(); let expected_run = expected_run.clone(); let pending = pending.clone(); let snapshot = __dsl_program.clone()?; (move || {
                                                                            let body = body.lock()
                                                                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                                                .take();
                                                                            if snapshot.handle.run != expected_run
                                                                                || snapshot.status != WorkflowStatus::Ready || snapshot.execution!=expected_execution {
                                                                                return effect_try_sync! {
                                                                                    once; body: {
                                                                                    let __dsl_result = Err(
                                                                                    WorkflowRunError::RecoveryTransitionMismatch { body }
                                                                                ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                                                            }
                                                                            *pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=None;
                                                                            let body = body.ok_or(WorkflowRunError::InputConsumed);
                                                                            effect_try_sync! {
                                                                                once; body: {
                                                                                let __dsl_result = Err(
                                                                                match body {
                                                                                    Ok(body) => WorkflowRunError::Body(body),
                                                                                    Err(error) => error,
                                                                                }
                                                                            ); move || __dsl_result }, on_error: ::core::convert::identity, }
                                                                        })()? } }
                                                                }, }; effect! {
                                                                    let __dsl_program = __dsl_program.clone(); let definition = definition.clone(); let engine_for_completion = engine_for_completion.clone(); let run_for_result = run_for_result.clone(); let step = __dsl_program.clone()?; match step {
                                                                    WorkflowStep::Suspended => {
                                                                        {
                                                                            let __dsl_program = effect_provide_some! { program: effect_map_error! { program: engine_for_completion.inspect(run_for_result.clone()), map: WorkflowRunError::Engine, }, project: |_: &R| (), }; effect! {
                                                                            let snapshot = __dsl_program.clone()?; match snapshot
                                                                                .map(|value| value.status) {
                                                                                Some(WorkflowStatus::Suspended) => {
                                                                                    effect_sync! { body: || WorkflowRunResult::Suspended, }
                                                                                }
                                                                                Some(WorkflowStatus::Ready) => {
                                                                                    effect_sync! { body: || WorkflowRunResult::Ready, }
                                                                                }
                                                                                _ => effect_try_sync! {
                                                                                    once; body: {
                                                                                    let __dsl_result = Err(
                                                                                    WorkflowRunError::UnregisteredSuspension
                                                                                ); move || __dsl_result }, on_error: ::core::convert::identity, },
                                                                            }? } }
                                                                    }
                                                                    WorkflowStep::Completed(outcome) => {
                                                                    let encoded = match definition
                                                                        .encode_outcome(&outcome) {
                                                                        Ok(value) => value,
                                                                        Err(error) => return effect_try_sync! { once; body: { let __dsl_result = Err(WorkflowRunError::Codec(error)); move || __dsl_result }, on_error: ::core::convert::identity, }?,
                                                                    };
                                                                    let outcome = Arc::new(Mutex::new(Some(outcome)));
                                                                    let failed_run = run_for_result.clone();
                                                                    let expected_run = run_for_result.clone();
                                                                    let expected = encoded.clone();
                                                                    {
                                                                        let __dsl_program = effect_provide_some! {
                                                                        program: effect_map_error! {
                                                                        program: engine_for_completion.complete(
                                                                        run_for_result.clone(), encoded,
                                                                    ), map: move |source| {
                                                                        WorkflowRunError::UncertainCompletion {
                                                                            run: failed_run.clone(), source,
                                                                        }
                                                                    }, }, project: |_: &R| (), }; effect! {
                                                                        let __dsl_program = __dsl_program.clone(); let expected = expected.clone(); let expected_run = expected_run.clone(); let outcome = outcome.clone(); let snapshot = __dsl_program?; (move || {
                                                                            if snapshot.handle.run != expected_run
                                                                                || !matches!(snapshot.status,
                                                                                    WorkflowStatus::Completed(ref recorded)
                                                                                        if recorded == &expected) {
                                                                                return effect_try_sync! {
                                                                                    once; body: {
                                                                                    let __dsl_result = Err(
                                                                                    WorkflowRunError::RecordedCompletionMismatch
                                                                                ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                                                            }
                                                                            let result = outcome.lock()
                                                                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                                                .take()
                                                                                .map(WorkflowRunResult::Completed)
                                                                                .ok_or(WorkflowRunError::InputConsumed);
                                                                            effect_try_sync! { once; body: { let __dsl_result = result; move || __dsl_result }, on_error: ::core::convert::identity, }
                                                                        })()? } }
                                                                    }
                                                                }? } }
                                                        })()? } }
                                                }
                                                }
                                            })()? } }
                                    }? } }
                            }? } }
                    }? }
        }
    }
}

// The reusable run recipe retains only encoded bytes and its own failed reset fence.
// Retrying this effect therefore cannot reset a replacement execution.
/// Retry only the reset checkpoint retained by this run recipe; validate returned ownership before clearing it or acknowledging completion.
fn recover_pending<R, BodyError>(
    engine: Arc<R::Engine>,
    snapshot: WorkflowSnapshot,
    pending: Arc<Mutex<Option<WorkflowCheckpoint>>>,
) -> Effect<WorkflowSnapshot, WorkflowRunError<EngineError<R>, BodyError>, R>
where
    R: HasWorkflowEngine + Send + Sync + 'static,
    BodyError: Send + 'static,
{
    let checkpoint = pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let Some(checkpoint) = checkpoint else {
        return effect_sync! { once; body: move || snapshot, };
    };
    if snapshot.status == WorkflowStatus::Running {
        let run = snapshot.handle.run.clone();
        let failed_run = run.clone();
        let failed_checkpoint = checkpoint.clone();
        let expected_execution = checkpoint.execution;
        let expected_run = run.clone();
        {
            let __dsl_program = effect_provide_some! {
            program: effect_map_error! {
            program: engine
            .transition(run, checkpoint, WorkflowStatus::Ready), map: move |source| WorkflowRunError::Recovery {
                body: None,
                source,
                run: failed_run.clone(),
                checkpoint: failed_checkpoint.clone(),
            }, }, project: |_: &R| (), };
            effect! {
                let __dsl_program = __dsl_program.clone(); let expected_run = expected_run.clone(); let pending = pending.clone(); let snapshot = __dsl_program.clone()?; (move || {
                if snapshot.status != WorkflowStatus::Ready
                    || snapshot.execution != expected_execution
                    || snapshot.handle.run != expected_run
                {
                    return effect_try_sync! {
                        once; body: {
                        let __dsl_result = Err(
                        WorkflowRunError::RecoveryTransitionMismatch { body: None },
                    ); move || __dsl_result }, on_error: ::core::convert::identity, };
                }
                *pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                effect_sync! { once; body: move || snapshot, }
            })()? }
        }
    } else {
        if snapshot.status == WorkflowStatus::Ready && snapshot.execution != checkpoint.execution {
            return effect_try_sync! {
                once; body: {
                let __dsl_result = Err(WorkflowRunError::RecoveryTransitionMismatch {
                body: None,
            }); move || __dsl_result }, on_error: ::core::convert::identity, };
        }
        // Ready acknowledges an already committed reset; completion acknowledges durable outcome.
        if snapshot.execution == checkpoint.execution
            && matches!(
                snapshot.status,
                WorkflowStatus::Ready | WorkflowStatus::Completed(_)
            )
        {
            *pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
        effect_sync! { once; body: move || snapshot, }
    }
}

#[cfg(test)]
mod correction_tests {
    use crate::{effect_retry, effect_try_sync, workflow, workflow_definition};

    use super::*;
    use crate::workflow::*;
    use crate::{KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis, json_i64};
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Domain;
    impl KeyDomain for Domain {
        const TAG: &'static str = "reset-retry-test";
    }
    struct FailingReset {
        inner: MemoryWorkflowEngine,
        reset_calls: AtomicUsize,
    }
    impl WorkflowEngine for FailingReset {
        type Error = WorkflowEngineError;
        fn register(&self, definition: WorkflowRegistration) -> Effect<(), Self::Error> {
            self.inner.register(definition)
        }
        fn start(
            &self,
            handle: WorkflowHandle,
            encoded_input: Arc<[u8]>,
        ) -> Effect<WorkflowSnapshot, Self::Error> {
            self.inner.start(handle, encoded_input)
        }
        fn inspect(&self, run: WorkflowRunId) -> Effect<Option<WorkflowSnapshot>, Self::Error> {
            self.inner.inspect(run)
        }
        fn transition(
            &self,
            run: WorkflowRunId,
            expected: WorkflowCheckpoint,
            next: WorkflowStatus,
        ) -> Effect<WorkflowSnapshot, Self::Error> {
            if next == WorkflowStatus::Ready && self.reset_calls.fetch_add(1, Ordering::SeqCst) == 0
            {
                effect_try_sync! { body: { let __dsl_error = || WorkflowEngineError::StateConflict; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }
            } else {
                self.inner.transition(run, expected, next)
            }
        }
        fn record_activity(
            &self,
            run: WorkflowRunId,
            activity: WorkflowActivityId,
            attempt: NonZeroU32,
            encoded_input: Arc<[u8]>,
            outcome: EncodedWorkflowOutcome,
        ) -> Effect<RecordedActivity, Self::Error> {
            self.inner
                .record_activity(run, activity, attempt, encoded_input, outcome)
        }
        fn inspect_activity(
            &self,
            run: WorkflowRunId,
            activity: WorkflowActivityId,
            attempt: NonZeroU32,
        ) -> Effect<Option<RecordedActivity>, Self::Error> {
            self.inner.inspect_activity(run, activity, attempt)
        }
        fn link_child(
            &self,
            parent: WorkflowRunId,
            child: RecordedChild,
        ) -> Effect<RecordedChild, Self::Error> {
            self.inner.link_child(parent, child)
        }
        fn wait_child(
            &self,
            parent: WorkflowRunId,
            child: WorkflowRunId,
        ) -> Effect<ChildWait, Self::Error> {
            self.inner.wait_child(parent, child)
        }
        fn set_deadline(
            &self,
            run: WorkflowRunId,
            deadline: RecordedDeadline,
        ) -> Effect<WorkflowSnapshot, Self::Error> {
            self.inner.set_deadline(run, deadline)
        }
        fn clear_deadline(
            &self,
            run: WorkflowRunId,
            expected: RecordedDeadline,
        ) -> Effect<DeadlineClear, Self::Error> {
            self.inner.clear_deadline(run, expected)
        }
        fn wake_due(&self, now: UtcMillis) -> Effect<Vec<WorkflowRunId>, Self::Error> {
            self.inner.wake_due(now)
        }
        fn wake_deadline(
            &self,
            run: WorkflowRunId,
            expected: RecordedDeadline,
            now: UtcMillis,
        ) -> Effect<DeadlineWake, Self::Error> {
            self.inner.wake_deadline(run, expected, now)
        }
        fn wait_deadline(
            &self,
            run: WorkflowRunId,
            deadline: RecordedDeadline,
            now: UtcMillis,
        ) -> Effect<DeadlineWait, Self::Error> {
            self.inner.wait_deadline(run, deadline, now)
        }
        fn deliver_signal(
            &self,
            run: WorkflowRunId,
            signal: RecordedSignal,
        ) -> Effect<RecordedSignal, Self::Error> {
            self.inner.deliver_signal(run, signal)
        }
        fn inspect_signal(
            &self,
            run: WorkflowRunId,
            signal: WorkflowSignalId,
            version: WorkflowVersion,
        ) -> Effect<Option<RecordedSignal>, Self::Error> {
            self.inner.inspect_signal(run, signal, version)
        }
        fn wait_signal(
            &self,
            run: WorkflowRunId,
            signal: WorkflowSignalId,
            version: WorkflowVersion,
        ) -> Effect<SignalWait, Self::Error> {
            self.inner.wait_signal(run, signal, version)
        }
        fn register_compensation(
            &self,
            run: WorkflowRunId,
            identity: WorkflowCompensationId,
            order: NonZeroU32,
        ) -> Effect<RecordedCompensation, Self::Error> {
            self.inner.register_compensation(run, identity, order)
        }
        fn finish_compensation(
            &self,
            run: WorkflowRunId,
            identity: WorkflowCompensationId,
            outcome: EncodedWorkflowOutcome,
        ) -> Effect<RecordedCompensation, Self::Error> {
            self.inner.finish_compensation(run, identity, outcome)
        }
        fn pending_compensations(
            &self,
            run: WorkflowRunId,
        ) -> Effect<Vec<RecordedCompensation>, Self::Error> {
            self.inner.pending_compensations(run)
        }
        fn inspect_race(
            &self,
            run: WorkflowRunId,
            race: WorkflowRaceId,
        ) -> Effect<Option<RecordedRace>, Self::Error> {
            self.inner.inspect_race(run, race)
        }
        fn record_race(
            &self,
            run: WorkflowRunId,
            race: RecordedRace,
        ) -> Effect<RecordedRace, Self::Error> {
            self.inner.record_race(run, race)
        }
        fn complete(
            &self,
            run: WorkflowRunId,
            outcome: EncodedWorkflowOutcome,
        ) -> Effect<WorkflowSnapshot, Self::Error> {
            self.inner.complete(run, outcome)
        }
        fn cancel(&self, run: WorkflowRunId) -> Effect<CancellationOutcome, Self::Error> {
            self.inner.cancel(run)
        }
        fn force_interrupt(&self, run: WorkflowRunId) -> Effect<CancellationOutcome, Self::Error> {
            self.inner.force_interrupt(run)
        }
    }
    struct Env(Arc<FailingReset>, crate::TestClock);
    impl crate::HasClock for Env {
        type Clock = crate::TestClock;
        fn clock(&self) -> &Self::Clock {
            &self.1
        }
    }
    impl HasWorkflowEngine for Env {
        type Engine = FailingReset;
        fn workflow_engine(&self) -> Arc<Self::Engine> {
            Arc::clone(&self.0)
        }
    }
    #[test]
    fn failed_reset_retries_exact_checkpoint_and_replays_encoded_owned_input() -> Result<(), String>
    {
        let namespace = KeyNamespace::<Domain>::new("run")
            .map_err(|error| format!("workflow namespace: {error:?}"))?;
        let run = WorkflowRunId::from_stable(
            StableKey::new(&namespace, [KeySegment::Text("retry".into())])
                .map_err(|error| format!("workflow run identity: {error:?}"))?,
        );
        let bound = NonZeroUsize::MIN.saturating_add(1023);
        let limits = crate::ParseLimits {
            bytes: bound,
            depth: bound,
            nodes: bound,
            collection: bound,
            issues: bound,
        };
        let definition = workflow_definition! {
            id: WorkflowDefinitionId::new("reset-retry")
                .map_err(|error| format!("workflow definition: {error:?}"))?,
            version: WorkflowVersion::new(NonZeroU32::MIN),
            input: json_i64(),
            success: json_i64(),
            error: json_i64(),
            compatibility: Arc::from(&b"retry"[..]),
            limits: limits,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let effect = definition.execute::<Env, String>(run.clone(), 7, move |input| {
            let calls = Arc::clone(&calls);
            workflow! {
                let calls = Arc::clone(&calls);
                let result = if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(String::from("body"))
                } else {
                    Ok(Ok(input))
                };
                workflow! { result; value: result, }?
            }
        });
        let env = Arc::new(Env(
            Arc::new(FailingReset {
                inner: MemoryWorkflowEngine::new(),
                reset_calls: AtomicUsize::new(0),
            }),
            crate::TestClock::new(std::time::Duration::ZERO),
        ));
        let first = futures_executor::block_on(effect.run(Arc::clone(&env)));
        let crate::Exit::Fail(WorkflowRunError::Recovery {
            body: Some(body),
            checkpoint: fence,
            run: failed_run,
            ..
        }) = first
        else {
            return Err(String::from("failed reset must retain ownership"));
        };
        assert_eq!(body, "body");
        assert_eq!(failed_run, run);
        let crate::Exit::Success(Some(snapshot)) =
            futures_executor::block_on(env.0.inspect(run.clone()).run(Arc::new(())))
        else {
            return Err(String::from("expected the retained run"));
        };
        assert_eq!(snapshot.checkpoint(), fence);
        assert!(matches!(
            futures_executor::block_on(effect.run(Arc::clone(&env))),
            crate::Exit::Success(WorkflowRunResult::Completed(Ok(7)))
        ));
        assert!(matches!(
            futures_executor::block_on(effect.run(Arc::clone(&env))),
            crate::Exit::Success(WorkflowRunResult::Completed(Ok(7)))
        ));
        assert_eq!(observed.load(Ordering::SeqCst), 2);
        let other = WorkflowRunId::from_stable(
            StableKey::new(&namespace, [KeySegment::Text("scheduled-retry".into())])
                .map_err(|error| format!("scheduled workflow identity: {error:?}"))?,
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        env.0.reset_calls.store(0, Ordering::SeqCst);
        let retry = effect_retry! {
        program: definition
        .execute::<Env, String>(other, 9, move |input| {
            let calls = Arc::clone(&calls);
            workflow! {
                let calls = Arc::clone(&calls);
                let result = if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(String::from("body"))
                } else {
                    Ok(Ok(input))
                };
                workflow! { result; value: result, }?
            }
        }), schedule: crate::Schedule::recurs(1), };
        assert!(matches!(
            futures_executor::block_on(retry.run(env)),
            crate::Exit::Success(WorkflowRunResult::Completed(Ok(9)))
        ));
        assert_eq!(counted.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
