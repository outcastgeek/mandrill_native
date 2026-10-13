//! Transport-neutral WebSocket frames and scoped connections.

use crate::internal::gate::{Gate, GateError};
use crate::internal::runtime::{self, Step};
use crate::{
    AdmissionCapacity, Cancellation, Clock, ClockError, Defect, Effect, Exit, HasClock,
    ProviderAdmission, Release, ResourceValue, TransportFuture,
};
use crate::{
    effect_resource, effect_socket, effect_try_future, effect_try_sync, effect_uninterruptible,
};
use futures_util::{
    FutureExt,
    future::{Either, select},
};
use std::num::NonZeroUsize;
use std::panic::AssertUnwindSafe;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[derive(Clone)]
/// An assembled text or binary WebSocket message, after fragment validation.
pub enum WebSocketMessage {
    /// The message contains UTF-8 text. Contains `Arc<str>`.
    Text(Arc<str>),
    /// The message contains binary data. Contains `Arc<[u8]>`.
    Binary(Arc<[u8]>),
}
#[derive(Clone)]
/// The close code and reason observed or sent during a WebSocket close handshake.
pub struct WebSocketCloseInfo {
    /// The current status of this operation. Type: `Option<u16>`.
    pub status: Option<u16>,
    /// Optional human-readable description of the close event. Type: `Option<Arc<str>>`.
    pub description: Option<Arc<str>>,
}
/// The next message or close event returned by a WebSocket receive operation.
pub enum WebSocketReceive {
    /// Deliver an assembled application message. Contains `WebSocketMessage`.
    Message(WebSocketMessage),
    /// The provider or stream has closed admission. Contains `WebSocketCloseInfo`.
    Closed(WebSocketCloseInfo),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The operation phase associated with a WebSocket failure.
pub enum WebSocketOperation {
    /// Establish a transport connection.
    Connect,
    /// Write a message through the transport.
    Send,
    /// Read the next transport message.
    Receive,
    /// Close the resource or stream to further work.
    Close,
    /// Abort the in-flight operation and join its owned work.
    Abort,
    /// Release the owned native resource.
    Dispose,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A typed reason a frame sequence cannot form a valid message.
pub enum InvalidWebSocketMessage {
    /// A continuation fragment arrived before a message start.
    UnexpectedContinuation,
    /// A new message began before the previous fragments ended.
    UnexpectedMessageStart,
    /// The utf8 failed validation.
    InvalidUtf8,
    /// A message fragment arrived after the close frame.
    FragmentAfterClose,
}
#[derive(Debug)]
/// A transport, protocol, capacity, or lifecycle failure in a WebSocket operation.
pub enum WebSocketError {
    /// The provider has closed admission for new work.
    Closed,
    /// The message exceeds the caller’s size bound.
    MessageTooLarge {
        /// The caller-supplied bound. Type: `NonZeroUsize`.
        limit: NonZeroUsize,
    },
    /// The message failed validation. Carries `InvalidWebSocketMessage`.
    InvalidMessage(InvalidWebSocketMessage),
    /// The selected transport provider failed.
    Transport {
        /// Registered operation associated with this value. Type: `WebSocketOperation`.
        operation: WebSocketOperation,
        /// Underlying defect retained with this failure. Type: `Defect`.
        cause: Defect,
    },
    /// The close handshake did not settle before its deadline.
    CloseTimeout,
}
#[derive(Clone, Copy)]
/// Explicit resource bounds for websocket.
pub struct WebSocketLimits {
    /// The encoded bytes for max message. Type: `NonZeroUsize`.
    pub max_message_bytes: NonZeroUsize,
    /// The encoded bytes for receive buffer. Type: `NonZeroUsize`.
    pub receive_buffer_bytes: NonZeroUsize,
    /// The timeout for close. Type: `Duration`.
    pub close_timeout: Duration,
}
#[derive(Clone)]
/// Caller-supplied bounds and policies for opening a WebSocket connection.
pub struct WebSocketConnectOptions {
    /// WebSocket URI supplied to the transport. Type: `Arc<str>`.
    pub uri: Arc<str>,
    /// HTTP headers sent or received with this message. Type: `Arc<crate::HttpHeaders>`.
    pub headers: Arc<crate::HttpHeaders>,
    /// Subprotocols offered during the connection handshake. Type: `Arc<[Arc<str>]>`.
    pub subprotocols: Arc<[Arc<str>]>,
    /// Explicit resource bounds applied to this operation. Type: `WebSocketLimits`.
    pub limits: WebSocketLimits,
}
#[derive(Clone, Copy, PartialEq, Eq)]
/// The continuation or terminal role of a WebSocket frame fragment.
pub enum WebSocketFragmentKind {
    /// The message contains UTF-8 text.
    Text,
    /// The message contains binary data.
    Binary,
    /// More items remain after this response.
    Continuation,
}
/// One protocol frame consumed while assembling a WebSocket message.
pub enum WebSocketFrame {
    /// Deliver application data rather than a terminal control message.
    Data {
        /// Typed discriminator for this state. Type: `WebSocketFragmentKind`.
        kind: WebSocketFragmentKind,
        /// The owned encoded bytes. Type: `Vec<u8>`.
        bytes: Vec<u8>,
        /// Whether this fragment terminates the current message. Type: `bool`.
        end: bool,
    },
    /// Close the resource or stream to further work. Contains `WebSocketCloseInfo`.
    Close(WebSocketCloseInfo),
}
/// Native transport owns its reactor. abort must make admitted I/O settle;
/// dispose joins native ownership and is never substituted with future dropping.
/// A returned `Arc<dyn WebSocket>` uses Rust vtable dispatch; its concrete
/// implementation and vtable are never serialized or reused across processes.
pub trait WebSocket: Send + Sync + 'static {
    /// # Parameters
    /// - `message` (`WebSocketMessage`): Text or payload transferred to the selected consumer.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `TransportFuture<()>`.
    fn send(&self, message: WebSocketMessage, cancellation: Cancellation) -> TransportFuture<()>;
    /// # Parameters
    /// - `buffer` (`NonZeroUsize`): The buffer supplied to this operation.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `TransportFuture<WebSocketFrame>`.
    fn receive(
        &self,
        buffer: NonZeroUsize,
        cancellation: Cancellation,
    ) -> TransportFuture<WebSocketFrame>;
    /// # Parameters
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `TransportFuture<()>`.
    fn close(&self, cancellation: Cancellation) -> TransportFuture<()>;
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn abort(&self) -> Result<(), Defect>;
    /// # Returns
    /// Type: `TransportFuture<()>`.
    fn dispose(&self) -> TransportFuture<()>;
    /// # Returns
    /// Type: `Option<Arc<str>>`.
    /// Returns the value when present, or `None`.
    fn subprotocol(&self) -> Option<Arc<str>>;
}
/// Connects to WebSocket peers through a provider-owned transport.
/// The returned trait object dispatches through a Rust vtable while the
/// session lifetime and cancellation remain governed by this trait's methods.
pub trait WebSocketTransport: Send + Sync + 'static {
    /// # Parameters
    /// - `options` (`WebSocketConnectOptions`): Caller-selected bounds and behavior for this operation.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `TransportFuture<Arc<dyn WebSocket>>`.
    fn connect(
        &self,
        options: WebSocketConnectOptions,
        cancellation: Cancellation,
    ) -> TransportFuture<Arc<dyn WebSocket>>;
}
/// Selects the WebSocket transport in an application environment.
pub trait HasWebSocketTransport {
    /// Transport selected by this environment; an unsized trait object is permitted.
    type Transport: WebSocketTransport + ?Sized;
    /// # Returns
    /// Type: `&Self::Transport`.
    fn websocket_transport(&self) -> &Self::Transport;
}
/// Typed inner used by websocket.
struct Inner {
    socket: Arc<dyn WebSocket>,
    limits: WebSocketLimits,
    sending: Gate,
    receiving: Gate,
    closing: Gate,
    operations: ProviderAdmission,
    ended: AtomicBool,
}
#[derive(Clone)]
/// A scoped connection handle that serializes send and receive operations.
pub struct WebSocketConnection {
    inner: Arc<Inner>,
}
impl WebSocketConnection {
    ///
    /// # Parameters
    /// - `socket` (`Arc<dyn WebSocket>`): The socket supplied to this operation.
    /// - `limits` (`WebSocketLimits`): Explicit resource bounds enforced while processing this input.
    ///
    /// # Returns
    /// Type: `Self`.
    fn new(socket: Arc<dyn WebSocket>, limits: WebSocketLimits) -> Self {
        Self {
            inner: Arc::new(Inner {
                socket,
                limits,
                sending: Gate::new(),
                receiving: Gate::new(),
                closing: Gate::new(),
                operations: ProviderAdmission::new(AdmissionCapacity::Unbounded),
                ended: AtomicBool::new(false),
            }),
        }
    }
    #[must_use]
    /// Borrow the negotiated WebSocket subprotocol when one was selected.
    ///
    /// # Returns
    /// Type: `Option<Arc<str>>`.
    /// Returns the value when present, or `None`.
    pub fn subprotocol(&self) -> Option<Arc<str>> {
        self.inner.socket.subprotocol()
    }
    ///
    /// # Parameters
    /// - `operation` (`WebSocketOperation`): Typed operation definition or callback selected for execution.
    /// - `cause` (`Defect`): The cause supplied to this operation.
    ///
    /// # Returns
    /// Type: `WebSocketError`.
    /// Returns `WebSocketError`.
    fn transport(operation: WebSocketOperation, cause: Defect) -> WebSocketError {
        WebSocketError::Transport { operation, cause }
    }
    ///
    /// # Returns
    /// Type: `Result<(), WebSocketError>`.
    fn abort_now(&self) -> Result<(), WebSocketError> {
        self.inner.ended.store(true, Ordering::Release);
        self.inner.sending.close();
        self.inner.receiving.close();
        self.inner
            .socket
            .abort()
            .map_err(|cause| Self::transport(WebSocketOperation::Abort, cause))
    }
    ///
    /// # Returns
    /// Type: `Exit<(), WebSocketError>`.
    /// Returns `Exit<(), WebSocketError>`.
    async fn release(self) -> Exit<(), WebSocketError> {
        let aborted = self.abort_now();
        self.inner.closing.close();
        self.inner.operations.stop().wait().await;
        let disposed = self
            .inner
            .socket
            .dispose()
            .await
            .map_err(|cause| Self::transport(WebSocketOperation::Dispose, cause));
        let mut cleanup = Exit::Success(());
        for error in aborted.err().into_iter().chain(disposed.err()) {
            cleanup = crate::internal::outcome::append_cleanup(cleanup, Exit::Fail(error));
        }
        cleanup
    }
    /// Request cancellation of the in-flight WebSocket operation.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<(), WebSocketError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn abort<R: Send + Sync + 'static>(&self) -> Effect<(), WebSocketError, R> {
        let connection = self.clone();
        effect_uninterruptible! { program: effect_try_sync! { body: move || connection.abort_now(), on_error: |error| error, }, }
    }
    /// Send one message through the connection’s serialized send path.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `message` (`WebSocketMessage`): Text or payload transferred to the selected consumer.
    ///
    /// # Returns
    /// Type: `Effect<(), WebSocketError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn send<R: Send + Sync + 'static>(
        &self,
        message: WebSocketMessage,
    ) -> Effect<(), WebSocketError, R> {
        let connection = self.clone();
        operation(move |cancellation| {
            let connection = connection.clone();
            let message = message.clone();
            async move {
                let size = match &message {
                    WebSocketMessage::Text(text) => text.len(),
                    WebSocketMessage::Binary(bytes) => bytes.len(),
                };
                if size > connection.inner.limits.max_message_bytes.get() {
                    return Exit::Fail(WebSocketError::MessageTooLarge {
                        limit: connection.inner.limits.max_message_bytes,
                    });
                }
                let _admission = match connection.inner.operations.admit() {
                    Ok(permit) => permit,
                    Err(_) => return Exit::Fail(WebSocketError::Closed),
                };
                let _sending = match connection.inner.sending.enter(cancellation.clone()).await {
                    Ok(permit) => permit,
                    Err(GateError::Interrupted) => return Exit::Interrupted,
                    Err(GateError::Closed) => return Exit::Fail(WebSocketError::Closed),
                };
                if connection.inner.ended.load(Ordering::Acquire) {
                    return Exit::Fail(WebSocketError::Closed);
                }
                match connection.inner.socket.send(message, cancellation).await {
                    Ok(()) => Exit::Success(()),
                    Err(cause) => Exit::Fail(Self::transport(WebSocketOperation::Send, cause)),
                }
            }
        })
    }
    /// Read the next assembled message or close event within the caller’s bound.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<WebSocketReceive, WebSocketError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn receive<R: Send + Sync + 'static>(&self) -> Effect<WebSocketReceive, WebSocketError, R> {
        let connection = self.clone();
        operation(move |cancellation| {
            let connection = connection.clone();
            async move {
                let _admission = match connection.inner.operations.admit() {
                    Ok(permit) => permit,
                    Err(_) => return Exit::Fail(WebSocketError::Closed),
                };
                let _receiving = match connection.inner.receiving.enter(cancellation.clone()).await
                {
                    Ok(permit) => permit,
                    Err(GateError::Interrupted) => return Exit::Interrupted,
                    Err(GateError::Closed) => return Exit::Fail(WebSocketError::Closed),
                };
                if connection.inner.ended.load(Ordering::Acquire) {
                    return Exit::Fail(WebSocketError::Closed);
                }
                let mut bytes = Vec::new();
                let mut kind = None;
                loop {
                    let frame = match connection
                        .inner
                        .socket
                        .receive(
                            connection.inner.limits.receive_buffer_bytes,
                            cancellation.clone(),
                        )
                        .await
                    {
                        Ok(frame) => frame,
                        Err(cause) => {
                            return Exit::Fail(Self::transport(WebSocketOperation::Receive, cause));
                        }
                    };
                    match frame {
                        WebSocketFrame::Close(info) => {
                            return Exit::Success(WebSocketReceive::Closed(info));
                        }
                        WebSocketFrame::Data {
                            kind: fragment,
                            bytes: mut data,
                            end,
                        } => {
                            match (kind, fragment) {
                                (None, WebSocketFragmentKind::Continuation) => {
                                    return Exit::Fail(WebSocketError::InvalidMessage(
                                        InvalidWebSocketMessage::UnexpectedContinuation,
                                    ));
                                }
                                (
                                    None,
                                    start @ (WebSocketFragmentKind::Text
                                    | WebSocketFragmentKind::Binary),
                                ) => kind = Some(start),
                                (Some(_), WebSocketFragmentKind::Continuation) => {}
                                (
                                    Some(_),
                                    WebSocketFragmentKind::Text | WebSocketFragmentKind::Binary,
                                ) => {
                                    return Exit::Fail(WebSocketError::InvalidMessage(
                                        InvalidWebSocketMessage::UnexpectedMessageStart,
                                    ));
                                }
                            }
                            if data.len()
                                > connection
                                    .inner
                                    .limits
                                    .max_message_bytes
                                    .get()
                                    .saturating_sub(bytes.len())
                            {
                                return Exit::Fail(WebSocketError::MessageTooLarge {
                                    limit: connection.inner.limits.max_message_bytes,
                                });
                            }
                            bytes.append(&mut data);
                            if end {
                                return match kind {
                                    Some(WebSocketFragmentKind::Text) => {
                                        match String::from_utf8(bytes) {
                                            Ok(text) => Exit::Success(WebSocketReceive::Message(
                                                WebSocketMessage::Text(text.into()),
                                            )),
                                            Err(_) => Exit::Fail(WebSocketError::InvalidMessage(
                                                InvalidWebSocketMessage::InvalidUtf8,
                                            )),
                                        }
                                    }
                                    Some(WebSocketFragmentKind::Binary) => {
                                        Exit::Success(WebSocketReceive::Message(
                                            WebSocketMessage::Binary(bytes.into()),
                                        ))
                                    }
                                    Some(WebSocketFragmentKind::Continuation) | None => {
                                        Exit::Fail(WebSocketError::InvalidMessage(
                                            InvalidWebSocketMessage::UnexpectedContinuation,
                                        ))
                                    }
                                };
                            }
                        }
                    }
                }
            }
        })
    }
    /// Close admission and settle owned operations before returning.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `HasClock + Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<(), WebSocketError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn close<R: HasClock + Send + Sync + 'static>(&self) -> Effect<(), WebSocketError, R> {
        let connection = self.clone();
        Effect::new(move |env: Arc<R>, context, finish| {
            let connection = connection.clone();
            Step::Suspended(Box::pin(async move {
                let work = async {
                    let _admission = match connection.inner.operations.admit() {
                        Ok(permit) => permit,
                        Err(_) => return Exit::Fail(WebSocketError::Closed),
                    };
                    let _closing = match connection
                        .inner
                        .closing
                        .enter(context.cancellation.clone())
                        .await
                    {
                        Ok(permit) => permit,
                        Err(GateError::Interrupted) => return Exit::Interrupted,
                        Err(GateError::Closed) => return Exit::Fail(WebSocketError::Closed),
                    };
                    if connection.inner.ended.swap(true, Ordering::AcqRel) {
                        return Exit::Fail(WebSocketError::Closed);
                    }
                    connection.inner.sending.close();
                    let closing = connection.inner.socket.close(context.cancellation.clone());
                    let timeout = env.clock().sleep(
                        connection.inner.limits.close_timeout,
                        context.cancellation.clone(),
                    );
                    match select(closing, timeout).await {
                        Either::Left((Ok(()), _)) => Exit::Success(()),
                        Either::Left((Err(cause), _)) => {
                            Exit::Fail(Self::transport(WebSocketOperation::Close, cause))
                        }
                        Either::Right((timer, closing)) => {
                            let aborted = connection.abort_now();
                            let settled = closing.await;
                            let primary = match timer {
                                Ok(tick) => {
                                    drop(tick);
                                    Exit::Fail(WebSocketError::CloseTimeout)
                                }
                                Err(ClockError::Interrupted) => Exit::Interrupted,
                                Err(ClockError::Overflow) => Exit::Die(Defect::from(
                                    crate::LibraryDefect::ClockDeadlineOverflow,
                                )),
                                Err(ClockError::Defect(defect)) => Exit::Die(defect),
                            };
                            let mut outcome = primary;
                            for error in aborted.err().into_iter().chain(
                                settled
                                    .err()
                                    .map(|cause| Self::transport(WebSocketOperation::Close, cause)),
                            ) {
                                outcome = crate::internal::outcome::append_cleanup(
                                    outcome,
                                    Exit::Fail(error),
                                );
                            }
                            outcome
                        }
                    }
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, runtime::publish(outcome, &context))
            }))
        })
    }
}
///
/// # Type parameters
/// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
/// - `R`: Environment type supplying required capabilities. Additional bound: `Send + Sync + 'static`.
/// - `F`: Callback or field-definition type. Additional bound: `std::future::Future<Output = Exit<A, WebSocketError>> + Send + 'static`.
/// - `Make`: Dependency-construction callback type. Additional bound: `Fn(Cancellation) -> F + Send + Sync + 'static`.
///
/// # Parameters
/// - `make` (`Make`): The make supplied to this operation.
///
/// # Returns
/// Type: `Effect<A, WebSocketError, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn operation<A, R, F, Make>(make: Make) -> Effect<A, WebSocketError, R>
where
    A: Send + 'static,
    R: Send + Sync + 'static,
    F: std::future::Future<Output = Exit<A, WebSocketError>> + Send + 'static,
    Make: Fn(Cancellation) -> F + Send + Sync + 'static,
{
    let make = Arc::new(make);
    Effect::new(move |env: Arc<R>, context, finish| {
        let make = Arc::clone(&make);
        Step::Suspended(Box::pin(async move {
            let outcome = AssertUnwindSafe(async { make(context.cancellation.clone()).await })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
            drop(env);
            runtime::finish(finish, runtime::publish(outcome, &context))
        }))
    })
}
/// Acquire a socket, run scoped work, and close the connection afterward.
///
/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `limits` (`WebSocketLimits`): Explicit resource bounds enforced while processing this input.
/// - `acquire` (`Effect<Arc<dyn WebSocket>, WebSocketError, R>`): Description that obtains the resource before its body runs.
/// - `use_connection` (`impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, WebSocketError, R> + Send + Sync + 'static`): The use connection supplied to this operation.
///
/// # Returns
/// Type: `Effect<A, WebSocketError, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn with_socket<A: Send + 'static, R: Send + Sync + 'static>(
    limits: WebSocketLimits,
    acquire: Effect<Arc<dyn WebSocket>, WebSocketError, R>,
    use_connection: impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, WebSocketError, R>
    + Send
    + Sync
    + 'static,
) -> Effect<A, WebSocketError, R> {
    effect_resource! {
    acquire: own_socket(acquire, limits), release: |connection, _| {
            Release::from_exit_future(move |_| connection.release())
        }, body: use_connection,
    }
}
/// Run scoped work using an already established connection.
///
/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `R`: Environment type. Bounds: `HasWebSocketTransport + Send + Sync + 'static`.
///
/// # Parameters
/// - `options` (`WebSocketConnectOptions`): Caller-selected bounds and behavior for this operation.
/// - `use_connection` (`impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, WebSocketError, R> + Send + Sync + 'static`): The use connection supplied to this operation.
///
/// # Returns
/// Type: `Effect<A, WebSocketError, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn with_connection<A: Send + 'static, R: HasWebSocketTransport + Send + Sync + 'static>(
    options: WebSocketConnectOptions,
    use_connection: impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, WebSocketError, R>
    + Send
    + Sync
    + 'static,
) -> Effect<A, WebSocketError, R> {
    let limits = options.limits;
    let acquire = effect_try_future! { with_env; body: move |env: Arc<R>, cancellation| {
        env.websocket_transport()
            .connect(options.clone(), cancellation)
    }, on_error: |cause| WebSocketConnection::transport(WebSocketOperation::Connect, cause), };
    effect_socket! {
        limits: limits,
        acquire: acquire,
        body: use_connection,
    }
}
/// Acquire and use a scoped socket while mapping transport errors into the caller’s domain.
///
/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `E`: Domain-error type. Bounds: `Send + 'static`.
/// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `limits` (`WebSocketLimits`): Explicit resource bounds enforced while processing this input.
/// - `acquire` (`Effect<Arc<dyn WebSocket>, E, R>`): Description that obtains the resource before its body runs.
/// - `map_error` (`impl Fn(WebSocketError) -> E + Send + Sync + 'static`): The map error supplied to this operation.
/// - `use_connection` (`impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, E, R> + Send + Sync + 'static`): The use connection supplied to this operation.
///
/// # Returns
/// Type: `Effect<A, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn with_socket_map_error<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    limits: WebSocketLimits,
    acquire: Effect<Arc<dyn WebSocket>, E, R>,
    map_error: impl Fn(WebSocketError) -> E + Send + Sync + 'static,
    use_connection: impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, E, R>
    + Send
    + Sync
    + 'static,
) -> Effect<A, E, R> {
    let map_error = Arc::new(map_error);
    effect_resource! {
    acquire: own_socket(acquire, limits), release: move |connection, _| {
            let map_error = Arc::clone(&map_error);
            Release::from_exit_future(move |_| async move {
                connection
                    .release()
                    .await
                    .map_error(move |error| map_error(error))
            })
        }, body: use_connection,
    }
}
/// Use an existing scoped connection while mapping transport errors.
///
/// # Type parameters
/// - `A`: Application value type. Bounds: `Send + 'static`.
/// - `E`: Domain-error type. Bounds: `Send + 'static`.
/// - `R`: Environment type. Bounds: `HasWebSocketTransport + Send + Sync + 'static`.
///
/// # Parameters
/// - `options` (`WebSocketConnectOptions`): Caller-selected bounds and behavior for this operation.
/// - `map_error` (`impl Fn(WebSocketError) -> E + Send + Sync + 'static`): The map error supplied to this operation.
/// - `use_connection` (`impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, E, R> + Send + Sync + 'static`): The use connection supplied to this operation.
///
/// # Returns
/// Type: `Effect<A, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn with_connection_map_error<
    A: Send + 'static,
    E: Send + 'static,
    R: HasWebSocketTransport + Send + Sync + 'static,
>(
    options: WebSocketConnectOptions,
    map_error: impl Fn(WebSocketError) -> E + Send + Sync + 'static,
    use_connection: impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, E, R>
    + Send
    + Sync
    + 'static,
) -> Effect<A, E, R> {
    let map_error = Arc::new(map_error);
    let connect_error = Arc::clone(&map_error);
    let limits = options.limits;
    let acquire = effect_try_future! { with_env; body: move |env: Arc<R>, cancellation| {
        env.websocket_transport()
            .connect(options.clone(), cancellation)
    }, on_error: move |cause| {
        connect_error(WebSocketConnection::transport(
            WebSocketOperation::Connect,
            cause,
        ))
    }, };
    effect_socket! {
        limits: limits,
        acquire: acquire,
        on_error: move |error| map_error(error),
        body: use_connection,
    }
}

///
/// # Type parameters
/// - `E`: Bounds: `Send + 'static`.
/// - `R`: Bounds: `Send + Sync + 'static`.
///
/// # Parameters
/// - `acquire` (`Effect<Arc<dyn WebSocket>, E, R>`): Description that obtains the resource before its body runs.
/// - `limits` (`WebSocketLimits`): Explicit resource bounds enforced while processing this input.
///
/// # Returns
/// Type: `Effect<WebSocketConnection, E, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn own_socket<E: Send + 'static, R: Send + Sync + 'static>(
    acquire: Effect<Arc<dyn WebSocket>, E, R>,
    limits: WebSocketLimits,
) -> Effect<WebSocketConnection, E, R> {
    Effect::new(move |env: Arc<R>, context, finish| {
        acquire.clone().execute(
            env,
            context,
            runtime::continuation(move |outcome| {
                let outcome = match outcome {
                    Exit::Success(socket) => {
                        Exit::Success(WebSocketConnection::new(socket, limits))
                    }
                    Exit::Fail(error) => Exit::Fail(error),
                    Exit::Die(defect) => Exit::Die(defect),
                    Exit::Interrupted => Exit::Interrupted,
                    Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
                };
                runtime::finish(finish, outcome)
            }),
        )
    })
}
