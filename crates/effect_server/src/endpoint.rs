//! Endpoint descriptions, registration metadata and owned middleware continuations.

use crate::{BodyOutcome, HasRequest, Reply, Request, ResourceValidators, ServerError};
use effect::{Effect, Exit, PrimaryOutcomeRef, Release, Scope};
use std::future::Future;
use std::sync::{Arc, Mutex};

/// One-shot native callback, evaluated by the original interpreter/context without Clone payload bounds.
pub(crate) fn native_once<A, E, R, F>(
    factory: impl FnOnce(Arc<R>, effect::NativeContext) -> F + Send + 'static,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Exit<A, E>> + Send + 'static,
{
    let factory = Mutex::new(Some(factory));
    Effect::from_exit_future_with_context(move |env, context| {
        let factory = factory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        async move {
            match factory {
                Some(factory) => factory(env, context).await,
                None => Exit::Die(effect::Defect::new(
                    "native HTTP operation already consumed",
                )),
            }
        }
    })
}
/// The awaited middleware continuation owns its reply/body delivery and scope lifetime.
pub type Continuation<E, R> = Arc<dyn Fn() -> Effect<(), E, R> + Send + Sync>;
/// Request execution middleware over one owned continuation, not a detached native callback.
pub type Middleware<E, R> =
    Arc<dyn Fn(Request, Continuation<E, R>) -> Effect<(), E, R> + Send + Sync>;
/// Typed endpoint ending in one completed reply. Construction is inert and reusable.
pub struct Endpoint<E: Send + 'static, R: Send + Sync + 'static> {
    /// Callback binds endpoint resources directly into the enclosing request scope.
    pub(crate) body: Arc<EndpointBody<E, R>>,
    /// Outer-to-inner stages applied around full body delivery, not Reply construction.
    pub(crate) pipeline: Pipeline<E, R>,
    /// Scoped expected-error handler; failure never recursively invokes itself.
    pub(crate) on_error: Option<Arc<ErrorRenderer<E, R>>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for Endpoint<E, R> {
    /// Share reusable descriptions without cloning request values or errors.
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
            pipeline: self.pipeline.clone(),
            on_error: self.on_error.clone(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Endpoint<E, R> {
    /// Lift an extracted typed Effect; ownership inside that Effect remains explicit.
    #[must_use]
    pub fn from_effect(effect: Effect<Reply<E, R>, E, R>) -> Self {
        Self {
            body: Arc::new(move |_| effect.clone()),
            pipeline: Pipeline::new(),
            on_error: None,
        }
    }
    /// Generated inline body retains Effect/resource binds under the request's scope.
    #[doc(hidden)]
    pub fn from_body<F>(
        factory: impl Fn(effect::__private::MacroContext<E, R>) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = Result<Reply<E, R>, Exit<Reply<E, R>, E>>> + Send + 'static,
    {
        let body = effect::__private::ScopedBody::from_body(factory);
        Self {
            body: Arc::new(move |scope| body.evaluate(scope)),
            pipeline: Pipeline::new(),
            on_error: None,
        }
    }
    /// Install middleware around this endpoint's complete awaited body lifecycle.
    #[must_use]
    pub fn pipe_through(mut self, pipeline: Pipeline<E, R>) -> Self {
        self.pipeline = pipeline.append(self.pipeline);
        self
    }
    /// Scope one expected-error renderer. Defects, interruption and cleanup remain distinct.
    pub fn on_error(
        mut self,
        handler: impl Fn(E) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static,
    ) -> Self {
        self.on_error = Some(Arc::new(handler));
        self
    }
    /// Check mutation preconditions before any endpoint/body/application writes.
    pub fn preconditions(
        self,
        validators: impl Fn(Request) -> Effect<ResourceValidators, E, R> + Send + Sync + 'static,
        map: impl Fn(ServerError) -> E + Send + Sync + 'static,
    ) -> Self
    where
        R: HasRequest,
    {
        let body = self.body.clone();
        let validators = Arc::new(validators);
        let map = Arc::new(map);
        let checked = Arc::new(move |scope: Scope<E>| {
            let body = body.clone();
            let validators = validators.clone();
            let map = map.clone();
            Effect::service(|env: &R| env.request().clone()).flat_map(move |request| {
                let body = body.clone();
                let scope = scope.clone();
                let map = map.clone();
                validators(request.clone()).flat_map(move |current| {
                    let decision = current.evaluate(&request).map_err(|error| map(error));
                    match decision {
                        Ok(crate::ConditionalOutcome::Proceed) => body(scope.clone()),
                        Ok(crate::ConditionalOutcome::NotModified) => {
                            let mut reply = Reply::empty();
                            reply.status = http::StatusCode::NOT_MODIFIED;
                            current.write(&mut reply.headers);
                            Effect::from_result_once(Ok(reply))
                        }
                        Ok(crate::ConditionalOutcome::PreconditionFailed) => {
                            let mut reply = Reply::empty();
                            reply.status = http::StatusCode::PRECONDITION_FAILED;
                            current.write(&mut reply.headers);
                            Effect::from_result_once(Ok(reply))
                        }
                        Err(error) => Effect::from_result_once(Err(error)),
                    }
                })
            })
        });
        Self {
            body: checked,
            pipeline: self.pipeline,
            on_error: self.on_error,
        }
    }
}
/// Ordered full-request middleware descriptions. Fn callbacks preserve reusable construction.
pub struct Pipeline<E: Send + 'static, R: Send + Sync + 'static> {
    /// Declaration order is preserved and later stages are awaited continuations.
    stages: Vec<Middleware<E, R>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for Pipeline<E, R> {
    /// Share stage descriptions, never active request scopes.
    fn clone(&self) -> Self {
        Self {
            stages: self.stages.clone(),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Default for Pipeline<E, R> {
    /// Empty middleware preserves the continuation exactly.
    fn default() -> Self {
        Self::new()
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Pipeline<E, R> {
    /// Create an inert ordered pipeline.
    #[must_use]
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }
    /// Append extracted middleware whose continuation must remain owned and awaited.
    pub fn stage(
        mut self,
        stage: impl Fn(Request, Continuation<E, R>) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        self.stages.push(Arc::new(stage));
        self
    }
    /// Run an Effect before the entire downstream body program.
    pub fn before(
        self,
        action: impl Fn(Request) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        let action = Arc::new(action);
        self.stage(move |request, next| {
            let next = next.clone();
            action(request).flat_map(move |()| next())
        })
    }
    /// Run an Effect after downstream delivery succeeds, before the enclosing scope finalizes.
    pub fn after(
        self,
        action: impl Fn(Request) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Self {
        let action = Arc::new(action);
        self.stage(move |request, next| {
            let action = action.clone();
            next().flat_map(move |()| action(request.clone()))
        })
    }
    /// Short-circuit with a typed expected failure before awaiting downstream work.
    pub fn guard(
        self,
        predicate: impl Fn(Request) -> Effect<bool, E, R> + Send + Sync + 'static,
        rejected: impl Fn() -> E + Send + Sync + 'static,
    ) -> Self {
        let predicate = Arc::new(predicate);
        let rejected = Arc::new(rejected);
        self.stage(move |request, next| {
            let rejected = rejected.clone();
            predicate(request).flat_map(move |allowed| {
                if allowed {
                    next()
                } else {
                    let rejected = rejected.clone();
                    Effect::fail_with(move || rejected())
                }
            })
        })
    }
    /// Observe the original full-delivery outcome, retaining release failures in core order.
    pub fn on_exit(
        self,
        release: impl for<'a> Fn(Request, PrimaryOutcomeRef<'a, E>) -> Release<E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let release = Arc::new(release);
        self.stage(move |request, next| {
            let release = release.clone();
            next().ensuring_with(move |outcome| release(request.clone(), outcome))
        })
    }
    /// Deliberate early response invokes the native reply continuation installed by the request boundary.
    pub fn respond(
        self,
        reply: impl Fn(Request) -> Effect<Reply<E, R>, E, R> + Send + Sync + 'static,
    ) -> Self
    where
        R: HasRequest,
    {
        let reply = Arc::new(reply);
        self.stage(move |request, _| {
            let reply = reply.clone();
            let request_for_reply = request.clone();
            Effect::service(move |_: &R| {
                request.with_extensions(|extensions| {
                    extensions.get::<ReplyContinuation<E, R>>().cloned()
                })
            })
            .flat_map(move |continuation| match continuation {
                Some(continuation) => {
                    let continuation = continuation.0.clone();
                    reply(request_for_reply.clone()).flat_map(move |reply| continuation(reply))
                }
                None => Effect::die(effect::Defect::new(
                    "response continuation unavailable outside request boundary",
                )),
            })
        })
    }
    /// Add staged response metadata through the request's explicit response-field owner.
    pub fn set_header(self, name: http::HeaderName, value: http::HeaderValue) -> Self {
        self.before(move |request| {
            let name = name.clone();
            let value = value.clone();
            Effect::sync(move || {
                request.with_extensions(|extensions| {
                    if let Some(headers) = extensions.get::<ResponseHeaders>() {
                        headers
                            .0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(name.clone(), value.clone());
                    }
                });
            })
        })
    }
    /// Compose child stages inside these stages, preserving declaration/nesting order.
    #[must_use]
    pub fn append(mut self, child: Self) -> Self {
        self.stages.extend(child.stages);
        self
    }
    /// Apply the middleware nesting exactly once around an owned awaited continuation.
    pub(crate) fn apply(&self, request: Request, terminal: Continuation<E, R>) -> Effect<(), E, R> {
        let mut next = terminal;
        for stage in self.stages.iter().rev() {
            let stage = stage.clone();
            let request = request.clone();
            let inner = next;
            next = Arc::new(move || stage(request.clone(), inner.clone()));
        }
        next()
    }
}
/// Native response-field owner shared by outer wrappers and short circuits.
#[derive(Clone, Default)]
pub struct ResponseHeaders(pub Arc<Mutex<http::HeaderMap>>);
/// A request-local continuation consuming one reply. It grants no authority beyond response ownership.
pub(crate) struct ReplyContinuation<E: Send + 'static, R: Send + Sync + 'static>(
    pub(crate) Arc<ReplyConsumer<E, R>>,
);
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for ReplyContinuation<E, R> {
    /// Share native continuation without copying its reply payload.
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
/// Registration-time metadata remains distinct from an endpoint's per-request execution.
#[derive(Clone, Default)]
pub struct Registration {
    /// Stable optional endpoint name used by native metadata and reverse links.
    pub name: Option<Arc<str>>,
    /// Safe classification tags selected by the application.
    pub tags: Vec<Arc<str>>,
    /// Human-readable endpoint summary.
    pub summary: Option<Arc<str>>,
    /// Human-readable endpoint description.
    pub description: Option<Arc<str>>,
    /// Native structured request content types accepted by the endpoint.
    pub accepts: Vec<http::HeaderValue>,
    /// Native response representation types advertised at registration time.
    pub produces: Vec<http::HeaderValue>,
    /// Native allowed hosts; actual matching uses the request host policy.
    pub hosts: Vec<http::HeaderValue>,
    /// Explicit application-owned authorization metadata, interpreted by selected policy.
    pub authorization: Option<Arc<str>>,
    /// Explicit anonymous selection, never a substitute for trusted authentication.
    pub allow_anonymous: bool,
    /// Native application metadata hooks retained by the registration description.
    pub extensions: http::Extensions,
}
/// A completed endpoint with registration-time metadata/policy/filter configuration.
pub struct Registered<E: Send + 'static, R: Send + Sync + 'static> {
    /// Request program never evaluates while registration metadata is constructed.
    pub endpoint: Endpoint<E, R>,
    /// Native registration metadata applied before serving.
    pub metadata: Registration,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Registered<E, R> {
    /// Pair an inert endpoint and checked registration metadata.
    #[must_use]
    pub fn new(endpoint: Endpoint<E, R>, metadata: Registration) -> Self {
        Self { endpoint, metadata }
    }
    /// Apply an extracted native metadata configuration callback before mounting.
    pub fn configure(mut self, configure: impl FnOnce(&mut Registration)) -> Self {
        configure(&mut self.metadata);
        self
    }
    /// Add a scoped pipeline/filter before request execution.
    #[must_use]
    pub fn pipe_through(mut self, pipeline: Pipeline<E, R>) -> Self {
        self.endpoint = self.endpoint.pipe_through(pipeline);
        self
    }
}
/// Completion observation retains native commitment state and the final typed request exit.
pub struct RequestCompletion<E> {
    /// Full application and cleanup outcome, moved once into the host's terminal observer.
    pub exit: Exit<(), E>,
    /// Native delivery outcome recorded before the scope's finalizers were joined.
    pub body: BodyOutcome,
    /// Whether response headers were committed to the native service.
    pub committed: bool,
}

/// Deferred endpoint body under the existing request scope.
type EndpointBody<E, R> = dyn Fn(Scope<E>) -> Effect<Reply<E, R>, E, R> + Send + Sync;

/// Typed endpoint failure renderer.
type ErrorRenderer<E, R> = dyn Fn(E) -> Effect<Reply<E, R>, E, R> + Send + Sync;

/// Continuation consuming an owned reply once per invocation.
type ReplyConsumer<E, R> = dyn Fn(Reply<E, R>) -> Effect<(), E, R> + Send + Sync;
