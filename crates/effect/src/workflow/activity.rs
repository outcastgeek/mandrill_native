//! Typed activity definitions and execution.

use super::encoding::{decode_json, encode_json};
use super::{
    Durable, EncodedWorkflowOutcome, HasWorkflowEngine, WorkflowActivityId, WorkflowEngine,
    WorkflowOutcomeKind, WorkflowRunId,
};
use crate::{Effect, JsonValue, ParseLimits, PureCodec, SchemaIssue};
use crate::{effect, effect_map_error, effect_provide_some, effect_try_sync, service};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

/// Type alias for engine error in workflow activity.
///
/// # Type parameters
/// - `R`: Environment type.
type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;
/// Type alias for activity effect in workflow activity.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `BodyError`: Error produced by the registered workflow body.
/// - `R`: Environment type.
pub type ActivityEffect<A, E, BodyError, R> =
    Effect<Result<A, E>, ActivityRunError<EngineError<R>, BodyError>, R>;

#[derive(Debug)]
/// An encoding or decoding failure at a workflow record boundary.
pub enum WorkflowCodecError {
    /// The value failed schema validation. Retains the `SchemaIssue` detail.
    Schema(SchemaIssue),
    /// The input is not valid JSON. Retains the `SchemaIssue` detail.
    MalformedJson(SchemaIssue),
    /// The configured byte limit was exceeded.
    ByteLimitExceeded,
    /// The identity differs from the required or recorded value.
    IdentityMismatch,
    /// The recorded version differs from the declared version.
    VersionMismatch,
}

#[derive(Debug)]
/// An engine, codec, or application failure while running a named activity.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
/// - `BodyError`: Error produced by the registered workflow body.
pub enum ActivityRunError<EngineError, BodyError> {
    /// The workflow engine rejected or could not complete the operation. Retains the `EngineError` detail.
    Engine(EngineError),
    /// The registered application body failed. Retains the `BodyError` detail.
    Body(BodyError),
    /// Encoding or decoding the typed value failed. Retains the `WorkflowCodecError` detail.
    Codec(WorkflowCodecError),
    /// The recorded activity differs from the value produced during replay.
    RecordedActivityMismatch,
    /// An owned input was already consumed.
    InputConsumed,
}

/// A registered activity's result boundary. The body may depend on any
/// capabilities in R; this description only controls durable representation.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub struct WorkflowActivity<A, E> {
    /// Stable identity of this record. Type: `WorkflowActivityId`.
    pub id: WorkflowActivityId,
    /// Codec or representation identity for successful results. Type: `PureCodec<A, JsonValue>`.
    pub success: PureCodec<A, JsonValue>,
    /// The typed error carried by this result. Type: `PureCodec<E, JsonValue>`.
    pub error: PureCodec<E, JsonValue>,
    /// Explicit resource bounds applied to this operation. Type: `ParseLimits`.
    pub limits: ParseLimits,
}

impl<A, E> Clone for WorkflowActivity<A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            success: self.success.clone(),
            error: self.error.clone(),
            limits: self.limits,
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> WorkflowActivity<A, E> {
    /// Consult the recorded result before calling the body. Both domain
    /// failures and successes are replayed; suspension/defect is not recorded.
    /// An external side effect can still succeed before recording fails, so
    /// callers must supply an idempotency key to that external service.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    /// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `body` (`impl Fn() -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `ActivityEffect<A, E, BodyError, R>`.
    pub fn run<R, BodyError>(
        &self,
        run: WorkflowRunId,
        attempt: NonZeroU32,
        body: impl Fn() -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static,
    ) -> ActivityEffect<A, E, BodyError, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
        BodyError: Send + 'static,
    {
        self.run_bound(run, attempt, Arc::<[u8]>::from([]), body)
    }

    ///
    /// # Type parameters
    /// - `R`: Environment type supplying required capabilities. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    /// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `encoded_input` (`Arc<[u8]>`): The encoded input supplied to this operation.
    /// - `body` (`impl Fn() -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `ActivityEffect<A, E, BodyError, R>`.
    /// Returns `ActivityEffect<A, E, BodyError, R>`.
    fn run_bound<R, BodyError>(
        &self,
        run: WorkflowRunId,
        attempt: NonZeroU32,
        encoded_input: Arc<[u8]>,
        body: impl Fn() -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static,
    ) -> ActivityEffect<A, E, BodyError, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
        BodyError: Send + 'static,
    {
        let description = self.clone();
        let body = Arc::new(body);
        {
            let __dsl_program = service! { env: |env: &R| env.workflow_engine(), };
            effect! { let __dsl_program = __dsl_program.clone(); let body = body.clone(); let description = description.clone(); let encoded_input = encoded_input.clone(); let run = run.clone(); let engine = __dsl_program.clone()?; {
                let description = description.clone();
                let body = Arc::clone(&body);
                let encoded_input = Arc::clone(&encoded_input);
                let activity = description.id.clone();
                let run_for_record = run.clone();
                { let __dsl_program = effect_provide_some! { program: effect_map_error! { program: engine
                    .inspect_activity(run.clone(), activity.clone(), attempt), map: ActivityRunError::Engine, }, project: |_: &R| (), }; effect! { let __dsl_program = __dsl_program.clone(); let activity = activity.clone(); let body = body.clone(); let description = description.clone(); let encoded_input = encoded_input.clone(); let engine = engine.clone(); let run_for_record = run_for_record.clone(); let recorded = __dsl_program.clone()?; match recorded {
                        Some(recorded) if recorded.encoded_input != encoded_input => {
                            effect_try_sync! { once; body: { let __dsl_result = Err(ActivityRunError::RecordedActivityMismatch); move || __dsl_result }, on_error: ::core::convert::identity, }
                        }
                        Some(recorded) => effect_try_sync! { once; body: { let __dsl_result = description
                                .decode(&recorded.outcome)
                                .map_err(ActivityRunError::Codec); move || __dsl_result }, on_error: ::core::convert::identity, },
                        None => {
                            let description = description.clone();
                            let engine = Arc::clone(&engine);
                            let activity = activity.clone();
                            let run = run_for_record.clone();
                            let encoded_input = Arc::clone(&encoded_input);
                            { let __dsl_program = effect_map_error! { program: body(), map: ActivityRunError::Body, }; effect! { let __dsl_program = __dsl_program.clone(); let activity = activity.clone(); let description = description.clone(); let encoded_input = encoded_input.clone(); let engine = engine.clone(); let run = run.clone(); let outcome = __dsl_program.clone()?; (move || {
                                    let encoded = match description.encode(&outcome) {
                                        Ok(encoded) => encoded,
                                        Err(error) => {
                                            return effect_try_sync! { once; body: { let __dsl_result = Err(
                                                ActivityRunError::Codec(error),
                                            ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                        }
                                    };
                                    let outcome = Arc::new(Mutex::new(Some(outcome)));
                                    let expected = encoded.clone();
                                    let expected_activity = activity.clone();
                                    let expected_input = Arc::clone(&encoded_input);
                                    { let __dsl_program = effect_provide_some! { program: effect_map_error! { program: engine
                                        .record_activity(
                                            run.clone(),
                                            activity.clone(),
                                            attempt,
                                            Arc::clone(&encoded_input),
                                            encoded,
                                        ), map: ActivityRunError::Engine, }, project: |_: &R| (), }; effect! { let __dsl_program = __dsl_program.clone(); let expected = expected.clone(); let expected_activity = expected_activity.clone(); let expected_input = expected_input.clone(); let outcome = outcome.clone(); let record = __dsl_program?; (move || {
                                            if record.identity != expected_activity
                                                || record.attempt != attempt
                                                || record.encoded_input != expected_input
                                                || record.outcome != expected
                                            {
                                                return effect_try_sync! { once; body: { let __dsl_result = Err(
                                                    ActivityRunError::RecordedActivityMismatch,
                                                ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                            }
                                            let result = outcome
                                                .lock()
                                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                .take()
                                                .ok_or(ActivityRunError::InputConsumed);
                                            effect_try_sync! { once; body: { let __dsl_result = result; move || __dsl_result }, on_error: ::core::convert::identity, }
                                        })()? } }
                                })()? } }
                        }
                    }? } }
            }? }
        }
    }

    /// Encode the typed value using this definition’s declared representation.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&Result<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Result<EncodedWorkflowOutcome, WorkflowCodecError>`.
    pub fn encode(
        &self,
        outcome: &Result<A, E>,
    ) -> Result<EncodedWorkflowOutcome, WorkflowCodecError> {
        let (kind, value) = match outcome {
            Ok(value) => (
                WorkflowOutcomeKind::Succeeded,
                self.success
                    .encode_ref(value)
                    .map_err(WorkflowCodecError::Schema)?,
            ),
            Err(error) => (
                WorkflowOutcomeKind::Failed,
                self.error
                    .encode_ref(error)
                    .map_err(WorkflowCodecError::Schema)?,
            ),
        };
        Ok(EncodedWorkflowOutcome {
            kind,
            payload: encode_json(&value, self.limits)?,
        })
    }

    /// Validate the encoded input and reconstruct its typed value.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `recorded` (`&EncodedWorkflowOutcome`): The recorded supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Result<A, E>, WorkflowCodecError>`.
    pub fn decode(
        &self,
        recorded: &EncodedWorkflowOutcome,
    ) -> Result<Result<A, E>, WorkflowCodecError> {
        let value = decode_json(&recorded.payload, self.limits)?;
        match recorded.kind {
            WorkflowOutcomeKind::Succeeded => self
                .success
                .decode_ref(&value)
                .map(Ok)
                .map_err(WorkflowCodecError::Schema),
            WorkflowOutcomeKind::Failed => self
                .error
                .decode_ref(&value)
                .map(Err)
                .map_err(WorkflowCodecError::Schema),
        }
    }
}

/// One typed input codec and activity body paired with the recorded result
/// description. The immutable input is shared with the body through Arc;
/// replay compares its encoded form before returning a saved outcome.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `BodyError`: Error produced by the registered workflow body.
/// - `R`: Environment type.
type TypedActivityBody<Q, A, E, BodyError, R> =
    dyn Fn(Arc<Q>) -> Effect<Result<A, E>, BodyError, R> + Send + Sync;

/// A named activity with typed input, output, error, and Effect body.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `BodyError`: Error produced by the registered workflow body.
/// - `R`: Environment type.
pub struct TypedWorkflowActivity<Q, A, E, BodyError, R>
where
    Q: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: HasWorkflowEngine + Send + Sync + 'static,
{
    description: WorkflowActivity<A, E>,
    input: PureCodec<Q, JsonValue>,
    body: Arc<TypedActivityBody<Q, A, E, BodyError, R>>,
}

impl<Q, A, E, BodyError, R> Clone for TypedWorkflowActivity<Q, A, E, BodyError, R>
where
    Q: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: HasWorkflowEngine + Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            description: self.description.clone(),
            input: self.input.clone(),
            body: Arc::clone(&self.body),
        }
    }
}

impl<Q, A, E, BodyError, R> TypedWorkflowActivity<Q, A, E, BodyError, R>
where
    Q: Send + Sync + 'static,
    A: Send + 'static,
    E: Send + 'static,
    BodyError: Send + 'static,
    R: HasWorkflowEngine + Send + Sync + 'static,
{
    #[must_use]
    /// Construct a `TypedWorkflowActivity` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `BodyError`: Error produced by the registered workflow body.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `description` (`WorkflowActivity<A, E>`): The description supplied to this operation.
    /// - `input` (`PureCodec<Q, JsonValue>`): The input value or factory consumed by this operation.
    /// - `body` (`impl Fn(Arc<Q>) -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        description: WorkflowActivity<A, E>,
        input: PureCodec<Q, JsonValue>,
        body: impl Fn(Arc<Q>) -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            description,
            input,
            body: Arc::new(body),
        }
    }

    /// Invoke the typed operation through its selected channel or handler.
    ///
    /// # Type parameters
    /// - `BodyError`: Error produced by the registered workflow body.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `attempt` (`NonZeroU32`): Ordinal execution or delivery attempt being recorded.
    /// - `input` (`Q`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Durable<Result<A, E>, ActivityRunError<EngineError<R>, BodyError>, R>`.
    pub fn call(
        &self,
        run: WorkflowRunId,
        attempt: NonZeroU32,
        input: Q,
    ) -> Durable<Result<A, E>, ActivityRunError<EngineError<R>, BodyError>, R> {
        let input = Arc::new(input);
        let codec = self.input.clone();
        let limits = self.description.limits;
        let description = self.description.clone();
        let body = Arc::clone(&self.body);
        let effect = {
            let __dsl_program = effect_try_sync! { body: {
                let input = Arc::clone(&input);
                move || {
                    let encoded = codec
                        .encode_ref(input.as_ref())
                        .map_err(WorkflowCodecError::Schema)?;
                    encode_json(&encoded, limits)
                }
            }, on_error: ActivityRunError::Codec, };
            effect! { let body = body.clone(); let description = description.clone(); let input = input.clone(); let run = run.clone(); let encoded_input = __dsl_program.clone()?; {
                let input = Arc::clone(&input);
                let body = Arc::clone(&body);
                description.run_bound(run.clone(), attempt, encoded_input, move || {
                    body(Arc::clone(&input))
                })
            }? }
        };
        Durable::from_effect(effect)
    }
}
