use super::encoding::{decode_json, encode_json};
use super::{
    HasWorkflowEngine, RecordedSignal, SignalWait, WorkflowCodecError, WorkflowEngine,
    WorkflowRunId, WorkflowSignalId, WorkflowVersion,
};
use crate::{Effect, JsonValue, ParseLimits, PureCodec};

#[derive(Debug)]
pub enum DurableSignalError<EngineError> {
    Engine(EngineError),
    Codec(WorkflowCodecError),
    RecordedSignalMismatch,
    InputConsumed,
}

pub enum DurableSignalWait<A> {
    Ready(A),
    Waiting,
}

/// A typed token is bound to the run, signal name, and definition version.
/// The engine records signal bytes before a waiter is resumed; inspecting a
/// completed token after recovery returns the same value.
pub struct DurableDeferred<A> {
    pub run: WorkflowRunId,
    pub signal: WorkflowSignalId,
    pub version: WorkflowVersion,
    pub codec: PureCodec<A, JsonValue>,
    pub limits: ParseLimits,
}

impl<A> Clone for DurableDeferred<A> {
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
