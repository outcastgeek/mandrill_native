//! Scoped axum host and the single native request interpretation boundary.

use crate::{
    BodyCompletion, BodyOutcome, BodyPolicy, Endpoint, Pipeline, Reply, Request, RequestCompletion,
    ResponseHeaders, Router, ServerError,
};
use crate::{
    effect_die, effect_future, effect_recover, effect_resource, effect_scope, effect_try_sync,
    effect_uninterruptible, endpoint, pipeline, registered,
};
use axum::body::Body;
use axum::extract::FromRequestParts;
use effect::effect;
use effect::{
    Cancellation, Defect, Effect, Exit, HasCancellation, HasTaskSpawner, NativeContext, Scope,
};
use futures_util::StreamExt;
use http::{HeaderValue, Method, StatusCode};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, watch};
use tokio::{pin, select};
use tower::service_fn;

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
}
/// Selection handshake gates retain their existing owners and send order.
struct NativeSelectionGates<R> {
    /// Observe the chosen request/environment before transferring the routed driver.
    selected: watch::Receiver<Option<Arc<NativeSelection<R>>>>,
    /// Release terminal execution with its decorated native context.
    start: oneshot::Sender<NativeContext>,
    /// Release matched authentication with the outer selection context.
    selection_start: oneshot::Sender<NativeContext>,
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
    observers: Vec<Arc<CompletionObserver<E, R>>>,
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
            observers: Vec::new(),
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
        self.observers.push(Arc::new(observe));
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
    /// Ordered native auth/culture/correlation stages before resolving R.
    pub(crate) request_hooks: Vec<RequestHook<E, R>>,
    /// Ordered mutable native stages run before axum matches a route.
    routing_hooks: Vec<RoutingHook<E, R>>,
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
            request_hooks: Vec::new(),
            routing_hooks: Vec::new(),
            response_hooks: Vec::new(),
            middleware: Pipeline::new(),
            decorators: Vec::new(),
            selection_decorators: Vec::new(),
            configure: Vec::new(),
        }
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
        self.routing_hooks.push(Arc::new(stage));
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
    routing_hooks: Vec<RoutingHook<E, R>>,
    /// All-response stages under selected request ownership.
    response_hooks: Vec<ResponseHook<E, R>>,
    /// Outer Effect middleware around short circuits and endpoint bodies.
    middleware: Pipeline<E, R>,
    /// Integrations wrap complete scope/finalization and terminal observation.
    decorators: Vec<RequestDecorator<E, R>>,
    /// Root-R outer context begins before native authentication and request selection.
    selection_decorators: Vec<RequestDecorator<E, R>>,
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
struct OuterDelivery(BodyCompletion);
/// Upstream completion is distinct from final layered transport completion.
#[derive(Clone)]
struct EndpointDelivery(BodyCompletion);
/// Selected environment handshake before endpoint work starts; native short circuits
/// perform the same post-route selection in the retained outer owner.
struct NativeSelection<R> {
    /// Final routed/native metadata and its checked body owner.
    request: Request,
    /// Actual selected request capabilities.
    environment: Arc<R>,
    /// A failed selection uses original services without rebasing root cancellation.
    selected: bool,
}
/// Request-local bridge between native routing and the one retained outer request scope.
struct NativeSlot<E: Send + 'static, R: Send + Sync + 'static> {
    /// Outer owner retains routing and endpoint fibers through body completion.
    scope: Scope<E>,
    /// Existing interpreter context admits native routing without creating a runtime.
    context: NativeContext,
    /// Route selection publishes R before any endpoint/body program executes.
    selection: watch::Sender<Option<Arc<NativeSelection<R>>>>,
    /// Classification is published before authentication or request-environment resolution.
    classified: watch::Sender<Option<Request>>,
    /// Root-R outer decoration supplies context before the first authentication effect.
    selection_start: Arc<Mutex<Option<oneshot::Receiver<NativeContext>>>>,
    /// Complete outer decoration supplies the endpoint's final trace/context.
    start: Arc<Mutex<Option<oneshot::Receiver<NativeContext>>>>,
    /// Final request metadata shared with native response decoration.
    request: Arc<Mutex<Request>>,
    /// One endpoint driver is joined explicitly before outer terminal observation.
    inner: Arc<Mutex<Option<effect::Fiber<(), E>>>>,
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
        }
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
    ) -> http::Response<Body> {
        let routing =
            crate::RoutingRequest::from_native(native, self.context.cancellation().child());
        let request = routing.request();
        request.insert_extension(ResponseHeaders::default());
        let mut disconnected = RequestDisconnect {
            request: request.clone(),
            armed: true,
        };
        let (send, receive) = oneshot::channel();
        let output = Arc::new(Mutex::new(Some(send)));
        let owner = self.clone();
        let routing = Arc::new(Mutex::new(Some(routing)));
        let task = effect_scope! {
        body: move |scope| {
                    let owner = owner.clone();
                    let output = output.clone();
                    let routing = routing.clone();
                    let router = router.clone();
                    crate::endpoint::native_once(move |env: Arc<R>, context| async move {
                        let routing = routing
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        let Some(routing) = routing else {
                            return Exit::Die(Defect::new("native routing owner already consumed"));
                        };
                        let cancellation = routing.request().cancellation().clone();
                        let interrupted = context.cancellation().clone();
                        let running =
                            owner.execute_native(routing, router, scope, output, context, env.clone());
                        pin!(running);
                        native_io(env.tokio().clone(),async {select! {exit=&mut running=>exit,()=cancellation.cancelled()=>{let _=interrupted.request();running.await}}}).await
                    })
                },
        };
        match self
            .context
            // Native body completion can precede request cleanup. Retain this admitted
            // owner through host-scope closure; explicit disconnect cancellation still
            // reaches its nested work through the existing request token.
            .run(
                self.scope.fork(effect_uninterruptible! { program: task, }),
                self.environment.clone(),
            )
            .await
        {
            Exit::Success(_) => match receive.await {
                Ok(response) => {
                    disconnected.armed = false;
                    response
                }
                Err(_) => terminal_response(StatusCode::INTERNAL_SERVER_ERROR),
            },
            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
                terminal_response(StatusCode::SERVICE_UNAVAILABLE)
            }
        }
    }
    /// Run native transforms once before matching; select R after native auth/culture,
    /// then let outer decorators own routing/body work and typed cleanup settlement.
    async fn execute_native(
        self: Arc<Self>,
        routing: crate::RoutingRequest,
        router: axum::Router,
        scope: Scope<E>,
        output: Arc<Mutex<Option<oneshot::Sender<http::Response<Body>>>>>,
        context: NativeContext,
        root: Arc<R>,
    ) -> Exit<(), E> {
        let mut request = routing.request();
        let mut routing = Some(routing);
        let mut failure = None;
        for stage in &self.routing_hooks {
            let Some(current) = routing.take() else {
                return Exit::Die(Defect::new("native routing owner missing"));
            };
            match context
                .run(stage(current, scope.clone()), root.clone())
                .await
            {
                Exit::Success(next) => {
                    request = next.request();
                    routing = Some(next);
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
        request.insert_extension(OuterDelivery(BodyCompletion::new(
            request.cancellation().clone(),
        )));
        let (selection, selected) = watch::channel(None::<Arc<NativeSelection<R>>>);
        let (start, gate) = oneshot::channel();
        let (classified, mut classification) = watch::channel(None::<Request>);
        let (selection_start, selection_gate) = oneshot::channel();
        let slot = NativeSlot {
            scope: scope.clone(),
            context: context.clone(),
            selection,
            classified,
            selection_start: Arc::new(Mutex::new(Some(selection_gate))),
            start: Arc::new(Mutex::new(Some(gate))),
            request: Arc::new(Mutex::new(request.clone())),
            inner: Arc::new(Mutex::new(None)),
        };
        let mut ready = None;
        let mut routed = None;
        let mut matched = false;
        if failure.is_none() {
            let Some(routing) = routing.take() else {
                return Exit::Die(Defect::new("native routing owner missing"));
            };
            let mut native = routing.into_native();
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
                        let classified = classification.borrow_and_update().clone();
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
        if !matched {
            request.insert_extension(RouteKind::Native);
        }
        let owner = self.clone();
        let decorators = self.selection_decorators.clone();
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
                        },
                        (scope, decorated, root),
                        output,
                        slot,
                        NativeSelectionGates {
                            selected,
                            start,
                            selection_start,
                        },
                        matched,
                    )
                    .await
            });
        for decorate in decorators.iter().rev() {
            selection_program = decorate(decorated_request.clone(), selection_program);
        }
        context.run(selection_program, root).await
    }
    /// Start matched authentication under the outer context, then retain that context across
    /// selected-R execution and the uniquely owned terminal/cleanup outcome.
    async fn complete_native_selection(
        self: Arc<Self>,
        state: NativeResponseState<E>,
        execution: (Scope<E>, NativeContext, Arc<R>),
        output: Arc<Mutex<Option<oneshot::Sender<http::Response<Body>>>>>,
        slot: NativeSlot<E, R>,
        gates: NativeSelectionGates<R>,
        matched: bool,
    ) -> Exit<(), E> {
        let NativeResponseState {
            mut request,
            mut ready,
            mut routed,
            mut failure,
        } = state;
        let (scope, context, root) = execution;
        let NativeSelectionGates {
            mut selected,
            start,
            selection_start,
        } = gates;
        let mut chosen = None;
        if matched {
            let _ = selection_start.send(context.clone());
            if let Some(fiber) = routed.take() {
                let observing = context.run(fiber.join(), root.clone());
                pin!(observing);
                loop {
                    let selection = selected.borrow_and_update().clone();
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
        let (environment, selected_request) = if let Some(chosen) = chosen {
            request = chosen.request.clone();
            (chosen.environment.clone(), chosen.selected)
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
        let body_state = Arc::new(Mutex::new(None::<BodyCompletion>));
        let final_state = body_state.clone();
        let response_request = request.clone();
        let policy = self.policy.clone();
        let hooks = self.response_hooks.clone();
        let observation_output = output.clone();
        let close_scope = scope.clone();
        let inner = slot.inner.clone();
        let mut complete = crate::endpoint::native_once(
            move |environment: Arc<R>, decorated| async move {
                let _ = start.send(decorated.clone());
                let mut exit = finish_native_response(
                    NativeResponseState {
                        request: response_request.clone(),
                        ready,
                        routed,
                        failure,
                    },
                    (scope, &decorated, environment.clone()),
                    output,
                    body_state,
                    policy.clone(),
                    hooks,
                )
                .await;
                if final_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_none()
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
                let endpoint = inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let Some(endpoint) = endpoint {
                    let endpoint = decorated
                        .run(
                            effect_uninterruptible! { program: endpoint.join(), },
                            environment.clone(),
                        )
                        .await;
                    exit = combine_completion(endpoint, exit);
                }
                let closed = decorated
                    .run(
                        effect_uninterruptible! { program: effect::__private::close_scope_owned(close_scope), },
                        environment.clone(),
                    )
                    .await;
                exit = combine_completion(exit, closed);
                let body = final_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_ref()
                    .map(BodyCompletion::outcome)
                    .unwrap_or(BodyOutcome::Open);
                let completion = RequestCompletion {
                    exit,
                    body,
                    committed: observation_output
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_none(),
                };
                let observations = policy
                    .observers
                    .iter()
                    .map(|observe| observe(&environment, &response_request, &completion))
                    .collect::<Vec<_>>();
                let mut exit = completion.exit;
                for observation in observations {
                    let observed = decorated
                        .run(
                            effect_uninterruptible! { program: observation, },
                            environment.clone(),
                        )
                        .await;
                    exit = combine_completion(exit, observed);
                }
                exit
            },
        );
        for decorate in self.decorators.iter().rev() {
            complete = decorate(request.clone(), complete);
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
        select! {exit=&mut running=>exit,()=request_cancel.cancelled()=>{if selected_request{let _=cancellation.request();}running.await}}
    }
    /// Admit managed request work before returning its native response observer.
    async fn dispatch(
        self: Arc<Self>,
        mut native: http::Request<Body>,
        endpoint: Endpoint<E, R>,
        metadata: crate::Registration,
        captures: crate::Captures,
    ) -> http::Response<Body> {
        let slot = native.extensions_mut().remove::<NativeSlot<E, R>>();
        let cancellation = slot
            .as_ref()
            .map(|slot| {
                slot.request
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .cancellation()
                    .clone()
            })
            .unwrap_or_else(|| self.context.cancellation().child());
        let request = Request::from_native(native, cancellation);
        if let Some(slot) = &slot {
            *slot
                .request
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = request.clone();
        }
        request.insert_extension(RouteKind::Endpoint);
        request.insert_extension(metadata);
        request.insert_extension(crate::route::NativeCaptures(captures));
        if !request.with_extensions(|extensions| extensions.get::<ResponseHeaders>().is_some()) {
            request.insert_extension(ResponseHeaders::default());
        }
        let mut disconnected = RequestDisconnect {
            request: request.clone(),
            armed: true,
        };
        let (send, receive) = oneshot::channel();
        let output = Arc::new(Mutex::new(Some(send)));
        let owner = self.clone();
        let request_for_work = request.clone();
        let work_slot = slot.clone();
        let program = crate::endpoint::native_once(move |env: Arc<R>, context| async move {
            let disconnected = request_for_work.cancellation().clone();
            let interrupted = context.cancellation().clone();
            native_io(env.tokio().clone(), async move {
                let executing = owner.execute(request_for_work, endpoint, output, context,work_slot); pin!(executing);
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
                    *slot
                        .inner
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(fiber);
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
        output: Arc<Mutex<Option<oneshot::Sender<http::Response<Body>>>>>,
        context: NativeContext,
        slot: Option<NativeSlot<E, R>>,
    ) -> Exit<(), E> {
        // Classification is known before starting the root-R outer decorator. The endpoint
        // waits for its context so authentication SQL and resolution inherit request tracing.
        let context = if let Some(slot) = &slot {
            slot.classified.send_replace(Some(request.clone()));
            let gate = slot
                .selection_start
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
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
        let context = if let Some(slot) = &slot {
            slot.selection.send_replace(Some(Arc::new(NativeSelection {
                request: request.clone(),
                environment: selected.clone(),
                selected: selected_request,
            })));
            let start = slot
                .start
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
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
        let endpoint_body = endpoint.body.clone();
        let on_error = endpoint
            .on_error
            .clone()
            .unwrap_or_else(|| self.policy.on_error.clone());
        let policy = self.policy.clone();
        let body_state = Arc::new(Mutex::new(None::<BodyCompletion>));
        let final_state = body_state.clone();
        let boundary_request = request.clone();
        let middleware = pipeline! { source: self.middleware.clone(), include: endpoint.pipeline, };
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
                    let continuation = crate::endpoint::ReplyContinuation(Arc::new(move |reply| {
                        deliver(
                            reply,
                            response_request.clone(),
                            response_scope.clone(),
                            response_policy.clone(),
                            response_output.clone(),
                            response_state.clone(),
                        )
                    }));
                    request.insert_extension(continuation.clone());
                    // A downstream endpoint scope closes before outer after/on-exit stages.
                    // Its reply still awaits native delivery within this same scope.
                    let endpoint_request = request.clone();
                    let recovery_output = output.clone();
                    let endpoint_body = endpoint_body.clone();
                    let input_error = policy.native_error.clone();
                    let reply = effect_scope! {
        body: move |endpoint_scope| {
                        let delivery_scope = endpoint_scope.clone();
                        let continuation = crate::endpoint::ReplyContinuation(Arc::new({
                            let request = endpoint_request.clone();
                            let output = output.clone();
                            let policy = policy.clone();
                            let state = state.clone();
                            move |reply| {
                                deliver(
                                    reply,
                                    request.clone(),
                                    delivery_scope.clone(),
                                    policy.clone(),
                                    output.clone(),
                                    state.clone(),
                                )
                            }
                        }));
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
         program: endpoint_body(endpoint_scope),
         body: move |reply| (continuation.0)(reply),
         }
                    },
        };
                    let terminal: crate::Continuation<E, R> = Arc::new(move || reply.clone());
                    let on_error = on_error.clone();
                    effect_recover! {
                        program: middleware.apply(request, terminal), recover: move |error| {
                        if recovery_output
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .is_none()
                        {
                            return effect_try_sync! { once; body: { let __dsl_result = Err(error); move || __dsl_result }, on_error: ::core::convert::identity, };
                        }
                        let continuation = continuation.clone();
                        effect! {
         bind;
         program: on_error(error),
         body: move |reply| (continuation.0)(reply),
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
                let body = final_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_ref()
                    .map(BodyCompletion::outcome)
                    .unwrap_or(BodyOutcome::Open);
                let committed = completion_output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_none();
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
) -> Exit<crate::reply::PreparedReply<E, R>, E> {
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
                    .prepare(request, scope, policy.body, policy.native_error.clone()), },
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
    output: Arc<Mutex<Option<oneshot::Sender<http::Response<Body>>>>>,
    body_state: Arc<Mutex<Option<BodyCompletion>>>,
    policy: ServerPolicy<E, R>,
    hooks: Vec<ResponseHook<E, R>>,
) -> Exit<(), E> {
    let NativeResponseState {
        request,
        ready,
        routed,
        failure,
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
        Exit::Fail(error) => match native_error_reply(
            error,
            request.clone(),
            scope.clone(),
            &policy,
            context,
            env.clone(),
        )
        .await
        {
            Exit::Success(prepared) => (prepared.response, Some(prepared.finish)),
            Exit::Fail(error) => return Exit::Fail(error),
            Exit::Die(defect) => return Exit::Die(defect),
            Exit::Interrupted => return Exit::Interrupted,
            Exit::FinalizationFailed(failed) => return Exit::FinalizationFailed(failed),
        },
        Exit::Die(defect) => return Exit::Die(defect),
        Exit::Interrupted => return Exit::Interrupted,
        Exit::FinalizationFailed(failed) => return Exit::FinalizationFailed(failed),
    };
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
    for hook in hooks {
        match context
            .run(hook(request.clone(), response), env.clone())
            .await
        {
            Exit::Success(next) => response = next,
            Exit::Fail(error) => {
                match native_error_reply(
                    error,
                    request.clone(),
                    scope.clone(),
                    &policy,
                    context,
                    env.clone(),
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
    let completion = request
        .with_extensions(|extensions| {
            extensions
                .get::<OuterDelivery>()
                .map(|delivery| delivery.0.clone())
        })
        .unwrap_or_else(|| BodyCompletion::new(request.cancellation().clone()));
    let body = std::mem::replace(response.body_mut(), Body::empty());
    *response.body_mut() = Body::new(crate::body::ObservedBody {
        body,
        completion: completion.clone(),
    });
    *body_state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(completion.clone());
    completion.arm();
    let sender = output
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    match sender {
        Some(sender) => {
            if sender.send(response).is_err() {
                let _ = request.cancellation().request();
            }
        }
        None => return Exit::Die(Defect::new("native response already committed")),
    }
    let produced = match producer {
        Some(producer) => context.run(producer, env.clone()).await,
        None => Exit::Success(()),
    };
    let delivered = match completion.wait().await {
        BodyOutcome::Completed => Exit::Success(()),
        BodyOutcome::Disconnected => Exit::Interrupted,
        BodyOutcome::Failed => Exit::Fail((policy.native_error)(ServerError::Transport(
            Defect::new("native response body failed"),
        ))),
        BodyOutcome::Open => Exit::Die(Defect::new("native body completion did not settle")),
    };
    combine_completion(produced, delivered)
}

/// Deliver one consumed reply, then await producer/native completion before request scope closure.
fn deliver<E: Send + 'static, R: Send + Sync + 'static>(
    mut reply: Reply<E, R>,
    request: Request,
    scope: Scope<E>,
    policy: ServerPolicy<E, R>,
    output: Arc<Mutex<Option<oneshot::Sender<http::Response<Body>>>>>,
    state: Arc<Mutex<Option<BodyCompletion>>>,
) -> Effect<(), E, R> {
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
               *state
                   .lock()
                   .unwrap_or_else(std::sync::PoisonError::into_inner) =
                   Some(prepared.completion.clone());
               request.insert_extension(EndpointDelivery(prepared.completion.clone()));
               if !request.with_extensions(|extensions| extensions.get::<OuterDelivery>().is_some()) {
                   prepared.completion.arm();
               }
               let sender = output
                   .lock()
                   .unwrap_or_else(std::sync::PoisonError::into_inner)
                   .take();
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
                               Some(outer) => effect_resource! {
                                   ensuring;
                                   program: prepared.finish,
                                   release: move |_| {
                                   let gate = outer.0.clone();
                                   effect_future! {
                                       body: move |_| {
                                       let gate = gate.clone();
                                       async move {
                                           let _ = gate.wait().await;
                                       }
                                   }, }
                                   },
                               },
                               None => prepared.finish,
                           }
                       }
                   }
                   None => effect_die! { defect: effect::Defect::new("response already committed"), },
               }
           },
    }
}
/// Preserve original primary plus observer failures using the existing ordered cleanup carrier.
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
                rest: Vec::new(),
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
        let description = Arc::new(self);
        let use_host = Arc::new(use_host);
        effect_scope! {
        body: move |scope| {
                    let server = description.as_ref().clone();
                    let use_host = use_host.clone();
                    crate::endpoint::native_once(move |env: Arc<R>, context| async move {
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
                        let boundary = Arc::new(Boundary {
                            scope: scope.clone(),
                            context: context.clone(),
                            environment: env.clone(),
                            policy: server.policy.clone(),
                            binding: server.environment,
                            hooks: server.request_hooks,
                            routing_hooks: server.routing_hooks,
                            response_hooks: server.response_hooks,
                            middleware: server.middleware,
                            decorators: server.decorators,
                            selection_decorators: server.selection_decorators,
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
                                        (protocols, native_stop, drain),
                                        (connection_scope, context, env),
                                        map,
                                        boundary,
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
                                    return match interrupted {Exit::Success(outcome)=>append_host_cleanup(primary,outcome),Exit::Fail(error)=>append_cleanup_value(primary,effect::CleanupFailure::ReleaseFailure(error)),Exit::Die(defect)=>append_cleanup_value(primary,effect::CleanupFailure::ReleaseDefect(defect)),Exit::Interrupted=>primary,Exit::FinalizationFailed(failure)=>append_host_cleanup(primary,Exit::<A,E>::FinalizationFailed(failure))};
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
                                Exit::Fail(error) => append_cleanup_value(
                                    failure,
                                    effect::CleanupFailure::ReleaseFailure(error),
                                ),
                                Exit::Die(defect) => append_cleanup_value(
                                    failure,
                                    effect::CleanupFailure::ReleaseDefect(defect),
                                ),
                                Exit::Interrupted => append_cleanup_value(
                                    failure,
                                    effect::CleanupFailure::ReleaseInterrupted,
                                ),
                                Exit::FinalizationFailed(cleanup) => {
                                    append_host_cleanup(failure, Exit::<(), E>::FinalizationFailed(cleanup))
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
    /// Pinned native future owned through completion.
    future: std::pin::Pin<Box<F>>,
}
impl<F: std::future::Future> std::future::Future for NativeIo<F> {
    /// Preserve the native operation's actual result type.
    type Output = F::Output;
    /// Enter the selected reactor only during one native poll.
    fn poll(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        let _guard = this.handle.enter();
        this.future.as_mut().poll(context)
    }
}
/// Wrap a future in caller-selected poll-scoped native reactor access.
fn native_io<F: std::future::Future>(handle: tokio::runtime::Handle, future: F) -> NativeIo<F> {
    NativeIo {
        handle,
        future: Box::pin(future),
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
        let boundary = boundary.clone();
        let service = service_fn(move |mut request: http::Request<Body>| {
            let entries = entries.clone();
            let boundary = boundary.clone();
            async move {
                let method = request.method().clone();
                let host_entries = entries
                    .iter()
                    .filter(|entry| host_matches(&entry.metadata, &request))
                    .collect::<Vec<_>>();
                let selected = host_entries
                    .iter()
                    .find(|entry| entry.methods.methods().contains(&method))
                    .copied()
                    .or_else(|| {
                        if method == Method::HEAD {
                            host_entries
                                .iter()
                                .find(|entry| entry.methods.methods().contains(&Method::GET))
                                .copied()
                        } else {
                            None
                        }
                    });
                let response = match selected {
                    Some(entry) => {
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
                                match (entry.endpoint)(captures.clone()) {
                                    Ok(endpoint) => {
                                        boundary
                                            .dispatch(
                                                request,
                                                endpoint,
                                                entry.metadata.clone(),
                                                captures,
                                            )
                                            .await
                                    }
                                    Err(error) => {
                                        let map = boundary.policy.native_error.clone();
                                        let endpoint = endpoint! {
                                            effect: effect_try_sync! {
                                                once;
                                                body: { let result = Err(map(error)); move || result },
                                            },
                                        };
                                        boundary
                                            .dispatch(
                                                request,
                                                endpoint,
                                                entry.metadata.clone(),
                                                captures,
                                            )
                                            .await
                                    }
                                }
                            }
                            Err(_) => terminal_response(StatusCode::BAD_REQUEST),
                        }
                    }
                    None => {
                        if host_entries.is_empty() {
                            terminal_response(StatusCode::NOT_FOUND)
                        } else {
                            let mut response = terminal_response(StatusCode::METHOD_NOT_ALLOWED);
                            let mut allowed = std::collections::BTreeSet::new();
                            for entry in host_entries {
                                for method in entry.methods.methods() {
                                    allowed.insert(method.as_str());
                                    if method == Method::GET {
                                        allowed.insert("HEAD");
                                    }
                                }
                            }
                            if let Ok(allow) = HeaderValue::from_str(
                                &allowed.into_iter().collect::<Vec<_>>().join(", "),
                            ) {
                                response.headers_mut().insert(http::header::ALLOW, allow);
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
                Ok::<_, Infallible>(
                    owner
                        .dispatch(
                            request,
                            registered.endpoint,
                            registered.metadata,
                            BTreeMap::new(),
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
/// Native host restrictions compare authority names and optional explicitly declared ports.
fn host_matches(metadata: &crate::Registration, request: &http::Request<Body>) -> bool {
    if metadata.hosts.is_empty() {
        return true;
    }
    if request.headers().get_all(http::header::HOST).iter().count() > 1 {
        return false;
    }
    let actual = request
        .uri()
        .authority()
        .map(|authority| headers::Host::from(authority.clone()))
        .or_else(|| {
            let mut values = request.headers().get_all(http::header::HOST).iter();
            <headers::Host as headers::Header>::decode(&mut values).ok()
        });
    actual.is_some_and(|actual| {
        metadata.hosts.iter().any(|expected| {
            let mut values = std::iter::once(expected);
            <headers::Host as headers::Header>::decode(&mut values).is_ok_and(|expected| {
                expected.hostname().eq_ignore_ascii_case(actual.hostname())
                    && expected
                        .port()
                        .is_none_or(|port| actual.port() == Some(port))
            })
        })
    })
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
            rest: Vec::new(),
        }),
        Exit::Fail(error) => Exit::FinalizationFailed(effect::Finalization {
            primary: effect::PrimaryOutcome::Failed(error),
            first: failure,
            rest: Vec::new(),
        }),
        Exit::Die(defect) => Exit::FinalizationFailed(effect::Finalization {
            primary: effect::PrimaryOutcome::Defected(defect),
            first: failure,
            rest: Vec::new(),
        }),
        Exit::Interrupted => Exit::FinalizationFailed(effect::Finalization {
            primary: effect::PrimaryOutcome::WasInterrupted,
            first: failure,
            rest: Vec::new(),
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

/// Accept native sockets under the host scope, joining and retiring completed handles as they
/// settle. Only live connection handles remain in the serving set; failures retain core ordering.
/// Hyper supplies HTTP parsing/framing/flow control/upgrades; this is a serving adapter only.
async fn serve_connections<E, R>(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    serving: (HttpProtocols, Cancellation, Duration),
    execution: (Scope<E>, NativeContext, Arc<R>),
    map: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
    boundary: Arc<Boundary<E, R>>,
) -> Exit<(), E>
where
    E: Send + 'static,
    R: HasTokio + HasCancellation + HasTaskSpawner + Send + Sync + 'static,
{
    let (protocols, stop, drain) = serving;
    let (scope, context, env) = execution;
    let mut connections = futures_util::stream::FuturesUnordered::new();
    let mut result = Exit::Success(());
    loop {
        let accepted = select! {
            () = stop.cancelled() => break,
            Some(connection)=connections.next(),if !connections.is_empty()=>{
                let connection:effect::Fiber<(),E>=connection;
                let exit=context.run(effect_uninterruptible! { program: connection.join(), },env.clone()).await;
                result=combine_completion(result,exit);continue;
            },
            accepted = listener.accept() => accepted
        };
        let (socket, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                result = combine_completion(Exit::Fail(map(ServerError::Io(error))), result);
                break;
            }
        };
        let router = router.clone();
        let stop = stop.clone();
        let map = map.clone();
        let boundary = boundary.clone();
        let connection = effect_uninterruptible! {
            program: crate::endpoint::native_once(move |env: Arc<R>, _| async move {
            native_io(env.tokio().clone(), async move {
                let builder = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
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
                let service=tower::service_fn(move |mut request: http::Request<hyper::body::Incoming>| {let router=router.clone();let boundary=boundary.clone();request.extensions_mut().insert(peer);let request=request.map(Body::new);async move {Ok::<_,Infallible>(boundary.dispatch_native(request,router).await)}});
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
            Exit::Success(connection) => connections.push(async move {
                let _ = connection.settled().await;
                connection
            }),
            Exit::Fail(error) => {
                result = combine_completion(Exit::Fail(error), result);
                break;
            }
            Exit::Die(defect) => {
                result = combine_completion(Exit::Die(defect), result);
                break;
            }
            Exit::Interrupted => {
                result = combine_completion(Exit::Interrupted, result);
                break;
            }
            Exit::FinalizationFailed(failure) => {
                result = combine_completion(Exit::FinalizationFailed(failure), result);
                break;
            }
        }
    }
    let _ = stop.request();
    while let Some(connection) = connections.next().await {
        let exit = context
            .run(
                effect_uninterruptible! { program: connection.join(), },
                env.clone(),
            )
            .await;
        result = combine_completion(result, exit);
    }
    result
}

/// Typed host failure renderer.
type ErrorRenderer<E, R> = dyn Fn(E) -> Effect<Reply<E, R>, E, R> + Send + Sync;

/// Observer of the selected environment and terminal request outcome.
type CompletionObserver<E, R> =
    dyn Fn(&R, &Request, &RequestCompletion<E>) -> Effect<(), E, R> + Send + Sync;

/// Native router configuration without endpoint execution.
type RouterConfigurator = dyn Fn(axum::Router) -> Result<axum::Router, ServerError> + Send + Sync;
