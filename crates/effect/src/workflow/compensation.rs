//! Recorded compensation identities and outcomes.

use super::encoding::{decode_json, encode_json};
use super::{
    CompensationState, EncodedWorkflowOutcome, HasWorkflowEngine, WorkflowCodecError,
    WorkflowCompensationId, WorkflowEngine, WorkflowOutcomeKind, WorkflowRunId,
};
use crate::{Defect, Effect, Exit, Finalization, JsonValue, ParseLimits, PureCodec};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

/// Type alias for engine error in workflow compensation.
///
/// # Type parameters
/// - `R`: Environment type.
type EngineError<R> = <<R as HasWorkflowEngine>::Engine as WorkflowEngine>::Error;

#[derive(Debug)]
/// A failure while recording or executing compensation for a workflow run.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
/// - `BodyError`: Error produced by the registered workflow body.
pub enum CompensationRunError<EngineError, BodyError> {
    /// The workflow engine rejected or could not complete the operation. Retains the `EngineError` detail.
    Engine(EngineError),
    /// The registered application body failed. Retains the `BodyError` detail.
    Body(BodyError),
    /// Encoding or decoding the typed value failed. Retains the `WorkflowCodecError` detail.
    Codec(WorkflowCodecError),
    /// Completion may have committed; the recorded outcome could not be confirmed.
    UncertainCompletion {
        /// Stable identity used for deduplication or lookup. Type: `WorkflowCompensationId`.
        identity: WorkflowCompensationId,
        /// The underlying failure or provider cause. Type: `EngineError`.
        source: EngineError,
    },
    /// The recorded compensation differs from the value produced during replay.
    RecordedCompensationMismatch,
    /// An owned input was already consumed.
    InputConsumed,
}

#[derive(Debug)]
/// The original workflow failure together with compensation results.
///
/// # Type parameters
/// - `Original`: Original failure type retained alongside a later failure.
/// - `DomainError`: Expected domain-error type.
/// - `EngineError`: Error returned by the workflow engine.
/// - `BodyError`: Error produced by the registered workflow body.
pub enum CompensatedFailure<Original, DomainError, EngineError, BodyError> {
    /// The original operation failed before cleanup. Retains the `Original` detail.
    Original(Original),
    /// The original operation failed and cleanup also reported domain.
    OriginalAndDomain {
        /// Value recorded before this transition. Type: `Original`.
        original: Original,
        /// Recorded recovery action for this workflow step. Type: `DomainError`.
        compensation: DomainError,
    },
    /// The original operation failed and cleanup also reported execution.
    OriginalAndExecution {
        /// Value recorded before this transition. Type: `Original`.
        original: Original,
        /// Recorded recovery action for this workflow step. Type: `CompensationRunError<EngineError, BodyError>`.
        compensation: CompensationRunError<EngineError, BodyError>,
    },
    /// The original operation failed and cleanup also reported defect.
    OriginalAndDefect {
        /// Value recorded before this transition. Type: `Original`.
        original: Original,
        /// Defect retained from the failed compensation step. Type: `Defect`.
        defect: Defect,
    },
    /// The original operation failed and cleanup also reported interruption.
    OriginalAndInterruption {
        /// Value recorded before this transition. Type: `Original`.
        original: Original,
    },
    /// The original operation failed and cleanup also reported finalization.
    OriginalAndFinalization {
        /// Value recorded before this transition. Type: `Original`.
        original: Original,
        /// Original outcome and ordered cleanup failures retained together. Type: `Finalization<CompensationRunError<EngineError, BodyError>>`.
        details: Finalization<CompensationRunError<EngineError, BodyError>>,
    },
    /// An owned input was already consumed.
    InputConsumed,
}

/// A registered recovery action associated with a durable workflow step.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub struct WorkflowCompensation<A, E> {
    /// Stable identity used for deduplication or lookup. Type: `WorkflowCompensationId`.
    pub identity: WorkflowCompensationId,
    /// Stable acceptance or delivery order. Type: `NonZeroU32`.
    pub order: NonZeroU32,
    /// Codec or representation identity for successful results. Type: `PureCodec<A, JsonValue>`.
    pub success: PureCodec<A, JsonValue>,
    /// The typed error carried by this result. Type: `PureCodec<E, JsonValue>`.
    pub error: PureCodec<E, JsonValue>,
    /// Explicit resource bounds applied to this operation. Type: `ParseLimits`.
    pub limits: ParseLimits,
}

impl<A, E> Clone for WorkflowCompensation<A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Encode the typed value using this definition’s declared representation.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `result` (`&Result<A, E>`): The result to record or return.
    ///
    /// # Returns
    /// Type: `Result<EncodedWorkflowOutcome, WorkflowCodecError>`.
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

    /// Validate the encoded input and reconstruct its typed value.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&EncodedWorkflowOutcome`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Result<Result<A, E>, WorkflowCodecError>`.
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
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    /// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `body` (`impl Fn(WorkflowCompensationId) -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Effect<Result<A, E>, CompensationRunError<EngineError<R>, BodyError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    /// - `Original`: Original failure type retained alongside a later failure. Additional bound: `Send + 'static`.
    /// - `BodyError`: Error produced by the registered workflow body. Additional bound: `Send + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `original` (`Original`): The original supplied to this operation.
    /// - `run` (`WorkflowRunId`): Stable identity of the workflow run being inspected or changed.
    /// - `body` (`impl Fn(WorkflowCompensationId) -> Effect<Result<A, E>, BodyError, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Effect<(), CompensatedFailure<Original, E, EngineError<R>, BodyError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
