//! Parent-child workflow identities and propagation.

use super::{
    ChildWait, Durable, HasWorkflowEngine, RecordedChild, WorkflowCodecError, WorkflowDefinition,
    WorkflowEngine, WorkflowHandle, WorkflowRunError, WorkflowRunId, WorkflowRunResult,
};
use crate::Effect;
use std::sync::{Arc, Mutex};

/// Type alias for engine error in workflow durable child.
///
/// # Type parameters
/// - `R`: Environment type.
type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;
/// Type alias for durable child run in workflow durable child.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `BodyError`: Error produced by the registered workflow body.
/// - `R`: Environment type.
pub type DurableChildRun<A, E, BodyError, R> =
    Durable<WorkflowRunResult<A, E>, DurableChildError<EngineError<R>, BodyError>, R>;

#[derive(Debug)]
/// A typed failure while starting or joining a durable child workflow.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
/// - `BodyError`: Error produced by the registered workflow body.
pub enum DurableChildError<EngineError, BodyError> {
    /// Encoding or decoding the typed value failed. Retains the `WorkflowCodecError` detail.
    Codec(WorkflowCodecError),
    /// The workflow engine rejected or could not complete the operation. Retains the `EngineError` detail.
    Engine(EngineError),
    /// Executing or recording the workflow run failed. Retains the `WorkflowRunError<EngineError, BodyError>` detail.
    Run(WorkflowRunError<EngineError, BodyError>),
    /// The recorded child differs from the value produced during replay.
    RecordedChildMismatch,
    /// An owned input was already consumed.
    InputConsumed,
}

/// Persist the parent-child relation before attempting the child. A repeated
/// evaluation must present the same child identity, definition and input.
pub struct DurableChild;

impl DurableChild {
    #[must_use]
    /// Wait for the represented condition or owned work to settle.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `parent` (`WorkflowRunId`): Parent scope, span, or workflow identity.
    /// - `child` (`WorkflowRunId`): Child workflow or scope-owned task being observed.
    ///
    /// # Returns
    /// Type: `Durable<ChildWait, <R::Engine as WorkflowEngine>::Error, R>`.
    pub fn wait<R>(
        parent: WorkflowRunId,
        child: WorkflowRunId,
    ) -> Durable<ChildWait, <R::Engine as WorkflowEngine>::Error, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        Durable::from_effect(Effect::service(|env: &R| env.workflow_engine()).flat_map(
            move |engine| {
                engine
                    .wait_child(parent.clone(), child.clone())
                    .provide_some(|_: &R| ())
            },
        ))
    }

    /// Execute or describe one run using the supplied environment and inputs.
    ///
    /// # Type parameters
    /// - `I`: Input type. Additional bound: `Send + 'static`.
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `parent` (`WorkflowRunId`): Parent scope, span, or workflow identity.
    /// - `child_run` (`WorkflowRunId`): The child run supplied to this operation.
    /// - `definition` (`WorkflowDefinition<I, A, E>`): The typed definition used by this operation.
    /// - `input` (`I`): The input value or factory consumed by this operation.
    /// - `body` (`impl Fn(I) -> Durable<Result<A, E>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `DurableChildRun<A, E, BodyError, R>`.
    pub fn run<I, A, E, BodyError, R>(
        parent: WorkflowRunId,
        child_run: WorkflowRunId,
        definition: WorkflowDefinition<I, A, E>,
        input: I,
        body: impl Fn(I) -> Durable<Result<A, E>, BodyError, R> + Send + Sync + 'static,
    ) -> DurableChildRun<A, E, BodyError, R>
    where
        I: Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
        BodyError: Send + 'static,
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        let input = Arc::new(Mutex::new(Some(input)));
        let input_for_encoding = Arc::clone(&input);
        let handle = WorkflowHandle {
            run: child_run.clone(),
            definition: definition.id.clone(),
            version: definition.version,
        };
        let body = Arc::new(body);
        let definition_for_encoding = definition.clone();
        Durable::from_effect(
            Effect::try_sync(
                move || {
                    let guard = input_for_encoding
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let input = guard.as_ref().ok_or(DurableChildError::InputConsumed)?;
                    definition_for_encoding
                        .encode_input_bytes(input)
                        .map_err(DurableChildError::Codec)
                },
                std::convert::identity,
            )
            .flat_map(move |encoded_input| {
                let input = Arc::clone(&input);
                let definition = definition.clone();
                let body = Arc::clone(&body);
                let parent = parent.clone();
                let child_run = child_run.clone();
                let child = RecordedChild {
                    handle: handle.clone(),
                    encoded_input,
                };
                Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
                    let input = Arc::clone(&input);
                    let definition = definition.clone();
                    let body = Arc::clone(&body);
                    let parent = parent.clone();
                    let child_run = child_run.clone();
                    let expected = child.clone();
                    engine
                        .link_child(parent.clone(), child.clone())
                        .map_error(DurableChildError::Engine)
                        .provide_some(|_: &R| ())
                        .flat_map(move |recorded| {
                            if recorded != expected {
                                return Effect::from_result_once(Err(
                                    DurableChildError::RecordedChildMismatch,
                                ));
                            }
                            let input = input
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take();
                            match input {
                                Some(input) => definition
                                    .execute(child_run.clone(), input, {
                                        let body = Arc::clone(&body);
                                        move |value| body(value)
                                    })
                                    .map_error(DurableChildError::Run),
                                None => {
                                    Effect::from_result_once(Err(DurableChildError::InputConsumed))
                                }
                            }
                        })
                })
            }),
        )
    }
}
