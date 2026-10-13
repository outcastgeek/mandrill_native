//! D19 semantic groups with selected typed inputs, native workflows and Effect renderers.
//!
//! Group factories build inert Server registration descriptions. Request decoding,
//! native Identity operations and renderer Effects execute only under Server's
//! existing request scope and body lifetime. Custom routes retain the same rules.
//! Native HTTP groups with compatible contracts may use IdentityDefinition::native_http
//! to retain their public protocol handlers and private codecs under scoped diagnostics;
//! selected callbacks provide their actual native state and native router/service.
//! Paths, decoding policies, permitted modes and public response fields stay explicit.

use effect::{Effect, effect, service};
use effect_server::{HasRequest, HttpMethodSet, Reply, Request, Router, ServerError};
use effect_server::{endpoint, registered, reply, router};
use std::sync::Arc;

/// Selected Identity workflow consuming one typed input under the request owner.
type Workflow<D, I, O, E, R> = dyn Fn(Arc<D>, I) -> Effect<O, E, R> + Send + Sync;
/// Safe response renderer consuming the workflow's owned result.
type RequestGuard<D, E, R> = dyn Fn(Arc<D>, Request) -> Effect<(), E, R> + Send + Sync;
/// Safe response renderer consuming the workflow's owned result.
type Renderer<O, E, R> = dyn Fn(O) -> Effect<Reply<E, R>, E, R> + Send + Sync;
/// Native protocol workflow retaining its original response body.
type NativeHttpWorkflow<D, E, R> =
    dyn Fn(Arc<D>, Request) -> Effect<http::Response<axum::body::Body>, E, R> + Send + Sync;
/// One-shot inert registration factory sharing the installed definition.
type RouteFactory<D, E, R> =
    dyn FnOnce(Arc<D>) -> Result<Router<E, R>, EndpointError> + Send + 'static;

/// Compose an endpoint policy before its resource guard without invoking a denied guard.
#[doc(hidden)]
pub fn authorized_guard<D: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    authorize: impl Fn(Request) -> Effect<(), E, R> + Send + Sync + 'static,
    guard: impl Fn(Arc<D>, Request) -> Effect<(), E, R> + Send + Sync + 'static,
) -> impl Fn(Arc<D>, Request) -> Effect<(), E, R> + Send + Sync + 'static {
    let guard = Arc::new(guard);
    move |identity, request| {
        let guard = guard.clone();
        let authorized = authorize(request.clone());
        effect! {
            let guard = guard.clone();
            let identity = identity.clone();
            let request = request.clone();
            authorized.clone()?;
            guard(identity, request)?
        }
    }
}

/// Typed Identity endpoint configuration failure before serving begins.
#[derive(Debug)]
pub enum EndpointError {
    /// Native Server route/prefix/registration configuration was invalid.
    Server(ServerError),
    /// A selected native Identity group rejected its component/mode configuration.
    Native(http::StatusCode),
    /// The same semantic group was selected more than once.
    DuplicateGroup(EndpointGroup),
}
impl From<ServerError> for EndpointError {
    /// Preserve the actual typed route configuration failure.
    fn from(error: ServerError) -> Self {
        Self::Server(error)
    }
}
impl std::fmt::Display for EndpointError {
    /// Render safe configuration categories without request or credential bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Server(_) => "invalid identity endpoint configuration",
            Self::Native(_) => "native identity group configuration rejected",
            Self::DuplicateGroup(_) => "duplicate identity endpoint group",
        })
    }
}
impl std::error::Error for EndpointError {
    /// Retain native Server diagnostics for explicit configuration inspection.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Server(error) => Some(error),
            Self::Native(_) | Self::DuplicateGroup(_) => None,
        }
    }
}
/// Required semantic groups remain explicit; selecting one does not select others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointGroup {
    /// Account registration and selected confirmation policy/views.
    Registration,
    /// Verified password/pending/completed sign-in.
    Login,
    /// Native password reset/recovery proof handling.
    Recovery,
    /// Native MFA/authenticator management and recovery hashes.
    Authenticator,
    /// Current trusted RP/origin/verification and native one-use ceremonies.
    Passkeys,
    /// Selected external verifier/admission and factor completion.
    External,
    /// Authorized self/admin account/role/preferences views and mutations.
    Management,
    /// Acknowledged selected session/cookie/browser invalidation.
    Logout,
    /// Native exact-value refresh rotation before credentials are returned.
    Refresh,
    /// Native server-backed same-origin browser context/CSRF issuance.
    Antiforgery,
}
/// Typed inert group factory receiving the exact installed Identity definition.
pub trait IdentityGroup<D, E: Send + 'static, R: Send + Sync + 'static> {
    /// Build registrations without evaluating input, workflow or renderer Effects.
    fn build(self, identity: Arc<D>) -> Result<Router<E, R>, EndpointError>;
}
impl<D, E: Send + 'static, R: Send + Sync + 'static, F> IdentityGroup<D, E, R> for F
where
    F: FnOnce(Arc<D>) -> Result<Router<E, R>, EndpointError>,
{
    /// Evaluate only the selected pure group configuration callback.
    fn build(self, identity: Arc<D>) -> Result<Router<E, R>, EndpointError> {
        self(identity)
    }
}
/// One selected route whose typed decoding/workflow/rendering all compose as Effects.
pub struct WorkflowGroup<
    D,
    I: Send + 'static,
    O: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    /// Explicit method and path; no hidden route or unsafe mutation method defaults.
    method: http::Method,
    /// Checked native path is validated during group registration.
    path: String,
    /// Selected typed input policy under the same native request/body owner.
    input: Arc<dyn Fn(Request) -> Effect<I, E, R> + Send + Sync>,
    /// Actual selected Identity workflow, consuming its typed input without Clone.
    workflow: Arc<Workflow<D, I, O, E, R>>,
    /// Safe selected renderer; HTML safety/native codecs remain its concrete return type.
    render: Arc<Renderer<O, E, R>>,
    /// Optional route-local authorization runs before decoding or executing this request.
    guard: Option<Arc<RequestGuard<D, E, R>>>,
}
impl<D, I: Send + 'static, O: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    WorkflowGroup<D, I, O, E, R>
{
    /// Select one complete typed route and its input/workflow/renderer callbacks without I/O.
    pub fn new(
        method: http::Method,
        path: String,
        input: impl Fn(Request) -> Effect<I, E, R> + Send + Sync + 'static,
        workflow: impl Fn(Arc<D>, I) -> Effect<O, E, R> + Send + Sync + 'static,
        render: impl Fn(O) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static,
    ) -> Self {
        Self {
            method,
            path,
            input: Arc::new(input),
            workflow: Arc::new(workflow),
            render: Arc::new(render),
            guard: None,
        }
    }
    /// Select exact endpoint/resource authorization in the same request scope.
    /// Guard failures retain their full Effect outcome and skip input/workflow/rendering.
    #[must_use]
    pub fn with_guard(
        mut self,
        guard: impl Fn(Arc<D>, Request) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        self.guard = Some(Arc::new(guard));
        self
    }
}
impl<
    D: Send + Sync + 'static,
    I: Send + 'static,
    O: Send + 'static,
    E: Send + 'static,
    R: HasRequest + Send + Sync + 'static,
> IdentityGroup<D, E, R> for WorkflowGroup<D, I, O, E, R>
{
    /// Await request input, workflow and rendering in order under the request owner.
    /// Building retains reusable callbacks without evaluating them or cloning I/O payloads.
    fn build(self, identity: Arc<D>) -> Result<Router<E, R>, EndpointError> {
        let input = self.input;
        let workflow = self.workflow;
        let render = self.render;
        let guard = self.guard;
        let program = effect! {
            let input = Arc::clone(&input);
            let workflow = Arc::clone(&workflow);
            let render = Arc::clone(&render);
            let guard = guard.clone();
            let identity = Arc::clone(&identity);
            let request = service! { env |env: &R| { env.request().clone() }, }?;
            if let Some(guard) = guard { guard(identity.clone(), request.clone())?; }
            let value = input(request)?;
            let value = workflow(identity, value)?;
            render(value)?
        };
        Ok(router! {
            methods(HttpMethodSet::new(vec![self.method])?, self.path.as_str(),
                registered! {
                    endpoint: endpoint! {
                        effect(program),
                    },
                });
        }?)
    }
}
/// One public native HTTP handler composed as an actual scoped Effect route.
/// The selected callback uses IdentityDefinition::native_http to bind its native
/// state/service to the invocation backend. Native protocol decoding and response
/// codecs remain native, and the original response body moves directly into Reply.
pub struct NativeHttpGroup<D, E: Send + 'static, R: Send + Sync + 'static> {
    /// Explicit native HTTP method selected by the application group.
    method: http::Method,
    /// Actual checked path; native groups do not invent a second mount prefix.
    path: String,
    /// Selected whole-operation adapter over the exact installed definition.
    workflow: Arc<NativeHttpWorkflow<D, E, R>>,
}
impl<D, E: Send + 'static, R: Send + Sync + 'static> NativeHttpGroup<D, E, R> {
    /// Retain the native protocol workflow without executing I/O or decoding a request.
    pub fn new(
        method: http::Method,
        path: String,
        workflow: impl Fn(Arc<D>, Request) -> Effect<http::Response<axum::body::Body>, E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            method,
            path,
            workflow: Arc::new(workflow),
        }
    }
}
impl<D: Send + Sync + 'static, E: Send + 'static, R: HasRequest + Send + Sync + 'static>
    IdentityGroup<D, E, R> for NativeHttpGroup<D, E, R>
{
    /// Validate registration, then await the native handler under the selected request.
    /// The original response body transfers directly into Reply without buffering or copying.
    fn build(self, identity: Arc<D>) -> Result<Router<E, R>, EndpointError> {
        let workflow = self.workflow;
        let program = effect! {
            let workflow = Arc::clone(&workflow);
            let identity = Arc::clone(&identity);
            let request = service! { env |env: &R| { env.request().clone() }, }?;
            let response = workflow(identity, request)?;
            reply! { native(response) }
        };
        Ok(router! {
            methods(HttpMethodSet::new(vec![self.method])?, self.path.as_str(),
                registered! {
                    endpoint: endpoint! {
                        effect(program),
                    },
                });
        }?)
    }
}
/// One nonempty semantic group containing heterogeneous typed workflow routes.
/// Each route retains its own input/output types and composes through Server's
/// request scope. Construction retains callbacks; building performs route validation only.
pub struct WorkflowRoutes<D, E: Send + 'static, R: Send + Sync + 'static> {
    /// Ordered pure registration callbacks, never request execution callbacks.
    routes: Vec<Box<RouteFactory<D, E, R>>>,
}
impl<D: 'static, E: Send + 'static, R: Send + Sync + 'static> WorkflowRoutes<D, E, R> {
    /// Start with a real route/group so an empty semantic success cannot be selected.
    pub fn new<G: IdentityGroup<D, E, R> + Send + 'static>(first: G) -> Self {
        Self {
            routes: vec![Box::new(move |identity| first.build(identity))],
        }
    }
    /// Add another typed workflow or native group in declaration order without I/O.
    pub fn route<G: IdentityGroup<D, E, R> + Send + 'static>(mut self, route: G) -> Self {
        self.routes
            .push(Box::new(move |identity| route.build(identity)));
        self
    }
}
impl<D: 'static, E: Send + 'static, R: Send + Sync + 'static> IdentityGroup<D, E, R>
    for WorkflowRoutes<D, E, R>
{
    /// Build each checked registration once while retaining the exact definition generation.
    fn build(self, identity: Arc<D>) -> Result<Router<E, R>, EndpointError> {
        let mut router = router! {}?;
        for route in self.routes {
            let child = route(identity.clone())?;
            router = router! {
                source: router,
                include(child);
            }?;
        }
        Ok(router)
    }
}
/// Inert D19 assembly retaining selected Identity and semantic-group order.
pub struct IdentityEndpoints<D, E: Send + 'static, R: Send + Sync + 'static> {
    /// Exact selected definition supplied to all native/workflow group factories.
    identity: Arc<D>,
    /// Explicit checked mount prefix.
    prefix: String,
    /// Ordered native Server registration description.
    router: Router<E, R>,
    /// Duplicate semantic groups reject instead of silently shadowing handlers.
    groups: Vec<EndpointGroup>,
}
impl<D: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>
    IdentityEndpoints<D, E, R>
{
    /// Retain the definition and validate prefix without executing request work.
    pub fn new(identity: D, prefix: &str) -> Result<Self, EndpointError> {
        effect_server::Route::literal(prefix)?;
        Ok(Self {
            identity: Arc::new(identity),
            prefix: prefix.to_owned(),
            router: Router::new(),
            groups: Vec::new(),
        })
    }
    /// Share the installed definition for policy lowering without cloning its native payload.
    #[doc(hidden)]
    #[must_use]
    pub fn definition(&self) -> Arc<D> {
        self.identity.clone()
    }
    /// Build one actual native/Effect route group using the installed definition.
    pub fn group(
        mut self,
        kind: EndpointGroup,
        group: impl IdentityGroup<D, E, R>,
    ) -> Result<Self, EndpointError> {
        if self.groups.contains(&kind) {
            return Err(EndpointError::DuplicateGroup(kind));
        }
        let router = group.build(self.identity.clone())?;
        self.groups.push(kind);
        self.router = self.router.include(router);
        Ok(self)
    }
    /// Contextually type an inline group at the installed definition's native boundary.
    /// The callback builds only inert routes and executes exactly once, as group does.
    pub fn group_with(
        self,
        kind: EndpointGroup,
        build: impl FnOnce(Arc<D>) -> Result<Router<E, R>, EndpointError>,
    ) -> Result<Self, EndpointError> {
        self.group(kind, build)
    }
    /// Contextually type route input, authorization, workflow and renderer from this definition.
    /// Only inert registration is built now. Each request runs its guard first, then
    /// moves decoded input and workflow output once through the existing Server owner.
    pub fn workflow_route<I: Send + 'static, O: Send + 'static>(
        self,
        kind: Option<EndpointGroup>,
        route: (http::Method, String),
        guard: impl Fn(Arc<D>, Request) -> Effect<(), E, R> + Send + Sync + 'static,
        input: impl Fn(Request) -> Effect<I, E, R> + Send + Sync + 'static,
        workflow: impl Fn(Arc<D>, I) -> Effect<O, E, R> + Send + Sync + 'static,
        render: impl Fn(O) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static,
    ) -> Result<Self, EndpointError>
    where
        R: HasRequest,
    {
        let group = WorkflowGroup::new(route.0, route.1, input, workflow, render).with_guard(guard);
        if let Some(kind) = kind {
            self.group(kind, group)
        } else {
            let router = group.build(self.identity.clone())?;
            Ok(self.include(router))
        }
    }
    /// Include an explicit custom typed route module under the same Identity prefix.
    #[must_use]
    pub fn include(mut self, router: Router<E, R>) -> Self {
        self.router = self.router.include(router);
        self
    }
    /// Apply a selected scoped request pipeline to every Identity group and custom route.
    #[must_use]
    pub fn pipe_through(mut self, pipeline: effect_server::Pipeline<E, R>) -> Self {
        self.router = self.router.pipe_through(pipeline);
        self
    }
    /// Retain selected expected-error Effect rendering without recursively rendering renderer errors.
    pub fn on_error(
        mut self,
        render: impl Fn(E) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static,
    ) -> Self {
        self.router = self.router.on_error(render);
        self
    }
    /// Finish the actual scoped native Server group; the root prefix includes directly.
    pub fn finish(self) -> Result<Router<E, R>, EndpointError> {
        if self.prefix == "/" {
            Ok(self.router)
        } else {
            Ok(Router::new().group(&self.prefix, self.router)?)
        }
    }
}
