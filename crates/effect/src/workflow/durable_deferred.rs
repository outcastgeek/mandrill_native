//! Typed durable signals and completion tokens.

use super::encoding::{decode_json, encode_json};
use super::{
    HasWorkflowEngine, RecordedSignal, SignalWait, WorkflowCodecError, WorkflowEngine,
    WorkflowRunId, WorkflowSignalId, WorkflowVersion,
};
use crate::{Effect, JsonValue, ParseLimits, PureCodec};

#[derive(Debug)]
/// A typed failure while recording or waiting for a durable signal.
///
/// # Type parameters
/// - `EngineError`: Error returned by the workflow engine.
pub enum DurableSignalError<EngineError> {
    /// The workflow engine rejected or could not complete the operation. Retains the `EngineError` detail.
    Engine(EngineError),
    /// Encoding or decoding the typed value failed. Retains the `WorkflowCodecError` detail.
    Codec(WorkflowCodecError),
    /// The recorded signal differs from the value produced during replay.
    RecordedSignalMismatch,
    /// An owned input was already consumed.
    InputConsumed,
}

/// The current recorded state of a workflow signal wait.
///
/// # Type parameters
/// - `A`: Application value type.
pub enum DurableSignalWait<A> {
    /// The operation is ready to continue. Carries `A`.
    Ready(A),
    /// Execution is suspended until the recorded condition changes.
    Waiting,
}

/// A typed token is bound to the run, signal name, and definition version.
/// The engine records signal bytes before a waiter is resumed; inspecting a
/// completed token after recovery returns the same value.
///
/// # Type parameters
/// - `A`: Application value type.
pub struct DurableDeferred<A> {
    /// Stable identity of the workflow run. Type: `WorkflowRunId`.
    pub run: WorkflowRunId,
    /// Identity of the workflow signal. Type: `WorkflowSignalId`.
    pub signal: WorkflowSignalId,
    /// The declared compatibility version. Type: `WorkflowVersion`.
    pub version: WorkflowVersion,
    /// The codec used for conversion and validation. Type: `PureCodec<A, JsonValue>`.
    pub codec: PureCodec<A, JsonValue>,
    /// Explicit resource bounds applied to this operation. Type: `ParseLimits`.
    pub limits: ParseLimits,
}

impl<A> Clone for DurableDeferred<A> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            run: self.run.clone(),
            signal: self.signal.clone(),
            version: self.version,
            codec: self.codec.clone(),
            limits: self.limits,
        }
    }
}

impl<A: Send + 'static> DurableDeferred<A> {
    /// Encode the typed value using this definition’s declared representation.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`&A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<RecordedSignal, WorkflowCodecError>`.
    pub fn encode(&self, value: &A) -> Result<RecordedSignal, WorkflowCodecError> {
        let json = self
            .codec
            .encode_ref(value)
            .map_err(WorkflowCodecError::Schema)?;
        Ok(RecordedSignal {
            identity: self.signal.clone(),
            version: self.version,
            payload: encode_json(&json, self.limits)?,
        })
    }

    /// Validate the encoded input and reconstruct its typed value.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `signal` (`&RecordedSignal`): Named value delivered to a waiting workflow.
    ///
    /// # Returns
    /// Type: `Result<A, WorkflowCodecError>`.
    pub fn decode(&self, signal: &RecordedSignal) -> Result<A, WorkflowCodecError> {
        if signal.identity != self.signal {
            return Err(WorkflowCodecError::IdentityMismatch);
        }
        if signal.version != self.version {
            return Err(WorkflowCodecError::VersionMismatch);
        }
        let json = decode_json(&signal.payload, self.limits)?;
        self.codec
            .decode_ref(&json)
            .map_err(WorkflowCodecError::Schema)
    }

    /// Record a terminal result once and publish it to current observers.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Effect<RecordedSignal, DurableSignalError<<R::Engine as WorkflowEngine>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn complete<R>(
        &self,
        value: A,
    ) -> Effect<RecordedSignal, DurableSignalError<<R::Engine as WorkflowEngine>::Error>, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        let token = self.clone();
        let value = std::sync::Mutex::new(Some(value));
        Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
            let value = value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some(value) = value else {
                return Effect::from_result_once(Err(DurableSignalError::InputConsumed));
            };
            match token.encode(&value) {
                Ok(signal) => {
                    let expected = signal.clone();
                    engine
                        .deliver_signal(token.run.clone(), signal)
                        .map_error(DurableSignalError::Engine)
                        .provide_some(|_: &R| ())
                        .flat_map(move |recorded| {
                            if recorded == expected {
                                Effect::sync_once(move || recorded)
                            } else {
                                Effect::from_result_once(Err(
                                    DurableSignalError::RecordedSignalMismatch,
                                ))
                            }
                        })
                }
                Err(error) => Effect::from_result_once(Err(DurableSignalError::Codec(error))),
            }
        })
    }

    /// Wait for the represented condition or owned work to settle.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<DurableSignalWait<A>, DurableSignalError<<R::Engine as WorkflowEngine>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn wait<R>(
        &self,
    ) -> Effect<DurableSignalWait<A>, DurableSignalError<<R::Engine as WorkflowEngine>::Error>, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        let token = self.clone();
        Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
            let token = token.clone();
            engine
                .wait_signal(token.run.clone(), token.signal.clone(), token.version)
                .map_error(DurableSignalError::Engine)
                .provide_some(|_: &R| ())
                .flat_map(move |state| match state {
                    SignalWait::Ready(recorded) => Effect::from_result_once(
                        token
                            .decode(&recorded)
                            .map(DurableSignalWait::Ready)
                            .map_err(DurableSignalError::Codec),
                    ),
                    SignalWait::Waiting => Effect::sync(|| DurableSignalWait::Waiting),
                })
        })
    }

    /// Observe a recorded result without installing a workflow waiter. This
    /// is used by at-least-once queue workers reconciling a lost ack.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasWorkflowEngine + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<Option<A>, DurableSignalError<<R::Engine as WorkflowEngine>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn inspect<R>(
        &self,
    ) -> Effect<Option<A>, DurableSignalError<<R::Engine as WorkflowEngine>::Error>, R>
    where
        R: HasWorkflowEngine + Send + Sync + 'static,
    {
        let token = self.clone();
        Effect::service(|env: &R| env.workflow_engine()).flat_map(move |engine| {
            let token = token.clone();
            engine
                .inspect_signal(token.run.clone(), token.signal.clone(), token.version)
                .map_error(DurableSignalError::Engine)
                .provide_some(|_: &R| ())
                .flat_map(move |recorded| {
                    Effect::from_result_once(
                        recorded
                            .map(|signal| token.decode(&signal))
                            .transpose()
                            .map_err(DurableSignalError::Codec),
                    )
                })
        })
    }
}
