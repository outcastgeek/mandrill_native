//! Finite transport and producer adapter for the café order-tracking story.
use effect::encoding::FrameParts;
use effect::rpc::{
    RpcCallId, RpcEnvelope, RpcEnvelopeKind, RpcOperation, RpcProduced, RpcStreamOperation,
    RpcStreamProducer, RpcStreamSession,
};
use effect::{Defect, Effect, effect};
use std::collections::VecDeque;
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
    /// Accept one encoded request; EOF follows it.
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
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) == 1
    }
}
impl RpcStreamSession for Session {
    /// Transport failures are distinct from the café's typed business errors.
    type Error = ();
    /// Defer consuming the next accepted frame until the Effect executes.
    fn receive_frame(&self) -> Effect<Option<Vec<u8>>, ()> {
        let input = self.input.clone();
        Effect::sync_once(move || input.lock().expect("input").pop_front())
    }
    /// Transfer ownership of a produced frame into the transport output.
    fn send(&self, frame: FrameParts) -> Effect<(), ()> {
        let output = self.output.clone();
        Effect::sync_once(move || {
            output.lock().expect("output").push(frame);
        })
    }
    /// Mark the accepted session closed during protected cleanup.
    fn close(&self) -> Effect<(), ()> {
        let closed = self.closed.clone();
        Effect::sync_once(move || {
            closed.fetch_add(1, Ordering::SeqCst);
        })
    }
}
/// Finite owned producer plumbing; the macro handler chooses its business values.
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
            match Effect::sync_once(move || values.lock().expect("producer values").pop_front())? {
                Some(value) => value,
                None => Effect::die(Defect::new("coffee order has no further status updates"))?,
            }
        }
    })
}

/// Encode a typed customer request at the local transport boundary.
pub fn request<Q: Send + 'static, A: Send + 'static, E: Send + 'static>(
    operation: &RpcOperation<Q, A, E>,
    call: &str,
    value: &Q,
) -> Vec<u8> {
    RpcEnvelope {
        operation: operation.spec().id.clone(),
        version: operation.spec().version,
        call: RpcCallId::new(call).expect("customer call"),
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: operation
            .request_codec()
            .encode_ref(value)
            .expect("typed customer request"),
        metadata: Default::default(),
    }
    .encode(super::rpc_policy())
    .expect("local envelope")
    .into_bytes()
    .expect("local frame")
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
) -> Vec<u8> {
    RpcEnvelope {
        operation: operation.spec().id.clone(),
        version: operation.spec().version,
        call: RpcCallId::new(call).expect("customer call"),
        kind: RpcEnvelopeKind::Request,
        sequence: None,
        payload: operation
            .request_codec()
            .encode_ref(value)
            .expect("typed watch request"),
        metadata: Default::default(),
    }
    .encode(super::rpc_policy())
    .expect("local envelope")
    .into_bytes()
    .expect("local frame")
}
