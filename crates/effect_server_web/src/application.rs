//! Inert Web application assembly over the real Server host/request owners.
//!
//! Startup acquires capabilities in a caller-owned scope before accepting traffic.
//! Native route/authentication/culture selection precedes request projection;
//! authorization/CSRF/admission/output-cache gates run inside the same selected R.
//! Completion uses ServerPolicy::on_exit after actual body delivery and cleanup.

use crate::{
    WebError,
    availability::Availability,
    localization::{IntoLocalization, PinnedCatalog},
    security::Security,
};
use crate::{
    effect_die, effect_fail, effect_from_result, effect_future, effect_map, effect_map_error,
    effect_sync, effect_try_sync,
};
use effect::effect;
use effect::memory::Admission;
use effect::{Effect, Exit, HasCancellation, HasClock, HasTaskSpawner, Scope, effect_scope};
use effect_server::{
    HasRequest, HasTokio, HttpProtocols, Pipeline, PreparedEndpointBytePools, PreparedHostResult,
    PreparedServer, Request, RequestCompletion, RequestHook, Router, Server,
    ServerError, ServerHost, ServerPolicy, StaticOverloadReply,
};
use effect_server::{pipeline, router, server};
use std::net::SocketAddr;
use std::sync::Arc;

/// Native configuration callback insertion point, evaluated after resource startup
/// and before the native listener accepts traffic. It creates no new host/runtime.
type NativeHook<C> =
    Arc<dyn Fn(Arc<C>, axum::Router) -> Result<axum::Router, ServerError> + Send + Sync>;
/// Selected complete-server installation, used by optional concrete integrations.
type ServerInstaller<C, E, R> =
    Arc<dyn Fn(Arc<C>, Server<E, R>) -> Result<Server<E, R>, E> + Send + Sync>;
/// Host-scoped feature acquisition returns actual typed request hooks.
type Feature<C, E, R> =
    Arc<dyn Fn(Arc<C>, Scope<E>) -> Effect<Vec<RequestHook<E, R>>, E, R> + Send + Sync>;
/// Selected full terminal observation; borrowed error payloads cannot escape into
/// the returned Effect. The callback captures owned metadata explicitly.
type Observer<E, R> =
    Arc<dyn Fn(&R, &Request, &RequestCompletion<E>) -> Effect<(), E, R> + Send + Sync>;
/// Selected request projection retaining startup capabilities and the sole Effect environment.
type RequestEnvironment<C, E, R> =
    Arc<dyn Fn(Arc<C>, Request) -> Effect<Arc<R>, E, R> + Send + Sync>;
type PreparedServerFactory<E, R> =
    Arc<dyn Fn(Server<E, R>) -> Result<PreparedServer<E, R>, ServerError> + Send + Sync>;

/// Reusable complete Web definition. C is immutable startup capability data; R
/// remains the sole environment/provider selection in every application Effect.
#[must_use]
pub struct Web<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Root/native/body/drain policy, shared without cloning E.
    policy: ServerPolicy<E, R>,
    /// Explicit Web configuration/selection error injection.
    errors: Arc<dyn Fn(WebError) -> E + Send + Sync>,
    /// Lazy startup capability acquisition in the real outer host scope.
    startup: Arc<dyn Fn(Scope<E>) -> Effect<C, E, R> + Send + Sync>,
    /// Projection after native route/auth/culture stages, preserving selected R.
    environment: RequestEnvironment<C, E, R>,
    /// Explicit native listening endpoint; no hidden port/protocol family.
    listen: SocketAddr,
    /// Selected native compiled transport protocol policy.
    protocols: HttpProtocols,
    /// Capability-derived reusable registration descriptions.
    routes: Arc<dyn Fn(Arc<C>) -> Router<E, R> + Send + Sync>,
    /// Prepared selected security, including runtime validation errors.
    security: Option<Result<Arc<Security<E, R>>, WebError>>,
    /// Ordered host-owned localization and other concrete feature acquisitions.
    features: Vec<Feature<C, E, R>>,
    /// Ordered native builder hooks before app-stage native configuration.
    builders: Vec<NativeHook<C>>,
    /// Ordered native app hooks applied before accepting traffic.
    apps: Vec<NativeHook<C>>,
    /// One root completion observer, evaluated after actual body cleanup.
    observer: Option<Observer<E, R>>,
    /// Optional prepared operational routes merged into the real router.
    availability: Option<Availability<E, R>>,
    /// Typed ordered middleware, including application-selected finite output gates.
    middleware: Pipeline<E, R>,
    /// Real optional native integration installers; no guessed bridges exist here.
    installers: Vec<ServerInstaller<C, E, R>>,
    /// Static typed configuration failures retained until scoped startup.
    configuration: Vec<WebError>,
    /// Selected immutable finite output definitions; engines stay in scoped sources.
    output_definition: Option<crate::output_cache::OutputCacheDefinition>,
}

/// A Web description that prepares the host's physical owners before traffic.
#[must_use]
pub struct PreparedWeb<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    definition: Web<C, E, R>,
    prepare_server: PreparedServerFactory<E, R>,
    admission: Option<Arc<Admission<'static, 1>>>,
    byte_pools: Option<Arc<PreparedEndpointBytePools>>,
    overload: Option<StaticOverloadReply>,
}
impl<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for PreparedWeb<C, E, R> {
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            prepare_server: self.prepare_server.clone(),
            admission: self.admission.clone(),
            byte_pools: self.byte_pools.clone(),
            overload: self.overload,
        }
    }
}

impl<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> PreparedWeb<C, E, R> {
    /// Apply a shared work-unit limit to actual prepared request-control records.
    pub fn with_admission(mut self, admission: Admission<'static, 1>) -> Self {
        self.admission = Some(Arc::new(admission));
        self
    }
    /// Select physical byte slots for shared request work across middleware and endpoints.
    pub fn with_byte_pools(mut self, pools: Arc<PreparedEndpointBytePools>) -> Self {
        self.byte_pools = Some(pools);
        self
    }

    /// Select the static response for prepared capacity refusal.
    pub fn with_overload(mut self, reply: StaticOverloadReply) -> Self {
        self.overload = Some(reply);
        self
    }

    /// Use the configured host in the original Web startup scope.
    /// The returned guard retains the host's terminal owners and complete Exit.
    pub fn with_host<A: Send + 'static>(
        &self,
        use_host: impl Fn(ServerHost) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<PreparedHostResult<A, E, R>, E, R>
    where
        R: HasRequest + HasClock + HasTaskSpawner + HasCancellation + HasTokio,
    {
        let definition = self.definition.clone();
        let prepare_server = self.prepare_server.clone();
        let admission = self.admission.clone();
        let byte_pools = self.byte_pools.clone();
        let overload = self.overload;
        let use_host = Arc::new(use_host);
        effect_scope! {
            body |scope: Scope<E>| {
                let definition = definition.clone();
                let prepare_server = prepare_server.clone();
                let admission = admission.clone();
                let byte_pools = byte_pools.clone();
                let use_host = use_host.clone();
                let errors = definition.errors.clone();
                let server = crate::web! { prepare; source: definition, scope: scope, }?;
                let host = effect_try_sync! {
                    once;
                    body: move || {
                        let server = match overload {
                            Some(reply) => server! { source: server, overload(reply) },
                            None => server,
                        };
                        let mut prepared = prepare_server(server).map_err(web_server_error)?;
                        if !prepared.has_prepared_request_storage() {
                            return Err(WebError::Configuration("prepared request storage required".into()));
                        }
                        if let Some(admission) = admission {
                            let admission = admission.try_clone().map_err(WebError::Memory)?;
                            prepared = server! {
                                prepared_configure;
                                source: prepared,
                                request_control_admission(admission),
                            }.map_err(WebError::Memory)?;
                        }
                        if let Some(pools) = byte_pools {
                            prepared = server! {
                                prepared_configure;
                                source: prepared,
                                request_byte_pools(pools),
                            }.map_err(WebError::Memory)?;
                        }
                        server! { with_host; source: prepared, body: move |host| use_host(host), }
                            .map_err(WebError::Memory)
                    },
                    on_error: move |error| errors(error),
                }?;
                effect_map_error! { program: host, map: |never| match never {}, }?
            },
        }
    }

    /// Serve until root cancellation, retaining the complete guarded outcome.
    pub fn run(&self) -> Effect<PreparedHostResult<(), E, R>, E, R>
    where
        R: HasRequest + HasClock + HasTaskSpawner + HasCancellation + HasTokio,
    {
        crate::web! { with_host; source: self, body: wait_for_cancellation, }
    }
}

pub(crate) fn web_server_error(error: ServerError) -> WebError {
    if let ServerError::Memory(error) = error {
        WebError::Memory(error)
    } else {
        WebError::Configuration(error.to_string())
    }
}
impl<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for Web<C, E, R> {
    /// Share inert startup/route/policy descriptions, never active host/request values.
    fn clone(&self) -> Self {
        Self {
            policy: self.policy.clone(),
            errors: self.errors.clone(),
            startup: self.startup.clone(),
            environment: self.environment.clone(),
            listen: self.listen,
            protocols: self.protocols,
            routes: self.routes.clone(),
            security: self.security.clone(),
            features: self.features.clone(),
            builders: self.builders.clone(),
            apps: self.apps.clone(),
            observer: self.observer.clone(),
            availability: self.availability.clone(),
            middleware: self.middleware.clone(),
            installers: self.installers.clone(),
            configuration: self.configuration.clone(),
            output_definition: self.output_definition.clone(),
        }
    }
}
/// Typed route description/function conversion used by D20's inline/extracted forms.
pub trait IntoWebRoutes<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Preserve an inert capability-derived registration callback.
    fn into_routes(self) -> Arc<dyn Fn(Arc<C>) -> Router<E, R> + Send + Sync>;
}
impl<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> IntoWebRoutes<C, E, R>
    for Router<E, R>
{
    /// Share static registration metadata across fresh hosts, without running endpoints.
    fn into_routes(self) -> Arc<dyn Fn(Arc<C>) -> Router<E, R> + Send + Sync> {
        Arc::new(move |_| self.clone())
    }
}
impl<C, E, R, F> IntoWebRoutes<C, E, R> for F
where
    C: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(Arc<C>) -> Router<E, R> + Send + Sync + 'static,
{
    /// Extracted route factories select typed startup capabilities once per host.
    fn into_routes(self) -> Arc<dyn Fn(Arc<C>) -> Router<E, R> + Send + Sync> {
        Arc::new(self)
    }
}
/// Prepared/checked nested security descriptions accepted by root assembly.
pub trait IntoWebSecurity<E: Send + 'static, R: Send + Sync + 'static> {
    /// Preserve the dynamic configuration error until scoped startup validation.
    fn into_security(self) -> Result<Arc<Security<E, R>>, WebError>;
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoWebSecurity<E, R> for Security<E, R> {
    /// Revalidate nonce capability selection without invoking callbacks.
    fn into_security(self) -> Result<Arc<Security<E, R>>, WebError> {
        self.validate().map(Arc::new)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> IntoWebSecurity<E, R>
    for Result<Security<E, R>, WebError>
{
    /// Nested security macro errors remain typed root startup failures.
    fn into_security(self) -> Result<Arc<Security<E, R>>, WebError> {
        self.and_then(Security::validate).map(Arc::new)
    }
}
impl<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static> Web<C, E, R> {
    /// Select all required root fields without running startup/native callbacks.
    pub fn new(
        policy: ServerPolicy<E, R>,
        errors: impl Fn(WebError) -> E + Send + Sync + 'static,
        startup: impl Fn(Scope<E>) -> Effect<C, E, R> + Send + Sync + 'static,
        environment: impl Fn(Arc<C>, Request) -> Effect<Arc<R>, E, R> + Send + Sync + 'static,
        listen: SocketAddr,
        protocols: HttpProtocols,
        routes: impl IntoWebRoutes<C, E, R>,
    ) -> Self {
        Self {
            policy,
            errors: Arc::new(errors),
            startup: Arc::new(startup),
            environment: Arc::new(environment),
            listen,
            protocols,
            routes: routes.into_routes(),
            security: None,
            features: Vec::new(),
            builders: Vec::new(),
            apps: Vec::new(),
            observer: None,
            availability: None,
            middleware: pipeline! {},
            installers: Vec::new(),
            configuration: Vec::new(),
            output_definition: None,
        }
    }
    /// Select a typed startup factory for this definition's prepared Server.
    /// The factory receives the fully configured server before listener creation.
    pub fn with_memory(
        self,
        prepare_server: impl Fn(Server<E, R>) -> Result<PreparedServer<E, R>, ServerError>
        + Send
        + Sync
        + 'static,
    ) -> PreparedWeb<C, E, R> {
        PreparedWeb {
            definition: self,
            prepare_server: Arc::new(prepare_server),
            admission: None,
            byte_pools: None,
            overload: None,
        }
    }
    /// Select one checked reusable browser/security description, validated at startup.
    pub fn security(mut self, security: impl IntoWebSecurity<E, R>) -> Self {
        self.security = Some(security.into_security());
        self
    }
    /// Add a native builder-stage hook with explicit typed startup capabilities.
    pub fn configure_builder(
        mut self,
        configure: impl Fn(Arc<C>, axum::Router) -> Result<axum::Router, ServerError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.builders.push(Arc::new(configure));
        self
    }
    /// Add a native app-stage hook after selected features, before traffic admission.
    /// This explicit native escape retains Server's host/body ownership boundary.
    pub fn configure_app(
        mut self,
        configure: impl Fn(Arc<C>, axum::Router) -> Result<axum::Router, ServerError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.apps.push(Arc::new(configure));
        self
    }
    /// Observe final body/cleanup outcome under the actual selected request environment.
    pub fn observe(
        mut self,
        observe: impl Fn(&R, &Request, &RequestCompletion<E>) -> Effect<(), E, R>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.observer = Some(Arc::new(observe));
        self
    }
    /// Add application-selected finite cache/admission middleware. Its output-cache
    /// constructor must execute authorization/preconditions before consulting a hit.
    pub fn output_cache(
        mut self,
        selection: impl crate::output_cache::IntoOutputCache<E, R>,
    ) -> Self {
        match selection.into_output_cache() {
            crate::output_cache::OutputCacheSelection::Definition(Ok(definition)) => {
                self.output_definition = Some(definition)
            }
            crate::output_cache::OutputCacheSelection::Definition(Err(error)) => {
                self.configuration.push(error)
            }
            crate::output_cache::OutputCacheSelection::Pipeline(pipeline) => {
                self.middleware = self.middleware.append(pipeline)
            }
        }
        self
    }
    /// Install one capability-derived complete native Server transform at its
    /// explicit pre-listen stage; optional bridges reuse this existing composition.
    pub fn install_server(
        mut self,
        install: impl Fn(Arc<C>, Server<E, R>) -> Result<Server<E, R>, E> + Send + Sync + 'static,
    ) -> Self {
        self.installers.push(Arc::new(install));
        self
    }
    /// Acquire host-scoped catalogues and pin selected immutable language generations
    /// before request projection. The original startup C remains owned and typed.
    pub fn localization<L: Send + Sync + 'static>(
        mut self,
        definition: impl IntoLocalization<L, R>,
    ) -> Self
    where
        R: HasClock + HasTaskSpawner,
    {
        let definition = match definition.into_localization() {
            Ok(definition) => definition,
            Err(error) => {
                self.configuration.push(WebError::Localization(error));
                return self;
            }
        };
        let errors = self.errors.clone();
        let localization_error = move |error| errors(WebError::Localization(error));
        self.features.push(Arc::new(move |_, scope| {
            let selected_error = localization_error.clone();
            effect_map! {
                program: definition.scoped(scope, localization_error.clone()),
                map: move |runtime| {
                    let errors = selected_error.clone();
                    let hook: RequestHook<E, R> = Arc::new(move |request| {
                        let response_request = request.clone();
                        let select_error = errors.clone();
                        let headers_error = errors.clone();
                        effect! {
                            bind;
                            program: effect_map_error! {
                                program: runtime.for_request(request), map: select_error,
                            },
                            body: move |catalog| {
                                let request = response_request.clone();
                                effect_try_sync! {
                                    once;
                                    body {
                                        request.with_extensions(|extensions| {
                                            if let Some(headers) =
                                                extensions.get::<effect_server::ResponseHeaders>()
                                            {
                                                catalog.response_headers(
                                                    &mut headers.0.lock().unwrap_or_else(
                                                        std::sync::PoisonError::into_inner,
                                                    ),
                                                )?;
                                            }
                                            Ok::<(), crate::localization::LocalizationError>(())
                                        })?;
                                        request.insert_extension::<PinnedCatalog<L>>(catalog);
                                        Ok(())
                                    },
                                    on_error: headers_error.clone(),
                                }
                            },
                        }
                    });
                    vec![hook]
                },
            }
        }));
        self
    }
    /// Select complete operational policy; route installation reuses the six named
    /// health/ready/live aliases and any explicitly protected diagnostic routes.
    pub fn availability(
        mut self,
        availability: impl crate::availability::IntoAvailability<E, R>,
    ) -> Self {
        match availability.into_availability() {
            Ok(availability) => self.availability = Some(availability),
            Err(error) => self.configuration.push(WebError::Availability(error)),
        }
        self
    }
    /// Prepare one real Server in the supplied outer host scope. Startup acquisition
    /// and dynamic policy failures remain typed; no listener starts during prepare.
    pub fn prepare(&self, scope: Scope<E>) -> Effect<Server<E, R>, E, R>
    where
        R: HasRequest + HasClock + HasTaskSpawner,
    {
        let definition = self.clone();
        let startup = self.startup.clone();
        let startup_scope = scope.clone();
        effect! {
        bind;
        program: effect! { defer; body: move || startup(startup_scope.clone()), },
        body: move |capabilities| {
                   let capabilities = Arc::new(capabilities);
                   let definition = definition.clone();
                   let scope = scope.clone();
                   effect_future! {
                       exit_with_context; body: move |env: Arc<R>, context| {
                       let capabilities = capabilities.clone();
                       let definition = definition.clone();
                       let scope = scope.clone();
                       async move {
                           if let Some(error) = definition.configuration.first() {
                               return Exit::Fail((definition.errors)(error.clone()));
                           }
                           let mut policy = definition.policy.clone();
                           if let Some(observe) = &definition.observer {
                               let observe = observe.clone();
                               policy = server! {
                                   policy;
                                   source: policy,
                                   on_exit(move |env, request, completion| observe(env, request, completion)),
                               };
                           }
                           let mut routes = (definition.routes)(capabilities.clone());
                           if let Some(availability) = &definition.availability {
                               let operational = match availability.routes() {
                                   Ok(routes) => routes,
                                   Err(error) => {
                                       return Exit::Fail((definition.errors)(WebError::Configuration(
                                           error.to_string(),
                                       )));
                                   }
                               };
                               routes = match router! {
                                   source: routes,
                                   include(operational),
                               } {
                                   Ok(routes) => routes,
                                   Err(error) => return Exit::Fail((definition.errors)(WebError::Configuration(error.to_string()))),
                               };
                           }
                           let resolve = definition.environment.clone();
                           let request_capabilities = capabilities.clone();
                           let mut server = match server! {
                               policy: policy,
                               environment: server! {
                                   request_binding;
                                   body: move |request| resolve(request_capabilities.clone(), request),
                               },
                               listen: definition.listen,
                               protocols: definition.protocols,
                               routes: routes,
                           } {
                               Ok(server) => server,
                               Err(error) => return Exit::Fail((definition.errors)(WebError::Configuration(error.to_string()))),
                           };
                           // First transport/security metadata, then native auth/culture hooks;
                           // selected environment precedes scoped authorization/CSRF/cache.
                           if let Some(security) = &definition.security {
                               let security = match security {
                                   Ok(security) => security.clone(),
                                   Err(error) => return Exit::Fail((definition.errors)(error.clone())),
                               };
                               server = match install_security(server, security, definition.errors.clone()) {
                                   Ok(server) => server,
                                   Err(error) => return Exit::Fail((definition.errors)(error)),
                               };
                           }
                           for feature in &definition.features {
                               let hooks = match effect::effect! {
                                   run_in_context;
                                   context: context,
                                   program: feature(capabilities.clone(), scope.clone()),
                                   environment: env.clone(),
                               }
                                   .await
                                   .into_failure()
                               {
                                   Ok(hooks) => hooks,
                                   Err(exit) => return exit,
                               };
                               for hook in hooks {
                                   server = server! {
                                       source: server,
                                       before_environment(move |request| hook(request)),
                                   };
                               }
                           }
                           if let Some(output) = &definition.output_definition {
                               let definition = output.clone();
                               server = server! {
                                   source: server,
                                   before_environment(move |request| {
                                   let definition = definition.clone();
                                   effect_sync! {
                                       body: move || {
                                       request.insert_extension(crate::output_cache::RootOutputCache {
                                           definition: definition.clone(),
                                       })
                                   }, }
                                   }),
                               };
                           }
                           server = server! {
                               source: server,
                               middleware(definition.middleware.clone()),
                           };
                           for configure in &definition.builders {
                               let configure = configure.clone();
                               let cap = capabilities.clone();
                               server = server! {
                                   source: server,
                                   configure_builder(move |router| configure(cap.clone(), router)),
                               };
                           }
                           for configure in &definition.apps {
                               let configure = configure.clone();
                               let cap = capabilities.clone();
                               server = server! {
                                   source: server,
                                   configure_app(move |router| configure(cap.clone(), router)),
                               };
                           }
                           for install in &definition.installers {
                               server = match install(capabilities.clone(), server) {
                                   Ok(server) => server,
                                   Err(error) => return Exit::Fail(error),
                               };
                           }
                           Exit::Success(server)
                       }
                   }, }
               },
        }
    }
    /// Acquire/start/use/drain through the existing Server host; the outer scope
    /// retains startup caches/catalogue workers until native requests finish cleanup.
    pub fn with_host<A: Send + 'static>(
        &self,
        use_host: impl Fn(ServerHost) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<A, E, R>
    where
        R: HasRequest + HasClock + HasTaskSpawner + HasCancellation + HasTokio,
    {
        let definition = self.clone();
        let use_host = Arc::new(use_host);
        effect_scope! {
            body |scope: Scope<E>| {
                let definition = definition.clone();
                let use_host = use_host.clone();
                let server = crate::web! { prepare; source: definition, scope: scope, }?;
                server! { with_host; source: server, body: move |host| use_host(host), }?
            },
        }
    }
    /// Serve until the selected root cancellation capability requests shutdown,
    /// then drain and join request/startup cleanup under the same interpreter.
    pub fn run(&self) -> Effect<(), E, R>
    where
        R: HasRequest + HasClock + HasTaskSpawner + HasCancellation + HasTokio,
    {
        crate::web! { with_host; source: self, body: wait_for_cancellation, }
    }
}
fn wait_for_cancellation<E: Send + 'static, R: HasCancellation + Send + Sync + 'static>(
    _: ServerHost,
) -> Effect<(), E, R> {
    effect_future! {
        with_env; body: |environment: Arc<R>, _| async move {
            environment.cancellation().cancelled().await
        },
    }
}
/// Install typed security onto actual Server hooks. Response metadata is staged
/// before downstream guards so typed short circuits retain the same outer fields.
fn install_security<E: Send + 'static, R: HasRequest + Send + Sync + 'static>(
    mut server: Server<E, R>,
    security: Arc<Security<E, R>>,
    errors: Arc<dyn Fn(WebError) -> E + Send + Sync>,
) -> Result<Server<E, R>, WebError> {
    let static_csp = security.csp.as_ref()
        .filter(|csp| !csp.needs_nonce())
        .map(|csp| csp.header(None))
        .transpose()?;
    let static_hsts = security.hsts.map(crate::security::HstsPolicy::header).transpose()?;
    let static_headers = Arc::new((static_csp, static_hsts));
    let native_security = security.clone();
    server = server! {
        source: server,
        configure_builder(move |mut router| {
        if let Some(assets) = &native_security.static_files {
            router = router.nest_service(&assets.prefix, assets.native());
        }
        if let Some(compression) = &native_security.compression {
            router = router.layer(compression.clone());
        }
        if let Some(cors) = &native_security.cors {
            router = router.layer(cors.native());
        }
        if let Some(limit) = native_security.body_limit {
            router = router.layer(tower_http::limit::RequestBodyLimitLayer::new(limit.bytes()));
        }
        Ok(router)
        }),
    };
    let routing_security = security.clone();
    let routing_errors = errors.clone();
    let needs_updated_head = security.method_override.is_some() && security.forwarded_from.is_some();
    server = server.before_routing_checked(needs_updated_head, move |routing, request, updated, _| {
        // Mutable head and encoded body move once into the native stage. The Server
        // supplies the original interpreter, socket peer, cancellation and reactor.
        let inputs = std::sync::Mutex::new(Some((routing, request, updated)));
        let security = routing_security.clone();
        let errors = routing_errors.clone();
        effect_future! {
            exit_with_context; body: move |env, context| {
            let inputs = inputs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let security = security.clone();
            let errors = errors.clone();
            async move {
                let Some((mut routing, request, updated)) = inputs else {
                    return Exit::Die(effect::Defect::new("web routing stage already consumed"));
                };
                if let Some(forwarding) = &security.forwarded_from {
                    let peer = request
                        .with_extensions(|extensions| extensions.get::<SocketAddr>().copied());
                    let Some(peer) = peer else {
                        return Exit::Fail(errors(WebError::Metadata(
                            "native transport peer".into(),
                        )));
                    };
                    let transport = match forwarding.resolve(peer, request.headers()) {
                        Ok(transport) => transport,
                        Err(error) => return Exit::Fail(errors(error)),
                    };
                    if let Err(error) = forwarded_head(&mut routing, &transport) {
                        return Exit::Fail(errors(error));
                    }
                    request.insert_extension(transport);
                }
                if let Some(nonce) = &security.nonce {
                    let nonce = match effect::effect! {
                        run_in_context;
                        context: context,
                        program: nonce(),
                        environment: env,
                    }.await {
                        Exit::Success(nonce) => nonce,
                        Exit::Fail(error) => return Exit::Fail(error),
                        Exit::Die(defect) => return Exit::Die(defect),
                        Exit::Interrupted => return Exit::Interrupted,
                        Exit::FinalizationFailed(failure) => {
                            return Exit::FinalizationFailed(failure);
                        }
                    };
                    request.insert_extension(nonce);
                }
                if let Some(override_policy) = &security.method_override {
                    let request = match updated {
                        Some(updated) => match updated.publish(&routing) {
                            Ok(request) => request,
                            Err(error) => return Exit::Success(Err(error)),
                        },
                        None => request,
                    };
                    match override_policy.resolve(&request).await {
                        Ok(Some(method)) => routing.parts_mut().method = method,
                        Ok(None) => {}
                        Err(error) => {
                            let reported = if let ServerError::SharedInput(shared) = &error {
                                shared.as_ref()
                            } else {
                                &error
                            };
                            if let ServerError::Memory(reason) = reported {
                                return Exit::Success(Err(*reason));
                            }
                            return Exit::Fail(errors(WebError::Metadata(
                                "method override input".into(),
                            )));
                        }
                    }
                }
                Exit::Success(Ok(routing))
            }
        }, }
    });
    let response_security = security.clone();
    let response_errors = errors.clone();
    server = server! {
           source: server,
           after_response(move |request, response| {
           let security = response_security.clone();
           let errors = response_errors.clone();
           let static_headers = static_headers.clone();
           let present = request
               .with_extensions(|extensions| extensions.get::<crate::csp::ResponseNonce>().cloned());
           let nonce = match present {
               Some(nonce) => effect_from_result! { once; result: Ok(Some(nonce)), },
               None => match &security.nonce {
                   Some(generate) => effect_map! { program: generate(), map: Some, },
                   None => effect_sync! { body { None }, },
               },
           };
           let response = std::sync::Mutex::new(Some(response));
           effect! {
    bind;
    program: nonce,
    body: move |nonce| {
               if let Some(nonce) = &nonce {
                   request.insert_extension(nonce.clone());
               }
               let secure = request.with_extensions(|extensions| {
                   extensions
                       .get::<crate::security::ForwardedTransport>()
                       .is_some_and(|transport| transport.scheme.as_deref() == Some("https"))
               }) || request.uri().scheme_str() == Some("https");
               let response = response
                   .lock()
                   .unwrap_or_else(std::sync::PoisonError::into_inner)
                   .take();
               let Some(mut response) = response else {
                   return effect_die! { defect: effect::Defect::new("web response wrapper already consumed"), };
               };
               let headers = security.wrap_headers_with(response.headers_mut(), secure, |csp| {
                   if let Some(header) = &static_headers.0 {
                       return Ok(header.clone());
                   }
                   match request.take_prepared_response_header_buffer().map_err(web_server_error)? {
                       Some(buffer) => csp.header_prepared(nonce.as_ref(), buffer),
                       None => csp.header(nonce.as_ref()),
                   }
               }, || Ok(static_headers.1.clone()));
               effect_from_result! {
                   once;
                   result: headers.map(|()| response).map_err(|error| errors(error)),
               }
           },
    }
           }),
       };
    if let Some(authenticate) = &security.authentication {
        let authenticate = authenticate.clone();
        let errors = errors.clone();
        server = server! {
                   source: server,
                   before_environment(move |request| {
                   let errors = errors.clone();
                   match request
                       .with_extensions(|extensions| extensions.get::<effect_server::RouteKind>().copied())
                   {
                       Some(effect_server::RouteKind::Native) => effect_sync! { body { () }, },
                       Some(effect_server::RouteKind::Endpoint) => {
                           effect! {
        bind;
        program: authenticate(request),
        body: move |accepted| {
                               if accepted {
                                   effect_sync! { body { () }, }
                               } else {
                                   let errors = errors.clone();
                                   effect_fail! { error: errors(WebError::Rejected("authentication")), }
                               }
                           },
        }
                       }
                       None => effect_fail! {
                           error: errors(WebError::Metadata("native route classification".into())),
                       },
                   }
                   }),
               };
    }
    let mut guards = pipeline! {};
    for (name, guard) in [
        ("authorization", security.authorization.clone()),
        ("antiforgery", security.antiforgery.clone()),
    ] {
        if let Some(guard) = guard {
            let errors = errors.clone();
            guards = pipeline! {
                source: guards,
                guard(move |request| guard(request), move || errors(WebError::Rejected(name))),
            };
        }
    }
    Ok(server! {
        source: server,
        middleware(guards),
    })
}

/// Apply only verified forwarding to the actual native routing head. Header/URI
/// authorities agree before route restrictions and request projection can inspect them.
fn forwarded_head(
    routing: &mut effect_server::RoutingRequest,
    transport: &crate::security::ForwardedTransport,
) -> Result<(), WebError> {
    if transport.scheme.is_none() && transport.authority.is_none() {
        return Ok(());
    }
    let mut parts = routing.parts().uri.clone().into_parts();
    if let Some(authority) = &transport.authority {
        parts.authority = Some(authority.clone());
        let value = http::HeaderValue::from_str(authority.as_str())
            .map_err(|_| WebError::Metadata("forwarded authority".into()))?;
        routing
            .parts_mut()
            .headers
            .insert(http::header::HOST, value);
    }
    if let Some(scheme) = &transport.scheme {
        parts.scheme = Some(
            scheme
                .parse()
                .map_err(|_| WebError::Metadata("forwarded scheme".into()))?,
        );
    }
    if parts.scheme.is_some() && parts.authority.is_none() {
        let hosts = routing.parts().headers.get_all(http::header::HOST);
        let mut values = hosts.iter();
        let (Some(value), None) = (values.next(), values.next()) else {
            return Err(WebError::Metadata("forwarded native Host".into()));
        };
        parts.authority = Some(
            value
                .to_str()
                .map_err(|_| WebError::Metadata("forwarded native Host".into()))?
                .parse()
                .map_err(|_| WebError::Metadata("forwarded native Host".into()))?,
        );
    }
    if parts.authority.is_some() && parts.scheme.is_none() {
        parts.scheme = Some(http::uri::Scheme::HTTP);
    }
    routing.parts_mut().uri =
        http::Uri::from_parts(parts).map_err(|_| WebError::Metadata("forwarded URI".into()))?;
    Ok(())
}

#[cfg(feature = "otel")]
impl<C: Send + Sync + 'static, E: Send + 'static, R: effect_otel::HasOtel + Send + Sync + 'static>
    Web<C, E, R>
{
    /// Compose the actual optional OTel Server bridge at the pre-listen stage.
    /// Existing provider ownership and final-body observation remain with OTel/Server.
    pub fn otel(self, observability: crate::observability::Observability<E, R>) -> Self {
        self.install_server(move |_, server| Ok(observability.install(server)))
    }
}
