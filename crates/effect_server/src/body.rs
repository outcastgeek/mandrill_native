//! Pull-driven native response bodies with explicit delivery acknowledgment.

use crate::{Request, ServerError};
use crate::{effect_from_result, effect_future, effect_sync, effect_try_future};
use axum::body::Body;
use bytes::Bytes;
use effect::effect;
use effect::{Cancellation, Effect, Exit, Scope, Stream};
use http::{HeaderMap, HeaderValue};
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::future::Future;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::{mpsc, oneshot, watch};

/// Explicit queue/chunk policy; no library-chosen resource capacity is implied.
#[derive(Clone, Copy)]
pub struct BodyPolicy {
    /// Number of pending native frames allowed for this body.
    pub frames: NonZeroUsize,
    /// Maximum data bytes transferred by one write operation.
    pub chunk_bytes: NonZeroUsize,
}
/// Caller-selected logical limits at the native response boundary.
#[derive(Clone, Copy, Debug)]
pub struct PreparedResponseLimits {
    /// Maximum number of response header occurrences.
    pub head_fields: usize,
    /// Maximum sum of response header names and values in bytes.
    pub head_bytes: usize,
    /// Maximum visible bytes in each individual DATA frame.
    pub data_frame_bytes: usize,
    /// Maximum number of trailer occurrences per frame.
    pub trailer_fields: usize,
    /// Maximum sum of trailer names and values in bytes per frame.
    pub trailer_bytes: usize,
}
/// The caller-selected output boundary which rejected a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputLimit {
    /// The response has too many header occurrences.
    HeadFields,
    /// Header names and values exceed their selected byte total.
    HeadBytes,
    /// One DATA frame exceeds its selected visible byte limit.
    DataFrameBytes,
    /// One trailer frame has too many field occurrences.
    TrailerFields,
    /// Trailer names and values exceed their selected byte total.
    TrailerBytes,
}
impl OutputLimit {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::HeadFields => 4,
            Self::HeadBytes => 5,
            Self::DataFrameBytes => 6,
            Self::TrailerFields => 7,
            Self::TrailerBytes => 8,
        }
    }
    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            4 => Some(Self::HeadFields),
            5 => Some(Self::HeadBytes),
            6 => Some(Self::DataFrameBytes),
            7 => Some(Self::TrailerFields),
            8 => Some(Self::TrailerBytes),
            _ => None,
        }
    }
}
pub(crate) fn check_output_headers(
    headers: &HeaderMap,
    fields: usize,
    bytes: usize,
    field_error: OutputLimit,
    byte_error: OutputLimit,
) -> Result<(), OutputLimit> {
    let mut count = 0usize;
    let mut total = 0usize;
    for (name, value) in headers.iter() {
        count = count.checked_add(1).ok_or(field_error)?;
        if count > fields {
            return Err(field_error);
        }
        total = total.checked_add(name.as_str().len())
            .and_then(|n| n.checked_add(value.as_bytes().len()))
            .ok_or(byte_error)?;
        if total > bytes {
            return Err(byte_error);
        }
    }
    Ok(())
}
/// Outcome at the native delivery boundary, before asynchronous scope cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyOutcome {
    /// Body remains owned by its native consumer.
    Open,
    /// Native consumer polled the body's terminal boundary.
    Completed,
    /// Consumer dropped the body before terminal delivery.
    Disconnected,
    /// Body encoding/native production failed after commitment.
    Failed,
}
/// Shared signal observes delivery but carries no application error payload.
#[derive(Clone)]
pub struct BodyCompletion {
    /// Last boundary state plus loss-free wait notification.
    state: watch::Sender<BodyOutcome>,
    /// Request cancellation is signaled by disconnect, never async cleanup in Drop.
    cancellation: Cancellation,
    /// Only the actual committed transport body may signal request disconnection.
    armed: Arc<OrdinaryBodyState>,
}
struct OrdinaryBodyState { armed: std::sync::atomic::AtomicBool, limit: std::sync::atomic::AtomicU8 }
impl BodyCompletion {
    /// Create one completion signal for a native response.
    pub(crate) fn new(cancellation: Cancellation) -> Self {
        let (state, _) = watch::channel(BodyOutcome::Open);
        Self {
            state,
            cancellation,
            armed: Arc::new(OrdinaryBodyState { armed: std::sync::atomic::AtomicBool::new(false), limit: std::sync::atomic::AtomicU8::new(0) }),
        }
    }
    /// Arm cancellation when response ownership reaches the actual transport boundary.
    pub(crate) fn arm(&self) {
        self.armed.armed.store(true, std::sync::atomic::Ordering::Release);
    }
    /// Borrow the acknowledged delivery state without waiting.
    #[must_use]
    pub fn outcome(&self) -> BodyOutcome {
        *self.state.borrow()
    }
    /// Await completion/disconnect; the request owner still joins its finalizers afterwards.
    pub async fn wait(&self) -> BodyOutcome {
        let mut state = self.state.subscribe();
        loop {
            let outcome = *state.borrow_and_update();
            if outcome != BodyOutcome::Open {
                return outcome;
            }
            if state.changed().await.is_err() {
                return BodyOutcome::Disconnected;
            }
        }
    }
    /// Record a committed producer failure; E remains in the request completion policy.
    pub(crate) fn fail(&self) {
        self.settle(BodyOutcome::Failed);
    }
    fn fail_limit(&self, limit: OutputLimit) {
        self.state.send_if_modified(|current| {
            if *current == BodyOutcome::Open {
                self.armed.limit.store(limit.code(), std::sync::atomic::Ordering::Release);
                *current = BodyOutcome::Failed;
                true
            } else { false }
        });
    }
    fn output_limit(&self) -> Option<OutputLimit> { OutputLimit::from_code(self.armed.limit.load(std::sync::atomic::Ordering::Acquire)) }
    /// Publish one terminal transition; concurrent Drop cannot replace completion.
    fn settle(&self, outcome: BodyOutcome) {
        let changed = self.state.send_if_modified(|current| {
            if *current == BodyOutcome::Open {
                *current = outcome;
                true
            } else {
                false
            }
        });
        if changed
            && outcome == BodyOutcome::Disconnected
            && self.armed.armed.load(std::sync::atomic::Ordering::Acquire)
        {
            let _ = self.cancellation.request();
        }
    }
}
/// Internal signal; the public ordinary BodyCompletion contract remains unchanged.
#[derive(Clone)]
pub(crate) enum BodySignal {
    Ordinary(BodyCompletion),
    Prepared(crate::prepared_body_completion::BodyRef),
}
pub(crate) enum BodySignalWait {
    Ordinary(BodyCompletion),
    Prepared(crate::prepared_body_completion::BodyWait),
}
pub(crate) struct BodyPoolAccess<E: Send + 'static>(pub(crate) Arc<crate::prepared_body_completion::BodyPool>, pub(crate) Arc<effect::PreparedCleanupConfig<E>>);
impl<E: Send + 'static> Clone for BodyPoolAccess<E> { fn clone(&self) -> Self { Self(self.0.clone(), self.1.clone()) } }
impl BodySignalWait {
    pub(crate) fn signal(&self) -> BodySignal {
        match self {
            Self::Ordinary(completion) => BodySignal::Ordinary(completion.clone()),
            Self::Prepared(wait) => BodySignal::Prepared(wait.signal()),
        }
    }
    pub(crate) async fn wait(self) -> BodyOutcome {
        match self {
            Self::Ordinary(completion) => completion.wait().await,
            Self::Prepared(wait) => wait.await,
        }
    }
}
impl BodySignal {
    pub(crate) fn ordinary(cancellation: Cancellation) -> Self {
        Self::Ordinary(BodyCompletion::new(cancellation))
    }
    pub(crate) fn admit_wait(&self) -> Result<BodySignalWait, effect::memory::MemoryError> {
        match self {
            Self::Ordinary(completion) => Ok(BodySignalWait::Ordinary(completion.clone())),
            Self::Prepared(completion) => completion.admit_wait().map(BodySignalWait::Prepared),
        }
    }
    pub(crate) fn arm(&self) {
        match self {
            Self::Ordinary(completion) => completion.arm(),
            Self::Prepared(completion) => completion.arm(),
        }
    }
    pub(crate) fn outcome(&self) -> BodyOutcome {
        match self {
            Self::Ordinary(completion) => completion.outcome(),
            Self::Prepared(completion) => completion.outcome(),
        }
    }
    pub(crate) fn fail(&self) {
        match self {
            Self::Ordinary(completion) => completion.fail(),
            Self::Prepared(completion) => completion.settle(BodyOutcome::Failed),
        }
    }
    pub(crate) fn fail_limit(&self, limit: OutputLimit) { match self { Self::Ordinary(c) => c.fail_limit(limit), Self::Prepared(c) => c.fail_limit(limit) } }
    pub(crate) fn output_limit(&self) -> Option<OutputLimit> { match self { Self::Ordinary(c) => c.output_limit(), Self::Prepared(c) => c.output_limit() } }
    fn settle(&self, outcome: BodyOutcome) {
        match self {
            Self::Ordinary(completion) => completion.settle(outcome),
            Self::Prepared(completion) => completion.settle(outcome),
        }
    }
}
pub(crate) struct BodyAdmission<E: Send + 'static> {
    pub(crate) completion: crate::prepared_body_completion::BodyRef,
    pub(crate) wait: crate::prepared_body_completion::BodyWait,
    pub(crate) channel: Option<crate::prepared_streaming_channel::BodyChannelVacancy>,
    pub(crate) resources: Option<crate::prepared_body_completion::PreparedBodyExitResources<E>>,
}
pub(crate) fn admitted_signal<E: Send + 'static>(
    cancellation: Cancellation,
    admission: Option<BodyAdmission<E>>,
) -> (BodySignal, BodySignalWait) {
    match admission {
        Some(BodyAdmission { completion, wait, channel, resources }) => { drop(channel); drop(resources); (BodySignal::Prepared(completion), BodySignalWait::Prepared(wait)) },
        None => {
            let completion = BodyCompletion::new(cancellation);
            (
                BodySignal::Ordinary(completion.clone()),
                BodySignalWait::Ordinary(completion),
            )
        }
    }
}
/// Pending native frame plus producer acknowledgment; no detached producer task exists.
struct Delivery {
    /// Frame moves from producer to transport without copying data.
    frame: Frame<Bytes>,
    /// Producer may advance only after the consumer pulls this frame.
    accepted: oneshot::Sender<()>,
}
/// Shared writer state. Finishing removes its sender even if a writer handle was shared.
struct WriterState {
    /// Native frame sender exists only while the body program is active.
    sender: Mutex<Option<mpsc::Sender<Delivery>>>,
    /// Explicit per-write and queue limits.
    policy: BodyPolicy,
}
/// Effect-owned bounded body writer; handles share only one checked native sink.
pub struct BodyWriter<E: Send + 'static, R: Send + Sync + 'static> {
    /// Single sink owner shared with generated callbacks.
    state: WriterStorage<E>,
    /// Explicit native-input to application error mapping.
    map_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    /// R remains the sole application/provider selector.
    environment: PhantomData<fn() -> R>,
}
enum WriterStorage<E: Send + 'static> {
    Ordinary(Arc<WriterState>),
    Prepared { writer: crate::prepared_streaming_channel::Writer, cleanup: Arc<effect::PreparedCleanupConfig<E>>, policy: BodyPolicy },
}
impl<E: Send + 'static> Clone for WriterStorage<E> {
    fn clone(&self) -> Self { match self { Self::Ordinary(state) => Self::Ordinary(state.clone()), Self::Prepared { writer, cleanup, policy } => Self::Prepared { writer: writer.clone(), cleanup: cleanup.clone(), policy: *policy } } }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for BodyWriter<E, R> {
    /// Share sink ownership without cloning E or body payloads.
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            map_error: self.map_error.clone(),
            environment: PhantomData,
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> BodyWriter<E, R> {
    pub(crate) fn reserve_json_writer(&self, policy: effect::JsonWriterPolicy) -> Result<effect::PreparedJsonWriterSinkReservation<NativeJsonSink>, effect::memory::MemoryError> {
        match &self.state {
            WriterStorage::Prepared { writer, .. } => writer.json_writers().ok_or(effect::memory::MemoryError::UnsupportedRunContract)?.reserve_sink(policy),
            WriterStorage::Ordinary(_) => Err(effect::memory::MemoryError::UnsupportedRunContract),
        }
    }
    /// Move one owned chunk into an awaited write. Reusing this one-shot Effect is a core defect.
    pub fn write(&self, bytes: Bytes) -> Effect<(), E, R> {
        match &self.state {
            WriterStorage::Ordinary(state) => {
                let state = state.clone(); let map = self.map_error.clone();
                effect_try_future! { once; body: move |_| async move {
                    if bytes.len() > state.policy.chunk_bytes.get() { return Err(ServerError::Limit("response chunk bytes".into())); }
                    send_frame(&state, Frame::data(bytes)).await
                }, on_error: move |error| map(error), }
            }
            WriterStorage::Prepared { writer, cleanup, policy } => {
                let writer = writer.clone(); let cleanup = cleanup.clone(); let policy = *policy; let map = self.map_error.clone();
                crate::endpoint::native_once(move |_, _| async move {
                    let mapper = match effect::PreparedExitMapper::admit(&cleanup) { Ok(mapper) => mapper, Err(error) => return Exit::Fail(map(ServerError::Memory(error))) };
                    let permit = match writer.begin() { Ok(permit) => permit, Err(effect::memory::MemoryError::Closed) => return Exit::Fail(map(ServerError::Disconnected)), Err(error) => return Exit::Fail(map(ServerError::Memory(error))) };
                    if bytes.len() > policy.chunk_bytes.get() { return Exit::Fail(map(ServerError::Limit("response chunk bytes".into()))); }
                    match permit.send(Frame::data(bytes)).await {
                        crate::prepared_streaming_channel::WriteStatus::Accepted => Exit::Success(()),
                        crate::prepared_streaming_channel::WriteStatus::Disconnected => Exit::Fail(map(ServerError::Disconnected)),
                        crate::prepared_streaming_channel::WriteStatus::QueueFailed(exit) => mapper.map(exit, |_| map(ServerError::Disconnected)),
                    }
                })
            }
        }
    }
    /// Transfer a region byte owner into an awaited write without copying.
    /// The native Bytes bridge allocates one owner control.
    pub fn write_lease(&self, view: effect::memory::BufferView<'static>) -> Effect<(), E, R> {
        self.write(Bytes::from_owner(view))
    }
    /// Transfer a prepared native frame into an awaited write.
    pub fn write_frame(&self, frame: crate::PreparedNativeFrame) -> Effect<(), E, R> {
        self.write(frame.into_bytes())
    }
    /// Flush at a native polling boundary. HTTP framing/coalescing remains transport-owned.
    pub fn flush(&self) -> Effect<(), E, R> {
        self.write(Bytes::new())
    }
    /// Move terminal HTTP trailers into the native body, preserving repeated fields.
    pub fn trailers(&self, trailers: HeaderMap) -> Effect<(), E, R> {
        if trailers.is_empty() {
            return effect_sync! { body: || (), };
        }
        match &self.state {
            WriterStorage::Ordinary(state) => { let state = state.clone(); let map = self.map_error.clone(); effect_try_future! { once; body: move |_| async move { send_frame(&state, Frame::trailers(trailers)).await }, on_error: move |error| map(error), } }
            WriterStorage::Prepared { writer, cleanup, .. } => { let writer = writer.clone(); let cleanup = cleanup.clone(); let map = self.map_error.clone(); crate::endpoint::native_once(move |_, _| async move {
                let mapper = match effect::PreparedExitMapper::admit(&cleanup) { Ok(mapper) => mapper, Err(error) => return Exit::Fail(map(ServerError::Memory(error))) };
                let permit = match writer.begin() { Ok(permit) => permit, Err(effect::memory::MemoryError::Closed) => return Exit::Fail(map(ServerError::Disconnected)), Err(error) => return Exit::Fail(map(ServerError::Memory(error))) };
                match permit.send(Frame::trailers(trailers)).await {
                    crate::prepared_streaming_channel::WriteStatus::Accepted => Exit::Success(()),
                    crate::prepared_streaming_channel::WriteStatus::Disconnected => Exit::Fail(map(ServerError::Disconnected)),
                    crate::prepared_streaming_channel::WriteStatus::QueueFailed(exit) => mapper.map(exit, |_| map(ServerError::Disconnected)),
                }
            }) }
        }
    }
    /// Emit a checked SSE record with LF framing and no injected field lines.
    pub fn sse(&self, record: SseRecord, policy: SsePolicy) -> Effect<(), E, R> {
        if let WriterStorage::Prepared { writer, cleanup, policy: body_policy } = &self.state
            && let Some(pool) = writer.bytes_pool()
        {
            let pool = Arc::clone(pool);
            let writer = writer.clone();
            let cleanup = cleanup.clone();
            let body_policy = *body_policy;
            let map = self.map_error.clone();
            return crate::endpoint::native_once(move |_, _| async move {
                let len = match record.prepared_len(policy) { Ok(len) => len, Err(error) => return Exit::Fail(map(error)) };
                let mapper = match effect::PreparedExitMapper::admit(&cleanup) { Ok(mapper) => mapper, Err(error) => return Exit::Fail(map(ServerError::Memory(error))) };
                let permit = match writer.begin() { Ok(permit) => permit, Err(effect::memory::MemoryError::Closed) => return Exit::Fail(map(ServerError::Disconnected)), Err(error) => return Exit::Fail(map(ServerError::Memory(error))) };
                if len > body_policy.chunk_bytes.get() { return Exit::Fail(map(ServerError::Limit("response chunk bytes".into()))); }
                let bytes = match pool.try_fill(len, |output| record.prepared_encode(output)) { Ok(bytes) => bytes, Err(error) => return Exit::Fail(map(ServerError::Memory(error))) };
                match permit.send(Frame::data(bytes)).await {
                    crate::prepared_streaming_channel::WriteStatus::Accepted => Exit::Success(()),
                    crate::prepared_streaming_channel::WriteStatus::Disconnected => Exit::Fail(map(ServerError::Disconnected)),
                    crate::prepared_streaming_channel::WriteStatus::QueueFailed(exit) => mapper.map(exit, |_| map(ServerError::Disconnected)),
                }
            });
        }
        let writer = self.clone();
        let map = self.map_error.clone();
        effect! {
            bind;
            program: effect_from_result! { once; result: record.frame(policy).map_err(|error| map(error)), },
            body: move |bytes| crate::stream! { write; writer: writer, bytes: bytes, },
        }
    }
    /// Close producer access; the effect still awaits the native body's terminal poll.
    pub(crate) fn finish(&self) {
        match &self.state {
            WriterStorage::Ordinary(state) => { let sender = state.sender.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take(); drop(sender); }
            WriterStorage::Prepared { writer, .. } => writer.finish(),
        }
    }
}
/// Send one frame and await native acceptance before the next producer step.
async fn send_frame(state: &WriterState, frame: Frame<Bytes>) -> Result<(), ServerError> {
    let sender = state
        .sender
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or(ServerError::Disconnected)?;
    let (accepted, wait) = oneshot::channel();
    sender
        .send(Delivery { frame, accepted })
        .await
        .map_err(|_| ServerError::Disconnected)?;
    wait.await.map_err(|_| ServerError::Disconnected)
}
/// Native body retained by axum; Drop only signals cancellation to its managed owner.
struct DeliveryBody {
    /// Receiver drives producer progress one frame at a time.
    receiver: mpsc::Receiver<Delivery>,
    /// Native terminal error/EOF is published at most once.
    ended: bool,
    /// Terminal delivery signal observed by the enclosing request Effect.
    completion: BodySignal,
}
impl HttpBody for DeliveryBody {
    /// Native encoded bytes, moved rather than copied.
    type Data = Bytes;
    /// Body failure terminates a committed response; E stays in request policy.
    type Error = std::io::Error;
    /// Pull one producer frame, acknowledging its backpressure boundary.
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.ended {
            return Poll::Ready(None);
        }
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(delivery)) => {
                let _ = delivery.accepted.send(());
                Poll::Ready(Some(Ok(delivery.frame)))
            }
            Poll::Ready(None) => {
                self.ended = true;
                if self.completion.outcome() == BodyOutcome::Failed {
                    Poll::Ready(Some(Err(std::io::Error::other(
                        "committed response body failed",
                    ))))
                } else {
                    self.completion.settle(BodyOutcome::Completed);
                    Poll::Ready(None)
                }
            }
            Poll::Pending => Poll::Pending,
        }
    }
    /// Completion is explicit; queue emptiness alone is not terminal delivery.
    fn is_end_stream(&self) -> bool {
        self.ended
    }
    /// A streaming program has no truthful finite size hint.
    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}
impl Drop for DeliveryBody {
    /// Signal disconnection; host/request ownership performs asynchronous cleanup.
    fn drop(&mut self) {
        self.completion.settle(BodyOutcome::Disconnected);
    }
}
/// Allocation-free marker for an already recorded native channel failure.
#[derive(Debug)]
struct PreparedBodyFailure;
impl std::fmt::Display for PreparedBodyFailure { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("prepared body failed") } }
impl std::error::Error for PreparedBodyFailure {}
/// The prepared receiver retains queue and acknowledgement owners through pull.
struct PreparedDeliveryBody {
    receiver: crate::prepared_streaming_channel::NativeReceiver,
    completion: BodySignal,
    ended: bool,
}
impl HttpBody for PreparedDeliveryBody {
    type Data = Bytes;
    type Error = PreparedBodyFailure;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.ended { return Poll::Ready(None); }
        match self.receiver.poll_pull(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(crate::prepared_streaming_channel::Pull::Item(frame)) => Poll::Ready(Some(Ok(frame))),
            Poll::Ready(crate::prepared_streaming_channel::Pull::End) => { self.ended = true; if self.completion.outcome() == BodyOutcome::Failed { Poll::Ready(Some(Err(PreparedBodyFailure))) } else { self.completion.settle(BodyOutcome::Completed); Poll::Ready(None) } },
            Poll::Ready(crate::prepared_streaming_channel::Pull::Failed) => { self.ended = true; self.completion.fail(); Poll::Ready(Some(Err(PreparedBodyFailure))) },
        }
    }
    fn is_end_stream(&self) -> bool { self.ended }
    fn size_hint(&self) -> SizeHint { SizeHint::default() }
}
impl Drop for PreparedDeliveryBody { fn drop(&mut self) { self.completion.settle(BodyOutcome::Disconnected); } }
pub(crate) fn prepared_channel<E: Send + 'static, R: Send + Sync + 'static>(
    policy: BodyPolicy, map_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>, completion: BodySignal,
    vacancy: crate::prepared_streaming_channel::BodyChannelVacancy, cleanup: Arc<effect::PreparedCleanupConfig<E>>,
) -> (Body, BodyWriter<E, R>, crate::prepared_streaming_channel::Writer, crate::prepared_streaming_channel::SettlementWait) {
    let (receiver, writer, settlement) = vacancy.split();
    let body = Body::new(PreparedDeliveryBody { receiver, completion, ended: false });
    let handle = BodyWriter { state: WriterStorage::Prepared { writer: writer.clone(), cleanup, policy }, map_error, environment: PhantomData };
    (body, handle, writer, settlement)
}
#[derive(Debug)]
pub(crate) enum ObservedBodyError { Native(axum::Error), Limit(OutputLimit) }
impl std::fmt::Display for ObservedBodyError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { match self { Self::Native(error) => write!(f, "{error}"), Self::Limit(limit) => write!(f, "output limit exceeded: {limit:?}") } } }
impl std::error::Error for ObservedBodyError { fn source(&self) -> Option<&(dyn std::error::Error + 'static)> { match self { Self::Native(error) => Some(error), Self::Limit(_) => None } } }
/// Wrap any native body so response construction cannot prematurely finalize its resources.
pub(crate) struct ObservedBody {
    /// Native body remains the authoritative data/trailer producer.
    pub(crate) body: Body,
    /// Shared lifecycle gate retained by the request owner.
    pub(crate) completion: BodySignal,
    pub(crate) limits: Option<PreparedResponseLimits>,
    pub(crate) failed: bool,
}
impl HttpBody for ObservedBody {
    /// Native body output remains bytes.
    type Data = Bytes;
    /// Preserve native body failures at the committed transport boundary.
    type Error = ObservedBodyError;
    /// Forward frames and publish only actual terminal consumption.
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.failed { return Poll::Ready(None); }
        match Pin::new(&mut self.body).poll_frame(cx) {
            Poll::Ready(None) => {
                self.completion.settle(BodyOutcome::Completed);
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(error))) => {
                self.completion.settle(BodyOutcome::Failed);
                Poll::Ready(Some(Err(ObservedBodyError::Native(error))))
            }
            Poll::Ready(Some(Ok(frame))) => {
                let error = self.limits.and_then(|limits| {
                    if frame.data_ref().is_some_and(|data| data.len() > limits.data_frame_bytes) { return Some(OutputLimit::DataFrameBytes); }
                    frame.trailers_ref().and_then(|headers| check_output_headers(headers, limits.trailer_fields, limits.trailer_bytes, OutputLimit::TrailerFields, OutputLimit::TrailerBytes).err())
                });
                if let Some(error) = error {
                    self.failed = true;
                    self.completion.fail_limit(error);
                    drop(frame);
                    let body = std::mem::replace(&mut self.body, Body::empty());
                    drop(body);
                    Poll::Ready(Some(Err(ObservedBodyError::Limit(error))))
                } else { Poll::Ready(Some(Ok(frame))) }
            }
            Poll::Pending => Poll::Pending,
        }
    }
    /// Native empty bodies complete immediately when mounted, with no pending data to deliver.
    fn is_end_stream(&self) -> bool {
        self.failed || self.body.is_end_stream()
    }
    /// Preserve native finite-body sizing.
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}
impl Drop for ObservedBody {
    /// Signal only an unfinished native body; cleanup remains managed asynchronous work.
    fn drop(&mut self) {
        self.completion.settle(if self.body.is_end_stream() {
            BodyOutcome::Completed
        } else {
            BodyOutcome::Disconnected
        });
    }
}
/// Build a native body and a bounded writer sharing one lifecycle gate.
pub(crate) fn channel<E: Send + 'static, R: Send + Sync + 'static>(
    policy: BodyPolicy,
    map_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    completion: BodySignal,
) -> (Body, BodyWriter<E, R>, BodySignal) {
    let (sender, receiver) = mpsc::channel(policy.frames.get());
    let writer = BodyWriter {
        state: WriterStorage::Ordinary(Arc::new(WriterState {
            sender: Mutex::new(Some(sender)),
            policy,
        })),
        map_error,
        environment: PhantomData,
    };
    (
        Body::new(DeliveryBody {
            receiver,
            ended: false,
            completion: completion.clone(),
        }),
        writer,
        completion,
    )
}
/// Deferred server-body program; native sink execution belongs to the enclosing request scope.
pub struct BodyProgram<E: Send + 'static, R: Send + Sync + 'static> {
    /// Scope-aware callback, sharing descriptions only.
    body: Arc<BodyFactory<E, R>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for BodyProgram<E, R> {
    /// Share a reusable body description, never a running producer.
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> BodyProgram<E, R> {
    /// Describe an extracted writer callback without executing it.
    pub fn new(
        body: impl Fn(BodyWriter<E, R>) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            body: Arc::new(move |writer, _| body(writer)),
        }
    }
    /// Generated inline body uses the same Effect/resource binder under the request owner.
    #[doc(hidden)]
    pub fn from_body<F>(
        factory: impl Fn(BodyWriter<E, R>, effect::__private::MacroContext<E, R>) -> F
        + Send
        + Sync
        + 'static,
    ) -> Self
    where
        F: Future<Output = Result<(), Exit<(), E>>> + Send + 'static,
    {
        let factory = Arc::new(factory);
        Self {
            body: Arc::new(move |writer, scope| {
                let factory = factory.clone();
                effect::__private::ScopedBody::from_body(move |context| {
                    factory(writer.clone(), context)
                })
                .evaluate(scope)
            }),
        }
    }
    /// Adapt a core pull-driven stream without collecting it or changing its resource owner.
    /// Pull failures retain their typed error, defect, interruption or ordered finalization.
    pub fn source<A: Send + 'static>(
        source: Stream<A, E, R>,
        write: impl Fn(A, BodyWriter<E, R>) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        let write = Arc::new(write);
        Self {
            body: Arc::new(move |writer, scope| {
                let write = write.clone();
                effect! {
                bind;
                program: source.open(scope),
                body: move |reader| {
                                   let write = write.clone();
                                   let writer = writer.clone();
                                   effect_future! {
                                       exit_with_context; body: move |env: Arc<R>, context| {
                                       let reader = reader.clone();
                                       let write = write.clone();
                                       let writer = writer.clone();
                                       async move {
                                           loop {
                                               match effect::effect! {
                                                   run_in_context;
                                                   context: context,
                                                   program: effect::stream! { pull; reader: reader, },
                                                   environment: env.clone(),
                                               }.await {
                                                   Exit::Success(effect::Chunk::Values(values)) => {
                                                       for value in values {
                                                           let result = effect::effect! {
                                                               run_in_context;
                                                               context: context,
                                                               program: write(value, writer.clone()),
                                                               environment: env.clone(),
                                                           }
                                                               .await;
                                                           if !matches!(result, Exit::Success(())) {
                                                               return result;
                                                           }
                                                       }
                                                   }
                                                   Exit::Success(effect::Chunk::End) => return Exit::Success(()),
                                                   Exit::Fail(error) => return Exit::Fail(error),
                                                   Exit::Die(defect) => return Exit::Die(defect),
                                                   Exit::Interrupted => return Exit::Interrupted,
                                                   Exit::FinalizationFailed(failure) => {
                                                       return Exit::FinalizationFailed(failure);
                                                   }
                                               }
                                           }
                                       }
                                   }, }
                               },
                }
            }),
        }
    }
    /// Pull and write owned region byte views, retaining the backing in native DATA.
    #[must_use]
    pub fn leases(source: Stream<effect::memory::BufferView<'static>, E, R>) -> Self {
        Self::source(source, |view, writer| crate::stream! { write_lease; writer: writer, view: view, })
    }
    /// Pull and write prepared native DATA without copying the frames again.
    #[must_use]
    pub fn frames(source: Stream<crate::PreparedNativeFrame, E, R>) -> Self {
        Self::source(source, |frame, writer| crate::stream! { write_frame; writer: writer, frame: frame, })
    }
    /// Execute with the supplied owner; no run boundary or native task is introduced.
    pub(crate) fn execute(&self, writer: BodyWriter<E, R>, scope: Scope<E>) -> Effect<(), E, R> {
        (self.body)(writer, scope)
    }
}
/// Caller-selected maximum SSE frame and cursor bytes.
#[derive(Clone, Copy)]
pub struct SsePolicy {
    /// Complete encoded event bytes, including delimiters.
    pub frame_bytes: NonZeroUsize,
    /// Maximum `id`/Last-Event-ID UTF-8 bytes.
    pub cursor_bytes: NonZeroUsize,
}
/// Owned SSE event fields; data may contain newlines and is framed per line.
pub struct SseRecord {
    /// Optional event type; CR/LF is rejected.
    pub event: Option<String>,
    /// Optional replay cursor; NUL/CR/LF is rejected.
    pub id: Option<String>,
    /// Optional client retry duration in milliseconds per the SSE protocol.
    pub retry: Option<std::time::Duration>,
    /// Event data moved into one bounded encoded frame.
    pub data: String,
}
impl SseRecord {
    // Visit exactly the same fields and normalized data lines as `frame`.
    fn prepared_fragments(&self, mut visit: impl FnMut(&[u8])) {
        if let Some(event) = &self.event { visit(b"event: "); visit(event.as_bytes()); visit(b"\n"); }
        if let Some(id) = &self.id { visit(b"id: "); visit(id.as_bytes()); visit(b"\n"); }
        if let Some(retry) = self.retry {
            let mut digits = std::fmt::NumBuffer::new();
            visit(b"retry: ");
            visit(retry.as_millis().format_into(&mut digits).as_bytes());
            visit(b"\n");
        }
        let data = self.data.as_bytes();
        let mut start = 0usize;
        let mut i = 0usize;
        while let Some(&byte) = data.get(i) {
            if byte == b'\r' || byte == b'\n' {
                visit(b"data: "); visit(data.get(start..i).unwrap_or_default()); visit(b"\n");
                i += 1;
                if byte == b'\r' && data.get(i) == Some(&b'\n') { i += 1; }
                start = i;
            } else { i += 1; }
        }
        visit(b"data: "); visit(data.get(start..).unwrap_or_default()); visit(b"\n\n");
    }
    fn prepared_len(&self, policy: SsePolicy) -> Result<usize, ServerError> {
        if self.event.as_ref().is_some_and(|event| event.contains(['\r', '\n'])) { return Err(ServerError::Invalid("SSE event".into())); }
        if let Some(id) = &self.id { validate_cursor(id, policy)?; }
        let mut len = Some(0usize);
        self.prepared_fragments(|fragment| { len = len.and_then(|value| value.checked_add(fragment.len())); });
        let len = len.ok_or_else(|| ServerError::Limit("SSE frame".into()))?;
        if len > policy.frame_bytes.get() { return Err(ServerError::Limit("SSE frame".into())); }
        Ok(len)
    }
    fn prepared_encode(&self, output: &mut [u8]) -> Result<(), effect::memory::MemoryError> {
        let mut offset = 0usize;
        let mut valid = true;
        self.prepared_fragments(|fragment| {
            if let Some(end) = offset.checked_add(fragment.len()) && let Some(target) = output.get_mut(offset..end) {
                target.copy_from_slice(fragment); offset = end;
            } else { valid = false; }
        });
        if valid && offset == output.len() { Ok(()) } else { Err(effect::memory::MemoryError::InvalidRange) }
    }
    /// Encode an SSE event without hidden retries, event names or cursor defaults.
    pub fn frame(self, policy: SsePolicy) -> Result<Bytes, ServerError> {
        let mut output = String::new();
        if let Some(event) = self.event {
            if event.contains(['\r', '\n']) {
                return Err(ServerError::Invalid("SSE event".into()));
            }
            output.push_str("event: ");
            output.push_str(&event);
            output.push('\n');
        }
        if let Some(id) = self.id {
            validate_cursor(&id, policy)?;
            output.push_str("id: ");
            output.push_str(&id);
            output.push('\n');
        }
        if let Some(retry) = self.retry {
            output.push_str(&format!("retry: {}\n", retry.as_millis()));
        }
        for line in self
            .data
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .split('\n')
        {
            output.push_str("data: ");
            output.push_str(line);
            output.push('\n');
        }
        output.push('\n');
        if output.len() > policy.frame_bytes.get() {
            return Err(ServerError::Limit("SSE frame".into()));
        }
        Ok(Bytes::from(output))
    }
}
/// Validate replay cursor syntax independently of event data bounds.
fn validate_cursor(cursor: &str, policy: SsePolicy) -> Result<(), ServerError> {
    if cursor.contains(['\0', '\r', '\n']) {
        return Err(ServerError::Invalid("SSE cursor".into()));
    }
    if cursor.len() > policy.cursor_bytes.get() {
        return Err(ServerError::Limit("SSE cursor".into()));
    }
    Ok(())
}
/// Read Last-Event-ID as one checked native header rather than a lossy query string.
pub fn sse_cursor(request: &Request, policy: SsePolicy) -> Result<Option<String>, ServerError> {
    let values = request
        .headers()
        .get_all("last-event-id")
        .iter()
        .collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [value] => {
            let cursor = value
                .to_str()
                .map_err(|_| ServerError::Invalid("SSE cursor".into()))?;
            validate_cursor(cursor, policy)?;
            Ok(Some(cursor.to_owned()))
        }
        _ => Err(ServerError::Duplicate("Last-Event-ID".into())),
    }
}
/// Apply the SSE media type as a named protocol value, independent of caller policies.
pub(crate) fn sse_content_type() -> HeaderValue {
    HeaderValue::from_static("text/event-stream; charset=utf-8")
}

/// Checked native JSON destination. Its synchronous calls must run on R's blocking provider.
pub(crate) struct NativeJsonSink { state: JsonSinkState }
enum JsonSinkState {
    Ordinary(Arc<WriterState>),
    Prepared { sink: crate::prepared_streaming_channel::BlockingSink, chunk_bytes: usize },
}
pub(crate) struct JsonWitness<E: Send + 'static> {
    pub(crate) receiver: effect::PreparedReceiver<crate::prepared_streaming_channel::RawBlockingFailure>,
    pub(crate) mapper: effect::PreparedExitMapper<E>,
    pub(crate) cleanup: effect::CleanupFailures<E>,
}
impl NativeJsonSink {
    pub(crate) fn ordinary_from_writer<E: Send + 'static, R: Send + Sync + 'static>(writer: &BodyWriter<E, R>) -> Option<Self> {
        match &writer.state { WriterStorage::Ordinary(state) => Some(Self { state: JsonSinkState::Ordinary(state.clone()) }), WriterStorage::Prepared { .. } => None }
    }
    /// Preadmit all prepared sink authorities before the codec runs.
    pub(crate) fn from_writer<E: Send + 'static, R: Send + Sync + 'static>(writer: &BodyWriter<E, R>) -> Result<(Self, Option<JsonWitness<E>>), effect::memory::MemoryError> {
        match &writer.state {
            WriterStorage::Ordinary(state) => Ok((Self { state: JsonSinkState::Ordinary(state.clone()) }, None)),
            WriterStorage::Prepared { writer, cleanup, policy } => {
                let mapper = effect::PreparedExitMapper::admit(cleanup)?;
                let carrier = cleanup.reserve_failures()?;
                let (sink, receiver) = writer.blocking_sink()?;
                Ok((Self { state: JsonSinkState::Prepared { sink, chunk_bytes: policy.chunk_bytes.get() } }, Some(JsonWitness { receiver, mapper, cleanup: carrier })))
            }
        }
    }
}
impl effect::JsonWriterSink for NativeJsonSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), effect::SchemaIssue> {
        match &mut self.state {
            JsonSinkState::Ordinary(state) => {
                let sender = state.sender.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().ok_or_else(json_sink_issue)?;
                for chunk in bytes.chunks(state.policy.chunk_bytes.get()) {
                    let (accepted, wait) = oneshot::channel();
                    sender.blocking_send(Delivery { frame: Frame::data(Bytes::copy_from_slice(chunk)), accepted }).map_err(|_| json_sink_issue())?;
                    wait.blocking_recv().map_err(|_| json_sink_issue())?;
                }
            }
            JsonSinkState::Prepared { sink, chunk_bytes } => {
                for chunk in bytes.chunks(*chunk_bytes) { sink.write_chunk(chunk).map_err(|()| json_sink_issue())?; }
            }
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<(), effect::SchemaIssue> { Ok(()) }
}
/// Typed schema failure for a disconnected/closed checked native JSON destination.
fn json_sink_issue() -> effect::SchemaIssue {
    effect::SchemaIssue::new(effect::SchemaIssueKind::InvalidDefinition)
}

/// Append staged trailers to finite/opaque native bodies without collecting their bytes.
struct TrailerBody {
    /// Authoritative upstream native body and its original trailers.
    body: Body,
    /// Additional staged fields emitted after upstream completion.
    trailers: Option<HeaderMap>,
    /// Upstream completion has been observed; only staged trailers may remain.
    ended: bool,
}
impl HttpBody for TrailerBody {
    /// Native frames remain byte buffers.
    type Data = Bytes;
    /// Preserve upstream native failures.
    type Error = axum::Error;
    /// Forward the native body then emit the staged terminal trailer frame exactly once.
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if !self.ended {
            match Pin::new(&mut self.body).poll_frame(cx) {
                Poll::Ready(None) => self.ended = true,
                frame @ Poll::Ready(_) | frame @ Poll::Pending => return frame,
            }
        }
        match self.trailers.take() {
            Some(trailers) if !trailers.is_empty() => {
                Poll::Ready(Some(Ok(Frame::trailers(trailers))))
            }
            Some(_) | None => Poll::Ready(None),
        }
    }
    /// Pending staged trailers prevent an empty-body shortcut from skipping delivery.
    fn is_end_stream(&self) -> bool {
        self.ended && self.trailers.is_none()
    }
    /// Native data size is unaffected by trailers.
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}
/// Preserve finite/opaque trailer ownership through the same completion wrapper.
pub(crate) fn with_trailers(body: Body, trailers: HeaderMap) -> Body {
    if trailers.is_empty() {
        body
    } else {
        Body::new(TrailerBody {
            body,
            trailers: Some(trailers),
            ended: false,
        })
    }
}

/// Reusable producer description under its request scope.
type BodyFactory<E, R> = dyn Fn(BodyWriter<E, R>, Scope<E>) -> Effect<(), E, R> + Send + Sync;
