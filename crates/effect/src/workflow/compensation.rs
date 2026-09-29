use super::encoding::{decode_json, encode_json};
use super::{
    CompensationState, EncodedWorkflowOutcome, HasWorkflowEngine, WorkflowCodecError,
    WorkflowCompensationId, WorkflowEngine, WorkflowOutcomeKind, WorkflowRunId,
};
use crate::{Defect, Effect, Exit, Finalization, JsonValue, ParseLimits, PureCodec};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;

#[derive(Debug)]
pub enum CompensationRunError<EngineError, BodyError> {
    Engine(EngineError),
    Body(BodyError),
    Codec(WorkflowCodecError),
    UncertainCompletion {
        identity: WorkflowCompensationId,
        source: EngineError,
    },
    RecordedCompensationMismatch,
    InputConsumed,
}

#[derive(Debug)]
pub enum CompensatedFailure<Original, DomainError, EngineError, BodyError> {
    Original(Original),
    OriginalAndDomain {
        original: Original,
        compensation: DomainError,
    },
    OriginalAndExecution {
        original: Original,
        compensation: CompensationRunError<EngineError, BodyError>,
    },
    OriginalAndDefect {
        original: Original,
        defect: Defect,
    },
    OriginalAndInterruption {
        original: Original,
    },
    OriginalAndFinalization {
        original: Original,
        details: Finalization<CompensationRunError<EngineError, BodyError>>,
    },
    InputConsumed,
}

pub struct WorkflowCompensation<A, E> {
    pub identity: WorkflowCompensationId,
    pub order: NonZeroU32,
    pub success: PureCodec<A, JsonValue>,
    pub error: PureCodec<E, JsonValue>,
    pub limits: ParseLimits,
}

impl<A, E> Clone for WorkflowCompensation<A, E> {
    fn clone(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            order: self.order,
            success: self.success.clone(),
            error: self.error.clone(),
            limits: self.limits,
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> WorkflowCompensation<A, E> {
    pub fn encode(
        &self,
        result: &Result<A, E>,
    ) -> Result<EncodedWorkflowOutcome, WorkflowCodecError> {
        let (kind, value) = match result {
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
        outcome: &EncodedWorkflowOutcome,
    ) -> Result<Result<A, E>, WorkflowCodecError> {
        let value = decode_json(&outcome.payload, self.limits)?;
        match outcome.kind {
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

    /// Register the compensation before attempting an external side effect.
    /// A missing completion record on recovery means the body may run again;
    /// the body receives its stable compensation identity as an idempotency
    /// key for the external operation.
    pub fn run<R, BodyError>(
        &self,
        run: WorkflowRunId,
        body: impl Fn(WorkflowCompensationId) -> Effect<Result<A, E>, BodyError, R>
        + Send
        + Sync
        + 'static,
    ) -> Effect<Result<A, E>, CompensationRunError<EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
        BodyError: Send + 'static,
    {
        let description = self.clone();
        let body = Arc::new(body);
        Effect::service(|env: &R| env.workflow_engine())
            .flat_map(move |engine| {
                let description = description.clone();
                let body = Arc::clone(&body);
                let run = run.clone();
                engine.register_compensation(
                    run.clone(), description.identity.clone(), description.order,
                ).map_error(CompensationRunError::Engine)
                    .provide_some(|_: &R| ())
                    .flat_map(move |recorded| {
                        if recorded.identity != description.identity
                            || recorded.order != description.order {
                            return Effect::from_result_once(Err(
                                CompensationRunError::RecordedCompensationMismatch
                            ));
                        }
                        match recorded.state {
                        CompensationState::Completed(outcome) => {
                            Effect::from_result_once(description.decode(&outcome)
                                .map_err(CompensationRunError::Codec))
                        }
                        CompensationState::Pending => {
                            let description = description.clone();
                            let engine = Arc::clone(&engine);
                            let run = run.clone();
                            body(description.identity.clone())
                                .map_error(CompensationRunError::Body)
                                .flat_map(move |result| {
                                    let encoded = match description.encode(&result) {
                                        Ok(value) => value,
                                        Err(error) => return Effect::from_result_once(
                                            Err(CompensationRunError::Codec(error))
                                        ),
                                    };
                                    let result = Arc::new(Mutex::new(Some(result)));
                                    let identity_on_error = description.identity.clone();
                                    let expected_identity = description.identity.clone();
                                    let expected_order = description.order;
                                    let expected = encoded.clone();
                                    engine.finish_compensation(
                                        run.clone(), description.identity.clone(), encoded,
                                    ).map_error(move |source| {
                                        CompensationRunError::UncertainCompletion {
                                            identity: identity_on_error.clone(), source,
                                        }
                                    })
                                        .provide_some(|_: &R| ())
                                        .flat_map(move |recorded| {
                                            if recorded.identity != expected_identity
                                                || recorded.order != expected_order
                                                || !matches!(recorded.state,
                                                    CompensationState::Completed(ref outcome)
                                                        if outcome == &expected) {
                                                return Effect::from_result_once(Err(
                                                    CompensationRunError::RecordedCompensationMismatch
                                                ));
                                            }
                                            let result = result.lock()
                                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                .take()
                                                .ok_or(CompensationRunError::InputConsumed);
                                            Effect::from_result_once(result)
                                        })
                                })
                        }
                        }
                    })
            })
    }

    /// Preserve the original failure even when compensation has its own
    /// typed domain, body, codec or uncertain journal failure.
    pub fn on_failure<R, Original, BodyError>(
        &self,
        original: Original,
        run: WorkflowRunId,
        body: impl Fn(WorkflowCompensationId) -> Effect<Result<A, E>, BodyError, R>
        + Send
        + Sync
        + 'static,
    ) -> Effect<(), CompensatedFailure<Original, E, EngineError<R>, BodyError>, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
        Original: Send + 'static,
        BodyError: Send + 'static,
    {
        let original = Mutex::new(Some(original));
        self.run(run, body)
            .materialize_exit()
            .map_error(|never: std::convert::Infallible| match never {})
            .flat_map(move |result| {
                let original = original
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                let failure = match original {
                    Some(original) => match result {
                        Exit::Success(Ok(_)) => CompensatedFailure::Original(original),
                        Exit::Success(Err(compensation)) => CompensatedFailure::OriginalAndDomain {
                            original,
                            compensation,
                        },
                        Exit::Fail(compensation) => CompensatedFailure::OriginalAndExecution {
                            original,
                            compensation,
                        },
                        Exit::Die(defect) => {
                            CompensatedFailure::OriginalAndDefect { original, defect }
                        }
                        Exit::Interrupted => {
                            CompensatedFailure::OriginalAndInterruption { original }
                        }
                        Exit::FinalizationFailed(details) => {
                            CompensatedFailure::OriginalAndFinalization { original, details }
                        }
                    },
                    None => CompensatedFailure::InputConsumed,
                };
                Effect::from_result_once(Err(failure))
            })
    }
}
