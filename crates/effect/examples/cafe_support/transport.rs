//! Finite transport and producer adapter for the café order-tracking story.

use effect::encoding::FrameParts;
use effect::rpc::{
    RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcOperation, RpcProduced, RpcStreamOperation,
    RpcStreamProducer, RpcStreamSession,
};
use effect::websocket::{
    WebSocket, WebSocketCloseInfo, WebSocketConnectOptions, WebSocketFragmentKind, WebSocketFrame,
    WebSocketMessage, WebSocketTransport,
};
use effect::{Cancellation, Defect, Effect, TransportFuture, effect};
use effect::{effect_die, effect_sync, effect_try_sync};
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// A finite in-process accepted transport, with observable close and responses.
pub struct Session {
    /// Frames remaining for the server to receive.
    input: Arc<Mutex<VecDeque<Vec<u8>>>>,
    /// Frames transferred by the server.
    output: Arc<Mutex<Vec<FrameParts>>>,
    /// Number of protected close operations completed.
    closed: Arc<AtomicUsize>,
}
impl Session {
    /// Accept one request and keep the peer open while its actual chunks are acknowledged.
    #[must_use]
    pub fn new(request: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            input: Arc::new(Mutex::new(VecDeque::from([request]))),
            output: Arc::new(Mutex::new(Vec::new())),
            closed: Arc::new(AtomicUsize::new(0)),
        })
    }
}
impl Session {
    /// Observe completion of protected transport cleanup.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) == 1
    }
}
impl RpcStreamSession for Session {
    /// Transport failures are distinct from the café's typed business errors.
    type Error = ();
    /// Accept each real frame; acknowledge only the chunk sequence actually delivered.
    fn send(&self, frame: FrameParts) -> Effect<(), ()> {
        let output = self.output.clone();
        let input = self.input.clone();
        effect_try_sync! { 
            once; body: move || {
            let envelope =
                RpcEnvelope::decode(frame.segments().1, super::rpc_policy()).map_err(|_| ())?;
            let acknowledgement = if matches!(envelope.kind, RpcEnvelopeKind::StreamChunk) {
                Some(
                    RpcEnvelope {
                        operation: envelope.operation,
                        version: envelope.version,
                        call: envelope.call,
                        kind: RpcEnvelopeKind::Acknowledge,
                        sequence: envelope.sequence,
                        payload: effect::JsonValue::Null,
                        metadata: Default::default(),
                    }
                    .encode(super::rpc_policy())
                    .map_err(|_| ())?
                    .into_bytes()
                    .map_err(|_| ())?,
                )
            } else {
                None
            };
            output.lock().map_err(|_| ())?.push(frame);
            if let Some(acknowledgement) = acknowledgement {
                input.lock().map_err(|_| ())?.push_back(acknowledgement);
            }
            Ok(())
        }, on_error: |error| error, }
    }
    /// Read the request or the ACK queued by the preceding send. The server awaits
    /// each send before this receive and performs no further receive after StreamEnd.
    fn receive_frame(&self) -> Effect<Option<Vec<u8>>, ()> {
        let input = self.input.clone();
        effect_try_sync! { 
            once; body: move || {
            input
                .lock()
                .map(|mut input| input.pop_front())
                .map_err(|_| ())
        }, on_error: |_| (), }
    }
    /// Complete protected native close after terminal response or a retained failure.
    fn close(&self) -> Effect<(), ()> {
        let closed = self.closed.clone();
        effect_sync! { 
            once; body: move || {
            closed.fetch_add(1, Ordering::SeqCst);
        }, }
    }
}
/// Finite owned producer plumbing; the macro handler chooses its business values.
#[must_use]
pub fn sequence<
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
>(
    chunks: Vec<C>,
    terminal: T,
) -> RpcStreamProducer<C, T, E, R> {
    let mut values = VecDeque::new();
    for chunk in chunks {
        values.push_back(RpcProduced::Chunk(chunk));
    }
    values.push_back(RpcProduced::Terminal(terminal));
    let values = Arc::new(Mutex::new(values));
    RpcStreamProducer::new(move || {
        let values = values.clone();
        effect! {
            let values = values.clone();
            let next = effect_sync! { once; body: move || values.lock().map(|mut values| values.pop_front()).map_err(|error| Defect::new(error.to_string())), }?;
            let next = match next { Ok(next) => next, Err(error) => effect_die! { defect: error, }?, };
            match next {
                Some(value) => value,
                None => effect_die! { defect: Defect::new("coffee order has no further status updates"), }?,
            }
        }
    })
}

/// Encode a typed customer request at the local transport boundary.
pub fn request<Q: Send + 'static, A: Send + 'static, E: Send + 'static>(
    operation: &RpcOperation<Q, A, E>,
    call: &str,
    value: &Q,
) -> Result<Vec<u8>, Defect> {
    RpcEnvelope {
        operation: operation.spec().id.clone(),
        version: operation.spec().version,
        call: RpcCallId::new(call)
            .map_err(|error| Defect::new(format!("customer call: {error:?}")))?,
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: operation
            .request_codec()
            .encode_ref(value)
            .map_err(|error| Defect::new(format!("typed customer request: {error:?}")))?,
        metadata: Default::default(),
    }
    .encode(super::rpc_policy())
    .map_err(|error| Defect::new(format!("local envelope: {error:?}")))?
    .into_bytes()
    .map_err(|error| Defect::new(format!("local frame: {error:?}")))
}
/// Decode the successful response of this finite, typed local order exchange.
/// Unexpected frame count, envelope kind or codec shape is an example-boundary defect.
pub fn response<Q: Send + 'static, A: Send + 'static, E: Send + 'static>(
    operation: &RpcOperation<Q, A, E>,
    bytes: &[u8],
) -> Result<A, Defect> {
    let policy = super::rpc_policy();
    let mut decoder = effect::encoding::FrameDecoder::new(policy.frame);
    let frames = decoder
        .push(bytes)
        .map_err(|error| Defect::new(format!("local response frame: {error:?}")))?;
    decoder
        .finish()
        .map_err(|error| Defect::new(format!("complete local response: {error:?}")))?;
    let [frame] = frames.as_slice() else {
        return Err(Defect::new("one reply per customer request"));
    };
    let envelope = RpcEnvelope::decode(frame, policy)
        .map_err(|error| Defect::new(format!("typed response envelope: {error:?}")))?;
    if !matches!(envelope.kind, RpcEnvelopeKind::Success) {
        return Err(Defect::new(format!("order reply: {:?}", envelope.kind)));
    }
    operation
        .success_codec()
        .decode_ref(&envelope.payload)
        .map_err(|error| Defect::new(format!("accepted ticket payload: {error:?}")))
}
/// Encode a typed request for a finite order-status transport.
pub fn stream_request<
    Q: Send + 'static,
    C: Send + 'static,
    T: Send + 'static,
    E: Send + 'static,
>(
    operation: &RpcStreamOperation<Q, C, T, E>,
    call: &str,
    value: &Q,
) -> Result<Vec<u8>, Defect> {
    RpcEnvelope {
        operation: operation.spec().id.clone(),
        version: operation.spec().version,
        call: RpcCallId::new(call)
            .map_err(|error| Defect::new(format!("customer call: {error:?}")))?,
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: operation
            .request_codec()
            .encode_ref(value)
            .map_err(|error| Defect::new(format!("typed watch request: {error:?}")))?,
        metadata: Default::default(),
    }
    .encode(super::rpc_policy())
    .map_err(|error| Defect::new(format!("local envelope: {error:?}")))?
    .into_bytes()
    .map_err(|error| Defect::new(format!("local frame: {error:?}")))
}

/// Finite native socket used by the phone story; frames and lifecycle belong to this transport.
pub struct PhoneSocket {
    /// Provider-owned inbound text frames, consumed once in order.
    input: Mutex<VecDeque<WebSocketFrame>>,
    /// Messages acknowledged by this native socket.
    sent: Mutex<Vec<WebSocketMessage>>,
    /// Completed close handshakes, observable after the checked owner settles.
    closed: AtomicUsize,
    /// Completed native ownership joins, distinct from the close handshake.
    disposed: AtomicUsize,
    /// Protocol selected at admission, without ambient request state.
    protocol: Option<Arc<str>>,
}
impl PhoneSocket {
    /// Admit an already accepted finite frame sequence with its negotiated protocol.
    #[must_use]
    pub fn new(messages: Vec<Arc<str>>, protocol: Option<Arc<str>>) -> Arc<Self> {
        Arc::new(Self {
            input: Mutex::new(
                messages
                    .into_iter()
                    .map(|message| WebSocketFrame::Data {
                        kind: WebSocketFragmentKind::Text,
                        bytes: message.as_bytes().to_vec(),
                        end: true,
                    })
                    .collect(),
            ),
            sent: Mutex::new(Vec::new()),
            closed: AtomicUsize::new(0),
            disposed: AtomicUsize::new(0),
            protocol,
        })
    }
    /// Confirm the scope completed exactly one handshake and one native ownership join.
    #[must_use]
    pub fn is_released(&self) -> bool {
        self.closed.load(Ordering::Acquire) == 1 && self.disposed.load(Ordering::Acquire) == 1
    }
}
impl WebSocket for PhoneSocket {
    /// Acknowledge one owned outgoing message before completing its native operation.
    fn send(&self, message: WebSocketMessage, _cancellation: Cancellation) -> TransportFuture<()> {
        let result = self
            .sent
            .lock()
            .map(|mut sent| sent.push(message))
            .map_err(|error| Defect::new(error.to_string()));
        Box::pin(async move { result })
    }
    /// Move the next complete frame; the adapter enforces its selected buffer/message limits.
    fn receive(
        &self,
        _buffer: NonZeroUsize,
        _cancellation: Cancellation,
    ) -> TransportFuture<WebSocketFrame> {
        let frame = self
            .input
            .lock()
            .map(|mut input| {
                input
                    .pop_front()
                    .unwrap_or(WebSocketFrame::Close(WebSocketCloseInfo {
                        status: Some(1000),
                        description: None,
                    }))
            })
            .map_err(|error| Defect::new(error.to_string()));
        Box::pin(async move { frame })
    }
    /// Complete the finite peer's close handshake without dropping an admitted future.
    fn close(&self, _cancellation: Cancellation) -> TransportFuture<()> {
        self.closed.fetch_add(1, Ordering::AcqRel);
        Box::pin(async { Ok(()) })
    }
    /// Finite native operations already settle synchronously; discard unread frames on abort.
    fn abort(&self) -> Result<(), Defect> {
        self.input
            .lock()
            .map_err(|error| Defect::new(error.to_string()))?
            .clear();
        Ok(())
    }
    /// Acknowledge native disposal after the adapter has settled admitted I/O and close.
    fn dispose(&self) -> TransportFuture<()> {
        self.disposed.fetch_add(1, Ordering::AcqRel);
        Box::pin(async { Ok(()) })
    }
    /// Borrow the explicitly negotiated protocol from this native admission.
    fn subprotocol(&self) -> Option<Arc<str>> {
        self.protocol.clone()
    }
}
/// Consumer-selected in-process phone transport; no network or runtime is installed.
pub struct PhoneTransport {
    /// Native fixture frames copied into each newly admitted connection.
    messages: Vec<Arc<str>>,
    /// Admitted sockets retained only to observe their completed native disposal.
    sockets: Mutex<Vec<Arc<PhoneSocket>>>,
}
impl PhoneTransport {
    /// Configure finite native input; application status selection stays in the story.
    #[must_use]
    pub fn new(messages: Vec<Arc<str>>) -> Self {
        Self {
            messages,
            sockets: Mutex::new(Vec::new()),
        }
    }
    /// Verify every admitted socket has completed its checked owner and native join.
    pub fn all_released(&self) -> Result<bool, Defect> {
        let sockets = self
            .sockets
            .lock()
            .map_err(|error| Defect::new(error.to_string()))?;
        Ok(!sockets.is_empty() && sockets.iter().all(|socket| socket.is_released()))
    }
}
impl WebSocketTransport for PhoneTransport {
    /// Admit native ownership with the offered protocol; Effect retains its cleanup owner.
    fn connect(
        &self,
        options: WebSocketConnectOptions,
        _cancellation: Cancellation,
    ) -> TransportFuture<Arc<dyn WebSocket>> {
        let socket = PhoneSocket::new(self.messages.clone(), options.subprotocols.first().cloned());
        let admitted = self
            .sockets
            .lock()
            .map(|mut sockets| {
                sockets.push(socket.clone());
                socket as Arc<dyn WebSocket>
            })
            .map_err(|error| Defect::new(error.to_string()));
        Box::pin(async move { admitted })
    }
}
