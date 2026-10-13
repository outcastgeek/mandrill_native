//! Acknowledged RPC stream server execution.

use super::{
    RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcEnvelopePolicy, RpcStreamOperation,
    RpcStreamSession, RpcWireError,
};
use crate::encoding::{FrameDecodeFailure, FrameDecoder};
use crate::{Effect, JsonValue, SchemaIssue};
use crate::{
    effect, effect_map_error, effect_provide_some, effect_scope, effect_scope_acquire, effect_sync,
    effect_try_sync,
};
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::Mutex;

/// Type alias for stream pull in rpc streaming server.
///
/// # Type parameters
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type StreamPull<C, T, E, R> = dyn Fn() -> Effect<RpcProduced<C, T>, E, R> + Send + Sync;
/// Type alias for stream invoke in rpc streaming server.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
type StreamInvoke<Q, C, T, E, R> =
    dyn Fn(Q, RpcStreamContext) -> Effect<RpcStreamProducer<C, T, E, R>, E, R> + Send + Sync;

/// The next encoded stream item or terminal reply produced by a handler.
///
/// # Type parameters
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
pub enum RpcProduced<C, T> {
    /// Deliver one ordered chunk of stream data. Contains `C`.
    Chunk(C),
    /// The stream or operation has reached its terminal state. Contains `T`.
    Terminal(T),
}

/// Pull is application-defined and may use any services in R. The server
/// invokes it again only after the preceding chunk is acknowledged.
///
/// # Type parameters
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub struct RpcStreamProducer<C, T, E, R>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    pull: Arc<StreamPull<C, T, E, R>>,
}

impl<C, T, E, R> Clone for RpcStreamProducer<C, T, E, R>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            pull: Arc::clone(&self.pull),
        }
    }
}

impl<C, T, E, R> RpcStreamProducer<C, T, E, R>
where
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    /// Construct a `RpcStreamProducer` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `pull` (`impl Fn() -> Effect<RpcProduced<C, T>, E, R> + Send + Sync + 'static`): The pull supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(pull: impl Fn() -> Effect<RpcProduced<C, T>, E, R> + Send + Sync + 'static) -> Self {
        Self {
            pull: Arc::new(pull),
        }
    }

    /// Request the next stream chunk within the reader’s scope.
    ///
    /// # Type parameters
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Effect<RpcProduced<C, T>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn pull(&self) -> Effect<RpcProduced<C, T>, E, R> {
        (self.pull)()
    }
}

#[derive(Clone)]
/// Call identity and cancellation context supplied to a streaming handler.
pub struct RpcStreamContext {
    /// Identity of the RPC call. Type: `RpcCallId`.
    pub call: RpcCallId,
    /// Application or protocol metadata attached to this value. Type: `BTreeMap<Arc<str>, Arc<str>>`.
    pub metadata: BTreeMap<Arc<str>, Arc<str>>,
}

/// A registered typed handler that creates one scoped RPC stream.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub struct RpcStreamHandler<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    operation: RpcStreamOperation<Q, C, T, E>,
    invoke: Arc<StreamInvoke<Q, C, T, E, R>>,
}

impl<Q, C, T, E, R> Clone for RpcStreamHandler<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            operation: self.operation.clone(),
            invoke: Arc::clone(&self.invoke),
        }
    }
}

impl<Q, C, T, E, R> RpcStreamHandler<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    /// Construct a `RpcStreamHandler` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `operation` (`RpcStreamOperation<Q, C, T, E>`): Typed operation definition or callback selected for execution.
    /// - `invoke` (`impl Fn(Q, RpcStreamContext) -> Effect<RpcStreamProducer<C, T, E, R>, E, R> + Send + Sync + 'static`): The invoke supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        operation: RpcStreamOperation<Q, C, T, E>,
        invoke: impl Fn(Q, RpcStreamContext) -> Effect<RpcStreamProducer<C, T, E, R>, E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            operation,
            invoke: Arc::new(invoke),
        }
    }

    #[must_use]
    /// Borrow the typed operation definition associated with this value.
    ///
    /// # Type parameters
    /// - `R`: Environment type.
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `&RpcStreamOperation<Q, C, T, E>`.
    pub fn operation(&self) -> &RpcStreamOperation<Q, C, T, E> {
        &self.operation
    }

    /// Project opening and each producer pull through existing Effect provision.
    /// The request, streaming call context and protocol identity move through their
    /// original owners; no producer task or independent stream scope is introduced.
    pub fn provide_some<Outer: Send + Sync + 'static>(
        self,
        project: impl Fn(&Outer) -> R + Send + Sync + 'static,
    ) -> RpcStreamHandler<Q, C, T, E, Outer> {
        let project = Arc::new(project);
        RpcStreamHandler::new(self.operation, move |request, context| {
            let for_open = Arc::clone(&project);
            let for_pull = Arc::clone(&project);
            (self.invoke)(request, context)
                .map(move |producer| {
                    let project = Arc::clone(&for_pull);
                    RpcStreamProducer::new(move || {
                        let project = Arc::clone(&project);
                        producer.pull().provide_some(move |outer| project(outer))
                    })
                })
                .provide_some(move |outer| for_open(outer))
        })
    }

    /// Invoke the typed operation through its selected channel or handler.
    ///
    /// # Type parameters
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `request` (`Q`): The typed request to process.
    /// - `context` (`RpcStreamContext`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `Effect<RpcStreamProducer<C, T, E, R>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn call(
        &self,
        request: Q,
        context: RpcStreamContext,
    ) -> Effect<RpcStreamProducer<C, T, E, R>, E, R> {
        (self.invoke)(request, context)
    }
}

#[derive(Debug)]
/// A registration, handler, framing, or stream lifecycle failure.
///
/// # Type parameters
/// - `E`: Domain-error type.
/// - `Backend`: Backend-specific error or provider type.
pub enum RpcStreamServerError<E, Backend> {
    /// The registered typed handler failed. Retains the `E` detail.
    Handler(E),
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
    /// The remote connection ended before settlement.
    Disconnected,
    /// The operation differs from the required or recorded value.
    OperationMismatch,
    /// The received request kind is invalid in the current protocol state. Retains the `RpcEnvelopeKind` detail.
    UnexpectedRequestKind(RpcEnvelopeKind),
    /// The received acknowledgement kind is invalid in the current protocol state. Retains the `RpcEnvelopeKind` detail.
    UnexpectedAcknowledgementKind(RpcEnvelopeKind),
    /// The correlation differs from the required or recorded value.
    CorrelationMismatch,
    /// The required sequence is missing.
    MissingSequence,
    /// The control payload failed validation.
    InvalidControlPayload,
    /// The sequence differs from the required or recorded value.
    SequenceMismatch {
        /// The value required by the requested condition. Type: `NonZeroU64`.
        expected: NonZeroU64,
        /// The value observed when the requested condition failed. Type: `NonZeroU64`.
        actual: NonZeroU64,
    },
    /// No further sequence can be allocated without exceeding its range.
    SequenceExhausted,
    /// Execution was cancelled.
    Cancelled,
    /// An owned input was already consumed.
    InputConsumed,
}

/// Dispatches registered streaming operations and enforces acknowledgement order.
///
/// # Type parameters
/// - `Q`: Request type.
/// - `C`: Stream chunk or item type.
/// - `T`: Value or item type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
pub struct RpcStreamServer<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    handler: RpcStreamHandler<Q, C, T, E, R>,
    policy: RpcEnvelopePolicy,
}

impl<Q, C, T, E, R> Clone for RpcStreamServer<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
            policy: self.policy,
        }
    }
}

impl<Q, C, T, E, R> RpcStreamServer<Q, C, T, E, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    #[must_use]
    /// Construct a `RpcStreamServer` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `C`: Type supplied by the enclosing impl or trait.
    /// - `E`: Domain-error type.
    /// - `Q`: Request type.
    /// - `R`: Environment type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `handler` (`RpcStreamHandler<Q, C, T, E, R>`): The handler invoked for the matching request or event.
    /// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(handler: RpcStreamHandler<Q, C, T, E, R>, policy: RpcEnvelopePolicy) -> Self {
        Self { handler, policy }
    }

    /// Own one accepted session until terminal completion, cancellation or
    /// failure. The scope awaits the provider's close effect on every exit.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `C`: Stream chunk or item type.
    /// - `T`: Value or item type.
    /// - `Session`: Active stream-session provider type. Additional bound: `RpcStreamSession`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `accepted` (`Arc<Session>`): The accepted supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), RpcStreamServerError<E, Session::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn serve<Session>(
        &self,
        accepted: Arc<Session>,
    ) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
    where
        Session: RpcStreamSession,
    {
        self.serve_with_first(accepted, None)
    }

    /// The protocol dispatcher has already read this frame to select the
    /// operation. It must be decoded again against the selected typed codec.
    ///
    /// # Type parameters
    /// - `Q`: Request type.
    /// - `C`: Stream chunk or item type.
    /// - `T`: Value or item type.
    /// - `Session`: Active stream-session provider type. Additional bound: `RpcStreamSession`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `accepted` (`Arc<Session>`): The accepted supplied to this operation.
    /// - `first` (`Vec<u8>`): First value or branch in the ordered composition.
    ///
    /// # Returns
    /// Type: `Effect<(), RpcStreamServerError<E, Session::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn serve_prefetched<Session>(
        &self,
        accepted: Arc<Session>,
        first: Vec<u8>,
    ) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
    where
        Session: RpcStreamSession,
    {
        self.serve_with_first(accepted, Some(first))
    }

    ///
    /// # Type parameters
    /// - `Session`: Active stream-session provider type. Additional bound: `RpcStreamSession`.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `accepted` (`Arc<Session>`): The accepted supplied to this operation.
    /// - `first` (`Option<Vec<u8>>`): First value or branch in the ordered composition.
    ///
    /// # Returns
    /// Type: `Effect<(), RpcStreamServerError<E, Session::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn serve_with_first<Session>(
        &self,
        accepted: Arc<Session>,
        first: Option<Vec<u8>>,
    ) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
    where
        Session: RpcStreamSession,
    {
        let server = self.clone();
        let first = first.map(|bytes| Arc::new(Mutex::new(Some(bytes))));
        effect_scope! {
        body: move |scope| {
                    let accepted = Arc::clone(&accepted);
                    let server = server.clone();
                    let first = first.clone();
                    {
                        let __dsl_program = effect_scope_acquire! {
        scope: scope, acquire: effect_sync! {
            body: {
                                let accepted = Arc::clone(&accepted);
                                move || Arc::clone(&accepted)
                            }, }, release: |session: Arc<Session>| {
                                effect_provide_some! {
                                    program: effect_map_error! {
                                    program: session
                                        .close(), map: RpcStreamServerError::Transport, }, project: |_: &R| (), }
                            },
        }; effect! {
            let __dsl_program = __dsl_program.clone(); let first = first.clone(); let server = server.clone(); let resource = __dsl_program.clone()?; {
                            let server = server.clone();
                            let first = first.clone();
                            {
                                let __dsl_program = resource.access(Arc::clone); effect! {
                                let __dsl_program = __dsl_program.clone(); let first = first.clone(); let server = server.clone(); let session = __dsl_program.clone()?; {
                                let server = server.clone();
                                let receive = match &first {
                                    Some(first) => match first
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                                        .take()
                                    {
                                        Some(bytes) => effect_sync! { once; body: move || Some(bytes), },
                                        None => effect_try_sync! {
                                            once; body: {
                                            let __dsl_result = Err(
                                            RpcStreamServerError::InputConsumed,
                                        ); move || __dsl_result }, on_error: ::core::convert::identity, },
                                    },
                                    None => effect_provide_some! {
                                        program: effect_map_error! {
                                        program: session
                                        .receive_frame(), map: RpcStreamServerError::Transport, }, project: |_: &R| (), },
                                };
                                {
                                    let __dsl_program = receive; effect! {
                                    let __dsl_program = __dsl_program.clone(); let server = server.clone(); let session = session.clone(); let bytes = __dsl_program.clone()?; (move || {
                                    let Some(bytes) = bytes else {
                                        return effect_try_sync! {
                                            once; body: {
                                            let __dsl_result = Err(
                                            RpcStreamServerError::Disconnected,
                                        ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                    };
                                    let request =
                                        decode_request(server.handler.operation(), &bytes, server.policy);
                                    match request {
                                        Ok((request, context)) => {
                                            let call = context.call.clone();
                                            let operation = server.handler.operation().clone();
                                            let session = Arc::clone(&session);
                                            let policy = server.policy;
                                            {
                                                let __dsl_program = effect_map_error! {
                                                program: server
                                                .handler
                                                .call(request, context)
                                                .either(), map: RpcStreamServerError::Handler, }; effect! {
                                                    let __dsl_program = __dsl_program.clone(); let call = call.clone(); let operation = operation.clone(); let session = session.clone(); let result = __dsl_program?; match result {
                                                    Ok(producer) => serve_pulls(
                                                        Arc::clone(&session),
                                                        operation.clone(),
                                                        producer,
                                                        call.clone(),
                                                        Some(NonZeroU64::MIN),
                                                        policy,
                                                    ),
                                                    Err(error) => send_error(
                                                        Arc::clone(&session),
                                                        operation.clone(),
                                                        call.clone(),
                                                        error,
                                                        policy,
                                                    ),
                                                }? } }
                                        }
                                        Err(error) => effect_try_sync! { once; body: { let __dsl_result = Err(error); move || __dsl_result }, on_error: ::core::convert::identity, },
                                    }
                                })()? } }
                            }? } }
                        }? } }
                },
        }
    }
}

///
/// # Type parameters
/// - `E`: Expected application-error type.
/// - `B`: Transformed success-value type.
///
/// # Parameters
/// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<RpcEnvelope, RpcStreamServerError<E, B>>`.
fn decode_frame<E, B>(
    bytes: &[u8],
    policy: RpcEnvelopePolicy,
) -> Result<RpcEnvelope, RpcStreamServerError<E, B>> {
    let mut decoder = FrameDecoder::new(policy.frame);
    let frames = decoder
        .push(bytes)
        .map_err(|FrameDecodeFailure { error, .. }| {
            RpcStreamServerError::Wire(RpcWireError::Frame(error))
        })?;
    decoder
        .finish()
        .map_err(|error| RpcStreamServerError::Wire(RpcWireError::Frame(error)))?;
    let mut frames = frames.into_iter();
    let Some(first) = frames.next() else {
        return Err(RpcStreamServerError::Wire(RpcWireError::InvalidEnvelope));
    };
    if frames.next().is_some() {
        return Err(RpcStreamServerError::Wire(RpcWireError::InvalidEnvelope));
    }
    RpcEnvelope::decode(&first, policy).map_err(RpcStreamServerError::Wire)
}

///
/// # Type parameters
/// - `Q`: Request payload type. Additional bound: `Send + 'static`.
/// - `C`: Stream chunk or item type. Additional bound: `Send + 'static`.
/// - `T`: Stored or transferred value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `B`: Transformed success-value type.
///
/// # Parameters
/// - `operation` (`&RpcStreamOperation<Q, C, T, E>`): Typed operation definition or callback selected for execution.
/// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<(Q, RpcStreamContext), RpcStreamServerError<E, B>>`.
fn decode_request<Q, C, T, E, B>(
    operation: &RpcStreamOperation<Q, C, T, E>,
    bytes: &[u8],
    policy: RpcEnvelopePolicy,
) -> Result<(Q, RpcStreamContext), RpcStreamServerError<E, B>>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
{
    let envelope = decode_frame(bytes, policy)?;
    if envelope.operation != operation.spec().id || envelope.version != operation.spec().version {
        return Err(RpcStreamServerError::OperationMismatch);
    }
    if envelope.kind != RpcEnvelopeKind::Request {
        return Err(RpcStreamServerError::UnexpectedRequestKind(envelope.kind));
    }
    crate::schema::internal::parser::check_limits(&envelope.payload, &operation.limits())
        .map_err(RpcStreamServerError::RequestSchema)?;
    let request = operation
        .request_codec()
        .decode_ref(&envelope.payload)
        .map_err(RpcStreamServerError::RequestSchema)?;
    Ok((
        request,
        RpcStreamContext {
            call: envelope.call,
            metadata: envelope.metadata,
        },
    ))
}

///
/// # Type parameters
/// - `Q`: Request payload type. Additional bound: `Send + 'static`.
/// - `C`: Stream chunk or item type. Additional bound: `Send + 'static`.
/// - `T`: Stored or transferred value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `Session`: Active stream-session provider type. Additional bound: `RpcStreamSession`.
///
/// # Parameters
/// - `session` (`Arc<Session>`): Scoped connection or stream session retained until settlement.
/// - `operation` (`RpcStreamOperation<Q, C, T, E>`): Typed operation definition or callback selected for execution.
/// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
/// - `error` (`E`): Expected or provider error preserved in the returned outcome.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Effect<(), RpcStreamServerError<E, Session::Error>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn send_error<Q, C, T, E, R, Session>(
    session: Arc<Session>,
    operation: RpcStreamOperation<Q, C, T, E>,
    call: RpcCallId,
    error: E,
    policy: RpcEnvelopePolicy,
) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession,
{
    let payload = match operation.error_codec().encode_ref(&error) {
        Ok(payload) => payload,
        Err(issue) => {
            return effect_try_sync! { once; body: { let __dsl_result = Err(RpcStreamServerError::ErrorSchema(issue)); move || __dsl_result }, on_error: ::core::convert::identity, };
        }
    };
    if let Err(issue) = crate::schema::internal::parser::check_limits(&payload, &operation.limits())
    {
        return effect_try_sync! { once; body: { let __dsl_result = Err(RpcStreamServerError::ErrorSchema(issue)); move || __dsl_result }, on_error: ::core::convert::identity, };
    }
    send_envelope(
        session,
        RpcEnvelope {
            operation: operation.spec().id.clone(),
            version: operation.spec().version,
            call,
            kind: RpcEnvelopeKind::DomainError,
            sequence: None,
            payload,
            metadata: BTreeMap::new(),
        },
        policy,
    )
}

///
/// # Type parameters
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `Session`: Active stream-session provider type. Additional bound: `RpcStreamSession`.
///
/// # Parameters
/// - `session` (`Arc<Session>`): Scoped connection or stream session retained until settlement.
/// - `envelope` (`RpcEnvelope`): Versioned encoded message crossing the transport boundary.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Effect<(), RpcStreamServerError<E, Session::Error>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn send_envelope<E, R, Session>(
    session: Arc<Session>,
    envelope: RpcEnvelope,
    policy: RpcEnvelopePolicy,
) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession,
{
    let frame = match envelope.encode(policy) {
        Ok(frame) => frame,
        Err(error) => {
            return effect_try_sync! { once; body: { let __dsl_result = Err(RpcStreamServerError::Wire(error)); move || __dsl_result }, on_error: ::core::convert::identity, };
        }
    };
    effect_provide_some! {
    program: effect_map_error! {
    program: session
    .send(frame), map: RpcStreamServerError::Transport, }, project: |_: &R| (), }
}

///
/// # Type parameters
/// - `Q`: Request payload type. Additional bound: `Send + 'static`.
/// - `C`: Stream chunk or item type. Additional bound: `Send + 'static`.
/// - `T`: Stored or transferred value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `Session`: Active stream-session provider type. Additional bound: `RpcStreamSession`.
///
/// # Parameters
/// - `session` (`Arc<Session>`): Scoped connection or stream session retained until settlement.
/// - `operation` (`RpcStreamOperation<Q, C, T, E>`): Typed operation definition or callback selected for execution.
/// - `producer` (`RpcStreamProducer<C, T, E, R>`): The producer supplied to this operation.
/// - `call` (`RpcCallId`): Identity and contents of the selected RPC call.
/// - `sequence` (`Option<NonZeroU64>`): Ordered stream position used to reject gaps or duplicates.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Effect<(), RpcStreamServerError<E, Session::Error>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn serve_pulls<Q, C, T, E, R, Session>(
    session: Arc<Session>,
    operation: RpcStreamOperation<Q, C, T, E>,
    producer: RpcStreamProducer<C, T, E, R>,
    call: RpcCallId,
    sequence: Option<NonZeroU64>,
    policy: RpcEnvelopePolicy,
) -> Effect<(), RpcStreamServerError<E, Session::Error>, R>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    Session: RpcStreamSession,
{
    {
        let __dsl_factory = move || {
            let session = Arc::clone(&session);
            let operation = operation.clone();
            let producer = producer.clone();
            let call = call.clone();
            {
                let __dsl_program = effect_map_error! {
                program: producer
                .pull()
                .either(), map: RpcStreamServerError::Handler, };
                effect! {
                    let __dsl_program = __dsl_program.clone(); let call = call.clone(); let operation = operation.clone(); let producer = producer.clone(); let session = session.clone(); let result = __dsl_program?; (move || match result {
                    Err(error) => send_error(
                        Arc::clone(&session),
                        operation.clone(),
                        call.clone(),
                        error,
                        policy,
                    ),
                    Ok(RpcProduced::Terminal(value)) => {
                        let payload = match operation.terminal_codec().encode_ref(&value) {
                            Ok(payload) => payload,
                            Err(issue) => {
                                return effect_try_sync! {
                                    once; body: {
                                    let __dsl_result = Err(
                                    RpcStreamServerError::TerminalSchema(issue),
                                ); move || __dsl_result }, on_error: ::core::convert::identity, };
                            }
                        };
                        if let Err(issue) =
                            crate::schema::internal::parser::check_limits(&payload, &operation.limits())
                        {
                            return effect_try_sync! {
                                once; body: {
                                let __dsl_result = Err(
                                RpcStreamServerError::TerminalSchema(issue),
                            ); move || __dsl_result }, on_error: ::core::convert::identity, };
                        }
                        send_envelope(
                            Arc::clone(&session),
                            RpcEnvelope {
                                operation: operation.spec().id.clone(),
                                version: operation.spec().version,
                                call: call.clone(),
                                kind: RpcEnvelopeKind::StreamEnd,
                                sequence: None,
                                payload,
                                metadata: BTreeMap::new(),
                            },
                            policy,
                        )
                    }
                    Ok(RpcProduced::Chunk(value)) => {
                        let Some(sequence) = sequence else {
                            return effect_try_sync! {
                                once; body: {
                                let __dsl_result = Err(
                                RpcStreamServerError::SequenceExhausted,
                            ); move || __dsl_result }, on_error: ::core::convert::identity, };
                        };
                        let payload = match operation.chunk_codec().encode_ref(&value) {
                            Ok(payload) => payload,
                            Err(issue) => {
                                return effect_try_sync! {
                                    once; body: {
                                    let __dsl_result = Err(
                                    RpcStreamServerError::ChunkSchema(issue),
                                ); move || __dsl_result }, on_error: ::core::convert::identity, };
                            }
                        };
                        if let Err(issue) =
                            crate::schema::internal::parser::check_limits(&payload, &operation.limits())
                        {
                            return effect_try_sync! {
                                once; body: {
                                let __dsl_result = Err(RpcStreamServerError::ChunkSchema(
                                issue,
                            )); move || __dsl_result }, on_error: ::core::convert::identity, };
                        }
                        let frame = RpcEnvelope {
                            operation: operation.spec().id.clone(),
                            version: operation.spec().version,
                            call: call.clone(),
                            kind: RpcEnvelopeKind::StreamChunk,
                            sequence: Some(sequence),
                            payload,
                            metadata: BTreeMap::new(),
                        };
                        let session_for_ack = Arc::clone(&session);
                        let operation_for_ack = operation.clone();
                        let producer_for_ack = producer.clone();
                        let call_for_ack = call.clone();
                        {
                            let __dsl_program = send_envelope(Arc::clone(&session), frame, policy); effect! {
                            let __dsl_program = __dsl_program.clone(); let call_for_ack = call_for_ack.clone(); let operation_for_ack = operation_for_ack.clone(); let producer_for_ack = producer_for_ack.clone(); let session_for_ack = session_for_ack.clone(); let _ = __dsl_program?; {
                            let session = Arc::clone(&session_for_ack);
                            let operation = operation_for_ack.clone();
                            let producer = producer_for_ack.clone();
                            let call = call_for_ack.clone();
                            {
                                let __dsl_program = effect_provide_some! {
                                program: effect_map_error! {
                                program: session
                                .receive_frame(), map: RpcStreamServerError::Transport, }, project: |_: &R| (), }; effect! {
                                    let __dsl_program = __dsl_program.clone(); let call = call.clone(); let operation = operation.clone(); let producer = producer.clone(); let session = session.clone(); let bytes = __dsl_program?; (move || {
                                    let Some(bytes) = bytes else {
                                        return effect_try_sync! {
                                            once; body: {
                                            let __dsl_result = Err(
                                            RpcStreamServerError::Disconnected,
                                        ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                    };
                                    match decode_ack(&operation, &call, sequence, &bytes, policy) {
                                        Ok(()) => {
                                            let next =
                                                sequence.get().checked_add(1).and_then(NonZeroU64::new);
                                            serve_pulls(
                                                Arc::clone(&session),
                                                operation.clone(),
                                                producer.clone(),
                                                call.clone(),
                                                next,
                                                policy,
                                            )
                                        }
                                        Err(error) => effect_try_sync! { once; body: { let __dsl_result = Err(error); move || __dsl_result }, on_error: ::core::convert::identity, },
                                    }
                                })()? } }
                        }? } }
                    }
                })()? }
            }
        };
        effect! { __dsl_factory()? }
    }
}

///
/// # Type parameters
/// - `Q`: Request payload type. Additional bound: `Send + 'static`.
/// - `C`: Stream chunk or item type. Additional bound: `Send + 'static`.
/// - `T`: Stored or transferred value type. Additional bound: `Send + 'static`.
/// - `E`: Expected application-error type. Additional bound: `Send + 'static`.
/// - `B`: Transformed success-value type.
///
/// # Parameters
/// - `operation` (`&RpcStreamOperation<Q, C, T, E>`): Typed operation definition or callback selected for execution.
/// - `call` (`&RpcCallId`): Identity and contents of the selected RPC call.
/// - `expected` (`NonZeroU64`): Value or revision required for the proposed state transition.
/// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
/// - `policy` (`RpcEnvelopePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<(), RpcStreamServerError<E, B>>`.
fn decode_ack<Q, C, T, E, B>(
    operation: &RpcStreamOperation<Q, C, T, E>,
    call: &RpcCallId,
    expected: NonZeroU64,
    bytes: &[u8],
    policy: RpcEnvelopePolicy,
) -> Result<(), RpcStreamServerError<E, B>>
where
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
{
    let envelope = decode_frame(bytes, policy)?;
    if envelope.operation != operation.spec().id || envelope.version != operation.spec().version {
        return Err(RpcStreamServerError::OperationMismatch);
    }
    if envelope.call != *call {
        return Err(RpcStreamServerError::CorrelationMismatch);
    }
    match envelope.kind {
        RpcEnvelopeKind::Cancel if envelope.payload == JsonValue::Null => {
            Err(RpcStreamServerError::Cancelled)
        }
        RpcEnvelopeKind::Cancel => Err(RpcStreamServerError::InvalidControlPayload),
        RpcEnvelopeKind::Acknowledge => {
            if envelope.payload != JsonValue::Null {
                return Err(RpcStreamServerError::InvalidControlPayload);
            }
            let actual = envelope
                .sequence
                .ok_or(RpcStreamServerError::MissingSequence)?;
            if actual != expected {
                return Err(RpcStreamServerError::SequenceMismatch { expected, actual });
            }
            Ok(())
        }
        kind @ (RpcEnvelopeKind::Request
        | RpcEnvelopeKind::Success
        | RpcEnvelopeKind::DomainError
        | RpcEnvelopeKind::StreamChunk
        | RpcEnvelopeKind::StreamEnd) => {
            Err(RpcStreamServerError::UnexpectedAcknowledgementKind(kind))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{effect_sync, effect_try_sync, rpc_stream_operation};

    use super::*;
    use crate::encoding::{FrameParts, FramePolicy};
    use crate::rpc::{OperationId, RpcVersion};
    use crate::{Exit, ParseLimits, json_i64};
    use futures_executor::block_on;
    use std::num::{NonZeroU32, NonZeroUsize};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct BoundarySession {
        incoming: Arc<Mutex<Option<Vec<u8>>>>,
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl RpcStreamSession for BoundarySession {
        type Error = ();

        fn send(&self, frame: FrameParts) -> Effect<(), Self::Error> {
            let sent = Arc::clone(&self.sent);
            effect_try_sync! {
            once; body: move || {
                sent.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(frame.into_bytes().map_err(|_| ())?);
                Ok(())
            }, on_error: std::convert::identity, }
        }

        fn receive_frame(&self) -> Effect<Option<Vec<u8>>, Self::Error> {
            let incoming = Arc::clone(&self.incoming);
            effect_sync! {
                body: move || {
                incoming
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
            }, }
        }

        fn close(&self) -> Effect<(), Self::Error> {
            effect_sync! { body: || (), }
        }
    }

    #[test]
    fn final_sequence_can_be_acknowledged_before_terminal_but_not_another_chunk()
    -> Result<(), String> {
        let limit = NonZeroUsize::MAX;
        let policy = RpcEnvelopePolicy {
            frame: FramePolicy {
                maximum_payload: limit,
            },
            json: ParseLimits {
                bytes: limit,
                depth: limit,
                nodes: limit,
                collection: limit,
                issues: limit,
            },
        };
        let operation_id = OperationId::new("final-sequence")
            .map_err(|error| format!("operation identity: {error:?}"))?;
        let version = RpcVersion::new(NonZeroU32::MIN);
        let call = RpcCallId::new("final-sequence-call")
            .map_err(|error| format!("call identity: {error:?}"))?;
        let operation = rpc_stream_operation! {
            id: operation_id.clone(),
            version: version,
            request: json_i64(),
            chunk: json_i64(),
            terminal: json_i64(),
            error: json_i64(),
            limits: policy.json,
        };
        let ack = RpcEnvelope {
            operation: operation_id.clone(),
            version,
            call: call.clone(),
            kind: RpcEnvelopeKind::Acknowledge,
            sequence: Some(NonZeroU64::MAX),
            payload: JsonValue::Null,
            metadata: BTreeMap::new(),
        }
        .encode(policy)
        .map_err(|error| format!("acknowledgement envelope: {error:?}"))?
        .into_bytes()
        .map_err(|error| format!("acknowledgement frame: {error:?}"))?;
        let session = || {
            Arc::new(BoundarySession {
                incoming: Arc::new(Mutex::new(Some(ack.clone()))),
                sent: Arc::new(Mutex::new(Vec::new())),
            })
        };

        let pulls = Arc::new(AtomicUsize::new(0));
        let producer = RpcStreamProducer::new({
            let pulls = Arc::clone(&pulls);
            move || {
                let pulls = Arc::clone(&pulls);
                {
                    let __dsl_program: Effect<RpcProduced<i64, i64>, i64> = effect_sync! {
                        body: move || {
                        if pulls.fetch_add(1, Ordering::SeqCst) == 0 {
                            RpcProduced::Chunk(42)
                        } else {
                            RpcProduced::Terminal(43)
                        }
                    }, };
                    __dsl_program
                }
            }
        });
        let terminal_session = session();
        let completed = block_on(
            serve_pulls(
                Arc::clone(&terminal_session),
                operation.clone(),
                producer,
                call.clone(),
                Some(NonZeroU64::MAX),
                policy,
            )
            .run(Arc::new(())),
        );
        assert!(matches!(completed, Exit::Success(())));
        assert_eq!(pulls.load(Ordering::SeqCst), 2);
        let sent = terminal_session
            .sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(sent.len(), 2);
        let [first, last] = sent.as_slice() else {
            return Err(String::from("expected a final chunk and terminal result"));
        };
        let first = decode_frame::<i64, ()>(first, policy)
            .map_err(|error| format!("final chunk: {error:?}"))?;
        let last = decode_frame::<i64, ()>(last, policy)
            .map_err(|error| format!("terminal result: {error:?}"))?;
        assert_eq!(first.kind, RpcEnvelopeKind::StreamChunk);
        assert_eq!(first.sequence, Some(NonZeroU64::MAX));
        assert_eq!(last.kind, RpcEnvelopeKind::StreamEnd);
        assert_eq!(last.sequence, None);
        drop(sent);

        let extra_chunk = RpcStreamProducer::new(|| {
            let __dsl_program: Effect<RpcProduced<i64, i64>, i64> =
                effect_sync! { body: || RpcProduced::Chunk(42), };
            __dsl_program
        });
        let exhausted_session = session();
        let exhausted = block_on(
            serve_pulls(
                Arc::clone(&exhausted_session),
                operation,
                extra_chunk,
                call,
                Some(NonZeroU64::MAX),
                policy,
            )
            .run(Arc::new(())),
        );
        assert!(matches!(
            exhausted,
            Exit::Fail(RpcStreamServerError::SequenceExhausted)
        ));
        assert_eq!(
            exhausted_session
                .sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1
        );
        Ok(())
    }
}
