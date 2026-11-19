//! Typed, lazy HTTP programs over Effect with one axum transport boundary.
//!
//! Request metadata is shared; request bodies have one checked owner. Endpoint
//! scopes survive response construction and close only after delivery or disconnect.
//! Mutable [`RoutingRequest`] stages run before actual native route selection and
//! typed response decoration covers native short circuits as well as endpoints.
//! Transport peers come from the accepted socket; no forwarded header supplies that root.
//! Native I/O requires the caller's Tokio capability, while other Effect providers
//! may coexist in the same environment. No runtime is constructed by this crate.
extern crate self as effect_server;

fn try_admission_lock<T>(mutex: &std::sync::Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, effect::memory::MemoryError> {
    match mutex.try_lock() {
        Ok(guard) => Ok(guard),
        Err(std::sync::TryLockError::Poisoned(error)) => Ok(error.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => Err(effect::memory::MemoryError::Busy),
    }
}

/// Typed callback blocks using this facade's existing Effect dependency.
pub use effect::{
    EffectEnvironment,
    codec, effect_all, effect_all_concurrent, effect_blocking, effect_cpu, effect_die, effect_fail,
    effect_finalizer, effect_for_each, effect_for_each_concurrent, effect_from_option, effect_from_result,
    effect_future, effect_map, effect_map_error, effect_on_interrupt, effect_permit,
    effect_provide_some, effect_provided, effect_race, effect_recover, effect_ref, effect_repeat,
    effect_resource, effect_retry, effect_sandbox, effect_schema_parse, effect_scope,
    effect_scope_acquire, effect_socket, effect_stream_for_each, effect_sync, effect_timeout,
    effect_try_blocking, effect_try_cpu, effect_try_future, effect_try_sync,
    effect_uninterruptible, effect_using, effect_with_layer, layer, pool, request_resolver,
    schedule, sink, weak_cache,
};

mod body;
mod endpoint;
mod host;
mod input;
mod prepared_body_completion;
#[allow(dead_code)] // Connection settlement wiring is not yet connected to the host.
mod prepared_connection_retention;
mod prepared_endpoint;
mod prepared_native_slot;
mod prepared_native_bytes;
pub use prepared_native_bytes::{PreparedNativeBytesCapacity, PreparedNativeBytesDemand, PreparedNativeBytesPool, PreparedNativeFrame};
#[allow(dead_code)] // prepared provider wiring is in progress
mod prepared_native_task;
mod prepared_observers;
mod prepared_request_control;
mod prepared_streaming_channel;
#[allow(dead_code)] // Prepared host terminal wiring is in progress.
mod prepared_terminal;
#[allow(dead_code)] // Prepared transport wiring is not yet connected to the host.
mod prepared_transport_body;
mod reply;
mod prepared_request_storage;
pub use prepared_request_storage::{PreparedRequestStorageCapacity, PreparedRequestStorageDemand, PreparedRequestStoragePool};
mod prepared_response_headers;
pub use prepared_response_headers::{PreparedResponseHeadersDemand, PreparedResponseHeadersPool};
mod route;
mod validators;
pub use prepared_native_task::{PreparedNativeTaskFuture, PreparedNativeTaskPool};
pub use prepared_endpoint::{PreparedInputEndpoint, PreparedInputEndpointPlan};
pub use prepared_streaming_channel::{PreparedBodyChannelCapacity, PreparedBodyChannelDemand};
pub use prepared_terminal::{
    PreparedConnectionTaskDemand, PreparedHostCapacity, PreparedHostResult, PreparedRequestControlCapacity, PreparedServer,
};
#[cfg(feature = "websocket")]
mod websocket;

pub use body::*;
/// Describe a reusable endpoint using core Effect/resource binds and ordinary Rust
/// control flow. Inline resources bind to the endpoint owner and survive until native
/// body delivery/disconnect and asynchronous cleanup. Descriptor form accepts exactly
/// one `body { ... }`, `effect(extracted_effect)`, or `source(existing_endpoint)`, followed by `pipe_through`,
/// `on_error |error| { ... }`/`on_error(callback)`, and `preconditions(callback, mapper)`.
/// Extracted Effects keep their own explicit resource ownership; no payload is cloned.
/// `body: callback` retains a native scoped future factory and its complete Exit errors.
/// A `prepared` endpoint with a `contract` can prepare its actual storage at
/// startup with `capacity(memory, region, runs, frames)`. Its optional
/// `on_capacity: reply` accepts a `StaticOverloadReply` for early route refusal.
/// `byte_pools: pools` selects an `Arc<PreparedEndpointBytePools>`. Its encoded
/// body, decoded byte destination and output destination are checked out together
/// before endpoint selection and are available through the selected `Request`.
/// Decoded storage bounds explicit writes into that destination. A caller codec
/// that returns an independently allocated value has its own allocation policy.
///
/// ```
/// use effect::effect_sync;
/// use effect_server::{Endpoint,ServerError,endpoint,reply};
/// let endpoint: Endpoint<ServerError,()> = endpoint! {
///     let greeting = effect_sync! { body { String::from("ready") } }?;
///     reply! { text(greeting) }
/// };
/// let _ = endpoint;
/// ```
pub use effect_macros::endpoint;
/// Compose middleware in declaration/nesting order around an owned awaited continuation.
/// `before`, `after`, `respond`, and `on_exit` accept extracted callbacks or inline
/// `|request| { ... }` bodies (`on_exit` also receives a borrowed outcome). `guard` takes
/// an Effect predicate and a fresh rejection factory; `stage` takes request and continuation;
/// `set_header` and `include` compose native metadata and nested pipelines. One-shot inner
/// bodies move callback payloads without requiring Clone. Before capturing an on-exit
/// outcome in asynchronous work, inspect it into owned data in leading pure `let` statements.
/// After/on-exit stages await downstream delivery and endpoint cleanup; a failing error
/// renderer is never recursively invoked, and failures after commitment cannot replace bytes.
///
/// ```
/// use effect_server::{Pipeline,ServerError,pipeline};
/// let pipeline: Pipeline<ServerError,()> = pipeline! {
///     before |request| { let _method=request.method().clone(); () }
///     on_exit |_request, outcome| {
///         let _succeeded=matches!(outcome,effect::PrimaryOutcomeRef::Succeeded);
///         ()
///     }
/// };
/// let _ = pipeline;
/// ```
/// `source(existing_pipeline)` must precede stages and reuses that owned description.
///
/// `prepared_stage` receives a [`PreparedNext`] cursor. Select prepared inner
/// request controls on the host to provision one run cell per inner dispatch.
/// A cursor clone retains that cell and its request through its last owner.
/// Calling `invoke()` repeatedly selects the same next stage.
///
/// ```rust,ignore
/// use effect_server::{Pipeline, PreparedNext, Request, ServerError, pipeline};
///
/// let middleware: Pipeline<ServerError, ()> = pipeline! {
///     prepared_stage(|_request: Request, next: PreparedNext<ServerError, ()>| next.invoke());
/// };
/// ```
///
/// The prepared cursor shares the configured stage descriptions. In a mixed
/// pipeline, each reached legacy stage still receives its ordinary Arc
/// continuation. Effects constructed by stage callbacks retain their own
/// allocation policy.
pub use effect_macros::pipeline;
/// Attach registration metadata to a completed endpoint without running its body.
/// Required `endpoint` is separate from `name`, `tags`, `summary`, `description`,
/// `accepts`, `produces`, `hosts`, `authorize`, `allow_anonymous`, and native
/// `metadata`/`configure` callbacks. `authorize` and `allow_anonymous` conflict.
/// `pipe_through`/`filter` and `on_error` add scoped endpoint policies. Native media
/// and host syntax are validated before serving; authorization metadata is interpreted
/// by the caller's trusted policy rather than granting access by itself.
///
/// ```
/// use effect_server::{Endpoint,ServerError,endpoint,reply,registered};
/// let endpoint: Endpoint<ServerError,()> = endpoint! {
///     reply! { text(String::from("ready")) }
/// };
/// let registered = registered! { endpoint: endpoint,
///     name: "readiness", summary: "Application readiness",
///     produces: vec![effect_server::native::http::HeaderValue::from_static("text/plain")] };
/// let _ = registered;
/// ```
pub use effect_macros::registered;
/// Stage response metadata and consume exactly one terminal body. Metadata precedes
/// the terminal and supports status/header/append/remove, location/created, language,
/// cache control/vary, validators/conditional GET, trailers, and native cookies.
/// Dynamic body statuses require `BodyStatus`; bodyless terminals control their own
/// status. Missing/multiple bodies and bodyless status/trailer combinations are errors.
/// Terminals cover text, HTML, render, JSON/writer codecs, problem/validation, bytes,
/// stream/SSE, negotiate, native, redirect, file path, no-content, and empty.
/// `source(reply)` applies metadata to an existing Reply; `finite(reply)` transfers
/// an owned FiniteReply through the native constructor without collecting a body.
/// `lease(view, media)` retains a region buffer view without copying its bytes,
/// using one native Bytes owner control. `frame(frame, media)` moves a prepared
/// native frame without adding an owner control or copying the frame again.
/// HTML strings require `HtmlContent::text` or the explicit trusted-markup constructor.
/// Codec JSON requires `JsonWriterCodec::require` and an explicit writer policy.
///
/// ```
/// use effect_server::{Reply,HtmlContent,ServerError,reply};
/// let reply: Reply<ServerError,()> = reply! {
///     html(HtmlContent::text("<user supplied>"))
/// };
/// let _ = reply;
/// ```
pub use effect_macros::reply;
/// Build a checked nonempty set of lazy media/language alternatives. Calls are
/// `variant(media, optional_language, renderer)`, `text(renderer)`, `html(renderer)`,
/// `stream(media, renderer)`, `json_codec(witness, policy)`, and
/// `language(tag, representation)`. The result is typed `Result<Representations<...>,
/// ServerError>`; duplicate alternatives fail during construction. Negotiation evaluates
/// quality, specificity, explicit exclusions, and media parameters before invoking a
/// selected renderer. HEAD retains selected fields without invoking that renderer.
///
/// ```
/// use effect_server::{ServerError,representations};
/// let choices = representations! {
///     text(|text: String| effect::Effect::<String,ServerError,()>::from_result_once(Ok(text)))
/// };
/// let _ = choices;
/// ```
pub use effect_macros::representations;
/// Build a reversible typed route. Begin with `literal(absolute_path)` or an extracted
/// `prefix(route)`, then literal segments and required/constrained captures. First captures
/// are decoded directly; later captures form nested tuples. Optional/default/catch-all
/// captures are terminal and lower to finite native registrations when necessary.
/// `name`, `tags`, and native `configure` describe registration metadata. Both request
/// decoding and reverse `Route::path` use the same scalar codec/refinement and escaping.
/// Construction returns `Result<Route<A>,ServerError>` and validates dynamic syntax.
///
/// ```
/// use effect_server::{Route,Scalar,ServerError,route};
/// fn user_route() -> Result<Route<u64>,ServerError> {
///     let id=Scalar::new(|text: &str| text.parse::<u64>().map_err(|_| ServerError::Invalid("id".into())), |id: &u64| Ok(id.to_string()));
///     route! { name("user"), literal("/users"), capture("id", id) }
/// }
/// ```
pub use effect_macros::route;
/// Assemble ordered native registrations without evaluating endpoint bodies. Standard
/// methods take `(typed_route, typed_handler)` or `(constant_path, endpoint_or_registered)`;
/// custom `methods` take an explicit `HttpMethodSet`. `include`, `group`,
/// `group_configured(prefix, child, native_callback)`, `native`, `fallback`, `pipe_through`,
/// and `on_error` retain their scopes. `when(condition, routes)` and `each(values, make)`
/// compose extracted modules. Feature-gated `websocket` accepts a native/core session policy.
/// The result is `Result<Router<E,R>,ServerError>`; explicit HEAD takes precedence over
/// GET fallback, and native 404/405 semantics include the appropriate Allow field.
///
/// ```
/// use effect_server::{Endpoint,Router,ServerError,endpoint,reply,router};
/// let endpoint: Endpoint<ServerError,()> = endpoint! {
///     reply! { text(String::from("ready")) }
/// };
/// let routes: Result<Router<ServerError,()>,ServerError> = router! {
///     get("/ready", endpoint)
/// };
/// let _ = routes;
/// ```
/// Start from an existing owned router with `source(existing_router)` before adding routes.
pub use effect_macros::router;
/// Reusable route-module syntax with the same checked grammar and result as `router!`.
/// Include the returned description using `include(module)`; request bodies remain lazy.
pub use effect_macros::routes;
/// Build an inert host description with required `policy`, `environment`, `listen`,
/// `protocols`, and `routes` fields. Extracted expressions evaluate once in written
/// order; neither endpoint bodies nor listening run during construction. The result
/// is `Result<Server<E,R>, ServerError>` and starts only with `run` or `with_host`.
/// `source(existing_server)` replaces the construction fields and returns the configured Server directly.
/// Optional calls are `configure_app`, `configure_builder`, `before_routing`,
/// `after_response`, `before_environment`,
/// `middleware`, `around_request`, `around_selection`, and feature-gated `websockets`.
/// `around_selection` begins under root R after classification and before authentication;
/// `around_request` begins under selected R after environment resolution.
/// Supply `memory`, `region`, `capacity`, `result_frames`, `cleanup` and
/// `request_controls` together to prepare the host and return
/// `Result<PreparedServer<E, R>, ServerError>`. This also works with
/// `source(existing_server)`. Optional `admission` couples each admitted request
/// control to its caller-provided numeric quota. An optional `overload` takes a
/// `StaticOverloadReply` for early prepared-request capacity refusal.
///
/// ```
/// use effect_server::{Server, ServerError, ServerPolicy, RequestBinding, Router, HttpProtocols, server};
/// fn describe<E: Send + 'static,R: Send + Sync + 'static>(
///     policy: ServerPolicy<E,R>, environment: RequestBinding<E,R>,
///     listen: std::net::SocketAddr, routes: Router<E,R>,
/// ) -> Result<Server<E,R>,ServerError> {
///     server! { policy: policy, environment: environment,
///         listen: listen, protocols: HttpProtocols::Http1, routes: routes,
///         http1_header_timeout: std::time::Duration::from_secs(5) }
/// }
/// ```
pub use effect_macros::server;
/// Build a response-body program with `source(core_stream)` plus
/// `write |value, writer| { ... }`, or one `body |writer| { ... }`/`body(callback)`.
/// `leases(core_stream)` pulls owned region buffer views. `frames(core_stream)`
/// pulls prepared native frames. Both move the byte owner through the normal
/// awaited write path. Region views require one native Bytes owner control per
/// frame, and prepared native frames reuse their existing backing.
/// Inline binds use the existing Effect interpreter and request scope; native writes
/// await bounded transport acceptance before the next producer step. This export does
/// not alter the core `effect::stream!` grammar or `yield_from` delegation semantics.
/// Source, body, lease and frame forms are mutually exclusive. SSE uses the same program and a
/// separately selected framing/cursor policy; no detached writer is started.
///
/// ```
/// use effect_server::{BodyProgram,ServerError,stream};
/// let body: BodyProgram<ServerError,()> = stream! {
///     body |writer| { writer.write(effect_server::native::bytes::Bytes::from_static(b"ready"))?; () }
/// };
/// let _ = body;
/// ```
pub use effect_macros::server_stream as stream;
pub use endpoint::*;
pub use host::*;
pub use input::*;
pub use prepared_endpoint::*;
pub use reply::*;
pub use route::*;
pub use validators::*;
#[cfg(feature = "websocket")]
pub use websocket::*;
/// Native types remain available at explicit configuration and extractor boundaries.
pub mod native {
    pub use axum;
    pub use bytes;
    pub use headers;
    pub use http;
    pub use tower;
}
/// Sealed expansion support. Consumers should use the documented macros.
#[doc(hidden)]
pub mod __private {
    pub use effect;
    pub use tokio::runtime::Handle as TokioHandle;
    pub use effect::__private::{MacroContext, ScopedBody};
}

#[cfg(test)]
mod contract_tests;
