//! Native axum WebSocket handshakes and core-owned duplex session adaptation.

use crate::{
    Endpoint, HasRequest, HttpMethodSet, Registration, Reply, Request, Router, ServerError,
};
use axum::extract::ws::{Message, WebSocket as NativeSocket, WebSocketUpgrade};
use effect::{
    Cancellation, Defect, Effect, ResourceValue, TransportFuture, WebSocket, WebSocketCloseInfo,
    WebSocketConnection, WebSocketFragmentKind, WebSocketFrame, WebSocketLimits, WebSocketMessage,
};
use futures_util::{SinkExt, StreamExt};
use http::{HeaderValue, Method};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// Explicit native buffer/handshake configuration; axum owns protocol validation.
#[derive(Clone)]
pub struct WebSocketOptions {
    /// Native receive-buffer allocation, measured in bytes.
    pub read_buffer_bytes: NonZeroUsize,
    /// Native write-buffer allocation, measured in bytes.
    pub write_buffer_bytes: NonZeroUsize,
    /// Maximum native pending write-buffer bytes.
    pub max_write_buffer_bytes: NonZeroUsize,
    /// Maximum native frame bytes.
    pub max_frame_bytes: NonZeroUsize,
    /// Core message/receive/close policy, all explicitly selected.
    pub limits: WebSocketLimits,
    /// Server-preferred subprotocols passed to axum's native negotiation.
    pub protocols: Vec<HeaderValue>,
    /// Native acceptance of unmasked frames is explicit and normally selected false by the caller.
    pub accept_unmasked_frames: bool,
}
/// Effect-owned authorization/handshake decision. Construction does not authenticate a request.
pub struct WebSocketPolicy<E: Send + 'static, R: Send + Sync + 'static> {
    /// Selected native options used after authorization.
    options: WebSocketOptions,
    /// Trusted application authentication/authorization callback in R.
    authorize: Arc<dyn Fn(Request) -> Effect<(), E, R> + Send + Sync>,
    /// Core socket errors mapped once into the application's E channel.
    on_error: Arc<dyn Fn(effect::WebSocketError) -> E + Send + Sync>,
    /// Native handshake/extraction errors mapped once into E.
    native_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> WebSocketPolicy<E, R> {
    /// Select explicit native bounds and real trusted authorization before upgrade.
    pub fn new(
        options: WebSocketOptions,
        authorize: impl Fn(Request) -> Effect<(), E, R> + Send + Sync + 'static,
        on_error: impl Fn(effect::WebSocketError) -> E + Send + Sync + 'static,
        native_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Result<Self, ServerError> {
        if options.max_write_buffer_bytes.get() <= options.write_buffer_bytes.get() {
            return Err(ServerError::Configuration(
                "WebSocket maximum write buffer must exceed write buffer".into(),
            ));
        }
        for protocol in &options.protocols {
            protocol
                .to_str()
                .map_err(|_| ServerError::Configuration("WebSocket subprotocol UTF-8".into()))?;
        }
        Ok(Self {
            options,
            authorize: Arc::new(authorize),
            on_error: Arc::new(on_error),
            native_error: Arc::new(native_error),
        })
    }
}
/// Native registration marker ensures extraction occurs before request parts move into their owner.
#[derive(Clone)]
pub(crate) struct WebSocketRoute;
/// One native upgrade owner; shared metadata does not clone the consumed handshake.
#[derive(Clone)]
pub(crate) struct UpgradeSlot(pub(crate) Arc<Mutex<Option<WebSocketUpgrade>>>);
/// Concurrent native send/receive owners; core retains assembly/close/finalization semantics.
struct AxumSocket {
    /// Independent sink lock permits duplex operation while a receive is pending.
    send: Arc<tokio::sync::Mutex<Option<futures_util::stream::SplitSink<NativeSocket, Message>>>>,
    /// Independent stream lock owns native receive ordering.
    receive: Arc<tokio::sync::Mutex<Option<futures_util::stream::SplitStream<NativeSocket>>>>,
    /// Cooperative abort releases pending native I/O through selection, never task abortion.
    abort: Cancellation,
    /// Selected native subprotocol retained as immutable metadata.
    protocol: Option<Arc<str>>,
}
impl AxumSocket {
    /// Split native duplex ownership once, preserving protocol metadata.
    fn new(socket: NativeSocket) -> Self {
        let protocol = socket
            .protocol()
            .and_then(|value| value.to_str().ok())
            .map(Arc::from);
        let (send, receive) = socket.split();
        Self {
            send: Arc::new(tokio::sync::Mutex::new(Some(send))),
            receive: Arc::new(tokio::sync::Mutex::new(Some(receive))),
            abort: Cancellation::new(),
            protocol,
        }
    }
}
impl WebSocket for AxumSocket {
    /// Move one native message under send ownership and cooperative cancellation.
    fn send(&self, message: WebSocketMessage, cancellation: Cancellation) -> TransportFuture<()> {
        let send = self.send.clone();
        let abort = self.abort.clone();
        Box::pin(async move {
            let message = match message {
                WebSocketMessage::Text(text) => Message::Text(text.to_string().into()),
                WebSocketMessage::Binary(bytes) => {
                    Message::Binary(bytes::Bytes::copy_from_slice(&bytes))
                }
            };
            tokio::select! { () = cancellation.cancelled() => Err(Defect::new("WebSocket send cancelled")), () = abort.cancelled() => Err(Defect::new("WebSocket aborted")), result = async { let mut send = send.lock().await; let sender = send.as_mut().ok_or_else(|| Defect::new("WebSocket send owner closed"))?; sender.send(message).await.map_err(|error| Defect::new(error.to_string())) } => result }
        })
    }
    /// Pull native complete frames; axum handles control frames and core handles message limits.
    /// Flush the native peer-close acknowledgment before core disposal can drop either half.
    fn receive(
        &self,
        _buffer: NonZeroUsize,
        cancellation: Cancellation,
    ) -> TransportFuture<WebSocketFrame> {
        let receive = self.receive.clone();
        let send = self.send.clone();
        let abort = self.abort.clone();
        Box::pin(async move {
            loop {
                let next = tokio::select! { () = cancellation.cancelled() => return Err(Defect::new("WebSocket receive cancelled")), () = abort.cancelled() => return Err(Defect::new("WebSocket aborted")), result = async { let mut receive = receive.lock().await; match receive.as_mut() { Some(stream) => stream.next().await, None => None } } => result };
                match next {
                    Some(Ok(Message::Text(text))) => {
                        return Ok(WebSocketFrame::Data {
                            kind: WebSocketFragmentKind::Text,
                            bytes: text.as_bytes().to_vec(),
                            end: true,
                        });
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        return Ok(WebSocketFrame::Data {
                            kind: WebSocketFragmentKind::Binary,
                            bytes: bytes.to_vec(),
                            end: true,
                        });
                    }
                    Some(Ok(Message::Close(frame))) => {
                        tokio::select! {
                            ()=cancellation.cancelled()=>return Err(Defect::new("WebSocket receive cancelled")),
                            ()=abort.cancelled()=>return Err(Defect::new("WebSocket aborted")),
                            result=async {let mut send=send.lock().await;let sender=send.as_mut().ok_or_else(||Defect::new("WebSocket send owner closed"))?;sender.flush().await.map_err(|error|Defect::new(error.to_string()))}=>result?,
                        }
                        return Ok(WebSocketFrame::Close(WebSocketCloseInfo {
                            status: frame.as_ref().map(|frame| frame.code),
                            description: frame.map(|frame| Arc::from(frame.reason.as_str())),
                        }));
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                    Some(Err(error)) => return Err(Defect::new(error.to_string())),
                    None => {
                        return Ok(WebSocketFrame::Close(WebSocketCloseInfo {
                            status: None,
                            description: None,
                        }));
                    }
                }
            }
        })
    }
    /// Start native close transmission; core joins peer close under its selected deadline.
    fn close(&self, cancellation: Cancellation) -> TransportFuture<()> {
        let send = self.send.clone();
        let abort = self.abort.clone();
        Box::pin(async move {
            tokio::select! { () = cancellation.cancelled() => Err(Defect::new("WebSocket close cancelled")), () = abort.cancelled() => Err(Defect::new("WebSocket aborted")), result = async { let mut send = send.lock().await; match send.as_mut() { Some(send) => send.send(Message::Close(None)).await.map_err(|error| Defect::new(error.to_string())), None => Ok(()) } } => result }
        })
    }
    /// Signal pending I/O cancellation; disposal below awaits ownership before dropping halves.
    fn abort(&self) -> Result<(), Defect> {
        let _ = self.abort.request();
        Ok(())
    }
    /// Join native owner locks and drop both halves exactly once; no asynchronous work runs in Drop.
    fn dispose(&self) -> TransportFuture<()> {
        let send = self.send.clone();
        let receive = self.receive.clone();
        let abort = self.abort.clone();
        Box::pin(async move {
            let _ = abort.request();
            send.lock().await.take();
            receive.lock().await.take();
            Ok(())
        })
    }
    /// Borrow immutable subprotocol selection, using a shared native metadata handle.
    fn subprotocol(&self) -> Option<Arc<str>> {
        self.protocol.clone()
    }
}
impl<E, R> Router<E, R>
where
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
{
    /// Register native GET/CONNECT upgrade methods with core-owned session execution.
    /// HTTP/2 extended CONNECT depends on the native serving/deployment support; this
    /// adapter does not certify an ingress proxy or invent another protocol stack.
    pub fn websocket(
        self,
        path: &str,
        policy: WebSocketPolicy<E, R>,
        session: impl Fn(ResourceValue<WebSocketConnection>) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Result<Self, ServerError> {
        let policy = Arc::new(policy);
        let session = Arc::new(session);
        let effect = Effect::service(|env: &R| env.request().clone()).flat_map(move |request| {
            let policy = policy.clone();
            let session = session.clone();
            let request_for_upgrade = request.clone();
            policy.authorize.as_ref()(request).flat_map(move |()| {
                let upgrade = request_for_upgrade
                    .with_extensions(|extensions| extensions.get::<UpgradeSlot>().cloned())
                    .and_then(|slot| {
                        slot.0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take()
                    });
                let Some(upgrade) = upgrade else {
                    return Effect::from_result_once(Err((policy.native_error)(
                        ServerError::Invalid("native WebSocket upgrade unavailable".into()),
                    )));
                };
                let options = &policy.options;
                let upgrade = upgrade
                    .read_buffer_size(options.read_buffer_bytes.get())
                    .write_buffer_size(options.write_buffer_bytes.get())
                    .max_write_buffer_size(options.max_write_buffer_bytes.get())
                    .max_frame_size(options.max_frame_bytes.get())
                    .max_message_size(options.limits.max_message_bytes.get())
                    .accept_unmasked_frames(options.accept_unmasked_frames)
                    .protocols(
                        options
                            .protocols
                            .iter()
                            .filter_map(|protocol| protocol.to_str().ok().map(str::to_owned))
                            .collect::<Vec<_>>(),
                    );
                let (send, receive) = oneshot::channel();
                let send = Arc::new(Mutex::new(Some(send)));
                let failed = send.clone();
                let response = upgrade
                    .on_failed_upgrade(move |error| {
                        if let Some(send) = failed
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take()
                        {
                            let _ = send.send(Err(error));
                        }
                    })
                    .on_upgrade(move |socket| async move {
                        if let Some(send) = send
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take()
                        {
                            let _ = send.send(Ok(socket));
                        }
                    });
                let map = policy.native_error.clone();
                let acquire = Effect::try_future_once(
                    move |_| async move {
                        receive
                            .await
                            .map_err(|_| ServerError::Disconnected)?
                            .map(|socket| Arc::new(AxumSocket::new(socket)) as Arc<dyn WebSocket>)
                            .map_err(|error| ServerError::Invalid(error.to_string()))
                    },
                    move |error| map(error),
                );
                let session = session.clone();
                let map = policy.on_error.clone();
                let work = effect::websocket::with_socket_map_error(
                    options.limits,
                    acquire,
                    move |error| map(error),
                    move |socket| session(socket),
                );
                Effect::from_result_once(Ok(Reply::native_owned(response, work)))
            })
        });
        let mut metadata = Registration::default();
        metadata.extensions.insert(WebSocketRoute);
        self.registered(
            HttpMethodSet::new(vec![Method::GET, Method::CONNECT])?,
            path,
            crate::Registered::new(Endpoint::from_effect(effect), metadata),
        )
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> crate::Server<E, R> {
    /// Retain server-selected native WebSocket configuration in request metadata before resolution.
    /// Individual route policies remain responsible for trusted authorization and core session bounds.
    #[must_use]
    pub fn websockets(self, options: WebSocketOptions) -> Self {
        self.before_environment(move |request| {
            let options = options.clone();
            Effect::sync(move || request.insert_extension(options.clone()))
        })
    }
}
