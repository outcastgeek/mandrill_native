use super::{
    OperationId, RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcOperationKind,
    RpcOperationSpec, RpcVersion, RpcWireError,
};
use crate::encoding::{FrameDecodeFailure, FrameDecoder, FrameParts};
use crate::{
    Effect, JsonValue, ParseLimits, PureCodec, Release, ResourceValue, SchemaIssue, Scope,
};
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

type ConnectorSession<R> = <<R as HasRpcStreamConnector>::Connector as RpcStreamConnector>::Session;
pub type RpcOpenStreamEffect<C, T, E, Backend, R> =
    Effect<RpcStreamReader<C, T, E, Backend, ConnectorSession<R>>, RpcStreamError<E, Backend>, R>;

/// Streaming has separate chunk and terminal schemas. A terminal result is
/// never smuggled through a unary response or encoded as a special chunk.
pub struct RpcStreamOperation<Q, C, T, E> {
    spec: RpcOperationSpec,
    request: PureCodec<Q, JsonValue>,
    chunk: PureCodec<C, JsonValue>,
    terminal: PureCodec<T, JsonValue>,
    domain_error: PureCodec<E, JsonValue>,
    limits: ParseLimits,
}

impl<Q, C, T, E> Clone for RpcStreamOperation<Q, C, T, E> {
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            request: self.request.clone(),
            chunk: self.chunk.clone(),
            terminal: self.terminal.clone(),
            domain_error: self.domain_error.clone(),
            limits: self.limits,
        }
    }
}

impl<Q: Send + 'static, C: Send + 'static, T: Send + 'static, E: Send + 'static>
    RpcStreamOperation<Q, C, T, E>
{
    #[must_use]
    pub fn new(
        id: OperationId,
        version: RpcVersion,
        request: PureCodec<Q, JsonValue>,
        chunk: PureCodec<C, JsonValue>,
        terminal: PureCodec<T, JsonValue>,
        domain_error: PureCodec<E, JsonValue>,
        limits: ParseLimits,
    ) -> Self {
        Self {
            spec: RpcOperationSpec {
                id,
                version,
                kind: RpcOperationKind::Stream,
            },
            request,
            chunk,
            terminal,
            domain_error,
            limits,
        }
    }

    #[must_use]
    pub fn spec(&self) -> &RpcOperationSpec {
        &self.spec
    }
    #[must_use]
    pub fn request_codec(&self) -> &PureCodec<Q, JsonValue> {
        &self.request
    }
    #[must_use]
    pub fn chunk_codec(&self) -> &PureCodec<C, JsonValue> {
        &self.chunk
    }
    #[must_use]
    pub fn terminal_codec(&self) -> &PureCodec<T, JsonValue> {
        &self.terminal
    }
    #[must_use]
    pub fn error_codec(&self) -> &PureCodec<E, JsonValue> {
        &self.domain_error
    }
    #[must_use]
    pub const fn limits(&self) -> ParseLimits {
        self.limits
    }

    /// Preserve request, chunk and terminal schemas while widening only the
    /// declared error channel of a derived streaming operation.
    #[must_use]
    pub fn with_error_codec<F: Send + 'static>(
        &self,
        domain_error: PureCodec<F, JsonValue>,
    ) -> RpcStreamOperation<Q, C, T, F> {
        RpcStreamOperation {
            spec: self.spec.clone(),
            request: self.request.clone(),
            chunk: self.chunk.clone(),
            terminal: self.terminal.clone(),
            domain_error,
            limits: self.limits,
        }
    }
}

/// One ordered, single-consumer session. `receive_frame` returns one complete
/// framed message; fragmented socket reads and actual I/O scheduling belong
/// to the provider. Session methods must preserve send and receive order.
pub trait RpcStreamSession: Send + Sync + 'static {
    type Error: Send + 'static;
    fn send(&self, frame: FrameParts) -> Effect<(), Self::Error>;
    fn receive_frame(&self) -> Effect<Option<Vec<u8>>, Self::Error>;
    fn close(&self) -> Effect<(), Self::Error>;
}

pub trait RpcStreamConnector: Send + Sync + 'static {
    type Error: Send + 'static;
    type Session: RpcStreamSession<Error = Self::Error>;
    fn open(&self) -> Effect<Arc<Self::Session>, Self::Error>;
}

pub trait HasRpcStreamConnector {
    type Connector: RpcStreamConnector;
    fn rpc_stream_connector(&self) -> Arc<Self::Connector>;
}

impl<T: RpcStreamConnector> HasRpcStreamConnector for Arc<T> {
    type Connector = T;
    fn rpc_stream_connector(&self) -> Arc<Self::Connector> {
        Arc::clone(self)
    }
}

#[derive(Debug)]
pub enum RpcStreamError<E, Backend> {
    Domain(E),
    RequestSchema(SchemaIssue),
    ChunkSchema(SchemaIssue),
    TerminalSchema(SchemaIssue),
    ErrorSchema(SchemaIssue),
    Wire(RpcWireError),
    Transport(Backend),
    /// The server may have accepted the request before the send failed.
    RequestUncertain {
        call: RpcCallId,
        source: Backend,
    },
    /// The server may have advanced past this chunk before the acknowledgement
    /// send failed. Reconcile this call and sequence before retrying.
    AcknowledgementUncertain {
        call: RpcCallId,
        sequence: NonZeroU64,
        source: Backend,
    },
    /// The local reader stopped, but the server may or may not have received
    /// the cancellation frame. Its external work can still be running.
    CancellationUncertain(Backend),
    Disconnected,
    MissingFrame,
    ExtraFrames,
    CorrelationMismatch,
    OperationMismatch,
    UnexpectedKind(RpcEnvelopeKind),
    MissingSequence,
    SequenceMismatch {
        expected: NonZeroU64,
        actual: NonZeroU64,
    },
    SequenceExhausted,
    AcknowledgementRequired(NonZeroU64),
    NoAcknowledgementPending,
    ConcurrentRead,
    ConcurrentAcknowledgement,
    Ended,
    Cancelled,
    Failed,
    InputConsumed,
}

pub enum RpcStreamItem<C, T> {
    Chunk { sequence: NonZeroU64, value: C },
    Terminal(T),
}

pub struct RpcStreamClient<Q, C, T, E, Backend> {
    operation: RpcStreamOperation<Q, C, T, E>,
    policy: RpcEnvelopePolicy,
    _backend: std::marker::PhantomData<fn() -> Backend>,
}

impl<Q, C, T, E, Backend> RpcStreamClient<Q, C, T, E, Backend>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
{
    #[must_use]
    pub fn new(operation: RpcStreamOperation<Q, C, T, E>, policy: RpcEnvelopePolicy) -> Self {
        Self {
            operation,
            policy,
            _backend: std::marker::PhantomData,
        }
    }

    pub fn open<R>(
        &self,
        scope: &Scope<RpcStreamError<E, Backend>>,
        request: Q,
        call: RpcCallId,
        metadata: BTreeMap<Arc<str>, Arc<str>>,
    ) -> RpcOpenStreamEffect<C, T, E, Backend, R>
    where
        R: HasRpcStreamConnector + Send + Sync + 'static,
        R::Connector: RpcStreamConnector<Error = Backend>,
    {
        let operation = self.operation.clone();
        let reader_operation = self.operation.clone();
        let policy = self.policy;
        let scope = scope.clone();
        let request = Mutex::new(Some((request, metadata)));
        let request_call = call.clone();
        Effect::try_sync(
            move || {
                let (request, metadata) = request
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(RpcStreamError::InputConsumed)?;
                let payload = operation
                    .request_codec()
                    .encode_ref(&request)
                    .map_err(RpcStreamError::RequestSchema)?;
                crate::schema::internal::parser::check_limits(&payload, &operation.limits())
                    .map_err(RpcStreamError::RequestSchema)?;
                RpcEnvelope {
                    operation: operation.spec().id.clone(),
                    version: operation.spec().version,
                    call: request_call.clone(),
                    kind: RpcEnvelopeKind::Request,
                    sequence: None,
                    payload,
                    metadata,
                }
                .encode(policy)
                .map_err(RpcStreamError::Wire)
            },
            std::convert::identity,
        )
        .flat_map(move |frame| {
            let frame = Arc::new(Mutex::new(Some(frame)));
            let scope = scope.clone();
            let operation = reader_operation.clone();
            let call = call.clone();
            let connector =
                Effect::service(|env: &R| env.rpc_stream_connector()).flat_map(|connector| {
                    connector
                        .open()
                        .map_error(RpcStreamError::Transport)
                        .provide_some(|_: &R| ())
                });
            scope
                .acquire(connector, |session| {
                    Release::from_effect(
                        session
                            .close()
                            .map_error(RpcStreamError::Transport)
                            .provide_some(|_: &R| ()),
                    )
                })
                .flat_map(move |session| {
                    let source = session.clone();
                    let operation = operation.clone();
                    let call = call.clone();
                    let frame = Arc::clone(&frame);
                    session.access(Arc::clone).flat_map(move |session| {
                        let frame = frame
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        let Some(frame) = frame else {
                            return Effect::from_result_once(Err(RpcStreamError::InputConsumed));
                        };
                        let reader = RpcStreamReader::new(
                            source.clone(),
                            operation.clone(),
                            call.clone(),
                            policy,
                        );
                        session
                            .send(frame)
                            .map_error({
                                let uncertain_call = call.clone();
                                move |source| RpcStreamError::RequestUncertain {
                                    call: uncertain_call.clone(),
                                    source,
                                }
                            })
                            .provide_some(|_: &R| ())
                            .map(move |_| reader.clone())
                    })
                })
        })
    }
}

enum StreamPhase {
    Open,
    Ended,
    Cancelled,
    Failed,
}

struct ReaderState {
    phase: StreamPhase,
    expected: Option<NonZeroU64>,
    awaiting_ack: Option<NonZeroU64>,
    reading: bool,
    acknowledging: bool,
}

pub struct RpcStreamReader<C, T, E, Backend, Session>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
    Session: RpcStreamSession<Error = Backend>,
{
    session: ResourceValue<Arc<Session>>,
    spec: RpcOperationSpec,
    chunk: PureCodec<C, JsonValue>,
    terminal: PureCodec<T, JsonValue>,
    domain_error: PureCodec<E, JsonValue>,
    limits: ParseLimits,
    call: RpcCallId,
    policy: RpcEnvelopePolicy,
    state: Arc<Mutex<ReaderState>>,
    _backend: std::marker::PhantomData<fn() -> Backend>,
}

impl<C, T, E, Backend, Session> Clone for RpcStreamReader<C, T, E, Backend, Session>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
    Session: RpcStreamSession<Error = Backend>,
{
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            spec: self.spec.clone(),
            chunk: self.chunk.clone(),
            terminal: self.terminal.clone(),
            domain_error: self.domain_error.clone(),
            limits: self.limits,
            call: self.call.clone(),
            policy: self.policy,
            state: Arc::clone(&self.state),
            _backend: std::marker::PhantomData,
        }
    }
}

impl<C, T, E, Backend, Session> RpcStreamReader<C, T, E, Backend, Session>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    Backend: Send + 'static,
    Session: RpcStreamSession<Error = Backend>,
{
    fn new<Q>(
        session: ResourceValue<Arc<Session>>,
        operation: RpcStreamOperation<Q, C, T, E>,
        call: RpcCallId,
        policy: RpcEnvelopePolicy,
    ) -> Self {
        Self {
            session,
            spec: operation.spec,
            chunk: operation.chunk,
            terminal: operation.terminal,
            domain_error: operation.domain_error,
            limits: operation.limits,
            call,
            policy,
            state: Arc::new(Mutex::new(ReaderState {
                phase: StreamPhase::Open,
                expected: Some(NonZeroU64::MIN),
                awaiting_ack: None,
                reading: false,
                acknowledging: false,
            })),
            _backend: std::marker::PhantomData,
        }
    }

    /// Pull exactly one framed item. A chunk must be acknowledged before the
    /// following item can be received; this is the backpressure boundary.
    pub fn next<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<RpcStreamItem<C, T>, RpcStreamError<E, Backend>, R> {
        let state = Arc::clone(&self.state);
        let reader = self.clone();
        Effect::try_sync(
            move || {
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match state.phase {
                    StreamPhase::Ended => return Err(RpcStreamError::Ended),
                    StreamPhase::Cancelled => return Err(RpcStreamError::Cancelled),
                    StreamPhase::Failed => return Err(RpcStreamError::Failed),
                    StreamPhase::Open => {}
                }
                if let Some(sequence) = state.awaiting_ack {
                    return Err(RpcStreamError::AcknowledgementRequired(sequence));
                }
                if state.reading {
                    return Err(RpcStreamError::ConcurrentRead);
                }
                state.reading = true;
                Ok(())
            },
            std::convert::identity,
        )
        .flat_map(move |_| {
            let reader = reader.clone();
            let state = Arc::clone(&reader.state);
            reader
                .session
                .access(Arc::clone)
                .flat_map(move |session| {
                    let reader = reader.clone();
                    session
                        .receive_frame()
                        .map_error(RpcStreamError::Transport)
                        .provide_some(|_: &R| ())
                        .flat_map(move |bytes| {
                            let result = bytes
                                .ok_or(RpcStreamError::Disconnected)
                                .and_then(|bytes| reader.decode_next(&bytes));
                            Effect::from_result_once(result)
                        })
                })
                .ensuring(Effect::sync(move || {
                    state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .reading = false;
                }))
        })
    }

    fn decode_next(&self, bytes: &[u8]) -> Result<RpcStreamItem<C, T>, RpcStreamError<E, Backend>> {
        let envelope = match decode_frame(bytes, self.policy) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.fail_protocol();
                return Err(RpcStreamError::Wire(error));
            }
        };
        if envelope.call != self.call {
            self.fail_protocol();
            return Err(RpcStreamError::CorrelationMismatch);
        }
        if envelope.operation != self.spec.id || envelope.version != self.spec.version {
            self.fail_protocol();
            return Err(RpcStreamError::OperationMismatch);
        }
        match envelope.kind {
            RpcEnvelopeKind::StreamChunk => {
                let sequence = envelope.sequence.ok_or_else(|| {
                    self.fail_protocol();
                    RpcStreamError::MissingSequence
                })?;
                crate::schema::internal::parser::check_limits(&envelope.payload, &self.limits)
                    .map_err(|error| {
                        self.fail_protocol();
                        RpcStreamError::ChunkSchema(error)
                    })?;
                let value = self.chunk.decode_ref(&envelope.payload).map_err(|error| {
                    self.fail_protocol();
                    RpcStreamError::ChunkSchema(error)
                })?;
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !matches!(state.phase, StreamPhase::Open) {
                    return Err(RpcStreamError::Cancelled);
                }
                let Some(expected) = state.expected else {
                    state.phase = StreamPhase::Failed;
                    return Err(RpcStreamError::SequenceExhausted);
                };
                if sequence != expected {
                    state.phase = StreamPhase::Failed;
                    return Err(RpcStreamError::SequenceMismatch {
                        expected,
                        actual: sequence,
                    });
                }
                state.expected = sequence.get().checked_add(1).and_then(NonZeroU64::new);
                state.awaiting_ack = Some(sequence);
                Ok(RpcStreamItem::Chunk { sequence, value })
            }
            RpcEnvelopeKind::StreamEnd => {
                crate::schema::internal::parser::check_limits(&envelope.payload, &self.limits)
                    .map_err(|error| {
                        self.fail_protocol();
                        RpcStreamError::TerminalSchema(error)
                    })?;
                let value = self
                    .terminal
                    .decode_ref(&envelope.payload)
                    .map_err(|error| {
                        self.fail_protocol();
                        RpcStreamError::TerminalSchema(error)
                    })?;
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !matches!(state.phase, StreamPhase::Open) {
                    return Err(RpcStreamError::Cancelled);
                }
                state.phase = StreamPhase::Ended;
                Ok(RpcStreamItem::Terminal(value))
            }
            RpcEnvelopeKind::DomainError => {
                crate::schema::internal::parser::check_limits(&envelope.payload, &self.limits)
                    .map_err(|error| {
                        self.fail_protocol();
                        RpcStreamError::ErrorSchema(error)
                    })?;
                let error = self
                    .domain_error
                    .decode_ref(&envelope.payload)
                    .map_err(|error| {
                        self.fail_protocol();
                        RpcStreamError::ErrorSchema(error)
                    })?;
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !matches!(state.phase, StreamPhase::Open) {
                    return Err(RpcStreamError::Cancelled);
                }
                state.phase = StreamPhase::Ended;
                Err(RpcStreamError::Domain(error))
            }
            kind @ (RpcEnvelopeKind::Request
            | RpcEnvelopeKind::Success
            | RpcEnvelopeKind::Acknowledge
            | RpcEnvelopeKind::Cancel) => {
                self.fail_protocol();
                Err(RpcStreamError::UnexpectedKind(kind))
            }
        }
    }

    fn fail_protocol(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .phase = StreamPhase::Failed;
    }

    pub fn acknowledge<R: Send + Sync + 'static>(
        &self,
        sequence: NonZeroU64,
    ) -> Effect<(), RpcStreamError<E, Backend>, R> {
        let reader = self.clone();
        let state = Arc::clone(&self.state);
        Effect::try_sync(
            move || {
                let mut state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match state.phase {
                    StreamPhase::Ended => return Err(RpcStreamError::Ended),
                    StreamPhase::Cancelled => return Err(RpcStreamError::Cancelled),
                    StreamPhase::Failed => return Err(RpcStreamError::Failed),
                    StreamPhase::Open => {}
                }
                if state.acknowledging {
                    return Err(RpcStreamError::ConcurrentAcknowledgement);
                }
                match state.awaiting_ack {
                    None => return Err(RpcStreamError::NoAcknowledgementPending),
                    Some(expected) if expected != sequence => {
                        return Err(RpcStreamError::SequenceMismatch {
                            expected,
                            actual: sequence,
                        });
                    }
                    Some(_) => {}
                }
                state.acknowledging = true;
                Ok(())
            },
            std::convert::identity,
        )
        .flat_map(move |_| {
            let reader = reader.clone();
            let state = Arc::clone(&reader.state);
            let envelope = RpcEnvelope {
                operation: reader.spec.id.clone(),
                version: reader.spec.version,
                call: reader.call.clone(),
                kind: RpcEnvelopeKind::Acknowledge,
                sequence: Some(sequence),
                payload: JsonValue::Null,
                metadata: BTreeMap::new(),
            };
            let session_handle = reader.session.clone();
            let final_state = Arc::clone(&reader.state);
            let uncertain_call = reader.call.clone();
            Effect::from_result_once(envelope.encode(reader.policy).map_err(RpcStreamError::Wire))
                .flat_map(move |frame| {
                    let frame = Mutex::new(Some(frame));
                    let state = Arc::clone(&state);
                    let uncertain_call = uncertain_call.clone();
                    session_handle.access(Arc::clone).flat_map(move |session| {
                        let frame = frame
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        match frame {
                            Some(frame) => session
                                .send(frame)
                                .map_error({
                                    let failed_state = Arc::clone(&state);
                                    let uncertain_call = uncertain_call.clone();
                                    move |source| {
                                        failed_state
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .phase = StreamPhase::Failed;
                                        RpcStreamError::AcknowledgementUncertain {
                                            call: uncertain_call.clone(),
                                            sequence,
                                            source,
                                        }
                                    }
                                })
                                .provide_some(|_: &R| ())
                                .map({
                                    let state = Arc::clone(&state);
                                    move |_| {
                                        state
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                            .awaiting_ack = None;
                                    }
                                }),
                            None => Effect::from_result_once(Err(RpcStreamError::InputConsumed)),
                        }
                    })
                })
                .ensuring(Effect::sync({
                    let state = final_state;
                    move || {
                        state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .acknowledging = false;
                    }
                }))
        })
    }

    pub fn cancel<R: Send + Sync + 'static>(&self) -> Effect<(), RpcStreamError<E, Backend>, R> {
        let reader = self.clone();
        Effect::try_sync(
            move || {
                let mut state = reader
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match state.phase {
                    StreamPhase::Open => state.phase = StreamPhase::Cancelled,
                    StreamPhase::Ended => return Err(RpcStreamError::Ended),
                    StreamPhase::Cancelled => return Err(RpcStreamError::Cancelled),
                    StreamPhase::Failed => return Err(RpcStreamError::Failed),
                }
                let envelope = RpcEnvelope {
                    operation: reader.spec.id.clone(),
                    version: reader.spec.version,
                    call: reader.call.clone(),
                    kind: RpcEnvelopeKind::Cancel,
                    sequence: None,
                    payload: JsonValue::Null,
                    metadata: BTreeMap::new(),
                };
                envelope.encode(reader.policy).map_err(RpcStreamError::Wire)
            },
            std::convert::identity,
        )
        .flat_map({
            let session = self.session.clone();
            move |frame| {
                let frame = Mutex::new(Some(frame));
                session.access(Arc::clone).flat_map(move |session| {
                    let frame = frame
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    match frame {
                        Some(frame) => session
                            .send(frame)
                            .map_error(RpcStreamError::CancellationUncertain)
                            .provide_some(|_: &R| ()),
                        None => Effect::from_result_once(Err(RpcStreamError::InputConsumed)),
                    }
                })
            }
        })
    }
}

fn decode_frame(bytes: &[u8], policy: RpcEnvelopePolicy) -> Result<RpcEnvelope, RpcWireError> {
    let mut decoder = FrameDecoder::new(policy.frame);
    let frames = decoder
        .push(bytes)
        .map_err(|FrameDecodeFailure { error, .. }| RpcWireError::Frame(error))?;
    decoder.finish().map_err(RpcWireError::Frame)?;
    let mut frames = frames.into_iter();
    let Some(frame) = frames.next() else {
        return Err(RpcWireError::InvalidEnvelope);
    };
    if frames.next().is_some() {
        return Err(RpcWireError::InvalidEnvelope);
    }
    RpcEnvelope::decode(&frame, policy)
}
