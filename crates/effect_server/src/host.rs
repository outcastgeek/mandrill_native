//! Scoped axum host and the single native request interpretation boundary.

use crate::{
    BodyOutcome, BodyPolicy, Endpoint, Pipeline, Reply, Request, RequestCompletion,
    ResponseHeaders, Router, ServerError,
};
use crate::body::{BodySignal, BodySignalWait};
use crate::{
    effect_die, effect_future, effect_recover, effect_resource, effect_scope, effect_try_sync,
    effect_uninterruptible, endpoint, pipeline, registered,
};
use axum::body::{Body, Bytes};
use axum::extract::FromRequestParts;
use effect::effect;
use effect::{
    Cancellation, Defect, Effect, Exit, HasCancellation, HasTaskSpawner, NativeContext, Scope,
    PreparedSemaphore, PreparedSemaphoreError, PreparedSemaphorePermit,
    memory::MemoryError,
};
use futures_util::StreamExt;
use http::{HeaderValue, Method, StatusCode};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, watch};
use tokio::{pin, select};
use tower::service_fn;

/// Static response metadata and payload for an admission-capacity refusal.
#[derive(Clone, Copy)]
pub struct StaticOverloadReply {
    status: crate::BodyStatus,
    body: &'static [u8],
}
impl StaticOverloadReply {
    /// Select an HTTP status that accepts a body and a static response payload.
    #[must_use]
    pub const fn new(status: crate::BodyStatus, body: &'static [u8]) -> Self {
        Self { status, body }
    }

    fn response(self) -> http::Response<Body> {
        let mut response = http::Response::new(Body::from(Bytes::from_static(self.body)));
        *response.status_mut() = self.status.native();
        response
    }
}

/// Move the request and its mutually exclusive terminal outcomes together.
struct NativeResponseState<E: Send + 'static> {
    /// Owned request whose headers, cancellation and response policy survive selection.
    request: Request,
    /// Already prepared response, consumed instead of observing the routed fiber.
    ready: Option<http::Response<Body>>,
    /// Retained driver joined when selection has not already produced a response or failure.
    routed: Option<effect::Fiber<http::Response<Body>, E>>,
    /// Primary failure moved intact into terminal response and cleanup processing.
    failure: Option<Exit<(), E>>,
    capacity: Option<effect::memory::MemoryError>,
}

struct EndpointDispatchResources {
    on_capacity: Option<StaticOverloadReply>,
    bytes: Option<crate::prepared_endpoint::EndpointByteLeases>,
}

/// Selection handshake gates retain their existing owners and send order.
struct NativeSelectionGates<E: Send + 'static, R: Send + Sync + 'static> {
    /// Observe the chosen request/environment before transferring the routed driver.
    selected: SelectionReceiver<E, R>,
    /// Release terminal execution with its decorated native context.
    start: NativeContextSender<E, R>,
    /// Release matched authentication with the outer selection context.
    selection_start: NativeContextSender<E, R>,
}

/// The serving adapter's Tokio reactor is selected explicitly through R.
pub trait HasTokio {
    /// Borrow a caller-owned Tokio handle. The caller keeps its runtime alive through drain.
    fn tokio(&self) -> &tokio::runtime::Handle;
}
impl<R: HasTokio> HasTokio for crate::RequestEnv<R> {
    /// Preserve the application's selected native reactor.
    fn tokio(&self) -> &tokio::runtime::Handle {
        self.services.tokio()
    }
}
/// Native protocol selection. axum's compiled serving stack supplies these transports.
#[derive(Clone, Copy)]
pub enum HttpProtocols {
    /// HTTP/1 requires the crate's `http1` feature.
    Http1,
    /// HTTP/2 requires the crate's `http2` feature.
    Http2,
    /// Negotiate HTTP/1 and HTTP/2 through the compiled native serving stack.
    Http1And2,
}
type ConnectionServing = (HttpProtocols, Cancellation, Duration, Option<Duration>, Option<PreparedHttpLimits>, Option<PreparedHttp2ResetLimits>, Option<PreparedHttpTimeouts>);
/// Caller-selected HTTP/1 native parser controls. Configuring max_headers causes
/// Hyper to allocate header metadata per request; these are not region-backed.
#[derive(Clone, Copy, Debug)]
pub struct PreparedHttp1Limits {
    /// Maximum header field count.
    pub max_headers: usize,
    /// Native incomplete-head read-buffer limit; at least 8192 bytes.
    /// Allocator rounding and other native buffers are separate.
    pub read_buffer_bytes: usize,
}
/// Caller-selected HTTP/2 protocol and native buffer controls.
/// Upstream header abuse handling permits up to four times the nominal list
/// size and additional protocol overhead; native storage is not region-backed.
#[derive(Clone, Copy, Debug)]
pub struct PreparedHttp2Limits {
    /// Advertised maximum decoded header-list size.
    pub max_header_list_bytes: u32,
    /// Maximum frame payload size.
    pub max_frame_bytes: u32,
    /// Initial stream receive window.
    pub stream_window_bytes: u32,
    /// Initial connection receive window.
    pub connection_window_bytes: u32,
    /// Native per-stream send-capacity target; an individual application
    /// buffer and its backing can exceed this value.
    pub max_send_buffer_bytes: usize,
}
/// Explicit optional native limits for the selected protocols; no defaults.
#[derive(Clone, Copy, Debug)]
pub struct PreparedHttpLimits {
    /// HTTP/1 controls, when selected.
    pub http1: Option<PreparedHttp1Limits>,
    /// HTTP/2 controls, when selected.
    pub http2: Option<PreparedHttp2Limits>,
}
/// Caller-selected native HTTP/2 reset-retention limits. These control protocol
/// behavior, not allocator bytes; zero keeps the native zero-limit meaning.
#[derive(Clone, Copy, Debug)]
pub struct PreparedHttp2ResetLimits {
    /// Maximum pending accepted streams reset by the peer.
    pub pending_accept_resets: usize,
    /// Maximum locally generated error resets before GOAWAY.
    pub local_error_resets: usize,
}
/// Caller-selected tracked HTTP connection deadlines. No timeout is implied.
#[derive(Clone, Copy, Debug)]
pub struct PreparedHttpTimeouts {
    /// Time from socket acceptance through the first fully parsed request head,
    /// including protocol/preface detection. This is not a TLS handshake deadline.
    pub first_head: Option<Duration>,
    /// Time without positive-byte network reads or writes while HTTP owns the socket.
    /// Separately upgraded sessions are outside this deadline.
    pub idle: Option<Duration>,
}
#[derive(Clone)]
struct PreparedRequestQueue {
    gate: Arc<PreparedSemaphore>,
    wait: Duration,
}
impl PreparedRequestQueue {
    async fn admit(&self, stop: &Cancellation) -> Result<PreparedSemaphorePermit, MemoryError> {
        if stop.is_requested() {
            return Err(MemoryError::NoSlot);
        }
        let deadline = tokio::time::Instant::now()
            .checked_add(self.wait)
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let acquire = self.gate.acquire(NonZeroUsize::MIN).map_err(|error| match error {
            PreparedSemaphoreError::Memory(error) => error,
            PreparedSemaphoreError::Semaphore(_) => MemoryError::NoSlot,
        })?;
        select! {
            biased;
            () = stop.cancelled() => Err(MemoryError::NoSlot),
            result = tokio::time::timeout_at(deadline, acquire) => {
                let permit = result.map_err(|_| MemoryError::NoSlot)?
                    .map_err(|_| MemoryError::NoSlot)?;
                if stop.is_requested() || tokio::time::Instant::now() >= deadline {
                    return Err(MemoryError::NoSlot);
                }
                Ok(permit)
            }
        }
    }
}
/// Explicit graceful-drain policy, measured in wall-clock duration.
#[derive(Clone, Copy)]
pub struct HostPolicy {
    /// Time allowed for native connections to drain before request cancellation.
    pub drain: Duration,
}
/// A request environment resolver runs after native routing and ordered pre-environment hooks.
pub struct RequestBinding<E: Send + 'static, R: Send + Sync + 'static> {
    /// Resolver receives request ownership and the original environment through Effect.
    resolve: Arc<dyn Fn(Request) -> Effect<Arc<R>, E, R> + Send + Sync>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RequestBinding<E, R> {
    /// Share environment selection code without cloning request/application values.
    fn clone(&self) -> Self {
        Self {
            resolve: self.resolve.clone(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> RequestBinding<E, R> {
    /// Select fresh request state through the same application R; no second runtime generic exists.
    pub fn new(resolve: impl Fn(Request) -> Effect<Arc<R>, E, R> + Send + Sync + 'static) -> Self {
        Self {
            resolve: Arc::new(resolve),
        }
    }
}
/// Explicit terminal/native policy. E remains typed; defects and ordered cleanup are retained.
pub struct ServerPolicy<E: Send + 'static, R: Send + Sync + 'static> {
    /// Response queue/chunk policy applied only after selecting this request environment.
    pub body: BodyPolicy,
    /// Native transport/input/configuration errors enter the application's E channel once.
    pub(crate) native_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    /// One scoped application-error renderer; its own failure is never rendered recursively.
    pub(crate) on_error: Arc<ErrorRenderer<E, R>>,
    /// Completion sees selected R and the full final exit after joined asynchronous cleanup.
    observers: Arc<Vec<Arc<CompletionObserver<E, R>>>>,
    /// Host drain policy retains request cleanup after native shutdown.
    pub host: HostPolicy,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for ServerPolicy<E, R> {
    /// Share immutable selected policies, never application failures.
    fn clone(&self) -> Self {
        Self {
            body: self.body,
            native_error: self.native_error.clone(),
            on_error: self.on_error.clone(),
            observers: self.observers.clone(),
            host: self.host,
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> ServerPolicy<E, R> {
    /// Construct the explicit error/body/drain boundary without acquiring a host.
    pub fn new(
        body: BodyPolicy,
        host: HostPolicy,
        native_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
        on_error: impl Fn(E) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            body,
            host,
            native_error: Arc::new(native_error),
            on_error: Arc::new(on_error),
            observers: Arc::new(Vec::new()),
        }
    }
    /// Append ordered final-body observation after request cleanup. Callback construction borrows E;
    /// its returned Effect owns any selected observation data and cannot retain borrowed errors.
    pub fn on_exit(
        mut self,
        observe: impl Fn(&R, &Request, &RequestCompletion<E>) -> Effect<(), E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Arc::make_mut(&mut self.observers).push(Arc::new(observe));
        self
    }
}
/// Native insertion point before environment resolution, after route classification is installed.
/// Matched endpoints also expose registration metadata/captures; native short circuits expose
/// [`RouteKind::Native`] so integrations can retain the native service's policy decision.
pub type RequestHook<E, R> = Arc<dyn Fn(Request) -> Effect<(), E, R> + Send + Sync>;
/// A mutable native request stage before actual router selection. The supplied scope
/// retains explicitly registered resources until all response work and cleanup settle.
pub type RoutingHook<E, R> = Arc<
    dyn Fn(crate::RoutingRequest, Scope<E>) -> Effect<crate::RoutingRequest, E, R> + Send + Sync,
>;
type RoutingStageResult = Result<crate::RoutingRequest, effect::memory::MemoryError>;
type CheckedRoutingHook<E, R> = Arc<
    dyn Fn(crate::RoutingRequest, Request, Option<crate::RoutingSnapshotReservation>, Scope<E>)
        -> Effect<RoutingStageResult, E, R>
        + Send
        + Sync,
>;
enum RoutingStage<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(RoutingHook<E, R>),
    Checked {
        updated_head: bool,
        stage: CheckedRoutingHook<E, R>,
    },
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RoutingStage<E, R> {
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(stage) => Self::Ordinary(stage.clone()),
            Self::Checked { updated_head, stage } => Self::Checked {
                updated_head: *updated_head,
                stage: stage.clone(),
            },
        }
    }
}
/// Typed all-response decoration after native/Effect route execution and before commitment.
/// Request metadata includes the exact transport peer and any forwarding/nonce state.
pub type ResponseHook<E, R> =
    Arc<dyn Fn(Request, http::Response<Body>) -> Effect<http::Response<Body>, E, R> + Send + Sync>;
/// Outer request decorator receives the complete scoped program, including joined cleanup and
/// terminal observers. Optional integrations supply it without introducing dependencies into Server.
pub type RequestDecorator<E, R> =
    Arc<dyn Fn(Request, Effect<(), E, R>) -> Effect<(), E, R> + Send + Sync>;
/// Inert application description. Acquiring/listening starts only at `run`/`with_host`.
pub struct Server<E: Send + 'static, R: Send + Sync + 'static> {
    /// Explicit application/native failure and ownership boundary.
    pub policy: ServerPolicy<E, R>,
    /// Fresh environment resolver selected through R.
    pub environment: RequestBinding<E, R>,
    /// Caller-selected native listener address; no hidden port exists.
    pub listen: SocketAddr,
    /// Native compiled transport selection.
    pub protocols: HttpProtocols,
    /// Ordered lazy registration descriptions.
    pub routes: Router<E, R>,
    /// Explicit HTTP/1 header deadline; absent preserves native builder behavior.
    http1_header_timeout: Option<Duration>,
    prepared_http_limits: Option<PreparedHttpLimits>,
    prepared_http2_reset_limits: Option<PreparedHttp2ResetLimits>,
    prepared_http_timeouts: Option<PreparedHttpTimeouts>,
    request_queue: Option<PreparedRequestQueue>,
    response_limits: Option<crate::PreparedResponseLimits>,
    overload_reply: Option<StaticOverloadReply>,
    endpoint_hosting:
        Option<Arc<crate::prepared_connection_retention::ConnectionRetentionPool<E, R>>>,
    native_slots: Option<Arc<crate::prepared_native_slot::NativeSlotRegistry<E, R>>>,
    request_controls: Option<Arc<crate::prepared_request_control::RequestControlPool<E, R>>>,
    body_pool: Option<Arc<crate::prepared_body_completion::BodyPool>>,
    observer_batches: Option<Arc<crate::prepared_observers::ObserverBatchPool<E, R>>>,
    request_storage: Option<Arc<crate::PreparedRequestStoragePool>>,
    request_byte_pools: Option<Arc<crate::PreparedEndpointBytePools>>,
    response_headers: Option<Arc<crate::PreparedResponseHeadersPool>>,
    /// Ordered native auth/culture/correlation stages before resolving R.
    pub(crate) request_hooks: Vec<RequestHook<E, R>>,
    /// Ordered mutable native stages run before axum matches a route.
    routing_hooks: Vec<RoutingStage<E, R>>,
    /// Ordered typed response stages cover every native/application response.
    response_hooks: Vec<ResponseHook<E, R>>,
    /// Outer middleware surrounds all request short circuits and body completion.
    pub(crate) middleware: Pipeline<E, R>,
    /// Outer decorators wrap complete scoped execution and terminal observation in declaration order.
    decorators: Vec<RequestDecorator<E, R>>,
    /// Root-environment decorators begin after classification and before authentication/selection.
    selection_decorators: Vec<RequestDecorator<E, R>>,
    /// Native builder/app hooks are applied before startup.
    pub(crate) configure: Vec<Arc<RouterConfigurator>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Server<E, R> {
    pub(crate) fn select_response_limits(&mut self, limits: crate::PreparedResponseLimits) { self.response_limits = Some(limits); }
    /// Select the reply for capacity refusal before a prepared request enters routing.
    #[must_use]
    pub fn with_overload_reply(mut self, reply: StaticOverloadReply) -> Self {
        self.overload_reply = Some(reply);
        self
    }
    pub(crate) fn observer_count(&self) -> usize { self.policy.observers.len() }
    pub(crate) fn has_prepared_request_storage(&self) -> bool {
        self.request_storage.is_some()
    }
    /// Construct an inert server with all runtime/listener/policy selections explicit.
    #[must_use]
    pub fn new(
        policy: ServerPolicy<E, R>,
        environment: RequestBinding<E, R>,
        listen: SocketAddr,
        protocols: HttpProtocols,
        routes: Router<E, R>,
    ) -> Self {
        Self {
            policy,
            environment,
            listen,
            protocols,
            routes,
            http1_header_timeout: None,
            prepared_http_limits: None,
            prepared_http2_reset_limits: None,
            prepared_http_timeouts: None,
            request_queue: None,
            response_limits: None,
            overload_reply: None,
            endpoint_hosting: None,
            native_slots: None,
            request_controls: None,
            body_pool: None,
            observer_batches: None,
            request_storage: None,
            request_byte_pools: None,
            response_headers: None,
            request_hooks: Vec::new(),
            routing_hooks: Vec::new(),
            response_hooks: Vec::new(),
            middleware: Pipeline::new(),
            decorators: Vec::new(),
            selection_decorators: Vec::new(),
            configure: Vec::new(),
        }
    }
    /// Select the HTTP/1 request-header deadline without changing HTTP/2 policy.
    /// Before listening, the host rejects this setting when HTTP/1 is disabled
    /// or unselected, or the duration exceeds the selected Tokio clock range.
    /// The selected runtime needs a time driver; the native timer and its sleep
    /// allocations remain ordinary runtime storage.
    #[must_use]
    pub fn with_http1_header_timeout(mut self, timeout: Duration) -> Self {
        self.http1_header_timeout = Some(timeout);
        self
    }
    /// Prepare finite endpoint/lifecycle tracking storage at startup. Hyper
    /// parser and Tokio task allocations remain ordinary native allocations.
    #[allow(dead_code)] // Kept internal until prepared host results retain terminal origins.
    pub(crate) fn with_prepared_endpoint_hosting(
        mut self,
        memory: &effect::memory::PreparedMemory<'static>,
        region: effect::memory::RegionId,
        connections: usize,
        requests_per_connection: usize,
        children_per_connection: usize,
        controls: Option<crate::PreparedRequestControlCapacity>,
    ) -> Result<Self, effect::memory::MemoryError> {
        let request_slots = connections.checked_mul(requests_per_connection).ok_or(effect::memory::MemoryError::ArithmeticOverflow)?;
        let native_slots = Arc::new(crate::prepared_native_slot::NativeSlotRegistry::prepare(memory, region, request_slots)?);
        let request_controls = Arc::new(crate::prepared_request_control::RequestControlPool::prepare(memory, region, request_slots, controls)?);
        let body_pool = controls.map(|controls| crate::prepared_body_completion::BodyPool::prepare(memory, region, controls.body_records, controls.body_listeners).map(Arc::new)).transpose()?;
        let observer_batches = Arc::new(crate::prepared_observers::ObserverBatchPool::prepare(memory, region, request_slots, self.policy.observers.len())?);
        self.endpoint_hosting = Some(Arc::new(
            crate::prepared_connection_retention::ConnectionRetentionPool::prepare(
                memory,
                region,
                connections,
                requests_per_connection,
                children_per_connection,
            )?,
        ));
        self.native_slots = Some(native_slots);
        self.request_controls = Some(request_controls);
        self.body_pool = body_pool;
        self.observer_batches = Some(observer_batches);
        Ok(self)
    }
    pub(crate) fn install_request_storage(&mut self, pool: Arc<crate::PreparedRequestStoragePool>) { self.request_storage = Some(pool); }
    pub(crate) fn install_request_byte_pools(&mut self, pools: Arc<crate::PreparedEndpointBytePools>) {
        self.request_byte_pools = Some(pools);
    }
    pub(crate) fn install_response_headers(&mut self, pool: Arc<crate::PreparedResponseHeadersPool>) { self.response_headers = Some(pool); }
    pub(crate) fn install_request_control_admission(
        &mut self,
        admission: effect::memory::Admission<'static, 1>,
    ) -> Result<(), effect::memory::MemoryError> {
        let pool = self.request_controls.as_mut().ok_or(effect::memory::MemoryError::UnsupportedRunContract)?;
        Arc::get_mut(pool).ok_or(effect::memory::MemoryError::Busy)?.install_admission(admission)
    }
    pub(crate) fn select_http_timeouts(&mut self, timeouts: PreparedHttpTimeouts) {
        self.prepared_http_timeouts = Some(timeouts);
    }
    pub(crate) fn select_request_queue(&mut self, gate: Arc<PreparedSemaphore>, wait: Duration) {
        self.request_queue = Some(PreparedRequestQueue { gate, wait });
    }
    pub(crate) fn select_http_limits(&mut self, limits: PreparedHttpLimits) -> Result<(), ServerError> {
        if limits.http1.is_some() && (!cfg!(feature = "http1") || matches!(self.protocols, HttpProtocols::Http2)) {
            return Err(ServerError::Configuration("HTTP/1 limits require HTTP/1".into()));
        }
        if limits.http2.is_some() && (!cfg!(feature = "http2") || matches!(self.protocols, HttpProtocols::Http1)) {
            return Err(ServerError::Configuration("HTTP/2 limits require HTTP/2".into()));
        }
        if limits.http1.is_some_and(|limit| limit.read_buffer_bytes < 8192) {
            return Err(ServerError::Configuration("HTTP/1 read buffer must be at least 8192 bytes".into()));
        }
        if limits.http2.is_some_and(|limit| !(16_384..=16_777_215).contains(&limit.max_frame_bytes) || limit.stream_window_bytes > i32::MAX as u32 || limit.connection_window_bytes > i32::MAX as u32 || limit.max_send_buffer_bytes > u32::MAX as usize) {
            return Err(ServerError::Configuration("HTTP/2 native limit out of range".into()));
        }
        self.prepared_http_limits = Some(limits);
        Ok(())
    }
    pub(crate) fn select_http2_reset_limits(&mut self, limits: PreparedHttp2ResetLimits) -> Result<(), ServerError> {
        if !cfg!(feature = "http2") || matches!(self.protocols, HttpProtocols::Http1) {
            return Err(ServerError::Configuration("HTTP/2 reset limits require HTTP/2".into()));
        }
        self.prepared_http2_reset_limits = Some(limits);
        Ok(())
    }
    pub(crate) fn body_channel_demand(&self, capacity: crate::PreparedBodyChannelCapacity) -> Result<crate::PreparedBodyChannelDemand, effect::memory::MemoryError> {
        capacity.demand(self.policy.body)
    }
    pub(crate) fn install_body_channels(&mut self, memory: &effect::memory::PreparedMemory<'static>, region: effect::memory::RegionId, capacity: crate::PreparedBodyChannelCapacity, cleanup: &effect::PreparedCleanupConfig<effect::QueueError>, panics: &effect::PreparedPanicPool) -> Result<(), effect::memory::MemoryError> {
        let pool = self.body_pool.as_mut().ok_or(effect::memory::MemoryError::UnsupportedRunContract)?;
        let pool = Arc::get_mut(pool).ok_or(effect::memory::MemoryError::Busy)?;
        if pool.channels_configured() { return Err(effect::memory::MemoryError::Busy); }
        let channels = crate::prepared_streaming_channel::BodyChannelPool::prepare(memory, region, self.policy.body, capacity, cleanup, panics)?;
        pool.install_channels(channels)
    }
    pub(crate) fn install_native_bytes_pool(&mut self, bytes: Arc<crate::PreparedNativeBytesPool>) -> Result<(), effect::memory::MemoryError> {
        let pool = self.body_pool.as_mut().ok_or(effect::memory::MemoryError::UnsupportedRunContract)?;
        Arc::get_mut(pool).ok_or(effect::memory::MemoryError::Busy)?.install_bytes_pool(bytes)
    }
    pub(crate) fn install_json_writers(&mut self, writers: Arc<effect::PreparedJsonWriterPool>) -> Result<(), effect::memory::MemoryError> {
        let pool = self.body_pool.as_mut().ok_or(effect::memory::MemoryError::UnsupportedRunContract)?;
        Arc::get_mut(pool).ok_or(effect::memory::MemoryError::Busy)?.install_json_writers(writers)
    }
    pub(crate) fn install_body_completion_admission(&mut self, admission: effect::memory::Admission<'static, 1>) -> Result<(), effect::memory::MemoryError> {
        let pool = self.body_pool.as_mut().ok_or(effect::memory::MemoryError::UnsupportedRunContract)?;
        Arc::get_mut(pool).ok_or(effect::memory::MemoryError::Busy)?.install_admission(admission)
    }
    pub(crate) fn install_connection_admission(&mut self, admission: effect::memory::Admission<'static, 1>) -> Result<(), effect::memory::MemoryError> {
        let pool = self.endpoint_hosting.as_mut().ok_or(effect::memory::MemoryError::UnsupportedRunContract)?;
        Arc::get_mut(pool).ok_or(effect::memory::MemoryError::Busy)?.install_admission(admission)
    }
    pub(crate) fn install_observer_batch_admission(&mut self, admission: effect::memory::Admission<'static, 1>) -> Result<(), effect::memory::MemoryError> {
        let pool = self.observer_batches.as_mut().ok_or(effect::memory::MemoryError::UnsupportedRunContract)?;
        Arc::get_mut(pool).ok_or(effect::memory::MemoryError::Busy)?.install_admission(admission)
    }
    /// Transform mutable native metadata before actual axum route selection. Method override
    /// can parse a selected bounded compatible form through `routing.request()`; unread bodies
    /// transfer without buffering, and the form cache stays shared with the endpoint.
    pub fn before_routing(
        mut self,
        stage: impl Fn(crate::RoutingRequest, Scope<E>) -> Effect<crate::RoutingRequest, E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.routing_hooks.push(RoutingStage::Ordinary(Arc::new(stage)));
        self
    }
    /// Transform routing with an admitted initial request and optional updated-head vacancy.
    /// A checked stage may report a typed memory refusal before authentication or body work.
    pub fn before_routing_checked(
        mut self,
        updated_head: bool,
        stage: impl Fn(crate::RoutingRequest, Request, Option<crate::RoutingSnapshotReservation>, Scope<E>)
            -> Effect<RoutingStageResult, E, R>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.routing_hooks.push(RoutingStage::Checked { updated_head, stage: Arc::new(stage) });
        self
    }
    /// Decorate every response before commitment, including native CORS/static/404/405 paths.
    /// Typed failures invoke the selected error renderer once; no detached body task is created.
    pub fn after_response(
        mut self,
        stage: impl Fn(Request, http::Response<Body>) -> Effect<http::Response<Body>, E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.response_hooks.push(Arc::new(stage));
        self
    }
    /// Insert ordered request-native work after routing and before application environment resolution.
    /// This runs for matched endpoints and native short circuits. Inspect [`RouteKind`] when a
    /// policy applies only to endpoints; registration metadata and captures exist only when matched.
    pub fn before_environment(
        mut self,
        hook: impl Fn(Request) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        self.request_hooks.push(Arc::new(hook));
        self
    }
    /// Wrap the already-owned complete request program after selecting R. The decorator retains
    /// body delivery, ordered asynchronous cleanup and terminal observation inside one Effect.
    pub fn around_request(
        mut self,
        decorate: impl Fn(Request, Effect<(), E, R>) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        self.decorators.push(Arc::new(decorate));
        self
    }
    /// Wrap classification through terminal completion under the original host R. This begins
    /// after [`RouteKind`] and matched endpoint metadata/captures are installed, before every
    /// `before_environment` hook and the request resolver. The decorated interpreter context is
    /// handed to authentication/resolution, then retained when selected R runs endpoint work,
    /// final native body delivery, ordered cleanup and observers. Native short circuits use the
    /// same boundary. The decorator itself keeps root R; `around_request` and observers use
    /// selected R (or root R on selection failure). An integration needing one request span
    /// installs it here once, rather than also installing that span with `around_request`.
    pub fn around_selection(
        mut self,
        decorate: impl Fn(Request, Effect<(), E, R>) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        self.selection_decorators.push(Arc::new(decorate));
        self
    }
    /// Wrap every Effect request, including downstream short circuits, through body settlement.
    #[must_use]
    pub fn middleware(mut self, pipeline: Pipeline<E, R>) -> Self {
        self.middleware = self.middleware.append(pipeline);
        self
    }
    /// Retain a native axum/Tower configuration hook before accepting traffic. These layers run
    /// inside `before_routing`/`after_response` ownership, including native short-circuit responses.
    pub fn configure_app(
        mut self,
        configure: impl Fn(axum::Router) -> Result<axum::Router, ServerError> + Send + Sync + 'static,
    ) -> Self {
        self.configure.push(Arc::new(configure));
        self
    }
    /// Native builder configuration runs at the same explicit pre-start construction point.
    pub fn configure_builder(
        self,
        configure: impl Fn(axum::Router) -> Result<axum::Router, ServerError> + Send + Sync + 'static,
    ) -> Self {
        self.configure_app(configure)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for Server<E, R> {
    /// Share only inert configuration/registration descriptions; each run acquires a new native host.
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.clone(),
            environment: self.environment.clone(),
            listen: self.listen,
            protocols: self.protocols,
            routes: self.routes.clone(),
            http1_header_timeout: self.http1_header_timeout,
            prepared_http_limits: self.prepared_http_limits,
            prepared_http2_reset_limits: self.prepared_http2_reset_limits,
            prepared_http_timeouts: self.prepared_http_timeouts,
            request_queue: self.request_queue.clone(),
            response_limits: self.response_limits,
            overload_reply: self.overload_reply,
            endpoint_hosting: self.endpoint_hosting.clone(),
            native_slots: self.native_slots.clone(),
            request_controls: self.request_controls.clone(),
            body_pool: self.body_pool.clone(),
            observer_batches: self.observer_batches.clone(),
            request_storage: self.request_storage.clone(),
            request_byte_pools: self.request_byte_pools.clone(),
            response_headers: self.response_headers.clone(),
            request_hooks: self.request_hooks.clone(),
            routing_hooks: self.routing_hooks.clone(),
            response_hooks: self.response_hooks.clone(),
            middleware: self.middleware.clone(),
            decorators: self.decorators.clone(),
            selection_decorators: self.selection_decorators.clone(),
            configure: self.configure.clone(),
        }
    }
}
/// Running host handle; requesting stop signals native drain and never aborts request finalizers.
#[derive(Clone)]
pub struct ServerHost {
    /// Actual listener address, including OS-selected port when explicitly requested.
    pub address: SocketAddr,
    /// Native graceful stop request.
    stop: Cancellation,
    /// Startup/shutdown acknowledgment shared by use/close paths.
    stopped: watch::Receiver<bool>,
}
impl ServerHost {
    /// Begin native graceful drain. Repeated calls share one terminal request.
    pub fn stop(&self) {
        let _ = self.stop.request();
    }
    /// Await native serving completion; enclosing Effect scope additionally joins request cleanup.
    pub async fn drained(&self) {
        let mut stopped = self.stopped.clone();
        while !*stopped.borrow_and_update() {
            if stopped.changed().await.is_err() {
                break;
            }
        }
    }
}
/// Retained native host/request interpretation context, never a global service locator.
struct Boundary<E: Send + 'static, R: Send + Sync + 'static> {
    response_limits: Option<crate::PreparedResponseLimits>,
    overload_reply: Option<StaticOverloadReply>,
    request_queue: Option<PreparedRequestQueue>,
    native_slots: Option<Arc<crate::prepared_native_slot::NativeSlotRegistry<E, R>>>,
    request_controls: Option<Arc<crate::prepared_request_control::RequestControlPool<E, R>>>,
    body_pool: Option<Arc<crate::prepared_body_completion::BodyPool>>,
    observer_batches: Option<Arc<crate::prepared_observers::ObserverBatchPool<E, R>>>,
    request_storage: Option<Arc<crate::PreparedRequestStoragePool>>,
    request_byte_pools: Option<Arc<crate::PreparedEndpointBytePools>>,
    response_headers: Option<Arc<crate::PreparedResponseHeadersPool>>,
    terminal: Option<crate::prepared_terminal::TerminalHandle<E, R>>,
    cleanup: Option<Arc<effect::PreparedCleanupConfig<E>>>,
    native_tasks: Option<Arc<dyn crate::prepared_native_task::NativeTaskDispatch<E, R>>>,
    connection_tasks: Option<Arc<PreparedConnectionSelection<E, R>>>,
    serving_cleanup: Mutex<Option<[effect::CleanupFailures<E>; 2]>>,
    cleanup_waiter: Mutex<Option<effect::CleanupReturnWaiter<E>>>,
    /// Host scope owns all admitted request fibers until their asynchronous cleanup settles.
    scope: Scope<E>,
    /// Existing interpreter context carries tracing and selected native instrumentation.
    context: NativeContext,
    /// Original application environment used only for pre-request selection.
    environment: Arc<R>,
    /// Shared immutable terminal/body policy.
    policy: ServerPolicy<E, R>,
    /// Selected fresh environment resolver.
    binding: RequestBinding<E, R>,
    /// Ordered native request stages after route matching.
    hooks: Vec<RequestHook<E, R>>,
    /// Actual outer native request transforms and response decoration.
    routing_hooks: Vec<RoutingStage<E, R>>,
    /// All-response stages under selected request ownership.
    response_hooks: Arc<[ResponseHook<E, R>]>,
    /// Outer Effect middleware around short circuits and endpoint bodies.
    middleware: Pipeline<E, R>,
    /// Integrations wrap complete scope/finalization and terminal observation.
    decorators: Vec<RequestDecorator<E, R>>,
    /// Root-R outer context begins before native authentication and request selection.
    selection_decorators: Arc<[RequestDecorator<E, R>]>,
}
/// Native route classification is installed by Server before post-route policy selection.
/// It carries no authentication authority; native layers retain their own ordered policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteKind {
    /// A matched Effect endpoint runs scoped application/authentication stages.
    Endpoint,
    /// A native service/layer/404/405 short circuit owns its native response decision.
    Native,
}
/// Actual outer transport completion also keeps endpoint resources alive through
/// native compression/layers that may finish reading upstream before wire framing ends.
#[derive(Clone)]
struct OuterDelivery(BodySignal);
/// Upstream completion is distinct from final layered transport completion.
#[derive(Clone)]
struct EndpointDelivery(BodySignal);
/// Selected environment handshake before endpoint work starts; native short circuits
/// perform the same post-route selection in the retained outer owner.
pub(crate) struct NativeSelection<R> {
    /// Final routed/native metadata and its checked body owner.
    request: Request,
    /// Actual selected request capabilities.
    environment: Arc<R>,
    /// A failed selection uses original services without rebasing root cancellation.
    selected: bool,
}
enum ClassificationSender<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(watch::Sender<Option<Request>>),
    Prepared(crate::prepared_request_control::ClassificationProducer<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for ClassificationSender<E, R> {
    fn clone(&self) -> Self { match self { Self::Ordinary(sender) => Self::Ordinary(sender.clone()), Self::Prepared(sender) => Self::Prepared(sender.clone()) } }
}
enum ClassificationPublication<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(watch::Sender<Option<Request>>),
    Prepared(crate::prepared_request_control::ClassificationPublication<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> ClassificationSender<E, R> {
    fn reserve(&self) -> Result<ClassificationPublication<E, R>, effect::memory::MemoryError> {
        match self { Self::Ordinary(sender) => Ok(ClassificationPublication::Ordinary(sender.clone())), Self::Prepared(sender) => sender.reserve().map(ClassificationPublication::Prepared) }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> ClassificationPublication<E, R> {
    fn publish(self, request: Request) { match self { Self::Ordinary(sender) => { sender.send_replace(Some(request)); }, Self::Prepared(publication) => publication.publish(request) } }
}
enum SelectionSender<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(watch::Sender<Option<Arc<NativeSelection<R>>>>),
    Prepared(crate::prepared_request_control::SelectionProducer<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for SelectionSender<E, R> {
    fn clone(&self) -> Self { match self { Self::Ordinary(sender) => Self::Ordinary(sender.clone()), Self::Prepared(sender) => Self::Prepared(sender.clone()) } }
}
enum SelectionPublication<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(watch::Sender<Option<Arc<NativeSelection<R>>>>),
    Prepared(crate::prepared_request_control::SelectionPublication<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> SelectionSender<E, R> {
    fn reserve(&self) -> Result<SelectionPublication<E, R>, effect::memory::MemoryError> {
        match self { Self::Ordinary(sender) => Ok(SelectionPublication::Ordinary(sender.clone())), Self::Prepared(sender) => sender.reserve().map(SelectionPublication::Prepared) }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> SelectionPublication<E, R> {
    fn publish(self, selection: NativeSelection<R>) { match self { Self::Ordinary(sender) => { sender.send_replace(Some(Arc::new(selection))); }, Self::Prepared(publication) => publication.publish(selection) } }
}
enum ClassificationReceiver<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(watch::Receiver<Option<Request>>),
    Prepared { receiver: effect::PreparedWatchReceiver<Option<Request>>, _owner: crate::prepared_request_control::RequestControlRef<E, R> },
}
impl<E: Send + 'static, R: Send + Sync + 'static> ClassificationReceiver<E, R> {
    fn snapshot(&mut self) -> Result<Option<Request>, effect::memory::MemoryError> {
        match self { Self::Ordinary(receiver) => Ok(receiver.borrow_and_update().clone()), Self::Prepared { receiver, .. } => { let Some(view) = receiver.borrow_and_update()? else { return Ok(None) }; let borrow = view.borrow(); Ok(borrow.as_ref().and_then(|borrow| borrow.get()).and_then(Clone::clone)) } }
    }
    async fn changed(&mut self) -> Result<(), ()> { match self { Self::Ordinary(receiver) => receiver.changed().await.map_err(|_| ()), Self::Prepared { receiver, .. } => receiver.changed().await.map_err(|_| ()) } }
}
enum SelectionReceiver<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(watch::Receiver<Option<Arc<NativeSelection<R>>>>),
    Prepared { receiver: effect::PreparedWatchReceiver<Option<NativeSelection<R>>>, _owner: crate::prepared_request_control::RequestControlRef<E, R> },
}
impl<E: Send + 'static, R: Send + Sync + 'static> SelectionReceiver<E, R> {
    fn snapshot(&mut self) -> Result<Option<(Request, Arc<R>, bool)>, effect::memory::MemoryError> {
        match self {
            Self::Ordinary(receiver) => Ok(receiver.borrow_and_update().as_ref().map(|selection| (selection.request.clone(), selection.environment.clone(), selection.selected))),
            Self::Prepared { receiver, .. } => { let Some(view) = receiver.borrow_and_update()? else { return Ok(None) }; let borrow = view.borrow(); Ok(borrow.as_ref().and_then(|borrow| borrow.get()).and_then(|selection| selection.as_ref()).map(|selection| (selection.request.clone(), selection.environment.clone(), selection.selected))) }
        }
    }
    async fn changed(&mut self) -> Result<(), ()> { match self { Self::Ordinary(receiver) => receiver.changed().await.map_err(|_| ()), Self::Prepared { receiver, .. } => receiver.changed().await.map_err(|_| ()) } }
}
type PreparedOrigin<E, R> = effect::LeasedRunResult<(), Reply<E, R>, E>;
pub(crate) type NativeResponse<E, R> = (
    http::Response<Body>,
    Option<crate::prepared_transport_body::TransportOrigin<E, R>>,
);
enum ObserverEffects<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(std::vec::IntoIter<Effect<(), E, R>>),
    Prepared(crate::prepared_observers::ObserverBatch<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Iterator for ObserverEffects<E, R> {
    type Item = Effect<(), E, R>;
    fn next(&mut self) -> Option<Self::Item> { match self { Self::Ordinary(effects) => effects.next(), Self::Prepared(effects) => effects.pop_front() } }
}
/// Complete front-phase request bundle. Failed acquisition drops all earlier
/// owners before returning, without mutating the incoming request.
struct PreparedRequestAdmission<E: Send + 'static, R: Send + Sync + 'static> {
    response_headers: Option<ResponseHeaders>,
    observers: crate::prepared_observers::ObserverBatch<E, R>,
    request_storage: Option<(crate::prepared_request_storage::PreparedRequestStorageReservation, crate::prepared_request_storage::SnapshotVacancy, crate::prepared_request_storage::SnapshotVacancy)>,
    endpoint_bytes: Option<crate::prepared_endpoint::EndpointByteLeases>,
    body: Option<crate::prepared_body_completion::BodyVacancy>,
    error_bodies: [Option<crate::prepared_body_completion::ReplyBodyVacancy<E>>; 2],
    native_slot: crate::prepared_native_slot::NativeSlotVacancy<E, R>,
    channels: crate::prepared_request_control::RequestChannels<E, R>,
    retention: crate::prepared_connection_retention::RequestRetention<E, R>,
    terminal: Option<crate::prepared_terminal::TerminalRequest<E, R>>,
    cleanup: [Option<effect::CleanupFailures<E>>; 2],
    queue_permit: Option<PreparedSemaphorePermit>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> PreparedRequestAdmission<E, R> {
    fn reserve(
        tracking: &crate::prepared_connection_retention::TrackedExecutor<E, R>,
        boundary: &Boundary<E, R>,
        permit: Option<PreparedSemaphorePermit>,
    ) -> Result<Self, effect::memory::MemoryError> {
        // Tracked hosting is only constructed by PreparedServer, which supplies
        // both startup-owned authorities. Reject missing configuration before
        // reserving any request record.
        let terminal = boundary.terminal.as_ref().ok_or(MemoryError::UnsupportedRunContract)?;
        let cleanup = boundary.cleanup.as_ref().ok_or(MemoryError::UnsupportedRunContract)?;
        let native_slots = boundary.native_slots.as_deref().ok_or(MemoryError::UnsupportedRunContract)?;
        let request_controls = boundary.request_controls.as_deref().ok_or(MemoryError::UnsupportedRunContract)?;
        let observer_batches = boundary.observer_batches.as_deref().ok_or(MemoryError::UnsupportedRunContract)?;
        // Clear old values before request metadata admission: a prior value can
        // itself retain request owners and must retire before those slots are tried.
        let response_headers = boundary.response_headers.as_deref().map(crate::PreparedResponseHeadersPool::reserve).transpose()?;
        let request_storage = match &boundary.request_storage {
            Some(pool) => Some((pool.reserve_request()?, pool.reserve_snapshot()?, pool.reserve_snapshot()?)),
            None => None,
        };
        let endpoint_bytes = boundary.request_byte_pools.as_deref().map(|pools| pools.reserve(None)).transpose()?;
        let native_slot = native_slots.reserve()?;
        let channels = request_controls.reserve()?;
        let observers = observer_batches.reserve()?;
        let body_pool = boundary.body_pool.as_deref();
        if channels.control.watches_configured() && body_pool.is_none() { return Err(effect::memory::MemoryError::UnsupportedRunContract); }
        // The initial failure renderer and the one possible failing response-hook
        // renderer can coexist, so both actual call sites reserve before work.
        let (body, error_bodies) = match body_pool {
            Some(pool) => (Some(pool.reserve()?), [Some(pool.reserve_reply(cleanup)?), Some(pool.reserve_reply(cleanup)?)]),
            None => (None, [None, None]),
        };
        if channels.control.has_first_inner_reservation() {
            let (Some(pool), Some(body)) = (body_pool, body.as_ref()) else {
                return Err(effect::memory::MemoryError::UnsupportedRunContract);
            };
            let outer_wait = BodySignalWait::Prepared(body.admit_wait()?);
            let vacancies = [Some(pool.reserve_reply(cleanup)?), Some(pool.reserve_reply(cleanup)?)];
            channels.control.install_first_inner_resources(outer_wait, vacancies)?;
        }
        let retention = tracking.try_handle()?.reserve_request()?;
        let terminal = Some(terminal.reserve_request()?);
        let cleanup = [
            Some(cleanup.reserve_failures()?),
            Some(cleanup.reserve_failures()?),
        ];
        request_controls.admit_quota(&channels.control)?;
        Ok(Self {
            response_headers,
            native_slot,
            channels,
            observers,
            request_storage,
            endpoint_bytes,
            body,
            error_bodies,
            retention,
            terminal,
            cleanup,
            queue_permit: permit,
        })
    }
}
pub(crate) struct NativeOutputState<E: Send + 'static, R: Send + Sync + 'static> {
    observers: Option<crate::prepared_observers::ObserverBatch<E, R>>,
    outer_vacancy: Option<crate::prepared_body_completion::BodyVacancy>,
    error_bodies: [Option<crate::prepared_body_completion::ReplyBodyVacancy<E>>; 2],
    outer_wait: Option<BodySignalWait>,
    native_slot: Option<crate::prepared_native_slot::NativeSlotVacancy<E, R>>,
    pub(crate) sender: Option<NativeResponseSender<E, R>>,
    pub(crate) retention: Option<crate::prepared_connection_retention::RequestRetention<E, R>>,
    pub(crate) terminal: Option<crate::prepared_terminal::TerminalRequest<E, R>>,
    installed_terminal: Option<crate::prepared_terminal::InstalledTerminalRequest<E, R>>,
    origin: Option<PreparedOrigin<E, R>>,
    admission: Option<crate::prepared_endpoint::PreparedOwnedEndpointAdmission<E, R>>,
    execution: Option<PreparedOrigin<E, R>>,
    route_capacity: Option<effect::memory::MemoryError>,
    cleanup: [Option<effect::CleanupFailures<E>>; 2],
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeOutputState<E, R> {
    pub(crate) fn empty() -> Self {
        Self { observers: None, outer_vacancy: None, error_bodies: [None, None], outer_wait: None, native_slot: None, sender: None, retention: None, terminal: None, installed_terminal: None, origin: None, admission: None, execution: None, route_capacity: None, cleanup: [None, None] }
    }
}
pub(crate) enum NativeResponseSender<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(oneshot::Sender<NativeResponse<E, R>>),
    Prepared(effect::PreparedSender<NativeResponse<E, R>>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeResponseSender<E, R> {
    #[allow(clippy::result_large_err)] // Refusal returns the original response and its origin.
    fn send(self, value: NativeResponse<E, R>) -> Result<(), NativeResponse<E, R>> {
        match self { Self::Ordinary(sender) => sender.send(value), Self::Prepared(sender) => sender.send(value) }
    }
}
enum NativeResponseReceiver<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(oneshot::Receiver<NativeResponse<E, R>>),
    Prepared { receiver: effect::PreparedReceiver<NativeResponse<E, R>>, _owner: crate::prepared_request_control::RequestControlRef<E, R> },
}
impl<E: Send + 'static, R: Send + Sync + 'static> std::future::Future for NativeResponseReceiver<E, R> {
    type Output = Result<NativeResponse<E, R>, ()>;
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        match self.get_mut() {
            Self::Ordinary(receiver) => std::pin::Pin::new(receiver).poll(cx).map(|result| result.map_err(|_| ())),
            Self::Prepared { receiver, .. } => std::pin::Pin::new(receiver).poll(cx).map(|result| result.map_err(|_| ())),
        }
    }
}
enum BodySignalState<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(Arc<Mutex<Option<BodySignal>>>),
    PreparedOuter(crate::prepared_request_control::OuterBodyState<E, R>),
    PreparedInner(crate::prepared_request_control::InnerBodyState<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for BodySignalState<E, R> {
    fn clone(&self) -> Self { match self { Self::Ordinary(value) => Self::Ordinary(value.clone()), Self::PreparedOuter(value) => Self::PreparedOuter(value.clone()), Self::PreparedInner(value) => Self::PreparedInner(value.clone()) } }
}
impl<E: Send + 'static, R: Send + Sync + 'static> BodySignalState<E, R> {
    fn is_none(&self) -> bool { match self { Self::Ordinary(value) => value.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_none(), Self::PreparedOuter(value) => value.is_none(), Self::PreparedInner(value) => value.is_none() } }
    fn outcome(&self) -> BodyOutcome { match self { Self::Ordinary(value) => value.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map(BodySignal::outcome).unwrap_or(BodyOutcome::Open), Self::PreparedOuter(value) => value.outcome(), Self::PreparedInner(value) => value.outcome() } }
    fn replace(&self, signal: BodySignal) { match self { Self::Ordinary(value) => { let previous = value.lock().unwrap_or_else(std::sync::PoisonError::into_inner).replace(signal); drop(previous); }, Self::PreparedOuter(value) => value.replace(signal), Self::PreparedInner(value) => value.replace(signal) } }
}
enum NativeRoutingOwner<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(Arc<Mutex<Option<crate::RoutingRequest>>>),
    Prepared(crate::prepared_request_control::RequestControlRef<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for NativeRoutingOwner<E, R> {
    fn clone(&self) -> Self {
        match self { Self::Ordinary(owner) => Self::Ordinary(owner.clone()), Self::Prepared(owner) => Self::Prepared(owner.clone()) }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeRoutingOwner<E, R> {
    fn take(&self) -> Option<crate::RoutingRequest> {
        match self {
            Self::Ordinary(owner) => owner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take(),
            Self::Prepared(owner) => owner.take_routing(),
        }
    }
}
pub(crate) enum NativeOutput<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(Arc<Mutex<NativeOutputState<E, R>>>),
    Prepared(crate::prepared_request_control::RequestControlRef<E, R>),
}

enum NativeEndpointBody<E: Send + 'static, R: Send + Sync + 'static> {
    Registered(Arc<crate::endpoint::EndpointBody<E, R>>),
    Prepared(NativeOutput<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for NativeEndpointBody<E, R> {
    fn clone(&self) -> Self {
        match self {
            Self::Registered(body) => Self::Registered(body.clone()),
            Self::Prepared(output) => Self::Prepared(output.clone()),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeEndpointBody<E, R> {
    fn program(&self, scope: Scope<E>) -> Effect<Reply<E, R>, E, R> {
        match self {
            Self::Registered(body) => body(scope),
            Self::Prepared(output) => {
                let output = output.clone();
                crate::endpoint::native_once(move |environment, _| async move {
                    let parts = {
                        let mut state = output
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        (
                            state.admission.take(),
                            state.retention.take(),
                            state.terminal.take(),
                        )
                    };
                    let (Some(admission), Some(retention), Some(terminal)) = parts else {
                        return Exit::Die(Defect::new("prepared request admission is missing"));
                    };
                    let (result, [connection, body, terminal_origin]) =
                        admission.publish_shared(environment).await;
                    let terminal = terminal.install(terminal_origin);
                    // Install the execution, body and terminal owners before
                    // retiring the duplicate connection root.
                    let (exit, execution) = unsafe { result.into_parts_unchecked() };
                    {
                        let mut state = output
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        state.origin = Some(body);
                        state.execution = Some(execution);
                        state.installed_terminal = Some(terminal);
                    }
                    retention.handoff_duplicate(connection);
                    exit
                })
            }
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeOutput<E, R> {
    fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, NativeOutputState<E, R>>> {
        match self { Self::Ordinary(output) => output.lock(), Self::Prepared(control) => control.lock() }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for NativeOutput<E, R> {
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(output) => Self::Ordinary(output.clone()),
            Self::Prepared(control) => { let control = control.clone(); control.add_producer(); Self::Prepared(control) }
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for NativeOutput<E, R> {
    fn drop(&mut self) { if let Self::Prepared(control) = self { control.retire_producer(); } }
}
enum InnerResponseSender<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(oneshot::Sender<http::Response<Body>>),
    Prepared(effect::PreparedSender<http::Response<Body>>, crate::prepared_request_control::InnerProducer<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> InnerResponseSender<E, R> {
    #[allow(clippy::result_large_err)] // The original response is returned on refusal.
    fn send(self, response: http::Response<Body>) -> Result<(), http::Response<Body>> {
        match self { Self::Ordinary(sender) => sender.send(response), Self::Prepared(sender, owner) => { let result = sender.send(response); drop(owner); result } }
    }
}
enum InnerOutput<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(Arc<Mutex<Option<oneshot::Sender<http::Response<Body>>>>>),
    Prepared(crate::prepared_request_control::InnerProducer<E, R>),
}
pub(crate) struct ReplyDelivery<E: Send + 'static, R: Send + Sync + 'static> {
    request: Request,
    scope: Scope<E>,
    policy: ServerPolicy<E, R>,
    output: InnerOutput<E, R>,
    state: BodySignalState<E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for ReplyDelivery<E, R> {
    fn clone(&self) -> Self {
        Self { request: self.request.clone(), scope: self.scope.clone(), policy: self.policy.clone(), output: self.output.clone(), state: self.state.clone() }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> ReplyDelivery<E, R> {
    pub(crate) fn invoke(self, reply: Reply<E, R>) -> Effect<(), E, R> {
        deliver(reply, self.request, self.scope, self.policy, self.output, self.state)
    }
}
fn reply_continuation<E: Send + 'static, R: Send + Sync + 'static>(
    delivery: ReplyDelivery<E, R>,
) -> Result<crate::endpoint::ReplyContinuation<E, R>, effect::memory::MemoryError> {
    match &delivery.output {
        InnerOutput::Prepared(producer) => {
            let producer = producer.clone();
            producer.try_continuation(delivery)
                .map(crate::endpoint::ReplyContinuation::Prepared)

        }
        InnerOutput::Ordinary(_) => Ok(crate::endpoint::ReplyContinuation::Ordinary(
            Arc::new(move |reply| delivery.clone().invoke(reply)),
        )),
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for InnerOutput<E, R> {
    fn clone(&self) -> Self { match self { Self::Ordinary(output) => Self::Ordinary(output.clone()), Self::Prepared(output) => Self::Prepared(output.clone()) } }
}
impl<E: Send + 'static, R: Send + Sync + 'static> InnerOutput<E, R> {
    fn install_body_vacancies(&self, vacancies: [Option<crate::prepared_body_completion::ReplyBodyVacancy<E>>; 2]) { if let Self::Prepared(output) = self { output.install_body_vacancies(vacancies); } }
    fn take_body_vacancy(&self) -> Option<crate::prepared_body_completion::ReplyBodyVacancy<E>> { match self { Self::Ordinary(_) => None, Self::Prepared(output) => output.take_body_vacancy() } }
    fn install_outer_wait(&self, wait: BodySignalWait) { if let Self::Prepared(output) = self { output.install_outer_wait(wait); } }
    fn take_outer_wait(&self) -> Option<BodySignalWait> { match self { Self::Ordinary(_) => None, Self::Prepared(output) => output.take_outer_wait() } }
    fn take(&self) -> Option<InnerResponseSender<E, R>> { match self { Self::Ordinary(output) => output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take().map(InnerResponseSender::Ordinary), Self::Prepared(output) => output.take().map(|(sender, owner)| InnerResponseSender::Prepared(sender, owner)) } }
    fn is_none(&self) -> bool { match self { Self::Ordinary(output) => output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_none(), Self::Prepared(output) => output.is_none() } }
}
enum InnerResponseReceiver<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(oneshot::Receiver<http::Response<Body>>),
    Prepared(crate::prepared_request_control::InnerReceiver<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> std::future::Future for InnerResponseReceiver<E, R> {
    type Output = Result<http::Response<Body>, ()>;
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        match self.get_mut() { Self::Ordinary(receiver) => std::pin::Pin::new(receiver).poll(cx).map(|result| result.map_err(|_| ())), Self::Prepared(receiver) => std::pin::Pin::new(receiver).poll(cx).map(|result| result.map_err(|_| ())) }
    }
}
enum NativeContextSender<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(oneshot::Sender<NativeContext>),
    Prepared(effect::PreparedSender<NativeContext>, crate::prepared_request_control::RequestControlRef<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeContextSender<E, R> {
    fn send(self, value: NativeContext) {
        match self {
            Self::Ordinary(sender) => drop(sender.send(value)),
            Self::Prepared(sender, owner) => {
                let sent = sender.send(value);
                drop(owner);
                drop(sent);
            }
        }
    }
}
enum NativeContextReceiver<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(oneshot::Receiver<NativeContext>),
    Prepared { receiver: effect::PreparedReceiver<NativeContext>, _owner: crate::prepared_request_control::RequestControlRef<E, R> },
}
impl<E: Send + 'static, R: Send + Sync + 'static> std::future::Future for NativeContextReceiver<E, R> {
    type Output = Result<NativeContext, ()>;
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        match self.get_mut() {
            Self::Ordinary(receiver) => std::pin::Pin::new(receiver).poll(cx).map(|result| result.map_err(|_| ())),
            Self::Prepared { receiver, .. } => std::pin::Pin::new(receiver).poll(cx).map(|result| result.map_err(|_| ())),
        }
    }
}
enum NativeContextGate<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(Arc<Mutex<Option<oneshot::Receiver<NativeContext>>>>),
    Prepared(crate::prepared_request_control::RequestControlRef<E, R>, bool),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for NativeContextGate<E, R> {
    fn clone(&self) -> Self { match self { Self::Ordinary(gate) => Self::Ordinary(gate.clone()), Self::Prepared(owner, selection) => Self::Prepared(owner.clone(), *selection) } }
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeContextGate<E, R> {
    fn take(&self) -> Option<NativeContextReceiver<E, R>> {
        match self {
            Self::Ordinary(gate) => gate.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take().map(NativeContextReceiver::Ordinary),
            Self::Prepared(owner, selection) => owner.take_gate(*selection).map(|receiver| NativeContextReceiver::Prepared { receiver, _owner: owner.clone() }),
        }
    }
}
enum NativeRequestShadow<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(Arc<Mutex<Request>>),
    Prepared(crate::prepared_request_control::RequestShadow<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for NativeRequestShadow<E, R> {
    fn clone(&self) -> Self { match self { Self::Ordinary(value) => Self::Ordinary(value.clone()), Self::Prepared(value) => Self::Prepared(value.clone()) } }
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeRequestShadow<E, R> {
    fn with<T>(&self, access: impl FnOnce(&Request) -> T) -> Option<T> {
        match self { Self::Ordinary(value) => Some(access(&value.lock().unwrap_or_else(std::sync::PoisonError::into_inner))), Self::Prepared(value) => value.with(access) }
    }
    fn replace(&self, request: Request) -> Result<(), Request> {
        match self {
            Self::Ordinary(value) => { let previous = std::mem::replace(&mut *value.lock().unwrap_or_else(std::sync::PoisonError::into_inner), request); drop(previous); Ok(()) },
            Self::Prepared(value) => value.replace(request),
        }
    }
}
/// Request-local bridge between native routing and the one retained outer request scope.
pub(crate) struct NativeSlot<E: Send + 'static, R: Send + Sync + 'static> {
    /// Outer owner retains routing and endpoint fibers through body completion.
    scope: Scope<E>,
    /// Existing interpreter context admits native routing without creating a runtime.
    context: NativeContext,
    /// Route selection publishes R before any endpoint/body program executes.
    selection: SelectionSender<E, R>,
    /// Classification is published before authentication or request-environment resolution.
    classified: ClassificationSender<E, R>,
    /// Root-R outer decoration supplies context before the first authentication effect.
    selection_start: NativeContextGate<E, R>,
    /// Complete outer decoration supplies the endpoint's final trace/context.
    start: NativeContextGate<E, R>,
    /// Final request metadata shared with native response decoration.
    request: NativeRequestShadow<E, R>,
    /// One endpoint driver is joined explicitly before outer terminal observation.
    inner: NativeInnerDriver<E, R>,
    output: NativeOutput<E, R>,
}
enum NativeInnerDriver<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(Arc<Mutex<Option<effect::Fiber<(), E>>>>),
    Prepared(crate::prepared_request_control::RequestControlRef<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for NativeInnerDriver<E, R> {
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(cell) => Self::Ordinary(Arc::clone(cell)),
            Self::Prepared(control) => Self::Prepared(control.clone()),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeInnerDriver<E, R> {
    fn replace(&self, fiber: effect::Fiber<(), E>) {
        let old = match self {
            Self::Ordinary(cell) => cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner).replace(fiber),
            Self::Prepared(control) => control.replace_inner_driver(fiber),
        };
        drop(old);
    }
    fn take(&self) -> Option<effect::Fiber<(), E>> {
        match self {
            Self::Ordinary(cell) => cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take(),
            Self::Prepared(control) => control.take_inner_driver(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for NativeSlot<E, R> {
    /// Clone only descriptions/shared ownership, never application failures or bodies.
    fn clone(&self) -> Self {
        Self {
            scope: self.scope.clone(),
            context: self.context.clone(),
            selection: self.selection.clone(),
            classified: self.classified.clone(),
            selection_start: self.selection_start.clone(),
            start: self.start.clone(),
            request: self.request.clone(),
            inner: self.inner.clone(),
            output: self.output.clone(),
        }
    }
}
#[expect(clippy::large_enum_variant, reason = "keep the ordinary slot inline without another per-request allocation")]
enum NativeSlotExtension<E: Send + 'static, R: Send + Sync + 'static> {
    Ordinary(NativeSlot<E, R>),
    Prepared(crate::prepared_native_slot::NativeSlotRef<E, R>),
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for NativeSlotExtension<E, R> {
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(slot) => Self::Ordinary(slot.clone()),
            Self::Prepared(slot) => Self::Prepared(slot.clone()),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> std::ops::Deref for NativeSlotExtension<E, R> {
    type Target = NativeSlot<E, R>;
    fn deref(&self) -> &Self::Target {
        match self { Self::Ordinary(slot) => slot, Self::Prepared(slot) => slot }
    }
}
impl<E, R> Boundary<E, R>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    /// Retain one outer owner before native route selection and across every response body.
    async fn dispatch_native(
        self: Arc<Self>,
        native: http::Request<Body>,
        router: axum::Router,
        admission: Option<PreparedRequestAdmission<E, R>>,
    ) -> NativeResponse<E, R> {
        let (queue_permit, retention, terminal, cleanup, native_slot, channels, body, error_bodies, observers, request_storage, endpoint_bytes, response_headers) = match admission {
            Some(admission) => (admission.queue_permit, Some(admission.retention), admission.terminal, admission.cleanup, Some(admission.native_slot), Some(admission.channels), admission.body, admission.error_bodies, Some(admission.observers), admission.request_storage, admission.endpoint_bytes, admission.response_headers),
            None => (None, None, None, [None, None], None, None, None, [None, None], None, None, None, None),
        };
        let cancellation = self.context.cancellation().child();
        let (routing, request) = match (request_storage, &self.request_storage) {
            (Some((reservation, snapshot, initial)), Some(_)) => {
                let routing = match crate::RoutingRequest::from_reserved_native(native, cancellation, reservation, initial) { Ok(value) => value, Err(_) => return (terminal_response(StatusCode::SERVICE_UNAVAILABLE), None) };
                let request = match routing.request_with_vacancy(snapshot) { Ok(value) => value, Err(_) => return (terminal_response(StatusCode::SERVICE_UNAVAILABLE), None) };
                (routing, request)
            }
            (None, None) => { let routing = crate::RoutingRequest::from_native(native, cancellation); let request = routing.request(); (routing, request) },
            _ => return (terminal_response(StatusCode::SERVICE_UNAVAILABLE), None),
        };
        if let Some(leases) = endpoint_bytes
            && let Err(error) = request.install_shared_endpoint_bytes(leases)
        {
            return (admission_refusal(error, self.overload_reply), None);
        }
        request.insert_extension(response_headers.unwrap_or_default());
        let mut disconnected = RequestDisconnect {
            request: request.clone(),
            armed: true,
        };
        let (output, receive) = match channels {
            Some(channels) => {
                let control = channels.control;
                if let Some(permit) = queue_permit {
                    control.install_queue_permit(permit);
                }
                let receive = NativeResponseReceiver::Prepared { receiver: channels.response_receiver, _owner: control.clone() };
                let state = NativeOutputState {
                    observers, outer_vacancy: body, error_bodies, outer_wait: None, native_slot, sender: Some(NativeResponseSender::Prepared(channels.response_sender)), retention, terminal,
                    installed_terminal: None, origin: None, admission: None, execution: None, route_capacity: None, cleanup,
                };
                *control.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = state;
                control.add_producer();
                (NativeOutput::Prepared(control), receive)
            }
            None => {
                let (send, receive) = oneshot::channel();
                let state = NativeOutputState {
            observers, outer_vacancy: body, error_bodies, outer_wait: None,
            native_slot,
            sender: Some(NativeResponseSender::Ordinary(send)),
            retention,
            terminal,
            installed_terminal: None,
            origin: None,
            admission: None,
            execution: None,
            route_capacity: None,
            cleanup,
                };
                (NativeOutput::Ordinary(Arc::new(Mutex::new(state))), NativeResponseReceiver::Ordinary(receive))
            }
        };
        let owner = self.clone();
        let routing = match &output {
            NativeOutput::Ordinary(_) => NativeRoutingOwner::Ordinary(Arc::new(Mutex::new(Some(routing)))),
            NativeOutput::Prepared(control) => {
                if let Err(routing) = control.install_routing(routing) {
                    drop(routing);
                    return (terminal_response(StatusCode::SERVICE_UNAVAILABLE), None);
                }
                NativeRoutingOwner::Prepared(control.clone())
            }
        };
        let fork_owner = match &output {
            NativeOutput::Prepared(control) => Some((
                control.clone(),
                control.retains_queue_permit().then(|| output.clone()),
            )),
            NativeOutput::Ordinary(_) => None,
        };
        let task = effect_scope! {
        body: move |scope| {
                    let owner = owner.clone();
                    let output = output.clone();
                    let routing = routing.clone();
                    let router = router.clone();
                    crate::endpoint::native_once(move |env: Arc<R>, context| async move {
                        let routing = routing.take();
                        let Some(routing) = routing else {
                            return Exit::Die(Defect::new("native routing owner already consumed"));
                        };
                        let cancellation = routing.cancellation().clone();
                        let interrupted = context.cancellation().clone();
                        let running =
                            owner.execute_native(routing, router, scope, output, context, env.clone());
                        pin!(running);
                        native_io(env.tokio().clone(),async {select! {exit=&mut running=>exit,()=cancellation.cancelled()=>{let _=interrupted.request();running.await}}}).await
                    })
                },
        };
        let fork = match fork_owner {
            Some((control, queue_guard)) => self.scope.fork_unit_with_task_guard(
                effect_uninterruptible! { program: task, }, control, queue_guard,
            ),
            None => self.scope.fork(effect_uninterruptible! { program: task, }),
        };
        match self
            .context
            // Native body completion can precede request cleanup. Retain this admitted
            // owner through host-scope closure; explicit disconnect cancellation still
            // reaches its nested work through the existing request token.
            .run(
                fork,
                self.environment.clone(),
            )
            .await
        {
            Exit::Success(_) => match receive.await {
                Ok(response) => {
                    disconnected.armed = false;
                    response
                }
                Err(_) => (terminal_response(StatusCode::INTERNAL_SERVER_ERROR), None),
            },
            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
                (terminal_response(StatusCode::SERVICE_UNAVAILABLE), None)
            }
        }
    }
    /// Run native transforms once before matching; select R after native auth/culture,
    /// then let outer decorators own routing/body work and typed cleanup settlement.
    async fn execute_native(
        self: Arc<Self>,
        mut routing: crate::RoutingRequest,
        router: axum::Router,
        scope: Scope<E>,
        output: NativeOutput<E, R>,
        context: NativeContext,
        root: Arc<R>,
    ) -> Exit<(), E> {
        let mut request = match &self.request_storage {
            Some(_) => match routing.take_initial_request() { Ok(value) => value, Err(error) => return Exit::Fail((self.policy.native_error)(ServerError::Memory(error))) },
            None => routing.request(),
        };
        let mut routing = Some(routing);
        let mut failure = None;
        let mut capacity = None;
        let mut ready = None;
        for stage in &self.routing_hooks {
            let Some(current) = routing.take() else {
                return Exit::Die(Defect::new("native routing owner missing"));
            };
            let snapshot = if self.request_storage.is_some() {
                match current.reserve_snapshot() {
                    Ok(snapshot) => Some(snapshot),
                    Err(error) if is_admission_capacity(error) => { ready = Some(admission_refusal(error, self.overload_reply)); capacity = Some(error); break; }
                    Err(error) => { failure = Some(Exit::Fail((self.policy.native_error)(ServerError::Memory(error)))); break; }
                }
            } else { None };
            let outcome = match stage {
                RoutingStage::Ordinary(stage) => match context
                    .run(stage(current, scope.clone()), root.clone()).await
                {
                    Exit::Success(next) => Exit::Success(Ok(next)),
                    Exit::Fail(error) => Exit::Fail(error),
                    Exit::Die(defect) => Exit::Die(defect),
                    Exit::Interrupted => Exit::Interrupted,
                    Exit::FinalizationFailed(failed) => Exit::FinalizationFailed(failed),
                },
                RoutingStage::Checked { updated_head, stage } => {
                    let updated = if *updated_head {
                        let vacancy = if self.request_storage.is_some() {
                            match current.reserve_snapshot() {
                                Ok(vacancy) => Some(vacancy),
                                Err(error) if is_admission_capacity(error) => {
                                    ready = Some(admission_refusal(error, self.overload_reply));
                                    capacity = Some(error);
                                    break;
                                }
                                Err(error) => {
                                    failure = Some(Exit::Fail((self.policy.native_error)(ServerError::Memory(error))));
                                    break;
                                }
                            }
                        } else { None };
                        Some(crate::RoutingSnapshotReservation(vacancy))
                    } else { None };
                    context.run(stage(current, request.clone(), updated, scope.clone()), root.clone()).await
                }
            };
            match outcome {
                Exit::Success(Ok(next)) => {
                    request = match snapshot {
                        Some(snapshot) => match next.request_with_vacancy(snapshot) { Ok(value) => value, Err(error) if is_admission_capacity(error) => { ready = Some(admission_refusal(error, self.overload_reply)); capacity = Some(error); break; }, Err(error) => { failure = Some(Exit::Fail((self.policy.native_error)(ServerError::Memory(error)))); break; } },
                        None => next.request(),
                    };
                    routing = Some(next);
                }
                Exit::Success(Err(error)) if is_admission_capacity(error) => {
                    ready = Some(admission_refusal(error, self.overload_reply));
                    capacity = Some(error);
                    break;
                }
                Exit::Success(Err(error)) => {
                    failure = Some(Exit::Fail((self.policy.native_error)(ServerError::Memory(error))));
                    break;
                }
                Exit::Fail(error) => {
                    failure = Some(Exit::Fail(error));
                    break;
                }
                Exit::Die(defect) => {
                    failure = Some(Exit::Die(defect));
                    break;
                }
                Exit::Interrupted => {
                    failure = Some(Exit::Interrupted);
                    break;
                }
                Exit::FinalizationFailed(failed) => {
                    failure = Some(Exit::FinalizationFailed(failed));
                    break;
                }
            }
        }
        // The selected native-router snapshot is reserved before publishing
        // the body signal or constructing slot/watch owners. A shortage can
        // therefore take the existing 503 completion path without those owners.
        let native_snapshot = if self.request_storage.is_some() && failure.is_none() && capacity.is_none() {
            match routing.as_ref().ok_or(effect::memory::MemoryError::ConstructionUnbound).and_then(crate::RoutingRequest::reserve_snapshot) {
                Ok(snapshot) => Some(snapshot),
                Err(error) if is_admission_capacity(error) => { ready = Some(admission_refusal(error, self.overload_reply)); capacity = Some(error); None },
                Err(error) => { failure = Some(Exit::Fail((self.policy.native_error)(ServerError::Memory(error)))); None },
            }
        } else { None };
        let outer_vacancy = output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).outer_vacancy.take();
        let outer_signal = match (outer_vacancy, &self.body_pool) {
            (Some(vacancy), Some(pool)) => { let (signal, wait) = vacancy.publish(request.cancellation().clone()); output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).outer_wait = Some(BodySignalWait::Prepared(wait)); let Some(cleanup) = &self.cleanup else { return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()) }; request.insert_extension(crate::body::BodyPoolAccess::<E>(pool.clone(), cleanup.clone())); BodySignal::Prepared(signal) },
            (None, None) => BodySignal::ordinary(request.cancellation().clone()),
            (Some(_), None) | (None, Some(_)) => return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()),
        };
        request.insert_extension(OuterDelivery(outer_signal));
        if capacity.is_some() {
            request.insert_extension(RouteKind::Native);
            return self.complete_native_selection(
                NativeResponseState { request, ready, routed: None, failure, capacity },
                (scope, context, root), output, None, false,
            ).await;
        }
        let (selection, selected, classified, mut classification) = match &output {
            NativeOutput::Prepared(owner) if owner.watches_configured() => {
                let Some(watches) = owner.take_watches() else { return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()); };
                let (classification, selection) = owner.watch_producers();
                (SelectionSender::Prepared(selection), SelectionReceiver::Prepared { receiver: watches.selection, _owner: owner.clone() }, ClassificationSender::Prepared(classification), ClassificationReceiver::Prepared { receiver: watches.classification, _owner: owner.clone() })
            }
            NativeOutput::Ordinary(_) | NativeOutput::Prepared(_) => {
                let (selection, selected) = watch::channel(None::<Arc<NativeSelection<R>>>);
                let (classified, classification) = watch::channel(None::<Request>);
                (SelectionSender::Ordinary(selection), SelectionReceiver::Ordinary(selected), ClassificationSender::Ordinary(classified), ClassificationReceiver::Ordinary(classification))
            }
        };
        let (start, gate, selection_start, selection_gate) = match &output {
            NativeOutput::Ordinary(_) => {
                let (start, gate) = oneshot::channel();
                let (selection_start, selection_gate) = oneshot::channel();
                (NativeContextSender::Ordinary(start), NativeContextGate::Ordinary(Arc::new(Mutex::new(Some(gate)))), NativeContextSender::Ordinary(selection_start), NativeContextGate::Ordinary(Arc::new(Mutex::new(Some(selection_gate)))))
            }
            NativeOutput::Prepared(owner) => {
                let Some(gates) = owner.take_gates() else { return Exit::Die(Defect::new("prepared context gates missing")); };
                (NativeContextSender::Prepared(gates.start, owner.clone()), NativeContextGate::Prepared(owner.clone(), false), NativeContextSender::Prepared(gates.selection, owner.clone()), NativeContextGate::Prepared(owner.clone(), true))
            }
        };
        let request_shadow = match &output {
            NativeOutput::Ordinary(_) => NativeRequestShadow::Ordinary(Arc::new(Mutex::new(request.clone()))),
            NativeOutput::Prepared(control) => match control.request_shadow(request.clone()) {
                Ok(shadow) => NativeRequestShadow::Prepared(shadow),
                Err(request) => { drop(request); return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()); }
            },
        };
        let slot = NativeSlot {
            scope: scope.clone(),
            context: context.clone(),
            selection,
            classified,
            selection_start: selection_gate,
            start: gate,
            request: request_shadow,
            inner: match &output {
                NativeOutput::Ordinary(_) => NativeInnerDriver::Ordinary(Arc::new(Mutex::new(None))),
                NativeOutput::Prepared(control) => NativeInnerDriver::Prepared(control.clone()),
            },
            output: output.clone(),
        };
        let vacancy = output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).native_slot.take();
        let slot = match (&output, vacancy) {
            (NativeOutput::Prepared(_), Some(vacancy)) => NativeSlotExtension::Prepared(vacancy.publish(slot)),
            (NativeOutput::Ordinary(_), None) => NativeSlotExtension::Ordinary(slot),
            (NativeOutput::Prepared(_), None) | (NativeOutput::Ordinary(_), Some(_)) => return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()),
        };
        let mut routed = None;
        let mut matched = false;
        if failure.is_none() && capacity.is_none() {
            let Some(routing) = routing.take() else {
                return Exit::Die(Defect::new("native routing owner missing"));
            };
            let native = match (&self.request_storage, native_snapshot) {
                (Some(_), Some(snapshot)) => match routing.into_native_with_vacancy(snapshot) {
                    Ok(value) => Some(value),
                    Err((error, routing)) if is_admission_capacity(error) => { drop(routing); ready = Some(admission_refusal(error, self.overload_reply)); capacity = Some(error); None },
                    Err((error, routing)) => { drop(routing); failure = Some(Exit::Fail((self.policy.native_error)(ServerError::Memory(error)))); None },
                },
                (None, None) => Some(routing.into_native()),
                (Some(_), None) | (None, Some(_)) => return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()),
            };
            if let Some(mut native) = native {
            native.extensions_mut().insert(slot.clone());
            let program = crate::endpoint::native_once(move |_, _| async move {
                match tower::ServiceExt::oneshot(router, native).await {
                    Ok(response) => Exit::Success(response),
                    Err(error) => match error {},
                }
            });
            match context.run(scope.fork(program), root.clone()).await {
                Exit::Success(fiber) => {
                    let observing = context.run(fiber.join(), root.clone());
                    pin!(observing);
                    loop {
                        let classified = match classification.snapshot() {
                            Ok(classified) => classified,
                            Err(error) => { failure = Some(Exit::Fail((self.policy.native_error)(ServerError::Memory(error)))); break; }
                        };
                        if let Some(classified) = classified {
                            request = classified;
                            matched = true;
                            break;
                        }
                        select! {
                            response=&mut observing=>{match response {Exit::Success(response)=>ready=Some(response),Exit::Fail(error)=>failure=Some(Exit::Fail(error)),Exit::Die(defect)=>failure=Some(Exit::Die(defect)),Exit::Interrupted=>failure=Some(Exit::Interrupted),Exit::FinalizationFailed(failed)=>failure=Some(Exit::FinalizationFailed(failed))};break;},
                            changed=classification.changed()=>{if changed.is_err(){failure=Some(Exit::Die(Defect::new("native route classification closed")));break;}}
                        }
                    }
                    if ready.is_none() && failure.is_none() {
                        routed = Some(fiber);
                    }
                }
                Exit::Fail(error) => failure = Some(Exit::Fail(error)),
                Exit::Die(defect) => failure = Some(Exit::Die(defect)),
                Exit::Interrupted => failure = Some(Exit::Interrupted),
                Exit::FinalizationFailed(failed) => {
                    failure = Some(Exit::FinalizationFailed(failed))
                }
            }
            }
        }
        if !matched {
            request.insert_extension(RouteKind::Native);
            if ready.is_some() {
                let refused = output.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                    .route_capacity.take();
                capacity = capacity.or(refused);
            }
        }
        let owner = self.clone();
        let decorated_request = request.clone();
        let mut selection_program =
            crate::endpoint::native_once(move |root: Arc<R>, decorated| async move {
                owner
                    .complete_native_selection(
                        NativeResponseState {
                            request,
                            ready,
                            routed,
                            failure,
                            capacity,
                        },
                        (scope, decorated, root),
                        output,
                        Some((slot, NativeSelectionGates {
                            selected,
                            start,
                            selection_start,
                        })),
                        matched,
                    )
                    .await
            });
        if capacity.is_none() {
            for decorate in self.selection_decorators.iter().rev() {
                selection_program = decorate(decorated_request.clone(), selection_program);
            }
        }
        context.run(selection_program, root).await
    }
    /// Start matched authentication under the outer context, then retain that context across
    /// selected-R execution and the uniquely owned terminal/cleanup outcome.
    async fn complete_native_selection(
        self: Arc<Self>,
        state: NativeResponseState<E>,
        execution: (Scope<E>, NativeContext, Arc<R>),
        output: NativeOutput<E, R>,
        bundle: Option<(NativeSlotExtension<E, R>, NativeSelectionGates<E, R>)>,
        matched: bool,
    ) -> Exit<(), E> {
        let NativeResponseState {
            mut request,
            mut ready,
            mut routed,
            mut failure,
            capacity,
        } = state;
        let (scope, context, root) = execution;
        let (slot, mut selected, start, selection_start) = match bundle {
            Some((slot, NativeSelectionGates { selected, start, selection_start })) => (Some(slot), Some(selected), Some(start), Some(selection_start)),
            None if !matched && capacity.is_some() => (None, None, None, None),
            None => return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()),
        };
        let mut chosen = None;
        if matched {
            let Some(selection_start) = selection_start else { return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()); };
            let Some(selected) = selected.as_mut() else { return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()); };
            selection_start.send(context.clone());
            if let Some(fiber) = routed.take() {
                let observing = context.run(fiber.join(), root.clone());
                pin!(observing);
                loop {
                    let selection = match selected.snapshot() {
                        Ok(selection) => selection,
                        Err(error) => { failure = Some(Exit::Fail((self.policy.native_error)(ServerError::Memory(error)))); break; }
                    };
                    if selection.is_some() {
                        chosen = selection;
                        break;
                    }
                    select! {
                        response=&mut observing=>{match response {Exit::Success(response)=>ready=Some(response),Exit::Fail(error)=>failure=Some(Exit::Fail(error)),Exit::Die(defect)=>failure=Some(Exit::Die(defect)),Exit::Interrupted=>failure=Some(Exit::Interrupted),Exit::FinalizationFailed(failed)=>failure=Some(Exit::FinalizationFailed(failed))};break;},
                        changed=selected.changed()=>{if changed.is_err(){failure=Some(Exit::Die(Defect::new("native environment selection closed")));break;}}
                    }
                }
                if ready.is_none() && failure.is_none() {
                    routed = Some(fiber);
                }
            } else {
                failure = Some(Exit::Die(Defect::new(
                    "matched native request driver missing",
                )));
            }
        }
        let (environment, selected_request) = if capacity.is_some() {
            (root.clone(), false)
        } else if let Some((chosen_request, environment, selected)) = chosen {
            request = chosen_request;
            (environment, selected)
        } else {
            if failure.is_none() {
                for hook in &self.hooks {
                    let exit = context.run(hook(request.clone()), root.clone()).await;
                    if !matches!(exit, Exit::Success(())) {
                        failure = Some(exit);
                        break;
                    }
                }
            }
            if failure.is_none() {
                match context
                    .run((self.binding.resolve)(request.clone()), root.clone())
                    .await
                {
                    Exit::Success(environment) => (environment, true),
                    Exit::Fail(error) => {
                        failure = Some(Exit::Fail(error));
                        (root.clone(), false)
                    }
                    Exit::Die(defect) => {
                        failure = Some(Exit::Die(defect));
                        (root.clone(), false)
                    }
                    Exit::Interrupted => {
                        failure = Some(Exit::Interrupted);
                        (root.clone(), false)
                    }
                    Exit::FinalizationFailed(failed) => {
                        failure = Some(Exit::FinalizationFailed(failed));
                        (root.clone(), false)
                    }
                }
            } else {
                (root.clone(), false)
            }
        };
        let body_state = match &output {
            NativeOutput::Ordinary(_) => BodySignalState::Ordinary(Arc::new(Mutex::new(None))),
            NativeOutput::Prepared(control) => BodySignalState::PreparedOuter(control.body_state()),
        };
        let final_state = body_state.clone();
        let response_request = request.clone();
        let policy = self.policy.clone();
        let hooks = if capacity.is_some() { None } else { Some(self.response_hooks.clone()) };
        let observation_output = output.clone();
        let terminal_output = output.clone();
        let close_scope = scope.clone();
        let inner = slot.as_ref().map(|slot| slot.inner.clone());
        let response_limits = self.response_limits;
        let mut complete = crate::endpoint::native_once(
            move |environment: Arc<R>, decorated| async move {
                if let Some(start) = start { start.send(decorated.clone()); }
                let mut exit = finish_native_response(
                    NativeResponseState {
                        request: response_request.clone(),
                        ready,
                        routed,
                        failure,
                        capacity,
                    },
                    (scope, &decorated, environment.clone()),
                    output,
                    body_state,
                    policy.clone(),
                    hooks,
                    response_limits,
                )
                .await;
                if final_state.is_none()
                    && !matches!(exit, Exit::Success(()))
                {
                    response_request.with_extensions(|extensions| {
                        if let Some(delivery) = extensions.get::<OuterDelivery>() {
                            delivery.0.fail();
                        }
                    });
                }
                let abandoned = response_request.with_extensions(|extensions| {
                    extensions
                        .get::<EndpointDelivery>()
                        .is_some_and(|delivery| {
                            matches!(
                                delivery.0.outcome(),
                                BodyOutcome::Disconnected | BodyOutcome::Failed
                            )
                        })
                });
                if abandoned {
                    let _ = response_request.cancellation().request();
                }
                let endpoint = inner.as_ref().and_then(NativeInnerDriver::take);
                let cleanup = observation_output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .cleanup[0]
                    .take();
                let has_prepared_cleanup = cleanup.is_some();
                let has_prepared_observers = matches!(&observation_output, NativeOutput::Prepared(_));
                let mut prepared_completion = None;
                if let Some(error) = capacity {
                    let Some(rest) = cleanup else { return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()); };
                    let mut completion = effect::PreparedCleanupAccumulator::from_reserved(
                        Exit::Fail((policy.native_error)(ServerError::Memory(error))), rest,
                    );
                    if let Some(endpoint) = endpoint {
                        let endpoint = decorated.run(effect_uninterruptible! { program: endpoint.join(), }, environment.clone()).await;
                        completion.append_cleanup(endpoint);
                    }
                    completion.append_cleanup(exit);
                    prepared_completion = Some(completion);
                    exit = Exit::Success(());
                } else if let Some(endpoint) = endpoint {
                    let endpoint = decorated
                        .run(
                            effect_uninterruptible! { program: endpoint.join(), },
                            environment.clone(),
                        )
                        .await;
                    if let Some(rest) = cleanup {
                        let mut completion =
                            effect::PreparedCleanupAccumulator::from_reserved(endpoint, rest);
                        completion.append_cleanup(exit);
                        prepared_completion = Some(completion);
                        exit = Exit::Success(());
                    } else {
                        exit = combine_completion(endpoint, exit);
                    }
                } else if let Some(rest) = cleanup {
                    prepared_completion = Some(effect::PreparedCleanupAccumulator::from_reserved(
                        exit, rest,
                    ));
                    exit = Exit::Success(());
                }
                let closed = decorated
                    .run(
                        effect_uninterruptible! { program: effect::__private::close_scope_owned(close_scope), },
                        environment.clone(),
                    )
                    .await;
                if let Some(completion) = &mut prepared_completion {
                    completion.append_cleanup(closed);
                } else {
                    exit = combine_completion(exit, closed);
                }
                let body = final_state.outcome();
                let (exit, spare) = match prepared_completion {
                    Some(completion) => completion.into_parts(),
                    None => (exit, None),
                };
                let completion = RequestCompletion {
                    exit,
                    body,
                    committed: observation_output
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .sender
                        .is_none(),
                };
                let observations = if has_prepared_observers {
                    let batch = observation_output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).observers.take();
                    let Some(mut batch) = batch else { return observer_invariant(completion.exit, spare); };
                    if batch.capacity() != policy.observers.len() { return observer_invariant(completion.exit, spare); }
                    for observe in policy.observers.iter() {
                        let observation = observe(&environment, &response_request, &completion);
                        if let Err(observation) = batch.push(observation) {
                            drop(observation);
                            return observer_invariant(completion.exit, spare);
                        }
                    }
                    ObserverEffects::Prepared(batch)
                } else {
                    ObserverEffects::Ordinary(policy.observers.iter().map(|observe| observe(&environment, &response_request, &completion)).collect::<Vec<_>>().into_iter())
                };
                let mut exit = completion.exit;
                let mut prepared_completion = if has_prepared_cleanup {
                    match effect::PreparedCleanupAccumulator::rejoin(exit, spare) {
                        Ok(completion) => {
                            exit = Exit::Success(());
                            Some(completion)
                        }
                        Err((original, _)) => return original,
                    }
                } else {
                    None
                };
                for observation in observations {
                    let observed = decorated
                        .run(
                            effect_uninterruptible! { program: observation, },
                            environment.clone(),
                        )
                        .await;
                    if let Some(completion) = &mut prepared_completion {
                        completion.append_cleanup(observed);
                    } else {
                        exit = combine_completion(exit, observed);
                    }
                }
                match prepared_completion {
                    Some(completion) => completion.finish(),
                    None => exit,
                }
            },
        );
        if capacity.is_none() {
            for decorate in self.decorators.iter().rev() {
                complete = decorate(request.clone(), complete);
            }
        }
        let cancellation = environment.cancellation().clone();
        let request_cancel = request.cancellation().clone();
        let reactor = environment.tokio().clone();
        let running = native_io(reactor, async {
            if selected_request {
                context.run_cancellable(complete, environment).await
            } else {
                context.run(complete, environment).await
            }
        });
        pin!(running);
        let exit = select! {exit=&mut running=>exit,()=request_cancel.cancelled()=>{if selected_request{let _=cancellation.request();}running.await}};
        let terminal = terminal_output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed_terminal
            .take();
        if let Some(terminal) = terminal {
            if matches!(exit, Exit::Success(())) {
                // Request scope, producer and observers have settled. Response
                // data and tracked native owners independently retain the root.
                unsafe { terminal.release_handled() };
            } else {
                terminal.retain_terminal();
            }
        }
        exit
    }
    /// Admit managed request work before returning its native response observer.
    async fn dispatch(
        self: Arc<Self>,
        mut native: http::Request<Body>,
        select: impl FnOnce(crate::Captures) -> (crate::Captures, Result<Endpoint<E, R>, ServerError>),
        metadata: crate::Registration,
        captures: crate::Captures,
        resources: EndpointDispatchResources,
    ) -> http::Response<Body> {
        let slot = native.extensions_mut().remove::<NativeSlotExtension<E, R>>();
        let refuse = |error| route_admission_refusal(
            slot.as_ref().map(|slot| &slot.output), error, resources.on_capacity,
        );
        let prepared_inner = slot.as_ref().and_then(|slot| match &slot.output {
            NativeOutput::Prepared(control) => control.reserve_inner(),
            NativeOutput::Ordinary(_) => None,
        });
        let (output, receive, first_resources) = match prepared_inner {
            Some(Ok((producer, receiver, first_resources))) => (InnerOutput::Prepared(producer), InnerResponseReceiver::Prepared(receiver), first_resources),
            Some(Err(error)) => return refuse(error),
            None => { let (send, receive) = oneshot::channel(); (InnerOutput::Ordinary(Arc::new(Mutex::new(Some(send)))), InnerResponseReceiver::Ordinary(receive), false) }
        };
        if let (Some(slot), InnerOutput::Prepared(_)) = (&slot, &output) {
            let Some((outer, pool)) = slot.request.with(|request| request.with_extensions(|extensions| (extensions.get::<OuterDelivery>().cloned(), extensions.get::<crate::body::BodyPoolAccess<E>>().cloned()))) else { return terminal_response(StatusCode::SERVICE_UNAVAILABLE) };
            let Some(outer) = outer else { return terminal_response(StatusCode::SERVICE_UNAVAILABLE) };
            let Some(pool) = pool else { return terminal_response(StatusCode::SERVICE_UNAVAILABLE) };
            if !first_resources {
                let wait = match outer.0.admit_wait() { Ok(wait) => wait, Err(error) => return refuse(error) };
                let first = match pool.0.reserve_reply(&pool.1) { Ok(vacancy) => vacancy, Err(error) => return refuse(error) };
                let second = match pool.0.reserve_reply(&pool.1) { Ok(vacancy) => vacancy, Err(error) => return refuse(error) };
                output.install_outer_wait(wait);
                output.install_body_vacancies([Some(first), Some(second)]);
            }
        }
        let publications = if let Some(slot) = &slot {
            let classification = match slot.classified.reserve() { Ok(publication) => publication, Err(error) => return refuse(error) };
            let selection = match slot.selection.reserve() { Ok(publication) => publication, Err(error) => return refuse(error) };
            Some((classification, selection))
        } else { None };
        let cancellation = match &slot {
            Some(slot) => match slot.request.with(|request| request.cancellation().clone()) {
                Some(cancellation) => cancellation,
                None => return terminal_response(StatusCode::SERVICE_UNAVAILABLE),
            },
            None => self.context.cancellation().child(),
        };
        let request = match &self.request_storage {
            Some(pool) => match Request::try_from_native_in(native, cancellation, pool) { Ok(request) => request, Err(error) => return refuse(error) },
            None => Request::from_native(native, cancellation),
        };
        if let Some(leases) = resources.bytes {
            request.install_endpoint_bytes(leases);
        }
        if let Some(slot) = &slot
            && let Err(request) = slot.request.replace(request.clone()) {
                drop(request);
                return terminal_response(StatusCode::SERVICE_UNAVAILABLE);
            }
        let mut disconnected = RequestDisconnect {
            request: request.clone(),
            armed: true,
        };
        let (captures, selected) = select(captures);
        let endpoint = match selected {
            Ok(endpoint) => endpoint,
            Err(error) => {
                let map = self.policy.native_error.clone();
                endpoint! {
                    effect: effect_try_sync! {
                        once;
                        body: { let result = Err(map(error)); move || result },
                    },
                }
            }
        };
        request.insert_extension(RouteKind::Endpoint);
        request.insert_extension(metadata);
        request.insert_extension(crate::route::NativeCaptures(captures));
        if !request.with_extensions(|extensions| extensions.get::<ResponseHeaders>().is_some()) {
            request.insert_extension(ResponseHeaders::default());
        }
        let owner = self.clone();
        let request_for_work = request.clone();
        let work_slot = slot.clone();
        let program = crate::endpoint::native_once(move |env: Arc<R>, context| async move {
            let disconnected = request_for_work.cancellation().clone();
            let interrupted = context.cancellation().clone();
            native_io(env.tokio().clone(), async move {
                let executing = owner.execute(request_for_work, endpoint, output, context,work_slot,publications); pin!(executing);
                select! { exit = &mut executing => exit, () = disconnected.cancelled() => { let _ = interrupted.request(); executing.await } }
            }).await
        });
        let context = slot
            .as_ref()
            .map(|slot| slot.context.clone())
            .unwrap_or_else(|| self.context.clone());
        let scope = slot
            .as_ref()
            .map(|slot| slot.scope.clone())
            .unwrap_or_else(|| self.scope.clone());
        let admitted = context
            .run(scope.fork(program), self.environment.clone())
            .await;
        match admitted {
            Exit::Success(fiber) => {
                if let Some(slot) = &slot {
                    slot.inner.replace(fiber);
                }
            }
            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
                return terminal_response(StatusCode::SERVICE_UNAVAILABLE);
            }
        }
        match receive.await {
            Ok(response) => {
                disconnected.armed = false;
                response
            }
            Err(_) => terminal_response(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
    /// Resolve selected R, await one request-owned scope, then invoke terminal observers once.
    async fn execute(
        &self,
        request: Request,
        endpoint: Endpoint<E, R>,
        output: InnerOutput<E, R>,
        context: NativeContext,
        slot: Option<NativeSlotExtension<E, R>>,
        publications: Option<(ClassificationPublication<E, R>, SelectionPublication<E, R>)>,
    ) -> Exit<(), E> {
        // Classification is known before starting the root-R outer decorator. The endpoint
        // waits for its context so authentication SQL and resolution inherit request tracing.
        let (classification_publication, mut selection_publication) = match publications {
            Some((classification, selection)) => (Some(classification), Some(selection)),
            None => (None, None),
        };
        let context = if let Some(slot) = &slot {
            let Some(classification) = classification_publication else { return Exit::Die(Defect::new("native classification publication missing")); };
            classification.publish(request.clone());
            let gate = slot.selection_start.take();
            match gate {
                Some(gate) => match gate.await {
                    Ok(context) => context,
                    Err(_) => return Exit::Interrupted,
                },
                None => return Exit::Die(Defect::new("native request selection already started")),
            }
        } else {
            context
        };
        // Pre-selection failures enter the same request policy/observer path using the
        // original services; they never recursively invoke a failing error renderer.
        let mut selection_failure = None;
        for hook in &self.hooks {
            let outcome = context
                .run(hook(request.clone()), self.environment.clone())
                .await;
            if !matches!(outcome, Exit::Success(())) {
                selection_failure = Some(outcome);
                break;
            }
        }
        let (selected, selected_request) = if selection_failure.is_some() {
            (self.environment.clone(), false)
        } else {
            match context
                .run(
                    (self.binding.resolve)(request.clone()),
                    self.environment.clone(),
                )
                .await
            {
                Exit::Success(environment) => (environment, true),
                Exit::Fail(error) => {
                    selection_failure = Some(Exit::Fail(error));
                    (self.environment.clone(), false)
                }
                Exit::Die(defect) => {
                    selection_failure = Some(Exit::Die(defect));
                    (self.environment.clone(), false)
                }
                Exit::Interrupted => {
                    selection_failure = Some(Exit::Interrupted);
                    (self.environment.clone(), false)
                }
                Exit::FinalizationFailed(failure) => {
                    selection_failure = Some(Exit::FinalizationFailed(failure));
                    (self.environment.clone(), false)
                }
            }
        };
        let endpoint = if let Some(failure) = selection_failure {
            endpoint! {
                effect: crate::endpoint::native_once(move |_, _| async move {
                match failure {
                    Exit::Success(()) => Exit::Die(Defect::new("invalid selection failure")),
                    Exit::Fail(error) => Exit::Fail(error),
                    Exit::Die(defect) => Exit::Die(defect),
                    Exit::Interrupted => Exit::Interrupted,
                    Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
                }
            }), }
        } else {
            endpoint
        };
        let endpoint_body = if selected_request
            && let Some(slot) = &slot
            && slot.output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).admission.is_some()
        {
            NativeEndpointBody::Prepared(slot.output.clone())
        } else {
            NativeEndpointBody::Registered(endpoint.body.clone())
        };
        let context = if let Some(slot) = &slot {
            let Some(publication) = selection_publication.take() else { return Exit::Die(Defect::new("native selection publication missing")); };
            publication.publish(NativeSelection {
                request: request.clone(),
                environment: selected.clone(),
                selected: selected_request,
            });
            let start = slot.start.take();
            match start {
                Some(start) => match start.await {
                    Ok(context) => context,
                    Err(_) => return Exit::Interrupted,
                },
                None => {
                    return Exit::Die(Defect::new("native request continuation already started"));
                }
            }
        } else {
            context
        };
        let cancellation = selected.cancellation().clone();
        let request_cancellation = request.cancellation().clone();
        let on_error = endpoint
            .on_error
            .clone()
            .unwrap_or_else(|| self.policy.on_error.clone());
        let policy = self.policy.clone();
        let body_state = match &output {
            InnerOutput::Ordinary(_) => BodySignalState::Ordinary(Arc::new(Mutex::new(None))),
            InnerOutput::Prepared(producer) => BodySignalState::PreparedInner(producer.body_state()),
        };
        let final_state = body_state.clone();
        let boundary_request = request.clone();
        let middleware = self.middleware.clone();
        let endpoint_middleware = endpoint.pipeline;
        let native_output = output.clone();
        let work = effect_scope! {
        body: move |scope| {
                    let request = boundary_request.clone();
                    let output = native_output.clone();
                    let policy = policy.clone();
                    let state = body_state.clone();
                    let response_scope = scope.clone();
                    let response_request = request.clone();
                    let response_output = output.clone();
                    let response_policy = policy.clone();
                    let response_state = state.clone();
                    let continuation = match reply_continuation(ReplyDelivery {
                        request: response_request, scope: response_scope, policy: response_policy,
                        output: response_output, state: response_state,
                    }) {
                        Ok(continuation) => continuation,
                        Err(error) => {
                            let error = (policy.native_error)(ServerError::Memory(error));
                            return effect_try_sync! { once; body: { let result = Err(error); move || result }, on_error: ::core::convert::identity, };
                        }
                    };
                    request.insert_extension(continuation.downgrade());
                    // A downstream endpoint scope closes before outer after/on-exit stages.
                    // Its reply still awaits native delivery within this same scope.
                    let endpoint_request = request.clone();
                    let recovery_output = output.clone();
                    let endpoint_body = endpoint_body.clone();
                    let input_error = policy.native_error.clone();
                    let pipeline_error = policy.native_error.clone();
                    let pipeline_run = match &output {
                        InnerOutput::Ordinary(_) => None,
                        InnerOutput::Prepared(producer) => producer.take_pipeline(),
                    };
                    let reply = effect_scope! {
        body: move |endpoint_scope| {
                        let delivery_scope = endpoint_scope.clone();
                        let continuation = match reply_continuation(ReplyDelivery {
                            request: endpoint_request.clone(), scope: delivery_scope,
                            policy: policy.clone(), output: output.clone(), state: state.clone(),
                        }) {
                            Ok(continuation) => continuation,
                            Err(error) => {
                                let error = (policy.native_error)(ServerError::Memory(error));
                                return effect_try_sync! { once; body: { let result = Err(error); move || result }, on_error: ::core::convert::identity, };
                            }
                        };
                        let accepts = endpoint_request.with_extensions(|extensions| {
                            extensions
                                .get::<crate::Registration>()
                                .map(|metadata| metadata.accepts.clone())
                                .unwrap_or_default()
                        });
                        if !accepts.is_empty() {
                            let actual = endpoint_request.singleton_header::<headers::ContentType>();
                            let checked = match actual {
                                Ok(Some(actual)) => HeaderValue::from_str(&actual.to_string())
                                    .ok()
                                    .is_some_and(|actual| {
                                        accepts
                                            .iter()
                                            .any(|expected| media_matches(expected, &actual))
                                    }),
                                Ok(None) => false,
                                Err(error) => return effect_try_sync! { once; body: { let __dsl_result = Err(input_error(error)); move || __dsl_result }, on_error: ::core::convert::identity, },
                            };
                            if !checked {
                                return effect_try_sync! {
                                    once; body: {
                                    let __dsl_result = Err(input_error(ServerError::Invalid(
                                    "request content type not registered in accepts".into(),
                                ))); move || __dsl_result }, on_error: ::core::convert::identity, };
                            }
                        }
                        effect! {
         bind;
         program: endpoint_body.program(endpoint_scope),
         body: move |reply| continuation.invoke(reply),
         }
                    },
        };
                    let on_error = on_error.clone();
                    let program = match middleware.apply(&endpoint_middleware, request, reply, pipeline_run) {
                        Ok(program) => program,
                        Err(error) => {
                            let error = pipeline_error(ServerError::Memory(error));
                            effect_try_sync! { once; body: { let result = Err(error); move || result }, on_error: ::core::convert::identity, }
                        }
                    };
                    effect_recover! {
                        program: program, recover: move |error| {
                        if recovery_output.is_none()
                        {
                            return effect_try_sync! { once; body: { let __dsl_result = Err(error); move || __dsl_result }, on_error: ::core::convert::identity, };
                        }
                        let continuation = continuation.clone();
                        effect! {
         bind;
         program: on_error(error),
         body: move |reply| continuation.invoke(reply),
         }
                    }, }
                },
        };
        let policy = self.policy.clone();
        let observation_request = request.clone();
        let outer_owned = slot.is_some();
        let completion_output = output.clone();
        let mut complete =
            crate::endpoint::native_once(move |selected: Arc<R>, context| async move {
                let exit = context.run(work, selected.clone()).await;
                if outer_owned {
                    return exit;
                }
                let body = final_state.outcome();
                let committed = completion_output.is_none();
                let completion = RequestCompletion {
                    exit,
                    body,
                    committed,
                };
                let observations = policy
                    .observers
                    .iter()
                    .map(|observe| observe(&selected, &observation_request, &completion))
                    .collect::<Vec<_>>();
                let mut exit = completion.exit;
                for observation in observations {
                    let observed = context
                        .run(
                            effect_uninterruptible! { program: observation, },
                            selected.clone(),
                        )
                        .await;
                    exit = combine_completion(exit, observed);
                }
                exit
            });
        if !outer_owned {
            for decorate in self.decorators.iter().rev() {
                complete = decorate(request.clone(), complete);
            }
        }
        let reactor = selected.tokio().clone();
        let run = native_io(reactor, async {
            if selected_request {
                context.run_cancellable(complete, selected).await
            } else {
                context.run(complete, selected).await
            }
        });
        pin!(run);
        select! { exit = &mut run => exit, () = request_cancellation.cancelled() => { if selected_request { let _ = cancellation.request(); } run.await } }
    }
}
/// Prepare one typed early/native error without recursively rendering a handler failure.
async fn native_error_reply<E: Send + 'static, R: Send + Sync + 'static>(
    error: E,
    request: Request,
    scope: Scope<E>,
    policy: &ServerPolicy<E, R>,
    context: &NativeContext,
    env: Arc<R>,
    body: (Option<crate::prepared_body_completion::ReplyBodyVacancy<E>>, bool),
) -> Exit<crate::reply::PreparedReply<E, R>, E> {
    let (body, body_required) = body;
    let body_admission = match body {
        Some(body) => Some(body.publish(request.cancellation().clone())),
        None if body_required => return Exit::Fail(error),
        None => None,
    };
    match context
        .run(
            effect_uninterruptible! { program: (policy.on_error)(error), },
            env.clone(),
        )
        .await
    {
        Exit::Success(reply) => {
            context
                .run(
                    effect_uninterruptible! {
                    program: reply
                    .prepare(request, scope, policy.body, policy.native_error.clone(), body_admission), },
                    env,
                )
                .await
        }
        Exit::Fail(error) => Exit::Fail(error),
        Exit::Die(defect) => Exit::Die(defect),
        Exit::Interrupted => Exit::Interrupted,
        Exit::FinalizationFailed(failed) => Exit::FinalizationFailed(failed),
    }
}
/// Decorate/commit one native response, then await its actual body and any owned producer.
async fn finish_native_response<E: Send + 'static, R: Send + Sync + 'static>(
    state: NativeResponseState<E>,
    execution: (Scope<E>, &NativeContext, Arc<R>),
    output: NativeOutput<E, R>,
    body_state: BodySignalState<E, R>,
    policy: ServerPolicy<E, R>,
    hooks: Option<Arc<[ResponseHook<E, R>]>>,
    response_limits: Option<crate::PreparedResponseLimits>,
) -> Exit<(), E> {
    let unused_error_bodies = std::mem::DropGuard::new(&output, |output| {
        let vacancies = { let mut state = output.lock().unwrap_or_else(std::sync::PoisonError::into_inner); std::mem::take(&mut state.error_bodies) };
        drop(vacancies);
    });
    let NativeResponseState {
        request,
        ready,
        routed,
        failure,
        capacity,
    } = state;
    let (scope, context, env) = execution;
    let response = match failure {
        Some(Exit::Fail(error)) => Exit::Fail(error),
        Some(Exit::Die(defect)) => return Exit::Die(defect),
        Some(Exit::Interrupted) => return Exit::Interrupted,
        Some(Exit::FinalizationFailed(failed)) => return Exit::FinalizationFailed(failed),
        Some(Exit::Success(())) => return Exit::Die(Defect::new("invalid native failure")),
        None => match ready {
            Some(response) => Exit::Success(response),
            None => match routed {
                Some(routed) => context.run(routed.join(), env.clone()).await,
                None => return Exit::Die(Defect::new("native response owner missing")),
            },
        },
    };
    let (mut response, mut producer) = match response {
        Exit::Success(response) => (response, None),
        Exit::Fail(error) => {
            let body = output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).error_bodies.first_mut().and_then(Option::take);
            match native_error_reply(
            error,
            request.clone(),
            scope.clone(),
            &policy,
            context,
            env.clone(),
            (body, matches!(&output, NativeOutput::Prepared(control) if control.watches_configured())),
        )
        .await
        {
            Exit::Success(prepared) => (prepared.response, Some(prepared.finish)),
            Exit::Fail(error) => return Exit::Fail(error),
            Exit::Die(defect) => return Exit::Die(defect),
            Exit::Interrupted => return Exit::Interrupted,
            Exit::FinalizationFailed(failed) => return Exit::FinalizationFailed(failed),
            }
        },
        Exit::Die(defect) => return Exit::Die(defect),
        Exit::Interrupted => return Exit::Interrupted,
        Exit::FinalizationFailed(failed) => return Exit::FinalizationFailed(failed),
    };
    if capacity.is_none() {
        request.with_extensions(|extensions| {
            if let Some(headers) = extensions.get::<ResponseHeaders>() {
                for (name, value) in headers
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                {
                    response.headers_mut().insert(name.clone(), value.clone());
                }
            }
        });
    }
    for hook in hooks.as_deref().unwrap_or(&[]) {
        if capacity.is_some() { break; }
        match context
            .run(hook(request.clone(), response), env.clone())
            .await
        {
            Exit::Success(next) => response = next,
            Exit::Fail(error) => {
                let body = output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).error_bodies.get_mut(1).and_then(Option::take);
                match native_error_reply(
                    error,
                    request.clone(),
                    scope.clone(),
                    &policy,
                    context,
                    env.clone(),
                    (body, matches!(&output, NativeOutput::Prepared(control) if control.watches_configured())),
                )
                .await
                {
                    Exit::Success(prepared) => {
                        response = prepared.response;
                        producer = Some(prepared.finish);
                    }
                    Exit::Fail(error) => return Exit::Fail(error),
                    Exit::Die(defect) => return Exit::Die(defect),
                    Exit::Interrupted => return Exit::Interrupted,
                    Exit::FinalizationFailed(failed) => return Exit::FinalizationFailed(failed),
                }
                break;
            }
            Exit::Die(defect) => return Exit::Die(defect),
            Exit::Interrupted => return Exit::Interrupted,
            Exit::FinalizationFailed(failed) => return Exit::FinalizationFailed(failed),
        }
    }
    drop(unused_error_bodies);
    if let Some(limits) = response_limits && let Err(limit) = crate::body::check_output_headers(response.headers(), limits.head_fields, limits.head_bytes, crate::OutputLimit::HeadFields, crate::OutputLimit::HeadBytes) {
        drop(response);
        let primary = Exit::Fail((policy.native_error)(ServerError::OutputLimit(limit)));
        let produced = match producer { Some(producer) => context.run(producer, env.clone()).await, None => Exit::Success(()) };
        let cleanup = output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).cleanup[1].take();
        return match cleanup { Some(rest) => { let mut result = effect::PreparedCleanupAccumulator::from_reserved(primary, rest); result.append_cleanup(produced); result.finish() }, None => combine_completion(primary, produced) };
    }
    let completion = request
        .with_extensions(|extensions| {
            extensions
                .get::<OuterDelivery>()
                .map(|delivery| delivery.0.clone())
        });
    let reserved_wait = output.lock().unwrap_or_else(std::sync::PoisonError::into_inner).outer_wait.take();
    let (completion, wait) = match (completion, reserved_wait) {
        (Some(completion), Some(wait)) => (completion, wait),
        (None, Some(wait)) => (wait.signal(), wait),
        (Some(completion @ BodySignal::Ordinary(_)), None) => { let wait = match completion.admit_wait() { Ok(wait) => wait, Err(_) => return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()) }; (completion, wait) },
        (None, None) if request.with_extensions(|extensions| extensions.get::<crate::body::BodyPoolAccess<E>>().is_none()) => { let completion = BodySignal::ordinary(request.cancellation().clone()); let wait = match completion.admit_wait() { Ok(wait) => wait, Err(_) => return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()) }; (completion, wait) },
        (Some(BodySignal::Prepared(_)), None) | (None, None) => return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()),
    };
    let body = std::mem::replace(response.body_mut(), Body::empty());
    *response.body_mut() = Body::new(crate::body::ObservedBody {
        body,
        completion: completion.clone(),
        limits: response_limits,
        failed: false,
    });
    body_state.replace(completion.clone());
    completion.arm();
    let (sender, origin) = {
        let mut output = output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (output.sender.take(), output.origin.take())
    };
    let transport_origin = match (origin, &output) {
        (Some(origin), output) => {
            let queue = match output {
                NativeOutput::Prepared(control) if control.retains_queue_permit() => Some(output.clone()),
                NativeOutput::Prepared(_) | NativeOutput::Ordinary(_) => None,
            };
            Some(crate::prepared_transport_body::TransportOrigin::Graph { origin, queue })
        }
        (None, NativeOutput::Prepared(_)) => Some(
            crate::prepared_transport_body::TransportOrigin::Native(output.clone()),
        ),
        (None, NativeOutput::Ordinary(_)) => None,
    };
    match sender {
        Some(sender) => {
            if sender.send((response, transport_origin)).is_err() {
                let _ = request.cancellation().request();
            }
        }
        None => return Exit::Die(Defect::new("native response already committed")),
    }
    let produced = match producer {
        Some(producer) => context.run(producer, env.clone()).await,
        None => Exit::Success(()),
    };
    let delivered = match wait.wait().await {
        BodyOutcome::Completed => Exit::Success(()),
        BodyOutcome::Disconnected => Exit::Interrupted,
        BodyOutcome::Failed => Exit::Fail((policy.native_error)(match completion.output_limit() { Some(limit) => ServerError::OutputLimit(limit), None => ServerError::Transport(Defect::new("native response body failed")) })),
        BodyOutcome::Open => Exit::Die(Defect::new("native body completion did not settle")),
    };
    let cleanup = output
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cleanup[1]
        .take();
    match cleanup {
        Some(rest) => {
            let mut completion = effect::PreparedCleanupAccumulator::from_reserved(produced, rest);
            completion.append_cleanup(delivered);
            completion.finish()
        }
        None => combine_completion(produced, delivered),
    }
}

/// Deliver one consumed reply, then await producer/native completion before request scope closure.
fn deliver<E: Send + 'static, R: Send + Sync + 'static>(
    mut reply: Reply<E, R>,
    request: Request,
    scope: Scope<E>,
    policy: ServerPolicy<E, R>,
        output: InnerOutput<E, R>,
    state: BodySignalState<E, R>,
) -> Effect<(), E, R> {
    let pool = request.with_extensions(|extensions| extensions.get::<crate::body::BodyPoolAccess<E>>().cloned());
    let body_admission = match (output.take_body_vacancy(), pool) {
        (Some(vacancy), Some(_)) => Some(vacancy.publish(request.cancellation().clone())),
        (None, Some(pool)) => match pool.0.reserve_reply(&pool.1) {
            Ok(vacancy) => Some(vacancy.publish(request.cancellation().clone())),
            Err(error) => {
                let map = policy.native_error.clone();
                return effect_try_sync! { once; body: move || Err(map(ServerError::Memory(error))), on_error: ::core::convert::identity, };
            }
        },
        (None, None) if matches!(&output, InnerOutput::Ordinary(_)) => None,
        (Some(_), None) | (None, None) => return effect_die! { defect: effect::LibraryDefect::PreparedConstructionUnbound.into(), },
    };
    request.with_extensions(|extensions| {
        if let Some(headers) = extensions.get::<ResponseHeaders>() {
            for (name, value) in headers
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
            {
                reply.headers.insert(name.clone(), value.clone());
            }
        }
    });
    effect! {
    bind;
    program: reply
           .prepare(
               request.clone(),
               scope,
               policy.body,
               policy.native_error.clone(),
               body_admission,
           ),
    body: move |prepared| {
               let produces = request.with_extensions(|extensions| {
                   extensions
                       .get::<crate::Registration>()
                       .map(|metadata| metadata.produces.clone())
                       .unwrap_or_default()
               });
               if let Some(actual) = prepared.response.headers().get(http::header::CONTENT_TYPE)
                   && !produces.is_empty()
                   && !produces
                       .iter()
                       .any(|expected| media_matches(expected, actual))
               {
                   return effect_try_sync! {
                       once; body: {
                       let __dsl_result = Err((policy.native_error)(ServerError::Invalid(
                       "response content type not registered in produces".into(),
                   ))); move || __dsl_result }, on_error: ::core::convert::identity, };
               }
               state.replace(prepared.completion.clone());
               request.insert_extension(EndpointDelivery(prepared.completion.clone()));
               if !request.with_extensions(|extensions| extensions.get::<OuterDelivery>().is_some()) {
                   prepared.completion.arm();
               }
               let sender = output.take();
               match sender {
                   Some(sender) => {
                       if sender.send(prepared.response).is_err() {
                           let _ = request.cancellation().request();
                       }
                       {
                           let outer = request.with_extensions(|extensions| {
                               extensions.get::<OuterDelivery>().cloned()
                           });
                           match outer {
                               Some(outer) => {
                                   let wait = match (&outer.0, output.take_outer_wait()) {
                                       (BodySignal::Ordinary(gate), None) => { let gate = gate.clone(); effect_future! { body: move |_| { let gate = gate.clone(); async move { let _ = gate.wait().await; } }, } },
                                       (BodySignal::Prepared(_), Some(wait)) => effect_future! { once; body: move |_| async move { let _ = wait.wait().await; }, },
                                       (BodySignal::Ordinary(_), Some(_)) | (BodySignal::Prepared(_), None) => effect_die! { defect: effect::LibraryDefect::PreparedConstructionUnbound.into(), },
                                   };
                                   effect_resource! { ensuring; program: prepared.finish, release: move |_| wait.clone(), }
                               },
                               None => match &output { InnerOutput::Ordinary(_) => prepared.finish, InnerOutput::Prepared(_) => effect_die! { defect: effect::LibraryDefect::PreparedConstructionUnbound.into(), } },
                           }
                       }
                   }
                   None => effect_die! { defect: effect::Defect::new("response already committed"), },
               }
           },
    }
}
/// Preserve original primary plus observer failures using the existing ordered cleanup carrier.
fn observer_invariant<E: Send + 'static>(exit: Exit<(), E>, spare: Option<effect::CleanupFailures<E>>) -> Exit<(), E> {
    match effect::PreparedCleanupAccumulator::rejoin(exit, spare) {
        Ok(mut completion) => { completion.append_cleanup(Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into())); completion.finish() },
        Err((original, _)) => original,
    }
}
fn combine_completion<E>(primary: Exit<(), E>, observed: Exit<(), E>) -> Exit<(), E> {
    let failure = match observed {
        Exit::Success(()) => return primary,
        Exit::Fail(error) => effect::CleanupFailure::ReleaseFailure(error),
        Exit::Die(defect) => effect::CleanupFailure::ReleaseDefect(defect),
        Exit::Interrupted => effect::CleanupFailure::ReleaseInterrupted,
        Exit::FinalizationFailed(finalization) => {
            let mut original = combine_completion(
                primary,
                match finalization.primary {
                    effect::PrimaryOutcome::Succeeded => Exit::Success(()),
                    effect::PrimaryOutcome::Failed(error) => Exit::Fail(error),
                    effect::PrimaryOutcome::Defected(defect) => Exit::Die(defect),
                    effect::PrimaryOutcome::WasInterrupted => Exit::Interrupted,
                },
            );
            original = append_cleanup(original, finalization.first);
            for failure in finalization.rest {
                original = append_cleanup(original, failure);
            }
            return original;
        }
    };
    append_cleanup(primary, failure)
}
/// Append one terminal policy failure without erasing earlier body/cleanup outcomes.
fn append_cleanup<E>(primary: Exit<(), E>, failure: effect::CleanupFailure<E>) -> Exit<(), E> {
    match primary {
        Exit::FinalizationFailed(mut finalization) => {
            finalization.rest.push(failure);
            Exit::FinalizationFailed(finalization)
        }
        primary @ Exit::Success(_)
        | primary @ Exit::Fail(_)
        | primary @ Exit::Die(_)
        | primary @ Exit::Interrupted => {
            let primary = match primary {
                Exit::Success(()) => effect::PrimaryOutcome::Succeeded,
                Exit::Fail(error) => effect::PrimaryOutcome::Failed(error),
                Exit::Die(defect) => effect::PrimaryOutcome::Defected(defect),
                Exit::Interrupted => effect::PrimaryOutcome::WasInterrupted,
                Exit::FinalizationFailed(_) => {
                    return Exit::Die(effect::Defect::new("invalid nested completion state"));
                }
            };
            Exit::FinalizationFailed(effect::Finalization {
                primary,
                first: failure,
                rest: Default::default(),
            })
        }
    }
}
/// Produce a named protocol error only when no typed reply was committed.
fn terminal_response(status: StatusCode) -> http::Response<Body> {
    let mut response = http::Response::new(Body::empty());
    *response.status_mut() = status;
    response
}

fn is_admission_capacity(error: effect::memory::MemoryError) -> bool {
    use effect::memory::MemoryError;
    matches!(
        error,
        MemoryError::Busy | MemoryError::NoSlot | MemoryError::Exhausted { .. }
            | MemoryError::CapacityExceeded { .. }
    )
}

fn admission_refusal(
    error: effect::memory::MemoryError,
    selected: Option<StaticOverloadReply>,
) -> http::Response<Body> {
    if is_admission_capacity(error) && let Some(reply) = selected {
        return reply.response();
    }
    terminal_response(StatusCode::SERVICE_UNAVAILABLE)
}

fn route_admission_refusal<E: Send + 'static, R: Send + Sync + 'static>(
    output: Option<&NativeOutput<E, R>>,
    error: effect::memory::MemoryError,
    selected: Option<StaticOverloadReply>,
) -> http::Response<Body> {
    if is_admission_capacity(error) && let Some(NativeOutput::Prepared(control)) = output {
        control.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .route_capacity = Some(error);
    }
    admission_refusal(error, selected)
}

impl<E, R> Server<E, R>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    /// Acquire/configure/start/use/drain/stop under one Effect scope. Partial startup is owned
    /// before later configuration can fail. Native serving uses the caller's Tokio reactor.
    pub fn with_host<A: Send + 'static>(
        self,
        use_host: impl Fn(ServerHost) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<A, E, R> {
        self.with_host_terminal(use_host, None, None, None, None, None, None, None)
    }
    #[allow(clippy::too_many_arguments)] // Existing prepared authorities are independently admitted.
    pub(crate) fn with_host_terminal<A: Send + 'static>(
        self,
        use_host: impl Fn(ServerHost) -> Effect<A, E, R> + Send + Sync + 'static,
        terminal: Option<crate::prepared_terminal::TerminalHandle<E, R>>,
        cleanup: Option<Arc<effect::PreparedCleanupConfig<E>>>,
        serving_cleanup: Option<[effect::CleanupFailures<E>; 2]>,
        host_cleanup: Option<effect::CleanupFailures<E>>,
        cleanup_waiter: Option<effect::CleanupReturnWaiter<E>>,
        native_tasks: Option<Arc<dyn crate::prepared_native_task::NativeTaskDispatch<E, R>>>,
        connection_tasks: Option<Arc<PreparedConnectionSelection<E, R>>>,
    ) -> Effect<A, E, R> {
        let terminal = terminal.map(|handle| Arc::new(Mutex::new(Some(handle))));
        let serving_cleanup = Arc::new(Mutex::new(serving_cleanup));
        let host_cleanup = Arc::new(Mutex::new(host_cleanup));
        let cleanup_waiter = Arc::new(Mutex::new(cleanup_waiter));
        let description = Arc::new(self);
        let use_host = Arc::new(use_host);
        effect_scope! {
        body: move |scope| {
                    let server = description.as_ref().clone();
                    let use_host = use_host.clone();
                    let terminal = terminal.clone();
                    let cleanup = cleanup.clone();
                    let serving_cleanup = serving_cleanup.clone();
                    let host_cleanup = host_cleanup.clone();
                    let cleanup_waiter = cleanup_waiter.clone();
                    let native_tasks = native_tasks.clone();
                    let connection_tasks = connection_tasks.clone();
                    crate::endpoint::native_once(move |env: Arc<R>, context| async move {
                        let host_cleanup = host_cleanup.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                        let selected = env.tokio().clone();
                        let valid = match server.protocols {
                            HttpProtocols::Http1 => cfg!(feature = "http1"),
                            HttpProtocols::Http2 => cfg!(feature = "http2"),
                            HttpProtocols::Http1And2 => cfg!(feature = "http1") && cfg!(feature = "http2"),
                        };
                        if !valid {
                            return Exit::Fail((server.policy.native_error)(ServerError::Configuration(
                                "requested HTTP protocol feature disabled".into(),
                            )));
                        }
                        if let Some(timeout) = server.http1_header_timeout {
                            if !cfg!(feature = "http1") || matches!(server.protocols, HttpProtocols::Http2) {
                                return Exit::Fail((server.policy.native_error)(ServerError::Configuration(
                                    "HTTP/1 header timeout requires HTTP/1".into(),
                                )));
                            }
                            let valid_deadline = {
                                let _entered = selected.enter();
                                tokio::time::Instant::now().checked_add(timeout).is_some()
                            };
                            if !valid_deadline {
                                return Exit::Fail((server.policy.native_error)(ServerError::Configuration(
                                    "HTTP/1 header timeout exceeds clock range".into(),
                                )));
                            }
                        }
                        {
                            let _entered = selected.enter();
                            if tokio::time::Instant::now().checked_add(server.policy.host.drain).is_none() {
                                return Exit::Fail((server.policy.native_error)(ServerError::Configuration("HTTP drain timeout exceeds clock range".into())));
                            }
                        }
                        if let Some(timeouts) = server.prepared_http_timeouts {
                            let _entered = selected.enter();
                            let now = tokio::time::Instant::now();
                            if timeouts.first_head.is_some_and(|duration| now.checked_add(duration).is_none()) || timeouts.idle.is_some_and(|duration| now.checked_add(duration).is_none()) {
                                return Exit::Fail((server.policy.native_error)(ServerError::Configuration("HTTP connection timeout exceeds clock range".into())));
                            }
                        }
                        if let Some(queue) = &server.request_queue {
                            let _entered = selected.enter();
                            if queue.wait.is_zero() || tokio::time::Instant::now().checked_add(queue.wait).is_none() {
                                return Exit::Fail((server.policy.native_error)(ServerError::Configuration("HTTP request queue requires a positive wait within clock range".into())));
                            }
                        }
                        let terminal = match terminal {
                            Some(terminal) => match terminal.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                                Some(handle) => Some(handle),
                                None => return Exit::Die(Defect::new("prepared host already consumed")),
                            },
                            None => None,
                        };
                        let boundary = Arc::new(Boundary {
                            response_limits: server.response_limits,
                            overload_reply: server.overload_reply,
                            request_queue: server.request_queue.clone(),
                            native_slots: server.native_slots.clone(),
                            request_controls: server.request_controls.clone(),
                            body_pool: server.body_pool.clone(),
                            observer_batches: server.observer_batches.clone(),
                            request_storage: server.request_storage.clone(),
                            request_byte_pools: server.request_byte_pools.clone(),
                            response_headers: server.response_headers.clone(),
                            terminal,
                            cleanup,
                            native_tasks,
                            connection_tasks,
                            serving_cleanup: Mutex::new(serving_cleanup.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()),
                            cleanup_waiter: Mutex::new(cleanup_waiter.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()),
                            scope: scope.clone(),
                            context: context.clone(),
                            environment: env.clone(),
                            policy: server.policy.clone(),
                            binding: server.environment,
                            hooks: server.request_hooks,
                            routing_hooks: server.routing_hooks,
                            response_hooks: server.response_hooks.into(),
                            middleware: server.middleware,
                            decorators: server.decorators,
                            selection_decorators: server.selection_decorators.into(),
                        });
                        let mut router = match mount(server.routes, boundary.clone()) {
                            Ok(router) => router,
                            Err(error) => return Exit::Fail((server.policy.native_error)(error)),
                        };
                        for configure in server.configure {
                            router = match configure(router) {
                                Ok(router) => router,
                                Err(error) => return Exit::Fail((server.policy.native_error)(error)),
                            };
                        }
                        let listener = match native_io(
                            selected.clone(),
                            tokio::net::TcpListener::bind(server.listen),
                        )
                        .await
                        {
                            Ok(listener) => listener,
                            Err(error) => {
                                return Exit::Fail((server.policy.native_error)(ServerError::Io(error)));
                            }
                        };
                        let address = match listener.local_addr() {
                            Ok(address) => address,
                            Err(error) => {
                                return Exit::Fail((server.policy.native_error)(ServerError::Io(error)));
                            }
                        };
                        let stop = Cancellation::new();
                        let native_stop = stop.clone();
                        let (stopped, observer) = watch::channel(false);
                        let host = ServerHost {
                            address,
                            stop: stop.clone(),
                            stopped: observer,
                        };
                        let _stop_guard = HostStopGuard(stop.clone());
                        let map = server.policy.native_error.clone();
                        let protocols = server.protocols;
                        let http1_header_timeout = server.http1_header_timeout;
                        let prepared_http_limits = server.prepared_http_limits;
                        let prepared_http2_reset_limits = server.prepared_http2_reset_limits;
                        let prepared_http_timeouts = server.prepared_http_timeouts;
                        let drain = server.policy.host.drain;
                        let connection_scope = scope.clone();
                        let serving =
                            effect_uninterruptible! {
                                program: crate::endpoint::native_once(move |env: Arc<R>, context| async move {
                                let result = native_io(
                                    selected,
                                    serve_connections(
                                        listener,
                                        router,
                                        (protocols, native_stop, drain, http1_header_timeout, prepared_http_limits, prepared_http2_reset_limits, prepared_http_timeouts),
                                        (connection_scope, context, env),
                                        map,
                                        boundary,
                                        server.endpoint_hosting,
                                    ),
                                )
                                .await;
                                let _ = stopped.send(true);
                                result
                            }), };
                        let running = match context.run(scope.fork(serving), env.clone()).await {
                            Exit::Success(running) => running,
                            Exit::Fail(error) => return Exit::Fail(error),
                            Exit::Die(defect) => return Exit::Die(defect),
                            Exit::Interrupted => return Exit::Interrupted,
                            Exit::FinalizationFailed(failure) => return Exit::FinalizationFailed(failure),
                        };
                        let final_host = host.clone();
                        let drain = server.policy.host.drain;
                        let using = match context.run(scope.fork(use_host(host)), env.clone()).await {
                            Exit::Success(using) => using,
                            Exit::Fail(error) => return Exit::Fail(error),
                            Exit::Die(defect) => return Exit::Die(defect),
                            Exit::Interrupted => return Exit::Interrupted,
                            Exit::FinalizationFailed(failure) => return Exit::FinalizationFailed(failure),
                        };
                        let used = context.run(using.join(), env.clone());
                        pin!(used);
                        let result = select! {
                            result=&mut used=>result,
                            status=running.settled()=>{
                                if status==effect::OutcomeStatus::Succeeded {used.await}
                                else {
                                    final_host.stop();
                                    let stopped=context.run(effect_uninterruptible! { program: running.join(), },env.clone()).await;
                                    let interrupted=context.run(effect_uninterruptible! { program: using.interrupt(), },env.clone()).await;
                                    let primary=match stopped {Exit::Success(())=>Exit::Die(Defect::new("host failure status without failure")),Exit::Fail(error)=>Exit::Fail(error),Exit::Die(defect)=>Exit::Die(defect),Exit::Interrupted=>Exit::Interrupted,Exit::FinalizationFailed(failure)=>Exit::FinalizationFailed(failure)};
                                    return match interrupted {Exit::Success(outcome)=>append_host_cleanup_selected(primary,outcome,host_cleanup,false),Exit::Fail(error)=>append_host_cleanup_selected(primary,Exit::<(),E>::Fail(error),host_cleanup,false),Exit::Die(defect)=>append_host_cleanup_selected(primary,Exit::<(),E>::Die(defect),host_cleanup,false),Exit::Interrupted=>primary,Exit::FinalizationFailed(failure)=>append_host_cleanup_selected(primary,Exit::<(),E>::FinalizationFailed(failure),host_cleanup,false)};
                                }
                            }
                        };
                        final_host.stop();
                        let drain_result = native_io(env.tokio().clone(), async {
                            tokio::time::timeout(drain, final_host.drained()).await
                        })
                        .await;
                        if drain_result.is_err() {
                            let _ = stop.request();
                        }
                        let stopped = context.run(effect_uninterruptible! { program: running.join(), }, env).await;
                        match result {
                            Exit::Success(value) => match stopped {
                                Exit::Success(()) => Exit::Success(value),
                                Exit::Fail(error) => Exit::Fail(error),
                                Exit::Die(defect) => Exit::Die(defect),
                                Exit::Interrupted => Exit::Interrupted,
                                Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
                            },
                            failure @ Exit::Fail(_)
                            | failure @ Exit::Die(_)
                            | failure @ Exit::Interrupted
                            | failure @ Exit::FinalizationFailed(_) => match stopped {
                                Exit::Success(()) => failure,
                                Exit::Fail(error) => append_host_cleanup_selected(failure, Exit::<(),E>::Fail(error), host_cleanup, true),
                                Exit::Die(defect) => append_host_cleanup_selected(failure, Exit::<(),E>::Die(defect), host_cleanup, true),
                                Exit::Interrupted => append_host_cleanup_selected(failure, Exit::<(),E>::Interrupted, host_cleanup, true),
                                Exit::FinalizationFailed(cleanup) => {
                                    append_host_cleanup_selected(failure, Exit::<(), E>::FinalizationFailed(cleanup), host_cleanup, true)
                                }
                            },
                        }
                    })
                },
        }
    }
    /// Serve until the selected environment requests cancellation, then drain and join cleanup.
    pub fn run(self) -> Effect<(), E, R> {
        self.with_host(|_| {
            effect_future! {
                with_env; body: |env: Arc<R>, _| async move {
                env.cancellation().cancelled().await
            }, }
        })
    }
}
/// Synchronous signal guard makes unwinding/abandonment request native shutdown;
/// the registered host/request scope still performs the asynchronous joins.
struct HostStopGuard(Cancellation);
impl Drop for HostStopGuard {
    /// Signal once without performing asynchronous cleanup in a destructor.
    fn drop(&mut self) {
        let _ = self.0.request();
    }
}
/// Preserve a serving failure while joining the canceled use program's cleanup.
fn append_host_cleanup_selected<A, B, E: Send + 'static>(
    primary: Exit<A, E>,
    cleanup: Exit<B, E>,
    rest: Option<effect::CleanupFailures<E>>,
    include_interrupted: bool,
) -> Exit<A, E> {
    let Some(rest) = rest else {
        return match cleanup {
            Exit::Interrupted if include_interrupted => {
                append_cleanup_value(primary, effect::CleanupFailure::ReleaseInterrupted)
            }
            other @ (Exit::Success(_)
            | Exit::Fail(_)
            | Exit::Die(_)
            | Exit::Interrupted
            | Exit::FinalizationFailed(_)) => append_host_cleanup(primary, other),
        };
    };
    let mut completion = effect::PreparedCleanupAccumulator::from_reserved(primary, rest);
    let cleanup = match cleanup {
        Exit::Success(value) => {
            drop(value);
            Exit::Success(())
        }
        Exit::Interrupted if !include_interrupted => Exit::Success(()),
        Exit::Interrupted => Exit::Interrupted,
        Exit::Fail(error) => Exit::Fail(error),
        Exit::Die(defect) => Exit::Die(defect),
        Exit::FinalizationFailed(mut failure) => {
            if matches!(failure.primary, effect::PrimaryOutcome::WasInterrupted) {
                failure.primary = effect::PrimaryOutcome::Succeeded;
            }
            Exit::FinalizationFailed(failure)
        }
    };
    completion.append_cleanup(cleanup);
    completion.finish()
}

fn append_host_cleanup<A, B, E>(primary: Exit<A, E>, cleanup: Exit<B, E>) -> Exit<A, E> {
    match cleanup {
        Exit::Success(_) | Exit::Interrupted => primary,
        Exit::Fail(error) => {
            append_cleanup_value(primary, effect::CleanupFailure::ReleaseFailure(error))
        }
        Exit::Die(defect) => {
            append_cleanup_value(primary, effect::CleanupFailure::ReleaseDefect(defect))
        }
        Exit::FinalizationFailed(failure) => {
            let mut result = match failure.primary {
                effect::PrimaryOutcome::Succeeded | effect::PrimaryOutcome::WasInterrupted => {
                    primary
                }
                effect::PrimaryOutcome::Failed(error) => {
                    append_cleanup_value(primary, effect::CleanupFailure::ReleaseFailure(error))
                }
                effect::PrimaryOutcome::Defected(defect) => {
                    append_cleanup_value(primary, effect::CleanupFailure::ReleaseDefect(defect))
                }
            };
            result = append_cleanup_value(result, failure.first);
            for failure in failure.rest {
                result = append_cleanup_value(result, failure);
            }
            result
        }
    }
}

/// Drive native futures under the selected Tokio handle on each poll; no enter guard crosses await.
struct NativeIo<F: std::future::Future> {
    /// Native reactor/caller runtime selection.
    handle: tokio::runtime::Handle,
    /// Structurally pinned native future, destroyed in place under the runtime.
    future: std::mem::ManuallyDrop<F>,
}
impl<F: std::future::Future> std::future::Future for NativeIo<F> {
    /// Preserve the native operation's actual result type.
    type Output = F::Output;
    /// Enter the selected reactor only during one native poll.
    fn poll(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // SAFETY: the inline future is structurally pinned and never moved
        // until its explicit in-place destructor runs.
        let this = unsafe { self.get_unchecked_mut() };
        let _guard = this.handle.enter();
        // SAFETY: this is the unique projection of the pinned future field.
        unsafe { std::pin::Pin::new_unchecked(&mut *this.future) }.poll(context)
    }
}
impl<F: std::future::Future> Drop for NativeIo<F> {
    fn drop(&mut self) {
        let future = std::mem::DropGuard::new(&mut self.future, |future| {
            // SAFETY: the field was initialized once and is destroyed in place once.
            unsafe { std::mem::ManuallyDrop::drop(future); }
        });
        let _guard = self.handle.enter();
        drop(future);
    }
}
/// Wrap a future in caller-selected poll-scoped native reactor access.
fn native_io<F: std::future::Future>(handle: tokio::runtime::Handle, future: F) -> NativeIo<F> {
    NativeIo {
        handle,
        future: std::mem::ManuallyDrop::new(future),
    }
}
// One immutable Allow value per host/port matching class, prepared at mount.
struct AllowCase { hostname: String, port: Option<u16>, value: Option<HeaderValue> }
struct RouteCatalogue<E: Send + 'static, R: Send + Sync + 'static> {
    entries: Box<[Arc<crate::route::RouteEntry<E, R>>]>,
    default_allow: Option<HeaderValue>,
    allow_cases: Vec<AllowCase>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> RouteCatalogue<E, R> {
    fn prepare(entries: Vec<Arc<crate::route::RouteEntry<E, R>>>) -> Result<Self, ServerError> {
        let mut classes: Vec<(String, Option<u16>)> = Vec::new();
        for entry in &entries {
            for value in &entry.metadata.hosts {
                let mut values = std::iter::once(value);
                let host = <headers::Host as headers::Header>::decode(&mut values).map_err(|_| ServerError::Configuration("registered Host".into()))?;
                for port in [None, host.port()] {
                    if !classes.iter().any(|(name, existing)| name.eq_ignore_ascii_case(host.hostname()) && *existing == port) {
                        classes.push((host.hostname().to_owned(), port));
                    }
                }
            }
        }
        let make_allow = |class: Option<(&str, Option<u16>)>| {
            let mut allowed = std::collections::BTreeSet::new();
            for entry in &entries {
                let matches = entry.metadata.hosts.is_empty() || class.is_some_and(|(hostname, port)| entry.metadata.hosts.iter().any(|value| {
                    let mut values = std::iter::once(value);
                    <headers::Host as headers::Header>::decode(&mut values).is_ok_and(|host| host.hostname().eq_ignore_ascii_case(hostname) && host.port().is_none_or(|expected| port == Some(expected)))
                }));
                if matches { for method in entry.methods.methods() { allowed.insert(method.as_str()); if method == Method::GET { allowed.insert("HEAD"); } } }
            }
            let value = HeaderValue::from_str(&allowed.into_iter().collect::<Vec<_>>().join(", ")).ok();
            // Bytes may promote on its first clone; perform that once at startup.
            if let Some(value) = &value { drop(value.clone()); }
            value
        };
        let default_allow = make_allow(None);
        let allow_cases = classes.into_iter().map(|(hostname, port)| {
            let value = make_allow(Some((&hostname, port)));
            AllowCase { hostname, port, value }
        }).collect();
        Ok(Self { entries: entries.into_boxed_slice(), default_allow, allow_cases })
    }
    fn allow(&self, request: &http::Request<Body>) -> Option<&HeaderValue> {
        let Some(host) = request_host(request) else { return self.default_allow.as_ref() };
        let exact = host.port().and_then(|port| self.allow_cases.iter().find(|case| case.port == Some(port) && case.hostname.eq_ignore_ascii_case(host.hostname())));
        let fallback = || self.allow_cases.iter().find(|case| case.port.is_none() && case.hostname.eq_ignore_ascii_case(host.hostname()));
        match exact.or_else(fallback) { Some(case) => case.value.as_ref(), None => self.default_allow.as_ref() }
    }
}

/// Mount native method/path registrations once, retaining typed metadata before requests arrive.
fn mount<E, R>(
    routes: Router<E, R>,
    boundary: Arc<Boundary<E, R>>,
) -> Result<axum::Router, ServerError>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    let mut grouped: BTreeMap<String, Vec<Arc<crate::route::RouteEntry<E, R>>>> = BTreeMap::new();
    for mut entry in routes.entries {
        validate_registration(&entry.metadata)?;
        let policy = routes.on_error.clone();
        let handler = entry.endpoint.clone();
        let pipeline = routes.pipeline.clone();
        entry.endpoint = Arc::new(move |captures| {
            let mut endpoint =
                endpoint! { source: handler(captures)?, pipe_through: pipeline.clone(), };
            if endpoint.on_error.is_none() {
                endpoint.on_error = policy.clone();
            }
            Ok(endpoint)
        });
        let entry = Arc::new(entry);
        for pattern in &entry.patterns {
            grouped
                .entry(pattern.clone())
                .or_default()
                .push(entry.clone());
        }
    }
    let mut router = axum::Router::new();
    for (pattern, entries) in grouped {
        let catalogue = Arc::new(RouteCatalogue::prepare(entries)?);
        let boundary = boundary.clone();
        let service = service_fn(move |mut request: http::Request<Body>| {
            let catalogue = catalogue.clone();
            let boundary = boundary.clone();
            async move {
                let entries = &catalogue.entries;
                let method = request.method().clone();
                let selected = entries
                    .iter()
                    .find(|entry| {
                        host_matches(&entry.metadata, &request)
                            && entry.methods.methods().contains(&method)
                    })
                    .or_else(|| {
                        if method == Method::HEAD {
                            entries.iter().find(|entry| {
                                host_matches(&entry.metadata, &request)
                                    && entry.methods.methods().contains(&Method::GET)
                            })
                        } else {
                            None
                        }
                    });
                let response = match selected {
                    Some(entry) => {
                        let on_capacity = entry.on_capacity.or(boundary.overload_reply);
                        let endpoint_bytes = if let Some(pools) = &entry.byte_pools {
                            let Some(slot) = request.extensions().get::<NativeSlotExtension<E, R>>() else {
                                return Ok::<_, Infallible>(terminal_response(
                                    StatusCode::SERVICE_UNAVAILABLE,
                                ));
                            };
                            match pools.reserve(boundary.request_byte_pools.as_deref()) {
                                Ok(leases) => Some(leases),
                                Err(error) => return Ok(route_admission_refusal(Some(&slot.output), error, on_capacity)),
                            }
                        } else {
                            None
                        };
                        if let Some(prepared) = &entry.prepared {
                            let Some(slot) = request.extensions().get::<NativeSlotExtension<E, R>>() else {
                                return Ok::<_, Infallible>(terminal_response(
                                    StatusCode::SERVICE_UNAVAILABLE,
                                ));
                            };
                            let has_front = {
                                let state = slot
                                    .output
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                state.retention.is_some() && state.terminal.is_some()
                            };
                            if !has_front {
                                return Ok(terminal_response(StatusCode::SERVICE_UNAVAILABLE));
                            }
                            let admission = match prepared.admit_shared() {
                                Ok(admission) => admission,
                                Err(error) => return Ok(route_admission_refusal(Some(&slot.output), error, on_capacity)),
                            };
                            let origins = match admission.reserve_output_origins() {
                                Ok(origins) => origins,
                                Err(error) => {
                                    drop(admission);
                                    return Ok(route_admission_refusal(Some(&slot.output), error, on_capacity));
                                }
                            };
                            let admission = match admission.into_owned(origins) {
                                Ok(admission) => admission,
                                Err((_admission, _origins)) => {
                                    return Ok(terminal_response(StatusCode::SERVICE_UNAVAILABLE));
                                }
                            };
                            slot.output
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .admission = Some(admission);
                        }
                        let input_pending = if let Some(admit) = &entry.prepared_input {
                            let Some(slot) = request.extensions().get::<NativeSlotExtension<E, R>>() else {
                                return Ok(terminal_response(StatusCode::SERVICE_UNAVAILABLE));
                            };
                            let has_front = {
                                let state = slot
                                    .output
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                state.retention.is_some() && state.terminal.is_some()
                            };
                            if !has_front {
                                return Ok(terminal_response(StatusCode::SERVICE_UNAVAILABLE));
                            }
                            match admit() {
                                Ok(pending) => Some((pending, slot.clone())),
                                Err(error) => return Ok(route_admission_refusal(Some(&slot.output), error, on_capacity)),
                            }
                        } else {
                            None
                        };
                        let (mut parts, body) = request.into_parts();
                        let captured =
                            axum::extract::Path::<BTreeMap<String, String>>::from_request_parts(
                                &mut parts,
                                &(),
                            )
                            .await;
                        #[cfg(feature = "websocket")]
                        if entry
                            .metadata
                            .extensions
                            .get::<crate::websocket::WebSocketRoute>()
                            .is_some()
                            && let Ok(upgrade) =
                                axum::extract::ws::WebSocketUpgrade::from_request_parts(
                                    &mut parts,
                                    &(),
                                )
                                .await
                        {
                            parts
                                .extensions
                                .insert(crate::websocket::UpgradeSlot(Arc::new(Mutex::new(Some(
                                    upgrade,
                                )))));
                        }
                        request = http::Request::from_parts(parts, body);
                        match captured {
                            Ok(axum::extract::Path(captures)) => {
                                boundary
                                    .dispatch(
                                        request,
                                        |captures| {
                                            if let Some((pending, slot)) = input_pending {
                                                let (captures, admission) = pending.call(captures);
                                                let endpoint = match admission {
                                                    Ok(admission) => {
                                                        slot.output
                                                            .lock()
                                                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                            .admission = Some(crate::prepared_endpoint::PreparedOwnedEndpointAdmission { pending: admission });
                                                        (entry.endpoint)(&captures)
                                                    }
                                                    Err(error) => Err(error),
                                                };
                                                (captures, endpoint)
                                            } else {
                                                let endpoint = (entry.endpoint)(&captures);
                                                (captures, endpoint)
                                            }
                                        },
                                        entry.metadata.clone(),
                                        captures,
                                        EndpointDispatchResources { on_capacity, bytes: endpoint_bytes },
                                    )
                                    .await
                            }
                            Err(_) => terminal_response(StatusCode::BAD_REQUEST),
                        }
                    }
                    None => {
                        let host_entries = entries
                            .iter()
                            .filter(|entry| host_matches(&entry.metadata, &request));
                        if host_entries.clone().next().is_none() {
                            terminal_response(StatusCode::NOT_FOUND)
                        } else {
                            let mut response = terminal_response(StatusCode::METHOD_NOT_ALLOWED);
                            if let Some(allow) = catalogue.allow(&request) {
                                response.headers_mut().insert(http::header::ALLOW, allow.clone());
                            }
                            response
                        }
                    }
                };
                Ok::<_, Infallible>(response)
            }
        });
        router = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            router.route_service(&pattern, service)
        }))
        .map_err(|_| ServerError::Configuration("native route registration conflict".into()))?;
    }
    for group in routes.groups {
        let mut child = *group.child;
        child.pipeline = pipeline! { source: routes.pipeline.clone(), include: child.pipeline, };
        if child.on_error.is_none() {
            child.on_error = routes.on_error.clone();
        }
        let child = mount(child, boundary.clone())?;
        router = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            router.nest(&group.prefix, child)
        }))
        .map_err(|_| ServerError::Configuration("native route group conflict".into()))?;
    }
    if let Some(mut endpoint) = routes.fallback {
        endpoint = endpoint! { source: endpoint, pipe_through: routes.pipeline.clone(), };
        if endpoint.on_error.is_none() {
            endpoint.on_error = routes.on_error.clone();
        }
        let owner = boundary.clone();
        router = router.fallback_service(service_fn(move |request| {
            let owner = owner.clone();
            let endpoint = endpoint.clone();
            async move {
                let registered = registered! { endpoint: endpoint, };
                let on_capacity = owner.overload_reply;
                Ok::<_, Infallible>(
                    owner
                        .dispatch(
                            request,
                            move |captures| (captures, Ok(registered.endpoint)),
                            registered.metadata,
                            BTreeMap::new(),
                            EndpointDispatchResources { on_capacity, bytes: None },
                        )
                        .await,
                )
            }
        }));
    }
    for configure in routes.native {
        router = configure(router)?;
    }
    Ok(router)
}
/// Match concrete registered media types using native MIME syntax and case-insensitive bases.
fn media_matches(expected: &HeaderValue, actual: &HeaderValue) -> bool {
    let expected = expected
        .to_str()
        .ok()
        .and_then(|value| crate::reply::media_parts(value).ok());
    let actual = actual
        .to_str()
        .ok()
        .and_then(|value| crate::reply::media_parts(value).ok());
    match (expected, actual) {
        (Some((expected, parameters)), Some((actual, actual_parameters))) => {
            expected == actual
                && parameters
                    .iter()
                    .all(|(name, value)| actual_parameters.get(name) == Some(value))
        }
        _ => false,
    }
}
/// Validate registration conventions before native serving can be started.
fn validate_registration(metadata: &crate::Registration) -> Result<(), ServerError> {
    for value in metadata.accepts.iter().chain(&metadata.produces) {
        let mut values = std::iter::once(value);
        <headers::ContentType as headers::Header>::decode(&mut values)
            .map_err(|_| ServerError::Configuration("registered media type".into()))?;
        if value.as_bytes().contains(&b'*') {
            return Err(ServerError::Configuration(
                "registered media type must be concrete".into(),
            ));
        }
    }
    for value in &metadata.hosts {
        let mut values = std::iter::once(value);
        <headers::Host as headers::Header>::decode(&mut values)
            .map_err(|_| ServerError::Configuration("registered Host".into()))?;
    }
    Ok(())
}
/// Decode the same request authority used by host restrictions. Duplicate Host
/// values reject constrained registrations even when URI authority exists.
fn request_host(request: &http::Request<Body>) -> Option<headers::Host> {
    if request.headers().get_all(http::header::HOST).iter().count() > 1 { return None; }
    request.uri().authority().map(|authority| headers::Host::from(authority.clone())).or_else(|| {
        let mut values = request.headers().get_all(http::header::HOST).iter();
        <headers::Host as headers::Header>::decode(&mut values).ok()
    })
}
/// Native host restrictions compare authority names and optional explicitly declared ports.
fn host_matches(metadata: &crate::Registration, request: &http::Request<Body>) -> bool {
    if metadata.hosts.is_empty() { return true; }
    request_host(request).is_some_and(|actual| metadata.hosts.iter().any(|expected| {
        let mut values = std::iter::once(expected);
        <headers::Host as headers::Header>::decode(&mut values).is_ok_and(|expected| expected.hostname().eq_ignore_ascii_case(actual.hostname()) && expected.port().is_none_or(|port| actual.port() == Some(port)))
    }))
}

/// Retain generic failed-use outcomes plus shutdown cleanup failures without copying A or E.
fn append_cleanup_value<A, E>(
    primary: Exit<A, E>,
    failure: effect::CleanupFailure<E>,
) -> Exit<A, E> {
    match primary {
        Exit::FinalizationFailed(mut finalization) => {
            finalization.rest.push(failure);
            Exit::FinalizationFailed(finalization)
        }
        Exit::Success(_) => Exit::FinalizationFailed(effect::Finalization {
            primary: effect::PrimaryOutcome::Succeeded,
            first: failure,
            rest: Default::default(),
        }),
        Exit::Fail(error) => Exit::FinalizationFailed(effect::Finalization {
            primary: effect::PrimaryOutcome::Failed(error),
            first: failure,
            rest: Default::default(),
        }),
        Exit::Die(defect) => Exit::FinalizationFailed(effect::Finalization {
            primary: effect::PrimaryOutcome::Defected(defect),
            first: failure,
            rest: Default::default(),
        }),
        Exit::Interrupted => Exit::FinalizationFailed(effect::Finalization {
            primary: effect::PrimaryOutcome::WasInterrupted,
            first: failure,
            rest: Default::default(),
        }),
    }
}

/// Typed macro conversion for extracted environment resolvers and prebuilt bindings.
pub trait IntoRequestBinding<E: Send + 'static, R: Send + Sync + 'static> {
    /// Retain the resolver as an inert reusable description.
    fn into_binding(self) -> RequestBinding<E, R>;
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoRequestBinding<E, R>
    for RequestBinding<E, R>
{
    /// A prebuilt binding already preserves the selected environment boundary.
    fn into_binding(self) -> RequestBinding<E, R> {
        self
    }
}
impl<E, R, F> IntoRequestBinding<E, R> for F
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(Request) -> Effect<Arc<R>, E, R> + Send + Sync + 'static,
{
    /// Extracted callbacks are selected once and remain lazy until a request arrives.
    fn into_binding(self) -> RequestBinding<E, R> {
        RequestBinding::new(self)
    }
}

/// Native response observers cannot abandon admitted work while request/body scopes remain active.
struct RequestDisconnect {
    /// Fresh native request cancellation owner.
    request: Request,
    /// Ownership transfers to the response body once a response is observed.
    armed: bool,
}
impl Drop for RequestDisconnect {
    /// Dropping the native service future signals cancellation to its retained managed driver.
    fn drop(&mut self) {
        if self.armed {
            let _ = self.request.cancellation().request();
        }
    }
}

/// Apply the same explicitly selected HTTP/1 timer to both native serving paths.
fn configure_http1_header_timeout<X>(
    builder: &mut hyper_util::server::conn::auto::Builder<X>,
    timeout: Option<Duration>,
) {
    #[cfg(feature = "http1")]
    if let Some(timeout) = timeout {
        // Hyper's timer returns boxed sleeps; this is ordinary native runtime storage.
        builder
            .http1()
            .timer(hyper_util::rt::TokioTimer::new())
            .header_read_timeout(timeout);
    }
    #[cfg(not(feature = "http1"))]
    let _ = (builder, timeout);
}

// Apply only explicitly selected controls; the native builder still owns its storage.
fn configure_prepared_http_limits<X>(builder: &mut hyper_util::server::conn::auto::Builder<X>, limits: Option<PreparedHttpLimits>, resets: Option<PreparedHttp2ResetLimits>) {
    #[cfg(feature = "http1")]
    if let Some(limit) = limits.and_then(|value| value.http1) {
        builder.http1().max_headers(limit.max_headers).max_buf_size(limit.read_buffer_bytes);
    }
    #[cfg(feature = "http2")]
    if let Some(limit) = limits.and_then(|value| value.http2) {
        builder.http2().adaptive_window(false).max_header_list_size(limit.max_header_list_bytes).max_frame_size(limit.max_frame_bytes).initial_stream_window_size(limit.stream_window_bytes).initial_connection_window_size(limit.connection_window_bytes).max_send_buf_size(limit.max_send_buffer_bytes);
    }
    #[cfg(feature = "http2")]
    if let Some(resets) = resets {
        builder.http2().max_pending_accept_reset_streams(Some(resets.pending_accept_resets)).max_local_error_reset_streams(Some(resets.local_error_resets));
    }
    #[cfg(not(feature = "http2"))]
    let _ = resets;
    #[cfg(not(any(feature = "http1", feature = "http2")))]
    let _ = (builder, limits);
}

// Socket activity is stored in the already reserved connection record.
struct MonitoredSocket<E: Send + 'static, R: Send + Sync + 'static> {
    socket: tokio::net::TcpStream,
    progress: Option<crate::prepared_connection_retention::ConnectionHandle<E, R>>,
    idle: Option<Duration>,
}
// The socket is Unpin and the retained metadata handle is never structurally pinned.
impl<E: Send + 'static, R: Send + Sync + 'static> Unpin for MonitoredSocket<E, R> {}
impl<E: Send + 'static, R: Send + Sync + 'static> tokio::io::AsyncRead for MonitoredSocket<E, R> {
    fn poll_read(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = std::pin::Pin::new(&mut self.socket).poll_read(cx, buf);
        if matches!(&result, std::task::Poll::Ready(Ok(()))) && buf.filled().len() > before && let Some(progress) = &self.progress { progress.activity(self.idle); }
        result
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> tokio::io::AsyncWrite for MonitoredSocket<E, R> {
    fn poll_write(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &[u8]) -> std::task::Poll<std::io::Result<usize>> {
        let result = std::pin::Pin::new(&mut self.socket).poll_write(cx, buf);
        if matches!(&result, std::task::Poll::Ready(Ok(count)) if *count > 0) && let Some(progress) = &self.progress { progress.activity(self.idle); }
        result
    }
    fn poll_write_vectored(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, bufs: &[std::io::IoSlice<'_>]) -> std::task::Poll<std::io::Result<usize>> {
        let result = std::pin::Pin::new(&mut self.socket).poll_write_vectored(cx, bufs);
        if matches!(&result, std::task::Poll::Ready(Ok(count)) if *count > 0) && let Some(progress) = &self.progress { progress.activity(self.idle); }
        result
    }
    fn is_write_vectored(&self) -> bool { self.socket.is_write_vectored() }
    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> { std::pin::Pin::new(&mut self.socket).poll_flush(cx) }
    fn poll_shutdown(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> { std::pin::Pin::new(&mut self.socket).poll_shutdown(cx) }
}
fn connection_deadline<E: Send + 'static, R: Send + Sync + 'static>(handle: &crate::prepared_connection_retention::ConnectionHandle<E, R>, timeouts: Option<PreparedHttpTimeouts>) -> Result<Option<(tokio::time::Instant, &'static str)>, ()> {
    let Some(timeouts) = timeouts else { return Ok(None) };
    let (accepted, last, first_head) = handle.progress().ok_or(())?;
    let first = match timeouts.first_head { Some(duration) => { let at = accepted.checked_add(duration).ok_or(())?; if first_head.is_some_and(|observed| observed < at) { None } else { Some((at, "HTTP first head deadline")) } }, None => None };
    let idle = match timeouts.idle { Some(duration) => Some((last.checked_add(duration).ok_or(())?, "HTTP network idle deadline")), None => None };
    Ok(match (first, idle) { (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }), (Some(a), None) | (None, Some(a)) => Some(a), (None, None) => None })
}
fn poll_native_task_receipts<E: Send + 'static, R: Send + Sync + 'static>(
    tasks: Option<&dyn crate::prepared_native_task::NativeTaskDispatch<E, R>>,
    handle: &crate::prepared_connection_retention::ConnectionHandle<E, R>,
    cx: &mut std::task::Context<'_>,
    outcomes: &mut effect::PreparedCleanupAccumulator<(), E>,
) -> std::task::Poll<()> {
    let Some(tasks) = tasks else { return std::task::Poll::Ready(()) };
    loop { match tasks.poll_connection(handle, cx) {
        std::task::Poll::Ready(Some(defect)) => outcomes.append_cleanup(Exit::Die(defect)),
        std::task::Poll::Ready(None) => return std::task::Poll::Ready(()),
        std::task::Poll::Pending => return std::task::Poll::Pending,
    } }
}

#[allow(clippy::too_many_arguments)]
/// Drive one accepted native connection through tracked child settlement.
async fn serve_tracked_connection<E, R>(
    connection: (tokio::net::TcpStream, SocketAddr),
    router: axum::Router,
    boundary: Arc<Boundary<E, R>>,
    serving: ConnectionServing,
    map: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    handle: crate::prepared_connection_retention::ConnectionHandle<E, R>,
    reactor: tokio::runtime::Handle,
    cleanup: (effect::CleanupFailures<E>, effect::CleanupFailures<E>),
    retention: Arc<crate::prepared_connection_retention::ConnectionRetentionPool<E, R>>,
    mut prepared_context: Option<effect::__private::prepared_graph::FutureContext<E>>,
) -> Exit<(), E>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    let (socket, peer) = connection;
    let (protocols, stop, drain, http1_header_timeout, prepared_http_limits, prepared_http2_reset_limits, prepared_http_timeouts) = serving;
    let executor_handle = match handle.try_clone() {
        Ok(value) => value,
        Err(error) => return Exit::Fail(map(ServerError::Memory(error))),
    };
    let executor =
        crate::prepared_connection_retention::TrackedExecutor::new(executor_handle, reactor, boundary.native_tasks.clone(), retention);
    let service_tracking = executor.clone();
    let monitoring = prepared_http_timeouts.is_some_and(|timeouts| timeouts.first_head.is_some() || timeouts.idle.is_some());
    let socket_progress = if monitoring { match handle.try_clone() { Ok(value) => Some(value), Err(error) => return Exit::Fail(map(ServerError::Memory(error))) } } else { None };
    let initial_deadline = match connection_deadline(&handle, prepared_http_timeouts) { Ok(value) => value, Err(()) => return Exit::Fail(map(ServerError::Limit("HTTP connection deadline exceeds clock range".into()))) };
    let mut builder = hyper_util::server::conn::auto::Builder::new(executor);
    configure_http1_header_timeout(&mut builder, http1_header_timeout);
    configure_prepared_http_limits(&mut builder, prepared_http_limits, prepared_http2_reset_limits);
    #[cfg(feature = "http2")]
    builder
        .http2()
        .max_concurrent_streams(handle.http2_max_streams());
    let builder = match protocols {
        #[cfg(feature = "http1")]
        HttpProtocols::Http1 => builder.http1_only(),
        #[cfg(feature = "http2")]
        HttpProtocols::Http2 => builder.http2_only(),
        HttpProtocols::Http1And2 => builder,
        #[cfg(not(feature = "http1"))]
        HttpProtocols::Http1 => {
            return Exit::Fail(map(ServerError::Configuration("HTTP/1 disabled".into())));
        }
        #[cfg(not(feature = "http2"))]
        HttpProtocols::Http2 => {
            return Exit::Fail(map(ServerError::Configuration("HTTP/2 disabled".into())));
        }
    };
    let tasks = boundary.native_tasks.clone();
    let queue_stop = boundary.request_queue.as_ref().map(|_| stop.clone());
    let service = tower::service_fn(move |mut request: http::Request<hyper::body::Incoming>| {
        let late = prepared_http_timeouts.is_some_and(|timeouts| service_tracking.first_head(timeouts.first_head, timeouts.idle));
        let immediate = (!late && queue_stop.is_none()).then(|| {
            PreparedRequestAdmission::reserve(&service_tracking, &boundary, None)
        });
        let tracking = service_tracking.clone();
        let boundary = boundary.clone();
        let queue_stop = queue_stop.clone();
        let router = router.clone();
        async move {
            // Until a queued request is granted a permit, Hyper's Incoming is
            // untouched. Dropping the service future unlinks its FIFO entry.
            let admission = match (&boundary.request_queue, queue_stop.as_ref()) {
                (Some(queue), Some(stop)) if !late => Some(
                    queue.admit(stop).await.and_then(|permit| {
                        PreparedRequestAdmission::reserve(&tracking, &boundary, Some(permit))
                    }),
                ),
                _ => immediate,
            };
            let (response, origin) = match admission {
                Some(Ok(admission)) => {
                    request.extensions_mut().insert(tracking);
                    request.extensions_mut().insert(peer);
                    boundary
                        .dispatch_native(request.map(Body::new), router, Some(admission))
                        .await
                }
                Some(Err(error)) => (admission_refusal(error, boundary.overload_reply), None),
                None => (terminal_response(StatusCode::SERVICE_UNAVAILABLE), None),
            };
            let response = response.map(|body| match origin {
                Some(origin) => crate::prepared_transport_body::TransportBody::Prepared(
                    crate::prepared_transport_body::PreparedTransportBody::new(body, origin),
                ),
                None => crate::prepared_transport_body::TransportBody::Ordinary(body),
            });
            Ok::<_, Infallible>(response)
        }
    });
    let service = hyper_util::service::TowerToHyperService::new(service);
    let (native_rest, refusal_rest) = cleanup;
    let mut task_outcomes = effect::PreparedCleanupAccumulator::from_reserved(Exit::Success(()), native_rest);
    let notification = handle.tasks_changed();
    pin!(notification);
    // Keep the same Notified future registered across Pending; after consuming
    // a signal, install and poll its successor before scanning receipt state.
    macro_rules! poll_task_notification {
        ($cx:expr) => {
            if tasks.is_some() && notification.as_mut().poll($cx).is_ready() {
                notification.as_mut().set(handle.tasks_changed());
                if notification.as_mut().poll($cx).is_ready() { $cx.waker().wake_by_ref(); }
            }
        };
    }
    let mut refusal = None;
    let mut deadline = None;
    let mut limit = None;
    let mut drain_overflow = false;
    let mut interrupted = false;
    let result = {
        let socket = MonitoredSocket { socket, progress: socket_progress, idle: prepared_http_timeouts.and_then(|timeouts| timeouts.idle) };
        let connection = builder.serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(socket), service);
        pin!(connection);
        let sleep = initial_deadline.map(|(at, _)| tokio::time::sleep_until(at));
        pin!(sleep);
        enum Action<T> { Native(T), Stop, Cancelled, Refused(crate::prepared_connection_retention::ConnectionRefusal), Limit(&'static str) }
        let action = select! {
            result = std::future::poll_fn(|cx| {
                poll_task_notification!(cx);
                let _ = poll_native_task_receipts(tasks.as_deref(), &handle, cx, &mut task_outcomes);
                let before = match connection_deadline(&handle, prepared_http_timeouts) { Ok(value) => value, Err(()) => return std::task::Poll::Ready(Action::Limit("HTTP connection deadline exceeds clock range")) };
                if let Some((at, label)) = before && tokio::time::Instant::now() >= at { return std::task::Poll::Ready(Action::Limit(label)); }
                let native = connection.as_mut().poll(cx);
                let _ = poll_native_task_receipts(tasks.as_deref(), &handle, cx, &mut task_outcomes);
                let next = match connection_deadline(&handle, prepared_http_timeouts) { Ok(value) => value, Err(()) => return std::task::Poll::Ready(Action::Limit("HTTP connection deadline exceeds clock range")) };
                if let Some((at, label)) = next && tokio::time::Instant::now() >= at { return std::task::Poll::Ready(Action::Limit(label)); }
                if let std::task::Poll::Ready(result) = native { return std::task::Poll::Ready(Action::Native(result)); }
                if let Some((at, label)) = next && let Some(mut timer) = sleep.as_mut().as_pin_mut() {
                    if timer.deadline() != at { timer.as_mut().reset(at); }
                    if timer.poll(cx).is_ready() { return std::task::Poll::Ready(Action::Limit(label)); }
                }
                std::task::Poll::Pending
            }) => result,
            () = stop.cancelled() => Action::Stop,
            () = std::future::poll_fn(|cx| {
                let Some(context) = prepared_context.as_mut() else { return std::task::Poll::Pending; };
                if context.is_requested() { return std::task::Poll::Ready(()); }
                let observed = {
                    let wait = context.cancelled();
                    pin!(wait);
                    wait.poll(cx).is_ready()
                };
                if observed || context.is_requested() { std::task::Poll::Ready(()) } else { std::task::Poll::Pending }
            }) => Action::Cancelled,
            error = handle.refused() => Action::Refused(error),
        };
        match action {
            Action::Native(result) => Some(result),
            Action::Limit(reason) => { limit = Some(reason); None },
            Action::Stop | Action::Cancelled | Action::Refused(_) => {
                interrupted = matches!(action, Action::Cancelled);
                if let Action::Refused(error) = action { refusal = Some(error); }
                match tokio::time::Instant::now().checked_add(drain) {
                    Some(until) => {
                        deadline = Some(until);
                        connection.as_mut().graceful_shutdown();
                        tokio::time::timeout_at(until, std::future::poll_fn(|cx| {
                            poll_task_notification!(cx);
                            let _ = poll_native_task_receipts(tasks.as_deref(), &handle, cx, &mut task_outcomes);
                            let result = connection.as_mut().poll(cx);
                            let _ = poll_native_task_receipts(tasks.as_deref(), &handle, cx, &mut task_outcomes);
                            result
                        })).await.ok()
                    }
                    None => { drain_overflow = true; None }
                }
            }
        }
    };
    handle.close();
    if let Some(tasks) = &tasks { tasks.cancel_connection(&handle); }
    let until = deadline.or_else(|| tokio::time::Instant::now().checked_add(drain));
    let children_timed_out = match (until, tasks.as_ref()) {
        (Some(until), Some(_)) => tokio::time::timeout_at(until, std::future::poll_fn(|cx| {
            poll_task_notification!(cx);
            let tasks_done = poll_native_task_receipts(tasks.as_deref(), &handle, cx, &mut task_outcomes).is_ready();
            if tasks_done && handle.children_are_retired() { std::task::Poll::Ready(()) } else { std::task::Poll::Pending }
        })).await.is_err(),
        (Some(until), None) => tokio::time::timeout_at(until, handle.children_retired()).await.is_err(),
        (None, _) => { drain_overflow = true; false }
    };
    let native_range_primary = limit.is_none() && result.is_none() && drain_overflow;
    let native = match (limit, result) {
        (Some(reason), _) => Exit::Fail(map(ServerError::Limit(reason.into()))),
        (None, Some(Ok(()))) => Exit::Success(()),
        (None, None) if drain_overflow => Exit::Fail(map(ServerError::Limit("HTTP drain deadline exceeds clock range".into()))),
        (None, None) => Exit::Fail(map(ServerError::Limit(
            "HTTP connection drain deadline".into(),
        ))),
        (None, Some(Err(error))) => Exit::Fail(map(ServerError::Transport(Defect::new(error.to_string())))),
    };
    let (primary, native_secondary) = match refusal.or_else(|| handle.refusal()) {
        Some(error) => {
            let refusal = match error { crate::prepared_connection_retention::ConnectionRefusal::Memory(error) => ServerError::Memory(error), crate::prepared_connection_retention::ConnectionRefusal::Provider(error) => ServerError::Provider(error) };
            (Exit::Fail(map(refusal)), Some(native))
        }
        None if interrupted => (Exit::Interrupted, Some(native)),
        None => (native, None),
    };
    let mut completion = effect::PreparedCleanupAccumulator::from_reserved(primary, refusal_rest);
    if let Some(native) = native_secondary { completion.append_cleanup(native); }
    completion.append_cleanup(task_outcomes.finish());
    if drain_overflow && !native_range_primary {
        completion.append_cleanup(Exit::Fail(map(ServerError::Limit("HTTP drain deadline exceeds clock range".into()))));
    }
    if children_timed_out {
        completion.append_cleanup(Exit::Fail(map(ServerError::Limit("HTTP child drain deadline".into()))));
    }
    completion.finish()
}

enum ServingCompletion<E: Send + 'static> {
    Ordinary(Exit<(), E>),
    Prepared(effect::PreparedCleanupAccumulator<(), E>),
}
impl<E: Send + 'static> ServingCompletion<E> {
    fn append(&mut self, observed: Exit<(), E>) {
        match self {
            Self::Ordinary(exit) => {
                let primary = std::mem::replace(exit, Exit::Success(()));
                *exit = combine_completion(primary, observed);
            }
            Self::Prepared(completion) => completion.append_cleanup(observed),
        }
    }
    fn prepend(&mut self, primary: Exit<(), E>, spare: &mut Option<effect::CleanupFailures<E>>) {
        let previous = std::mem::replace(self, Self::Ordinary(Exit::Success(()))).finish();
        *self = match spare.take() {
            Some(rest) => {
                let mut completion =
                    effect::PreparedCleanupAccumulator::from_reserved(primary, rest);
                completion.append_cleanup(previous);
                Self::Prepared(completion)
            }
            None => Self::Ordinary(combine_completion(primary, previous)),
        };
    }
    fn finish(self) -> Exit<(), E> {
        match self {
            Self::Ordinary(exit) => exit,
            Self::Prepared(completion) => completion.finish(),
        }
    }
}

// Move raw channel failures out of pool gates before mapping each scalar cause.
fn append_orphan_channel_failure<E: Send + 'static>(
    completion: &mut effect::PreparedCleanupAccumulator<(), E>,
    failure: crate::prepared_streaming_channel::BodyChannelFailure,
    map: &Arc<dyn Fn(ServerError) -> E + Send + Sync>,
) {
    use crate::prepared_streaming_channel::BodyChannelFailure;
    fn append_queue<E: Send + 'static>(completion: &mut effect::PreparedCleanupAccumulator<(), E>, exit: Exit<(), effect::QueueError>, map: &Arc<dyn Fn(ServerError) -> E + Send + Sync>) {
        fn append_cause<E: Send + 'static>(completion: &mut effect::PreparedCleanupAccumulator<(), E>, cause: effect::CleanupFailure<effect::QueueError>, map: &Arc<dyn Fn(ServerError) -> E + Send + Sync>) {
            let exit = match cause { effect::CleanupFailure::ReleaseFailure(_) => Exit::Fail(map(ServerError::Disconnected)), effect::CleanupFailure::ReleaseDefect(defect) => Exit::Die(defect), effect::CleanupFailure::ReleaseInterrupted => Exit::Interrupted };
            completion.append_cleanup(exit);
        }
        match exit {
            Exit::Success(()) => {},
            Exit::Fail(_) => completion.append_cleanup(Exit::Fail(map(ServerError::Disconnected))),
            Exit::Die(defect) => completion.append_cleanup(Exit::Die(defect)),
            Exit::Interrupted => completion.append_cleanup(Exit::Interrupted),
            Exit::FinalizationFailed(details) => {
                match details.primary { effect::PrimaryOutcome::Succeeded => {}, effect::PrimaryOutcome::Failed(_) => completion.append_cleanup(Exit::Fail(map(ServerError::Disconnected))), effect::PrimaryOutcome::Defected(defect) => completion.append_cleanup(Exit::Die(defect)), effect::PrimaryOutcome::WasInterrupted => completion.append_cleanup(Exit::Interrupted) }
                append_cause(completion, details.first, map);
                for cause in details.rest { append_cause(completion, cause, map); }
            }
        }
    }
    match failure {
        BodyChannelFailure::Queue(exit) => append_queue(completion, exit, map),
        BodyChannelFailure::Memory(error) => completion.append_cleanup(Exit::Fail(map(ServerError::Memory(error)))),
        BodyChannelFailure::MemoryWithCleanup { primary, cleanup } => { completion.append_cleanup(Exit::Fail(map(ServerError::Memory(primary)))); append_queue(completion, cleanup, map); }
    }
}

// The accepted socket and its already-admitted connection authorities move into
// the scheduled graph only after the graph and its provider are admitted.
struct ConnectionInput<E: Send + 'static, R: Send + Sync + 'static> {
    socket: tokio::net::TcpStream,
    peer: SocketAddr,
    router: axum::Router,
    boundary: Arc<Boundary<E, R>>,
    serving: ConnectionServing,
    map: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    ticket: crate::prepared_connection_retention::ConnectionTicket<E, R>,
    cleanup: (effect::CleanupFailures<E>, effect::CleanupFailures<E>),
    retention: Arc<crate::prepared_connection_retention::ConnectionRetentionPool<E, R>>,
    reactor: tokio::runtime::Handle,
}
impl<E: Send + 'static, R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static> ConnectionInput<E, R> {
    async fn run(self, prepared_context: Option<effect::__private::prepared_graph::FutureContext<E>>) -> Exit<(), E> {
        let Self { socket, peer, router, boundary, serving, map, ticket, cleanup, retention, reactor } = self;
        let handle = match ticket.try_handle() {
            Ok(handle) => handle,
            Err(error) => return Exit::Fail(map(ServerError::Memory(error))),
        };
        let actor = match &boundary.native_tasks {
            Some(tasks) => match handle.try_clone() {
                Ok(owner) => Some((tasks.clone(), owner)),
                Err(error) => return Exit::Fail(map(ServerError::Memory(error))),
            },
            None => None,
        };
        let selected_reactor = reactor.clone();
        let (tracked, _owner) = ticket.start(move |_| {
            let future = serve_tracked_connection((socket, peer), router, boundary, serving, map, handle, reactor, cleanup, retention, prepared_context);
            crate::prepared_native_task::NativeConnectionActor::new(future, actor)
        });
        native_io(selected_reactor, tracked).await
    }
}

fn prepared_connection_recipe<E, R>(
    input: ConnectionInput<E, R>,
) -> impl effect::__private::prepared_graph::Description<Value = (), Error = E, Env = R>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner
        + effect::prepared_provider::HasPreparedProvider<(), E, effect::prepared_provider::TaskProvider>
        + Send + Sync + 'static,
{
    effect::effect_prepare! {
        scheduled;
        provider_kind: effect::prepared_provider::TaskProvider,
        body: move |_environment: effect::__private::prepared_graph::Environment<R>, context: effect::__private::prepared_graph::FutureContext<E>| input.run(Some(context)),
    }
}

trait PreparedConnectionDispatch<E: Send + 'static, R: Send + Sync + 'static>: Send + Sync {
    fn admit(&self, environment: Arc<R>) -> Result<effect::__private::prepared_graph::PendingOwnedInputGraph<ConnectionInput<E, R>, (), E>, effect::memory::MemoryError>;
}
#[allow(clippy::type_complexity)] // Retains the exact typed late-input graph.
struct PreparedConnectionPlan<F, D: effect::__private::prepared_graph::Description> {
    graph: effect::__private::prepared_graph::PreparedGraph<effect::__private::prepared_graph::InputRecipe<F, ConnectionInput<D::Error, D::Env>>, D>,
}
impl<F, D, E, R> PreparedConnectionDispatch<E, R> for PreparedConnectionPlan<F, D>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    D: effect::__private::prepared_graph::Description<Value = (), Error = E, Env = R>,
    F: Fn(ConnectionInput<E, R>) -> D + Send + Sync + 'static,
{
    fn admit(&self, environment: Arc<R>) -> Result<effect::__private::prepared_graph::PendingOwnedInputGraph<ConnectionInput<E, R>, (), E>, effect::memory::MemoryError> {
        self.graph.admit_owned_input(environment)
    }
}

fn prepare_connection_dispatch<E, R>(
    memory: &effect::memory::PreparedMemory<'static>,
    region: effect::memory::RegionId,
    capacity: usize,
    frames: effect::memory::FramePool<'static>,
    contract: effect::DynamicRunContract,
    cleanup: effect::PreparedCleanupConfig<E>,
    panics: effect::PreparedPanicPool,
) -> Result<Arc<dyn PreparedConnectionDispatch<E, R>>, effect::memory::MemoryError>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner
        + effect::prepared_provider::HasPreparedProvider<(), E, effect::prepared_provider::TaskProvider>
        + Send + Sync + 'static,
{
    let plan = effect::effect_prepare! {
        graph;
        input: prepared_connection_recipe::<E, R>,
        contract: contract,
        cleanup: cleanup,
        panic_diagnostics: panics,
    };
    let demand = plan.demand(capacity)?;
    if capacity != 0 {
        if let effect::DynamicRunContract::GeneratedGraph { contexts, listeners, .. } = contract
            && (contexts < 2 || listeners < 2) { return Err(effect::memory::MemoryError::NoSlot); }
        if frames.capacity() < demand.frames_per_run { return Err(effect::memory::MemoryError::NoSlot); }
        let available = frames.layout();
        if available.align() < demand.frame_layout.align() {
            return Err(effect::memory::MemoryError::UnsupportedAlignment { requested: demand.frame_layout.align(), supported: available.align() });
        }
        if available.size() < demand.frame_layout.size() {
            return Err(effect::memory::MemoryError::ValueTooLarge { requested: effect::memory::Bytes::new(demand.frame_layout.size()), capacity: effect::memory::Bytes::new(available.size()) });
        }
    }
    let graph = effect::effect_prepare! { prepare; plan: plan, memory: *memory, region: region, capacity: capacity, frames: frames }?;
    Ok(Arc::new(PreparedConnectionPlan { graph }))
}

// A driver and its pre-admitted terminal ticket move together until the
// connection has settled. No user future is polled while a metadata gate is held.
struct PreparedConnectionDriver<E: Send + 'static, R: Send + Sync + 'static> {
    driver: effect::PreparedOwnedDriver<(), E>,
    terminal: crate::prepared_terminal::TerminalRequest<E, R>,
}
pub(crate) struct PreparedConnectionDrivers<E: Send + 'static, R: Send + Sync + 'static> {
    records: effect::memory::FixedVector<'static, PreparedConnectionDriver<E, R>>,
    cursor: usize,
}
impl<E: Send + 'static, R: Send + Sync + 'static> PreparedConnectionDrivers<E, R> {
    fn poll_next(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Exit<(), E>> {
        use std::future::Future as _;
        let len = self.records.len();
        if len == 0 { self.cursor = 0; return std::task::Poll::Pending; }
        let start = self.cursor % len;
        let tail = len - start;
        for offset in 0..len {
            let index = if offset < tail { start + offset } else { offset - tail };
            let Some(record) = self.records.as_mut_slice().get_mut(index) else { continue };
            if let std::task::Poll::Ready((result, [first, second, terminal])) = std::pin::Pin::new(&mut record.driver).poll(cx) {
                let last = self.records.len() - 1;
                self.records.as_mut_slice().swap(index, last);
                let Some(record) = self.records.pop() else { return std::task::Poll::Ready(Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into())) };
                let remaining = self.records.len();
                self.cursor = if index < remaining && index + 1 < remaining {
                    index + 1
                } else if index < remaining {
                    // The old last element moved here and is next in the cycle.
                    index
                } else {
                    0
                };
                let installed = record.terminal.install_connection(terminal);
                // The installed terminal origin already retains the graph root
                // through every moved error and cleanup descendant.
                let (exit, execution) = unsafe { result.into_parts_unchecked() };
                drop((first, second, execution));
                if matches!(&exit, Exit::Success(())) {
                    // Success contains no business value or error to retain.
                    unsafe { installed.release_handled() };
                } else {
                    installed.retain_terminal();
                }
                return std::task::Poll::Ready(exit);
            }
        }
        self.cursor = if start + 1 == len { 0 } else { start + 1 };
        std::task::Poll::Pending
    }
}

pub(crate) fn connection_task_demand<E, R>(capacity: usize, contract: effect::DynamicRunContract) -> Result<crate::PreparedConnectionTaskDemand, effect::memory::MemoryError>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner
        + effect::prepared_provider::HasPreparedProvider<(), E, effect::prepared_provider::TaskProvider>
        + Send + Sync + 'static,
{
    let plan = effect::effect_prepare! { graph; input: prepared_connection_recipe::<E, R>, contract: contract };
    #[allow(clippy::type_complexity)] // Match the exact prepared recipe type for payload sizing.
    fn plan_payload<F, D: effect::__private::prepared_graph::Description>(_: &effect::__private::prepared_graph::PreparedGraphPlan<effect::__private::prepared_graph::InputRecipe<F, ConnectionInput<D::Error, D::Env>>, D>) -> usize {
        std::mem::size_of::<PreparedConnectionPlan<F, D>>()
    }
    let startup_payload = plan_payload(&plan).checked_add(std::mem::size_of::<PreparedConnectionSelection<E, R>>()).ok_or(effect::memory::MemoryError::ArithmeticOverflow)?;
    let graph = plan.demand(capacity)?;
    let bytes = std::alloc::Layout::array::<PreparedConnectionDriver<E, R>>(capacity)
        .map_err(|_| effect::memory::MemoryError::ArithmeticOverflow)?.size();
    Ok(crate::PreparedConnectionTaskDemand { graph, driver_bytes: effect::memory::Bytes::new(bytes), startup_arc_controls: 2, startup_arc_payload_bytes: effect::memory::Bytes::new(startup_payload) })
}

pub(crate) struct PreparedConnectionSelection<E: Send + 'static, R: Send + Sync + 'static> {
    dispatch: Arc<dyn PreparedConnectionDispatch<E, R>>,
    drivers: Mutex<Option<PreparedConnectionDrivers<E, R>>>,
}
impl<E, R> PreparedConnectionSelection<E, R>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner
        + effect::prepared_provider::HasPreparedProvider<(), E, effect::prepared_provider::TaskProvider>
        + Send + Sync + 'static,
{
    pub(crate) fn prepare(memory: &effect::memory::PreparedMemory<'static>, region: effect::memory::RegionId, capacity: usize, frames: effect::memory::FramePool<'static>, contract: effect::DynamicRunContract, cleanup: effect::PreparedCleanupConfig<E>, panics: effect::PreparedPanicPool) -> Result<Self, effect::memory::MemoryError> {
        let records = effect::memory::FixedVector::prepare(memory.allocator(region)?, capacity)?;
        let dispatch = prepare_connection_dispatch(memory, region, capacity, frames, contract, cleanup, panics)?;
        Ok(Self { dispatch, drivers: Mutex::new(Some(PreparedConnectionDrivers { records, cursor: 0 })) })
    }
}

async fn next_prepared_connection<E: Send + 'static, R: Send + Sync + 'static>(drivers: &mut Option<PreparedConnectionDrivers<E, R>>) -> Exit<(), E> {
    std::future::poll_fn(|cx| match drivers.as_mut() {
        Some(drivers) => drivers.poll_next(cx),
        None => std::task::Poll::Pending,
    }).await
}

async fn serve_connections<E, R>(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    serving: ConnectionServing,
    execution: (Scope<E>, NativeContext, Arc<R>),
    map: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    boundary: Arc<Boundary<E, R>>,
    endpoint_hosting: Option<
        Arc<crate::prepared_connection_retention::ConnectionRetentionPool<E, R>>,
    >,
) -> Exit<(), E>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    let (protocols, stop, drain, http1_header_timeout, prepared_http_limits, prepared_http2_reset_limits, prepared_http_timeouts) = serving;
    let (scope, context, env) = execution;
    let mut connections = if boundary.connection_tasks.is_none() { Some(futures_util::stream::FuturesUnordered::new()) } else { None };
    let mut selected_drivers = match &boundary.connection_tasks {
        Some(selection) => match selection.drivers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            Some(drivers) => Some(drivers),
            None => return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()),
        },
        None => None,
    };
    let prepared = boundary
        .serving_cleanup
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let (mut result, mut spare) = match prepared {
        Some([rest, spare]) => (
            ServingCompletion::Prepared(effect::PreparedCleanupAccumulator::from_reserved(
                Exit::Success(()),
                rest,
            )),
            Some(spare),
        ),
        None => (ServingCompletion::Ordinary(Exit::Success(())), None),
    };
    if boundary.body_pool.as_ref().is_some_and(|pool| pool.channels_configured()) && !matches!(&result, ServingCompletion::Prepared(_)) {
        return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into());
    }
    let mut cleanup_waiter = boundary
        .cleanup_waiter
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    loop {
        if selected_drivers.as_ref().is_some_and(|drivers| drivers.records.capacity() == 0) {
            stop.cancelled().await;
            break;
        }
        if selected_drivers.as_ref().is_some_and(|drivers| drivers.records.len() == drivers.records.capacity()) {
            select! {
                () = stop.cancelled() => break,
                exit = next_prepared_connection(&mut selected_drivers) => result.append(exit),
            }
            continue;
        }
        let (ticket, connection_cleanup) = if let Some(pool) = &endpoint_hosting {
            let Some(waiter) = &mut cleanup_waiter else {
                result.prepend(
                    Exit::Die(Defect::new("prepared cleanup waiter missing")),
                    &mut spare,
                );
                break;
            };
            loop {
                let ticket = select! {
                    () = stop.cancelled() => break (None, None),
                    Some(connection) = async { match connections.as_mut() { Some(connections) => connections.next().await, None => std::future::pending().await } }, if connections.as_ref().is_some_and(|connections| !connections.is_empty()) => {
                        let connection: effect::Fiber<(), E> = connection;
                        let exit = context.run(effect_uninterruptible! { program: connection.join(), }, env.clone()).await;
                        result.append(exit);
                        continue;
                    },
                    exit = next_prepared_connection(&mut selected_drivers), if selected_drivers.as_ref().is_some_and(|drivers| !drivers.records.is_empty()) => { result.append(exit); continue; },
                    admitted = pool.reserve_when_available() => match admitted {
                        Ok(ticket) => ticket,
                        Err(error) => { result.prepend(Exit::Fail(map(ServerError::Memory(error))), &mut spare); break (None, None); }
                    },
                };
                match waiter.try_reserve_pair() {
                    Ok([first, second]) => break (Some(ticket), Some((first, second))),
                    Err(effect::memory::MemoryError::NoSlot) => {
                        // Do not hold the connection ticket or either partial buffer while waiting.
                        drop(ticket);
                        select! {
                            () = stop.cancelled() => break (None, None),
                            Some(connection) = async { match connections.as_mut() { Some(connections) => connections.next().await, None => std::future::pending().await } }, if connections.as_ref().is_some_and(|connections| !connections.is_empty()) => {
                                let connection: effect::Fiber<(), E> = connection;
                                let exit = context.run(effect_uninterruptible! { program: connection.join(), }, env.clone()).await;
                                result.append(exit);
                            },
                            exit = next_prepared_connection(&mut selected_drivers), if selected_drivers.as_ref().is_some_and(|drivers| !drivers.records.is_empty()) => result.append(exit),
                            available = waiter.wait_pair_available() => if let Err(error) = available {
                                result.prepend(Exit::Fail(map(ServerError::Memory(error))), &mut spare);
                                break (None, None);
                            },
                        }
                    }
                    Err(error) => {
                        drop(ticket);
                        result.prepend(Exit::Fail(map(ServerError::Memory(error))), &mut spare);
                        break (None, None);
                    }
                }
            }
        } else {
            (None, None)
        };
        if endpoint_hosting.is_some() && ticket.is_none() {
            break;
        }
        let accepted = select! {
            () = stop.cancelled() => break,
            Some(connection)=async { match connections.as_mut() { Some(connections) => connections.next().await, None => std::future::pending().await } },if connections.as_ref().is_some_and(|connections| !connections.is_empty())=>{
                let connection:effect::Fiber<(),E>=connection;
                let exit=context.run(effect_uninterruptible! { program: connection.join(), },env.clone()).await;
                result.append(exit);continue;
            },
            exit = next_prepared_connection(&mut selected_drivers), if selected_drivers.as_ref().is_some_and(|drivers| !drivers.records.is_empty()) => { result.append(exit); continue; },
            accepted = listener.accept() => accepted
        };
        let (socket, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                result.prepend(Exit::Fail(map(ServerError::Io(error))), &mut spare);
                break;
            }
        };
        if let Some(ticket) = &ticket { ticket.accepted(tokio::time::Instant::now()); }
        // This socket has not been passed to a parser or task. A transient
        // physical admission refusal closes it and releases all partial owners.
        let selected_admission = if let Some(selection) = &boundary.connection_tasks {
            let Some(terminal) = boundary.terminal.as_ref() else {
                drop(socket); drop(ticket); drop(connection_cleanup);
                result.prepend(Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()), &mut spare); break;
            };
            let terminal = match terminal.reserve_request() {
                Ok(value) => value,
                Err(effect::memory::MemoryError::NoSlot | effect::memory::MemoryError::Busy | effect::memory::MemoryError::CapacityExceeded { .. }) => { drop(socket); drop(ticket); drop(connection_cleanup); continue; }
                Err(error) => { drop(socket); drop(ticket); drop(connection_cleanup); result.prepend(Exit::Fail(map(ServerError::Memory(error))), &mut spare); break; }
            };
            let pending = match selection.dispatch.admit(env.clone()) {
                Ok(value) => value,
                Err(effect::memory::MemoryError::NoSlot | effect::memory::MemoryError::Busy) => { drop(socket); drop(terminal); drop(ticket); drop(connection_cleanup); continue; }
                Err(error) => { drop(socket); drop(terminal); drop(ticket); drop(connection_cleanup); result.prepend(Exit::Fail(map(ServerError::Memory(error))), &mut spare); break; }
            };
            Some((terminal, pending))
        } else { None };
        if let (Some(pool), Some(ticket)) = (&endpoint_hosting, &ticket)
            && let Err(error) = pool.admit_quota(ticket) {
                drop(socket); drop(selected_admission); drop(connection_cleanup);
                if matches!(error, effect::memory::MemoryError::NoSlot | effect::memory::MemoryError::Busy | effect::memory::MemoryError::CapacityExceeded { .. }) { continue; }
                result.prepend(Exit::Fail(map(ServerError::Memory(error))), &mut spare);
                break;
        }
        if let Some((terminal, pending)) = selected_admission {
            let Some(ticket) = ticket else { result.prepend(Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()), &mut spare); break; };
            let Some(cleanup) = connection_cleanup else { result.prepend(Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()), &mut spare); break; };
            let Some(retention) = endpoint_hosting.clone() else { result.prepend(Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()), &mut spare); break; };
            let input = ConnectionInput { socket, peer, router: router.clone(), boundary: boundary.clone(), serving: (protocols, stop.clone(), drain, http1_header_timeout, prepared_http_limits, prepared_http2_reset_limits, prepared_http_timeouts), map: map.clone(), ticket, cleanup, retention, reactor: env.tokio().clone() };
            let record = PreparedConnectionDriver { driver: pending.publish_input(input), terminal };
            if let Some(drivers) = selected_drivers.as_mut() {
                if let Err(record) = drivers.records.push(record) { drop(record); result.prepend(Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()), &mut spare); break; }
            } else { drop(record); result.prepend(Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()), &mut spare); break; }
            continue;
        }
        let retention = endpoint_hosting.clone();
        let router = router.clone();
        let stop = stop.clone();
        let map = map.clone();
        let boundary = boundary.clone();
        let connection = effect_uninterruptible! {
            program: crate::endpoint::native_once(move |env: Arc<R>, _| async move {
            if let Some(ticket) = ticket {
                let Some(cleanup) = connection_cleanup else { return Exit::Die(Defect::new("prepared connection cleanup missing")); };
                let Some(retention) = retention else { return Exit::Die(effect::LibraryDefect::PreparedConstructionUnbound.into()); };
                return ConnectionInput {
                    socket, peer, router, boundary,
                    serving: (protocols, stop, drain, http1_header_timeout, prepared_http_limits, prepared_http2_reset_limits, prepared_http_timeouts),
                    map, ticket, cleanup, retention, reactor: env.tokio().clone(),
                }.run(None).await;
            }
            native_io(env.tokio().clone(), async move {
                let mut builder = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
                configure_http1_header_timeout(&mut builder, http1_header_timeout);
                let builder = match protocols {
                    #[cfg(feature = "http1")]
                    HttpProtocols::Http1 => builder.http1_only(),
                    #[cfg(feature = "http2")]
                    HttpProtocols::Http2 => builder.http2_only(),
                    HttpProtocols::Http1And2 => builder,
                    #[cfg(not(feature = "http1"))]
                    HttpProtocols::Http1 => return Exit::Fail(map(ServerError::Configuration("HTTP/1 disabled".into()))),
                    #[cfg(not(feature = "http2"))]
                    HttpProtocols::Http2 => return Exit::Fail(map(ServerError::Configuration("HTTP/2 disabled".into()))),
                };
                let service=tower::service_fn(move |mut request: http::Request<hyper::body::Incoming>| {let router=router.clone();let boundary=boundary.clone();request.extensions_mut().insert(peer);let request=request.map(Body::new);async move {Ok::<_,Infallible>(boundary.dispatch_native(request,router,None).await.0)}});
                let service = hyper_util::service::TowerToHyperService::new(service);
                let connection = builder.serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(socket), service); pin!(connection);
                let result = select! {
                    result = &mut connection => Some(result), () = stop.cancelled() => {
                    connection.as_mut().graceful_shutdown(); tokio::time::timeout(drain, &mut connection).await.ok()
                } };
                match result { Some(Ok(())) => Exit::Success(()), None=>Exit::Fail(map(ServerError::Limit("HTTP connection drain deadline".into()))), Some(Err(error)) => Exit::Fail(map(ServerError::Transport(Defect::new(error.to_string())))) }
            }).await
        }), };
        let admitted = context.run(scope.fork(connection), env.clone()).await;
        match admitted {
            Exit::Success(connection) => if let Some(connections) = &mut connections { connections.push(async move {
                let _ = connection.settled().await;
                connection
            }); },
            Exit::Fail(error) => {
                result.prepend(Exit::Fail(error), &mut spare);
                break;
            }
            Exit::Die(defect) => {
                result.prepend(Exit::Die(defect), &mut spare);
                break;
            }
            Exit::Interrupted => {
                result.prepend(Exit::Interrupted, &mut spare);
                break;
            }
            Exit::FinalizationFailed(failure) => {
                result.prepend(Exit::FinalizationFailed(failure), &mut spare);
                break;
            }
        }
    }
    let _ = stop.request();
    while let Some(connection) = match connections.as_mut() { Some(connections) => connections.next().await, None => None } {
        let exit = context
            .run(
                effect_uninterruptible! { program: connection.join(), },
                env.clone(),
            )
            .await;
        result.append(exit);
    }
    while selected_drivers.as_ref().is_some_and(|drivers| !drivers.records.is_empty()) {
        result.append(next_prepared_connection(&mut selected_drivers).await);
    }
    if let (Some(pool), ServingCompletion::Prepared(completion)) = (&boundary.body_pool, &mut result) {
        while let Some((_phase, failure)) = pool.take_channel_failure() {
            append_orphan_channel_failure(completion, failure, &map);
        }
    }
    result.finish()
}

/// Typed host failure renderer.
type ErrorRenderer<E, R> = dyn Fn(E) -> Effect<Reply<E, R>, E, R> + Send + Sync;

/// Observer of the selected environment and terminal request outcome.
type CompletionObserver<E, R> =
    dyn Fn(&R, &Request, &RequestCompletion<E>) -> Effect<(), E, R> + Send + Sync;

/// Native router configuration without endpoint execution.
type RouterConfigurator = dyn Fn(axum::Router) -> Result<axum::Router, ServerError> + Send + Sync;
