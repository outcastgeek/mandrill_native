use crate::internal::gate::{Gate, GateError};
use crate::internal::runtime::{self, Step};
use crate::{
    AdmissionCapacity, Cancellation, Clock, ClockError, Defect, Effect, Exit, HasClock,
    ProviderAdmission, Release, ResourceBinding, ResourceValue, TransportFuture,
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
pub enum WebSocketMessage {
    Text(Arc<str>),
    Binary(Arc<[u8]>),
}
#[derive(Clone)]
pub struct WebSocketCloseInfo {
    pub status: Option<u16>,
    pub description: Option<Arc<str>>,
}
pub enum WebSocketReceive {
    Message(WebSocketMessage),
    Closed(WebSocketCloseInfo),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebSocketOperation {
    Connect,
    Send,
    Receive,
    Close,
    Abort,
    Dispose,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidWebSocketMessage {
    UnexpectedContinuation,
    UnexpectedMessageStart,
    InvalidUtf8,
    FragmentAfterClose,
}
#[derive(Debug)]
pub enum WebSocketError {
    Closed,
    MessageTooLarge {
        limit: NonZeroUsize,
    },
    InvalidMessage(InvalidWebSocketMessage),
    Transport {
        operation: WebSocketOperation,
        cause: Defect,
    },
    CloseTimeout,
}
#[derive(Clone, Copy)]
pub struct WebSocketLimits {
    pub max_message_bytes: NonZeroUsize,
    pub receive_buffer_bytes: NonZeroUsize,
    pub close_timeout: Duration,
}
#[derive(Clone)]
pub struct WebSocketConnectOptions {
    pub uri: Arc<str>,
    pub headers: Arc<crate::HttpHeaders>,
    pub subprotocols: Arc<[Arc<str>]>,
    pub limits: WebSocketLimits,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WebSocketFragmentKind {
    Text,
    Binary,
    Continuation,
}
pub enum WebSocketFrame {
    Data {
        kind: WebSocketFragmentKind,
        bytes: Vec<u8>,
        end: bool,
    },
    Close(WebSocketCloseInfo),
}
/// Native transport owns its reactor. abort must make admitted I/O settle;
/// dispose joins native ownership and is never substituted with future dropping.
pub trait WebSocket: Send + Sync + 'static {
    fn send(&self, message: WebSocketMessage, cancellation: Cancellation) -> TransportFuture<()>;
    fn receive(
        &self,
        buffer: NonZeroUsize,
        cancellation: Cancellation,
    ) -> TransportFuture<WebSocketFrame>;
    fn close(&self, cancellation: Cancellation) -> TransportFuture<()>;
    fn abort(&self) -> Result<(), Defect>;
    fn dispose(&self) -> TransportFuture<()>;
    fn subprotocol(&self) -> Option<Arc<str>>;
}
pub trait WebSocketTransport: Send + Sync + 'static {
    fn connect(
        &self,
        options: WebSocketConnectOptions,
        cancellation: Cancellation,
    ) -> TransportFuture<Arc<dyn WebSocket>>;
}
pub trait HasWebSocketTransport {
    type Transport: WebSocketTransport + ?Sized;
    fn websocket_transport(&self) -> &Self::Transport;
}
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
pub struct WebSocketConnection {
    inner: Arc<Inner>,
}
impl WebSocketConnection {
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
    pub fn subprotocol(&self) -> Option<Arc<str>> {
        self.inner.socket.subprotocol()
    }
    fn transport(operation: WebSocketOperation, cause: Defect) -> WebSocketError {
        WebSocketError::Transport { operation, cause }
    }
    fn abort_now(&self) -> Result<(), WebSocketError> {
        self.inner.ended.store(true, Ordering::Release);
        self.inner.sending.close();
        self.inner.receiving.close();
        self.inner
            .socket
            .abort()
            .map_err(|cause| Self::transport(WebSocketOperation::Abort, cause))
    }
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
    pub fn abort<R: Send + Sync + 'static>(&self) -> Effect<(), WebSocketError, R> {
        let connection = self.clone();
        Effect::try_sync(move || connection.abort_now(), |error| error).uninterruptible()
    }
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
pub fn with_socket<A: Send + 'static, R: Send + Sync + 'static>(
    limits: WebSocketLimits,
    acquire: Effect<Arc<dyn WebSocket>, WebSocketError, R>,
    use_connection: impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, WebSocketError, R>
    + Send
    + Sync
    + 'static,
) -> Effect<A, WebSocketError, R> {
    ResourceBinding::new(own_socket(acquire, limits), |connection, _| {
        Release::from_exit_future(move |_| connection.release())
    })
    .use_with(use_connection)
}
pub fn with_connection<A: Send + 'static, R: HasWebSocketTransport + Send + Sync + 'static>(
    options: WebSocketConnectOptions,
    use_connection: impl Fn(ResourceValue<WebSocketConnection>) -> Effect<A, WebSocketError, R>
    + Send
    + Sync
    + 'static,
) -> Effect<A, WebSocketError, R> {
    let limits = options.limits;
    let acquire = Effect::try_future_with_env(
        move |env: Arc<R>, cancellation| {
            env.websocket_transport()
                .connect(options.clone(), cancellation)
        },
        |cause| WebSocketConnection::transport(WebSocketOperation::Connect, cause),
    );
    with_socket(limits, acquire, use_connection)
}
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
    ResourceBinding::new(own_socket(acquire, limits), move |connection, _| {
        let map_error = Arc::clone(&map_error);
        Release::from_exit_future(move |_| async move {
            connection
                .release()
                .await
                .map_error(move |error| map_error(error))
        })
    })
    .use_with(use_connection)
}
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
    let acquire = Effect::try_future_with_env(
        move |env: Arc<R>, cancellation| {
            env.websocket_transport()
                .connect(options.clone(), cancellation)
        },
        move |cause| {
            connect_error(WebSocketConnection::transport(
                WebSocketOperation::Connect,
                cause,
            ))
        },
    );
    with_socket_map_error(
        limits,
        acquire,
        move |error| map_error(error),
        use_connection,
    )
}

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
