//! Pull-driven native response bodies with explicit delivery acknowledgment.

use crate::{Request, ServerError};
use crate::{effect_future, effect_sync, effect_try_future, effect_try_sync};
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
    armed: Arc<std::sync::atomic::AtomicBool>,
}
impl BodyCompletion {
    /// Create one completion signal for a native response.
    pub(crate) fn new(cancellation: Cancellation) -> Self {
        let (state, _) = watch::channel(BodyOutcome::Open);
        Self {
            state,
            cancellation,
            armed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    /// Arm cancellation when response ownership reaches the actual transport boundary.
    pub(crate) fn arm(&self) {
        self.armed.store(true, std::sync::atomic::Ordering::Release);
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
            && self.armed.load(std::sync::atomic::Ordering::Acquire)
        {
            let _ = self.cancellation.request();
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
    state: Arc<WriterState>,
    /// Explicit native-input to application error mapping.
    map_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    /// R remains the sole application/provider selector.
    environment: PhantomData<fn() -> R>,
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
    /// Move one owned chunk into an awaited write. Reusing this one-shot Effect is a core defect.
    pub fn write(&self, bytes: Bytes) -> Effect<(), E, R> {
        let state = self.state.clone();
        let map = self.map_error.clone();
        effect_try_future! {
        once; body: move |_| async move {
            if bytes.len() > state.policy.chunk_bytes.get() {
                return Err(ServerError::Limit("response chunk bytes".into()));
            }
            send_frame(&state, Frame::data(bytes)).await
        }, on_error: move |error| map(error), }
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
        let state = self.state.clone();
        let map = self.map_error.clone();
        effect_try_future! { once; body: move |_| async move { send_frame(&state, Frame::trailers(trailers)).await }, on_error: move |error| map(error), }
    }
    /// Emit a checked SSE record with LF framing and no injected field lines.
    pub fn sse(&self, record: SseRecord, policy: SsePolicy) -> Effect<(), E, R> {
        let writer = self.clone();
        let map = self.map_error.clone();
        effect! {
        bind;
        program: effect_try_sync! { once; body: { let __dsl_result = record.frame(policy).map_err(move |error| map(error)); move || __dsl_result }, on_error: ::core::convert::identity, },
        body: move |bytes| writer.write(bytes),
        }
    }
    /// Close producer access; the effect still awaits the native body's terminal poll.
    pub(crate) fn finish(&self) {
        self.state
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
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
    completion: BodyCompletion,
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
/// Wrap any native body so response construction cannot prematurely finalize its resources.
pub(crate) struct ObservedBody {
    /// Native body remains the authoritative data/trailer producer.
    pub(crate) body: Body,
    /// Shared lifecycle gate retained by the request owner.
    pub(crate) completion: BodyCompletion,
}
impl HttpBody for ObservedBody {
    /// Native body output remains bytes.
    type Data = Bytes;
    /// Preserve native body failures at the committed transport boundary.
    type Error = axum::Error;
    /// Forward frames and publish only actual terminal consumption.
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match Pin::new(&mut self.body).poll_frame(cx) {
            Poll::Ready(None) => {
                self.completion.settle(BodyOutcome::Completed);
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(error))) => {
                self.completion.settle(BodyOutcome::Failed);
                Poll::Ready(Some(Err(error)))
            }
            frame @ Poll::Ready(_) | frame @ Poll::Pending => frame,
        }
    }
    /// Native empty bodies complete immediately when mounted, with no pending data to deliver.
    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
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
    request: &Request,
    policy: BodyPolicy,
    map_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
) -> (Body, BodyWriter<E, R>, BodyCompletion) {
    let (sender, receiver) = mpsc::channel(policy.frames.get());
    let completion = BodyCompletion::new(request.cancellation().clone());
    let writer = BodyWriter {
        state: Arc::new(WriterState {
            sender: Mutex::new(Some(sender)),
            policy,
        }),
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
                                               match context.run(reader.pull(), env.clone()).await {
                                                   Exit::Success(effect::Chunk::Values(values)) => {
                                                       for value in values {
                                                           let result = context
                                                               .run(write(value, writer.clone()), env.clone())
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
pub(crate) struct NativeJsonSink {
    /// Same bounded transport owner as Effect-owned body writes.
    state: Arc<WriterState>,
}
impl NativeJsonSink {
    /// Retain a sink handle without selecting another runtime or copying body data.
    pub(crate) fn from_writer<E: Send + 'static, R: Send + Sync + 'static>(
        writer: &BodyWriter<E, R>,
    ) -> Self {
        Self {
            state: writer.state.clone(),
        }
    }
}
impl effect::JsonWriterSink for NativeJsonSink {
    /// Convert a borrowed codec fragment into owned native bytes on the selected blocking worker.
    /// This unavoidable native ownership conversion copies only the current bounded fragment.
    fn write(&mut self, bytes: &[u8]) -> Result<(), effect::SchemaIssue> {
        let sender = self
            .state
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(json_sink_issue)?;
        for chunk in bytes.chunks(self.state.policy.chunk_bytes.get()) {
            let (accepted, wait) = oneshot::channel();
            sender
                .blocking_send(Delivery {
                    frame: Frame::data(Bytes::copy_from_slice(chunk)),
                    accepted,
                })
                .map_err(|_| json_sink_issue())?;
            wait.blocking_recv().map_err(|_| json_sink_issue())?;
        }
        Ok(())
    }
    /// Structural completion belongs to the document; body completion is still awaited separately.
    fn finish(&mut self) -> Result<(), effect::SchemaIssue> {
        Ok(())
    }
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
