use super::encoding::{decode_json, encode_json};
use super::{
    Durable, EncodedWorkflowOutcome, HasWorkflowEngine, WorkflowActivityId, WorkflowEngine,
    WorkflowOutcomeKind, WorkflowRunId,
};
use crate::{Effect, JsonValue, ParseLimits, PureCodec, SchemaIssue};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;
pub type ActivityEffect<A, E, BodyError, R> =
    Effect<Result<A, E>, ActivityRunError<EngineError<R>, BodyError>, R>;

#[derive(Debug)]
pub enum WorkflowCodecError {
    Schema(SchemaIssue),
    MalformedJson(SchemaIssue),
    ByteLimitExceeded,
    IdentityMismatch,
    VersionMismatch,
}

#[derive(Debug)]
pub enum ActivityRunError<EngineError, BodyError> {
    Engine(EngineError),
    Body(BodyError),
    Codec(WorkflowCodecError),
    RecordedActivityMismatch,
    InputConsumed,
}

/// A registered activity's result boundary. The body may depend on any
/// capabilities in R; this description only controls durable representation.
pub struct WorkflowActivity<A, E> {
    pub id: WorkflowActivityId,
    pub success: PureCodec<A, JsonValue>,
    pub error: PureCodec<E, JsonValue>,
    pub limits: ParseLimits,
}

impl<A, E> Clone for WorkflowActivity<A, E> {
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
        Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
            let description = description.clone();
            let body = Arc::clone(&body);
            let encoded_input = Arc::clone(&encoded_input);
            let activity = description.id.clone();
            let run_for_record = run.clone();
            engine
                .inspect_activity(run.clone(), activity.clone(), attempt)
                .map_error(ActivityRunError::Engine)
                .provide_some(|_: &R| ())
                .flat_map(move |recorded| match recorded {
                    Some(recorded) if recorded.encoded_input != encoded_input => {
                        Effect::from_result_once(Err(ActivityRunError::RecordedActivityMismatch))
                    }
                    Some(recorded) => Effect::from_result_once(
                        description
                            .decode(&recorded.outcome)
                            .map_err(ActivityRunError::Codec),
                    ),
                    None => {
                        let description = description.clone();
                        let engine = Arc::clone(&engine);
                        let activity = activity.clone();
                        let run = run_for_record.clone();
                        let encoded_input = Arc::clone(&encoded_input);
                        body()
                            .map_error(ActivityRunError::Body)
                            .flat_map(move |outcome| {
                                let encoded = match description.encode(&outcome) {
                                    Ok(encoded) => encoded,
                                    Err(error) => {
                                        return Effect::from_result_once(Err(
                                            ActivityRunError::Codec(error),
                                        ));
                                    }
                                };
                                let outcome = Arc::new(Mutex::new(Some(outcome)));
                                let expected = encoded.clone();
                                let expected_activity = activity.clone();
                                let expected_input = Arc::clone(&encoded_input);
                                engine
                                    .record_activity(
                                        run.clone(),
                                        activity.clone(),
                                        attempt,
                                        Arc::clone(&encoded_input),
                                        encoded,
                                    )
                                    .map_error(ActivityRunError::Engine)
                                    .provide_some(|_: &R| ())
                                    .flat_map(move |record| {
                                        if record.identity != expected_activity
                                            || record.attempt != attempt
                                            || record.encoded_input != expected_input
                                            || record.outcome != expected
                                        {
                                            return Effect::from_result_once(Err(
                                                ActivityRunError::RecordedActivityMismatch,
                                            ));
                                        }
                                        let result = outcome
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .take()
                                            .ok_or(ActivityRunError::InputConsumed);
                                        Effect::from_result_once(result)
                                    })
                            })
                    }
                })
        })
    }

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
type TypedActivityBody<Q, A, E, BodyError, R> =
    dyn Fn(Arc<Q>) -> Effect<Result<A, E>, BodyError, R> + Send + Sync;

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
        let effect = Effect::try_sync(
            {
                let input = Arc::clone(&input);
                move || {
                    let encoded = codec
                        .encode_ref(input.as_ref())
                        .map_err(WorkflowCodecError::Schema)?;
                    encode_json(&encoded, limits)
                }
            },
            ActivityRunError::Codec,
        )
        .flat_map(move |encoded_input| {
            let input = Arc::clone(&input);
            let body = Arc::clone(&body);
            description.run_bound(run.clone(), attempt, encoded_input, move || {
                body(Arc::clone(&input))
            })
        });
        Durable::from_effect(effect)
    }
}
