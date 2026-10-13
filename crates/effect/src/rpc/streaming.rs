//! Acknowledged RPC stream clients and sessions.

use super::{
    OperationId, RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcOperationKind,
    RpcOperationSpec, RpcVersion, RpcWireError,
};
use crate::encoding::{FrameDecodeFailure, FrameDecoder, FrameParts};
use crate::{Effect, JsonValue, ParseLimits, PureCodec, ResourceValue, SchemaIssue, Scope};
use crate::{
    effect, effect_map, effect_map_error, effect_provide_some, effect_resource,
    effect_scope_acquire, effect_sync, effect_try_sync, effect_uninterruptible, service,
};
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

/// Type alias for connector session in rpc streaming.
///
/// # Type parameters
/// - `R`: Environment type.
type ConnectorSession<R> = <<R as HasRpcStreamConnector>::Connector as RpcStreamConnector>::Session;
/// Type alias for rpc open stream effect in rpc streaming.
///
/// # Type parameters
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `R`: Environment type.
pub type RpcOpenStreamEffect<C, T, E, Backend, R> =
    Effect<RpcStreamReader<C, T, E, Backend, ConnectorSession<R>>, RpcStreamError<E, Backend>, R>;

/// Streaming has separate chunk and terminal schemas. A terminal result is
/// never smuggled through a unary response or encoded as a special chunk.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
pub struct RpcStreamOperation<Q, C, T, E> {
    spec: RpcOperationSpec,
    request: PureCodec<Q, JsonValue>,
    chunk: PureCodec<C, JsonValue>,
    terminal: PureCodec<T, JsonValue>,
    domain_error: PureCodec<E, JsonValue>,
    limits: ParseLimits,
}

impl<Q, C, T, E> Clone for RpcStreamOperation<Q, C, T, E> {
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Construct a `RpcStreamOperation` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `id` (`OperationId`): Stable identity of the selected record or operation.
    /// - `version` (`RpcVersion`): Registered schema, event, or workflow compatibility version.
    /// - `request` (`PureCodec<Q, JsonValue>`): The typed request to process.
    /// - `chunk` (`PureCodec<C, JsonValue>`): The chunk supplied to this operation.
    /// - `terminal` (`PureCodec<T, JsonValue>`): The terminal supplied to this operation.
    /// - `domain_error` (`PureCodec<E, JsonValue>`): The domain error supplied to this operation.
    /// - `limits` (`ParseLimits`): Explicit resource bounds enforced while processing this input.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Borrow the stable operation specification and its codecs.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `C`: Stream chunk or item type. Bound: `Send + 'static`.
    /// - `T`: Value or item type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `&RpcOperationSpec`.
    pub fn spec(&self) -> &RpcOperationSpec {
        &self.spec
    }
    #[must_use]
    /// Borrow the codec used to encode and decode requests.
    ///
    /// # Type parameters
    /// - `C`: Stream chunk or item type. Bound: `Send + 'static`.
    /// - `T`: Value or item type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `Q`: Request type.
    ///
    /// # Returns
    /// Type: `&PureCodec<Q, JsonValue>`.
    pub fn request_codec(&self) -> &PureCodec<Q, JsonValue> {
        &self.request
    }
    #[must_use]
    /// Borrow the codec used for one streaming item.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `T`: Value or item type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `C`: Type supplied by the enclosing impl or trait.
    ///
    /// # Returns
    /// Type: `&PureCodec<C, JsonValue>`.
    pub fn chunk_codec(&self) -> &PureCodec<C, JsonValue> {
        &self.chunk
    }
    #[must_use]
    /// Borrow the codec used for the stream’s terminal result.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `C`: Stream chunk or item type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `&PureCodec<T, JsonValue>`.
    pub fn terminal_codec(&self) -> &PureCodec<T, JsonValue> {
        &self.terminal
    }
    #[must_use]
    /// Borrow the codec used for expected domain errors.
    ///
    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `C`: Stream chunk or item type. Bound: `Send + 'static`.
    /// - `T`: Value or item type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `&PureCodec<E, JsonValue>`.
    pub fn error_codec(&self) -> &PureCodec<E, JsonValue> {
        &self.domain_error
    }
    #[must_use]
    /// # Type parameters
    /// - `Q`: Request type. Bound: `Send + 'static`.
    /// - `C`: Stream chunk or item type. Bound: `Send + 'static`.
    /// - `T`: Value or item type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `ParseLimits`.
    pub const fn limits(&self) -> ParseLimits {
        self.limits
    }

    /// Preserve request, chunk and terminal schemas while widening only the
    /// declared error channel of a derived streaming operation.
    #[must_use]
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `F`: Bounds: `Send + 'static`.
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `Q`: Request type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `domain_error` (`PureCodec<F, JsonValue>`): The domain error supplied to this operation.
    ///
    /// # Returns
    /// Type: `RpcStreamOperation<Q, C, T, F>`.
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
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;
    /// # Parameters
    /// - `frame` (`FrameParts`): The frame supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn send(&self, frame: FrameParts) -> Effect<(), Self::Error>;
    /// # Returns
    /// Type: `Effect<Option<Vec<u8>>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn receive_frame(&self) -> Effect<Option<Vec<u8>>, Self::Error>;
    /// # Returns
    /// Type: `Effect<(), Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn close(&self) -> Effect<(), Self::Error>;
}

/// Opens typed streaming RPC sessions through an injected transport.
pub trait RpcStreamConnector: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;
    /// Streaming RPC session returned by this connector.
    type Session: RpcStreamSession<Error = Self::Error>;
    /// # Returns
    /// Type: `Effect<Arc<Self::Session>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn open(&self) -> Effect<Arc<Self::Session>, Self::Error>;
}

/// Selects the streaming RPC connector in an application environment.
pub trait HasRpcStreamConnector {
    /// Streaming RPC connector selected by this environment.
    type Connector: RpcStreamConnector;
    /// # Returns
    /// Type: `Arc<Self::Connector>`.
    fn rpc_stream_connector(&self) -> Arc<Self::Connector>;
}

impl<T: RpcStreamConnector> HasRpcStreamConnector for Arc<T> {
    /// Type alias for connector in rpc streaming.
    type Connector = T;
    ///
    /// # Returns
    /// Type: `Arc<Self::Connector>`.
    /// Returns `Arc<Self::Connector>`.
    fn rpc_stream_connector(&self) -> Arc<Self::Connector> {
        Arc::clone(self)
    }
}

#[derive(Debug)]
/// A framing, sequence, acknowledgement, or transport failure in a stream.
///
/// # Type parameters
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
pub enum RpcStreamError<E, Backend> {
    /// The application’s expected domain operation failed. Retains the `E` detail.
    Domain(E),
    /// The request failed its declared schema validation. Retains the `SchemaIssue` detail.
    RequestSchema(SchemaIssue),
    /// A stream chunk failed its declared schema validation. Retains the `SchemaIssue` detail.
    ChunkSchema(SchemaIssue),
    /// The stream’s terminal value failed schema validation. Retains the `SchemaIssue` detail.
    TerminalSchema(SchemaIssue),
    /// An expected-error payload failed schema validation. Retains the `SchemaIssue` detail.
    ErrorSchema(SchemaIssue),
    /// RPC framing or envelope validation failed. Retains the `RpcWireError` detail.
    Wire(RpcWireError),
    /// The selected transport provider failed. Retains the `Backend` detail.
    Transport(Backend),
    /// The server may have accepted the request before the send failed.
    RequestUncertain {
        /// Identity of the RPC call. Type: `RpcCallId`.
        call: RpcCallId,
        /// The underlying failure or provider cause. Type: `Backend`.
        source: Backend,
    },
    /// The server may have advanced past this chunk before the acknowledgement
    /// send failed. Reconcile this call and sequence before retrying.
    AcknowledgementUncertain {
        /// Identity of the RPC call. Type: `RpcCallId`.
        call: RpcCallId,
        /// The ordered sequence position. Type: `NonZeroU64`.
        sequence: NonZeroU64,
        /// The underlying failure or provider cause. Type: `Backend`.
        source: Backend,
    },
    /// The local reader stopped, but the server may or may not have received
    /// the cancellation frame. Its external work can still be running.
    CancellationUncertain(Backend),
    /// The remote connection ended before settlement.
    Disconnected,
    /// The required frame is missing.
    MissingFrame,
    /// Unexpected additional frames were present.
    ExtraFrames,
    /// The correlation differs from the required or recorded value.
    CorrelationMismatch,
    /// The operation differs from the required or recorded value.
    OperationMismatch,
    /// The received kind is invalid in the current protocol state. Retains the `RpcEnvelopeKind` detail.
    UnexpectedKind(RpcEnvelopeKind),
    /// The required sequence is missing.
    MissingSequence,
    /// The sequence differs from the required or recorded value.
    SequenceMismatch {
        /// The value required by the requested condition. Type: `NonZeroU64`.
        expected: NonZeroU64,
        /// The value observed when the requested condition failed. Type: `NonZeroU64`.
        actual: NonZeroU64,
    },
    /// No further sequence can be allocated without exceeding its range.
    SequenceExhausted,
    /// The current stream item must be acknowledged before another pull. Retains the `NonZeroU64` detail.
    AcknowledgementRequired(NonZeroU64),
    /// No acknowledgement pending is available.
    NoAcknowledgementPending,
    /// Another read is already in progress.
    ConcurrentRead,
    /// Another acknowledgement is already in progress.
    ConcurrentAcknowledgement,
    /// The owned operation has already reached its end boundary.
    Ended,
    /// Execution was cancelled.
    Cancelled,
    /// The underlying operation failed.
    Failed,
    /// An owned input was already consumed.
    InputConsumed,
}

/// One ordered item or terminal result received from a streaming RPC.
///
/// # Type parameters
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
pub enum RpcStreamItem<C, T> {
    /// A streaming reply carries one ordered chunk.
    Chunk {
        /// Sequence number used for acknowledgement. Type: `NonZeroU64`.
        sequence: NonZeroU64,
        /// Typed chunk value. Type: `C`.
        value: C,
    },
    /// The stream or operation has reached its terminal state. Contains `T`.
    Terminal(T),
}

/// A typed client that opens RPC streams through a supplied connector.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
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
    /// Construct a `RpcStreamClient` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `Backend`: Backend-specific error or provider type.
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `operation` (`RpcStreamOperation<Q, C, T, E>`): Typed operation definition or callback selected for execution.
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(operation: RpcStreamOperation<Q, C, T, E>, policy: RpcEnvelopePolicy) -> Self {
        Self {
            operation,
            policy,
            _backend: std::marker::PhantomData,
        }
    }

    /// Lazily encode one owned request, open R's connector session, and register its closure with scope.
    ///
    /// Call identity, envelope policy and operation codecs validate the stream. The
    /// request/metadata pair is single-use; no provider is chosen by the operation.
    /// Once acquired, the session is scope-owned, including cancellation or failed
    /// reader construction, with close failures retained as cleanup outcomes.
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
        {
            let __dsl_program = effect_try_sync! {
            body: move || {
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
            }, on_error: std::convert::identity, };
            effect! {
                            let __dsl_program = __dsl_program.clone(); let call = call.clone(); let reader_operation = reader_operation.clone(); let scope = scope.clone(); let frame = __dsl_program?; {
                        let frame = Arc::new(Mutex::new(Some(frame)));
                        let scope = scope.clone();
                        let operation = reader_operation.clone();
                        let call = call.clone();
                        let connector =
                            {
                                let __dsl_program = service! { env: |env: &R| env.rpc_stream_connector(), }; effect! {
                                let connector = __dsl_program.clone()?; {
                                effect_provide_some! {
                                    program: effect_map_error! {
                                    program: connector
                                    .open(), map: RpcStreamError::Transport, }, project: |_: &R| (), }
                            }? } };
                        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let release_closed = Arc::clone(&closed);
                        {
                            let __dsl_program = effect_scope_acquire! {
            scope: scope, acquire: connector, release: move |session| {
                                let closed = Arc::clone(&release_closed);
                                effect_provide_some! {
                                    program: effect_map_error! {
                                    program: {
                                    let __dsl_factory = move || {
                                        if closed.swap(true, std::sync::atomic::Ordering::AcqRel) {
                                            effect_sync! { body { () }, }
                                        } else {
                                            session.close()
                                        }
                                    }; effect! { __dsl_factory()? } }, map: RpcStreamError::Transport, }, project: |_: &R| (), }
                            },
            }; effect! {
                let __dsl_program = __dsl_program.clone(); let call = call.clone(); let closed = closed.clone(); let frame = frame.clone(); let operation = operation.clone(); let session = __dsl_program?; {
                                let source = session.clone();
                                let operation = operation.clone();
                                let call = call.clone();
                                let frame = Arc::clone(&frame);
                                let closed = Arc::clone(&closed);
                                {
                                    let __dsl_program = session.access(Arc::clone); effect! {
                                    let __dsl_program = __dsl_program.clone(); let call = call.clone(); let closed = closed.clone(); let frame = frame.clone(); let operation = operation.clone(); let source = source.clone(); let session = __dsl_program?; (move || {
                                    let frame = frame
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                                        .take();
                                    let Some(frame) = frame else {
                                        return effect_try_sync! { once; body: { let __dsl_result = Err(RpcStreamError::InputConsumed); move || __dsl_result }, on_error: ::core::convert::identity, };
                                    };
                                    let reader = RpcStreamReader::new(
                                        source.clone(),
                                        operation.clone(),
                                        call.clone(),
                                        policy,
                                        Arc::clone(&closed),
                                    );
                                    effect_map! {
                                        program: effect_provide_some! {
                                        program: effect_map_error! {
                                        program: session
                                        .send(frame), map: {
                                            let uncertain_call = call.clone();
                                            move |source| RpcStreamError::RequestUncertain {
                                                call: uncertain_call.clone(),
                                                source,
                                            }
                                        }, }, project: |_: &R| (), }, map: move |_| reader.clone(), }
                                })()? } }
                            }? } }
                    }? }
        }
    }
}

/// Variants of stream phase used by rpc streaming.
enum StreamPhase {
    Open,
    Ended,
    Cancelled,
    Failed,
}

/// Typed reader state used by rpc streaming.
struct ReaderState {
    phase: StreamPhase,
    expected: Option<NonZeroU64>,
    awaiting_ack: Option<NonZeroU64>,
    reading: bool,
    acknowledging: bool,
}

/// A scoped reader that acknowledges an item before pulling the next one.
///
/// # Type parameters
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
/// - `Session`: Active stream-session provider type.
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
    closed: Arc<std::sync::atomic::AtomicBool>,
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
    ///
    /// # Returns
    /// Type: `Self`.
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
            closed: Arc::clone(&self.closed),
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
    ///
    /// # Type parameters
    /// - `Q`: Request payload type.
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `session` (`ResourceValue<Arc<Session>>`): Scoped connection or stream session retained until settlement.
    /// - `operation` (`RpcStreamOperation<Q, C, T, E>`): Typed operation definition or callback selected for execution.
    /// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
    fn new<Q>(
        session: ResourceValue<Arc<Session>>,
        operation: RpcStreamOperation<Q, C, T, E>,
        call: RpcCallId,
        policy: RpcEnvelopePolicy,
        closed: Arc<std::sync::atomic::AtomicBool>,
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
            closed,
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

    /// Lazily receive exactly one framed chunk or terminal result.
    ///
    /// A chunk must be acknowledged before another pull, and overlapping pulls are
    /// rejected before acquiring read ownership. Once reading starts, cancellation or
    /// failure triggers protected close exactly once; close failure is retained with
    /// the primary outcome. Terminal values use their own codec, never a special chunk.
    pub fn next<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<RpcStreamItem<C, T>, RpcStreamError<E, Backend>, R> {
        let state = Arc::clone(&self.state);
        let reader = self.clone();
        {
            let __dsl_program = effect_try_sync! {
            body: move || {
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
            }, on_error: std::convert::identity, };
            effect! {
                    let reader = reader.clone(); let _ = __dsl_program.clone()?; {
                let reader = reader.clone();
                let state = Arc::clone(&reader.state);
                let close = reader.close_once::<R>();
                {
                    let __dsl_program = {
                    let __dsl_program = {
                    let __dsl_program = reader
                    .session
                    .access(Arc::clone); effect! {
                        let reader = reader.clone(); let session = __dsl_program.clone()?; {
                        let reader = reader.clone();
                        {
                            let __dsl_program = effect_provide_some! {
                            program: effect_map_error! {
                            program: session
                            .receive_frame(), map: RpcStreamError::Transport, }, project: |_: &R| (), }; effect! {
                                let reader = reader.clone(); let bytes = __dsl_program.clone()?; {
                                let result = bytes
                                    .ok_or(RpcStreamError::Disconnected)
                                    .and_then(|bytes| reader.decode_next(&bytes));
                                effect_try_sync! { once; body: { let __dsl_result = result; move || __dsl_result }, on_error: ::core::convert::identity, }
                            }? } }
                    }? } }; let __dsl_cleanup = effect_sync! {
                        body: move || {
                        state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .reading = false;
                    }, }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }; effect_resource! {
                        acquire: effect_sync! { body: || (), }, release: move |(), reason| match reason {
                        crate::PrimaryOutcomeRef::Succeeded => crate::Release::sync(|| ()),
                        crate::PrimaryOutcomeRef::Failed(_)
                        | crate::PrimaryOutcomeRef::Defected(_)
                        | crate::PrimaryOutcomeRef::WasInterrupted => {
                            crate::Release::from_effect(close.clone())
                        }
                    }, body: move |_| __dsl_program.clone(), } }
            }? }
        }
    }

    ///
    /// # Type parameters
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
    ///
    /// # Returns
    /// Type: `Result<RpcStreamItem<C, T>, RpcStreamError<E, Backend>>`.
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

    ///
    /// # Returns
    /// Type: `()`.
    fn fail_protocol(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .phase = StreamPhase::Failed;
    }

    /// Lazily acknowledge the outstanding chunk's exact sequence before the next pull.
    ///
    /// Invalid/concurrent acknowledgement is rejected before taking ownership. Once
    /// acknowledgement begins, interruption or backend failure triggers protected close
    /// once and retains any close failure alongside the primary outcome.
    pub fn acknowledge<R: Send + Sync + 'static>(
        &self,
        sequence: NonZeroU64,
    ) -> Effect<(), RpcStreamError<E, Backend>, R> {
        let reader = self.clone();
        let state = Arc::clone(&self.state);
        {
            let __dsl_program = effect_try_sync! {
            body: move || {
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
            }, on_error: std::convert::identity, };
            effect! {
                           let reader = reader.clone(); let _ = __dsl_program.clone()?; {
                       let reader = reader.clone();
                       let close = reader.close_once::<R>();
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
                       {
                           let __dsl_program = {
                           let __dsl_program = {
                           let __dsl_program = effect_try_sync! { once; body: { let __dsl_result = envelope.encode(reader.policy).map_err(RpcStreamError::Wire); move || __dsl_result }, on_error: ::core::convert::identity, }; effect! {
                           let __dsl_program = __dsl_program.clone(); let session_handle = session_handle.clone(); let state = state.clone(); let uncertain_call = uncertain_call.clone(); let frame = __dsl_program?; {
                               let frame = Mutex::new(Some(frame));
                               let state = Arc::clone(&state);
                               let uncertain_call = uncertain_call.clone();
                               effect! {
            bind;
            program: session_handle.access(Arc::clone),
            body: move |session| {
                                   let frame = frame
                                       .lock()
                                       .unwrap_or_else(std::sync::PoisonError::into_inner)
                                       .take();
                                   match frame {
                                       Some(frame) => effect_map! {
                                           program: effect_provide_some! {
                                           program: effect_map_error! {
                                           program: session
                                           .send(frame), map: {
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
                                           }, }, project: |_: &R| (), }, map: {
                                               let state = Arc::clone(&state);
                                               move |_| {
                                                   state
                                                       .lock()
                                                       .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                       .awaiting_ack = None;
                                               }
                                           }, },
                                       None => effect_try_sync! { once; body: { let __dsl_result = Err(RpcStreamError::InputConsumed); move || __dsl_result }, on_error: ::core::convert::identity, },
                                   }
                               },
            }
                           }? } }; let __dsl_cleanup = effect_sync! {
                               body: {
                               let state = final_state;
                               move || {
                                   state
                                       .lock()
                                       .unwrap_or_else(std::sync::PoisonError::into_inner)
                                       .acknowledging = false;
                               }
                           }, }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }; effect_resource! {
                               acquire: effect_sync! { body: || (), }, release: move |(), reason| match reason {
                               crate::PrimaryOutcomeRef::Succeeded => crate::Release::sync(|| ()),
                               crate::PrimaryOutcomeRef::Failed(_)
                               | crate::PrimaryOutcomeRef::Defected(_)
                               | crate::PrimaryOutcomeRef::WasInterrupted => {
                                   crate::Release::from_effect(close.clone())
                               }
                           }, body: move |_| __dsl_program.clone(), } }
                   }? }
        }
    }

    /// Claim this session's close flag before invoking the backend, with interruption
    /// masked. All reader/acknowledgement/scope paths share the flag; failed attempts
    /// retain their cleanup outcome rather than retrying an already claimed close.
    fn close_once<R: Send + Sync + 'static>(&self) -> Effect<(), RpcStreamError<E, Backend>, R> {
        let closed = Arc::clone(&self.closed);
        effect_uninterruptible! {
        program: {
        let __dsl_program = self.session
        .access(Arc::clone); effect! {
            let closed = closed.clone(); let session = __dsl_program.clone()?; {
            let closed = Arc::clone(&closed);
            {
                let __dsl_factory = move || {
                if closed.swap(true, std::sync::atomic::Ordering::AcqRel) {
                    effect_sync! { body { () }, }
                } else {
                    effect_provide_some! {
                        program: effect_map_error! {
                        program: session
                        .close(), map: RpcStreamError::Transport, }, project: |_: &R| (), }
                }
            }; effect! { __dsl_factory()? } }
        }? } }, }
    }

    /// Lazily mark the stream cancelled and attempt protected session closure once.
    ///
    /// Shares close ownership with read, acknowledgement and scope cleanup, so a late
    /// close path does not close a replacement or repeat a previous close attempt.
    pub fn cancel<R: Send + Sync + 'static>(&self) -> Effect<(), RpcStreamError<E, Backend>, R> {
        let reader = self.clone();
        effect_uninterruptible! {
                   program: {
                   let __dsl_program = {
                   let __dsl_program = effect_try_sync! {
                   body: move || {
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
                   }, on_error: std::convert::identity, }; let session = self.session.clone(); effect! {
                       let __dsl_program = __dsl_program.clone(); let session = session.clone(); let frame = __dsl_program?; {
                       let frame = Mutex::new(Some(frame));
                       effect! {
        bind;
        program: session.access(Arc::clone),
        body: move |session| {
                           let frame = frame
                               .lock()
                               .unwrap_or_else(std::sync::PoisonError::into_inner)
                               .take();
                           match frame {
                               Some(frame) => effect_provide_some! {
                                   program: effect_map_error! {
                                   program: session
                                   .send(frame), map: RpcStreamError::CancellationUncertain, }, project: |_: &R| (), },
                               None => effect_try_sync! { once; body: { let __dsl_result = Err(RpcStreamError::InputConsumed); move || __dsl_result }, on_error: ::core::convert::identity, },
                           }
                       },
        }
                   }? } }; let __dsl_cleanup = self.close_once::<R>(); effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }, }
    }
}

///
/// # Parameters
/// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<RpcEnvelope, RpcWireError>`.
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
