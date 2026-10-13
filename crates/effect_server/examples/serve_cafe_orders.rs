//! Retrieve customer 7's saved order 42 as HTML and JSON over a finite HTTP/1 host.
//! Run with
//! `CAFE_CHUNK_BYTES=4096 CAFE_JSON_BYTES=1024 CAFE_TELEMETRY_CAPACITY=8 CAFE_DRAIN_MS=3000 cargo +1.99.0 run -p effect-server --example serve_cafe_orders`.
//! The caller chooses the bounded frame size and graceful-drain deadline. The finite run
//! applies the supplied telemetry count to spans, metric descriptors and series per descriptor.
//! prints HTTP 200 HTML and JSON responses for order 42, then releases the host and providers.
//! The shared café telemetry support exports the authorized load spans locally; no collector is needed.
//! The public demonstration bearer identifies customer 7 on a loopback listener; it is an
//! explicit local policy, while the Identity consumer demonstrates native credential validation.
//! Business steps run in endpoint DSLs; only the outer owned host executes Effect.
//! The supplied record catalog projects `R = ()` and `Infallible` explicitly into the café's
//! request environment/error channel; absence and authorization remain separate domain failures.

use effect::{
    codec, effect_die, effect_map_error, effect_provide_some, effect_resource, effect_sync,
    effect_try_sync, service,
};
#[path = "../../effect_otel/examples/cafe_telemetry/mod.rs"]
pub mod cafe_telemetry;
#[cfg(not(test))]
use cafe_telemetry::TelemetryCapacity;
#[cfg(not(test))]
use effect::effect_try_future;
use effect::schema;
use effect::{Effect, JsonWriterPolicy, ParsePolicy};
#[cfg(not(test))]
use effect_otel::BorrowedProviders;
use effect_otel::{Otel, otel};
use effect_server::effect_from_option;
use effect_server::{
    BodyPolicy, Endpoint, HostPolicy, HtmlContent, HttpProtocols, HttpStatus, JsonWriterCodec,
    Reply, Request, RequestBinding, ResourceValidators, Route, Scalar, Server, ServerError,
    ServerPolicy, endpoint, pipeline, registered, reply, representations, route, router, routes,
    server,
};
#[cfg(not(test))]
use std::num::NonZeroUsize;
use std::sync::Arc;
#[cfg(not(test))]
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Domain tracing encloses the moved load result and authorization under one request context.
fn get_order(id: OrderId) -> Endpoint<CafeError, Cafe> {
    endpoint! {
        let customer=service! { env: |env:&Cafe| env.customer, }?;
        let order=otel! {
            name:"cafe.order.load",body {
            let loaded=effect_provide_some! { program: effect_map_error! { program: load_order(id), map: |never|match never {}, }, project: |_:&Cafe|(), }?;
            let order=require_order(loaded)?;
            authorize_order(customer,&order)?;
            Otel::tag("order.id",order.id.0.to_string())?;
            Otel::event("order.loaded")?;
            order
        }}?;
        let policy=service! { env: |env:&Cafe|env.json, }?;
        let choices=effect_try_sync! { once; body: { let __dsl_result = order_representations(policy).map_err(CafeError::Http); move || __dsl_result }, on_error: ::core::convert::identity, }?;
        let etag = match "\"order-42\"".parse() {
            Ok(etag) => etag,
            Err(error) => effect_die! { defect: effect::Defect::new(format!("order ETag: {error:?}")), }?,
        };
        reply! { status:HttpStatus::Ok, conditional_get(ResourceValidators {exists:true,etag:Some(etag),last_modified:None}); negotiate(choices,order); }
    }
}
/// An order's application identity, separate from HTTP path values.
#[derive(Clone, Copy)]
struct OrderId(u64);
/// A selected café customer; authorization compares this identity with the saved owner.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CustomerId(u64);
/// The moved application payload deliberately has no Clone implementation.
struct CoffeeOrder {
    /// Saved order identity used by typed reverse links.
    id: OrderId,
    /// Customer allowed to retrieve this order.
    owner: CustomerId,
    /// Display name selected by the café.
    drink: String,
}
/// Native input/transport failures and domain rejection remain distinct.
#[derive(Debug)]
pub(crate) enum CafeError {
    /// Structured HTTP/native failure.
    Http(ServerError),
    /// A checked native telemetry operation failed.
    Otel(effect_otel::NativeError),
    /// No selected customer or that customer does not own the order.
    Unauthorized,
    /// No saved order has this identity.
    NotFound,
}
/// Native request/provider composition selected by the outer host.
mod cafe_runtime;
pub(crate) use cafe_runtime::{Cafe, drain_providers, environment, order_policy};
/// One reversible codec enforces positive order IDs for routing and reverse links.
fn order_id() -> Scalar<OrderId> {
    Scalar::new(
        |raw| {
            raw.parse::<u64>()
                .ok()
                .filter(|id| *id > 0)
                .map(OrderId)
                .ok_or_else(|| ServerError::Invalid("positive order ID".into()))
        },
        |id: &OrderId| {
            if id.0 > 0 {
                Ok(id.0.to_string())
            } else {
                Err(ServerError::Invalid("positive order ID".into()))
            }
        },
    )
}
/// A reusable route module accepts an extracted codec with its concrete application type.
fn order_route() -> Result<Route<OrderId>, ServerError> {
    route! { name:"coffee-order", literal("/orders"); capture("order_id",order_id()); }
}
/// The pure local catalog constructs an owned optional order without request-provider requirements.
fn load_order(id: OrderId) -> Effect<Option<CoffeeOrder>, std::convert::Infallible, ()> {
    effect_sync! {
        body {
        (id.0 == 42).then(|| CoffeeOrder {
            id,
            owner: CustomerId(7),
            drink: "Oat latte".into(),
        })
        },
    }
}
/// Absence becomes the café's typed not-found case, without erasing transport or authorization errors.
fn require_order(order: Option<CoffeeOrder>) -> Effect<CoffeeOrder, CafeError, Cafe> {
    effect_from_option! {
        once; value: order, missing { CafeError::NotFound },
    }
}
/// Extracted renderer owns its input once after representation selection.
fn render_order_html(order: CoffeeOrder) -> Effect<HtmlContent, CafeError, Cafe> {
    effect_sync! {
        once; body { HtmlContent::text(format!("Order {}: {}", order.id.0, order.drink)) },
    }
}
/// Schema fields retain direct writer witnesses without cloning the owned order or building a DOM.
fn order_json(policy: ParsePolicy) -> Result<JsonWriterCodec<CoffeeOrder>, ServerError> {
    let id = codec! { transform_pure; source: effect::json_u64(), decode: |id| Ok(OrderId(id)), encode: |id: &OrderId| Ok(id.0), };
    let owner = codec! { transform_pure; source: effect::json_u64(), decode: |owner| Ok(CustomerId(owner)), encode: |owner: &CustomerId| Ok(owner.0), };
    let codec =
        schema! {struct CoffeeOrder {id:id,owner:owner,drink:effect::json_string()} policy:policy}
            .map_err(ServerError::Schema)?;
    JsonWriterCodec::require(codec)
}
/// JSON and HTML stay lazy; the JSON alternative writes directly on selected blocking R.
fn order_representations(
    policy: ParsePolicy,
) -> Result<effect_server::Representations<CoffeeOrder, CafeError, Cafe>, ServerError> {
    representations! {json_codec(order_json(policy)?,JsonWriterPolicy {bytes:policy.limits.bytes,depth:policy.limits.depth}); html(render_order_html);}
}
/// A real selected policy compares the authenticated local customer with the saved owner.
fn authorize_order(
    customer: Option<CustomerId>,
    order: &CoffeeOrder,
) -> Effect<(), CafeError, Cafe> {
    effect_try_sync! {
        once; body: {
        let __dsl_result = if customer == Some(order.owner) {
        Ok(())
    } else {
        Err(CafeError::Unauthorized)
    }; move || __dsl_result }, on_error: ::core::convert::identity, }
}
/// Expected errors select a fresh finite reply; renderer failure is never recursively handled.
fn error_reply(error: CafeError) -> Effect<Reply<CafeError, Cafe>, CafeError, Cafe> {
    let (status, message) = match error {
        CafeError::Unauthorized => (
            http::StatusCode::UNAUTHORIZED,
            String::from("Customer may not read this order"),
        ),
        CafeError::NotFound => (http::StatusCode::NOT_FOUND, String::from("Order not found")),
        CafeError::Http(error) => (http::StatusCode::BAD_REQUEST, error.to_string()),
        CafeError::Otel(error) => (http::StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    effect_try_sync! {
        once; body: move || {
        let status = HttpStatus::new(status)?;
        Ok(reply! { status: status, text(message); })
    }, on_error: CafeError::Http, }
}
/// An extracted reusable Effect constructs the readiness reply only when the endpoint runs.
fn health_reply() -> Effect<Reply<CafeError, Cafe>, CafeError, Cafe> {
    effect_sync! { body: || reply! {text(String::from("ready"));}, }
}
/// Extracted endpoint form keeps the same request/body owner as the inline order endpoint.
fn health_endpoint() -> Endpoint<CafeError, Cafe> {
    endpoint! {effect(health_reply())}
}
/// Record safe method metadata in the selected request span before endpoint work.
fn begin_request(request: Request) -> Effect<(), CafeError, Cafe> {
    Otel::event_with(
        "cafe.request.started",
        vec![effect_otel::KeyValue::new(
            "http.request.method",
            request.method().to_string(),
        )],
    )
}
/// Borrow the original outcome only while selecting owned telemetry; release remains awaited.
fn record_request_outcome(
    _request: Request,
    outcome: effect::PrimaryOutcomeRef<'_, CafeError>,
) -> effect::Release<CafeError, Cafe> {
    let status = match outcome {
        effect::PrimaryOutcomeRef::Succeeded => "succeeded",
        effect::PrimaryOutcomeRef::Failed(_) => "failed",
        effect::PrimaryOutcomeRef::Defected(_) => "defected",
        effect::PrimaryOutcomeRef::WasInterrupted => "interrupted",
    };
    effect_resource! {
        release_effect; program: Otel::event_with(
        "cafe.request.finished",
        vec![effect_otel::KeyValue::new("cafe.outcome", status)],
    ), }
}
/// Inert DSL-led assembly demonstrates typed routes, extracted modules and anonymous metadata.
pub(crate) fn describe(
    protocols: HttpProtocols,
    body: BodyPolicy,
    host: HostPolicy,
) -> Result<Server<CafeError, Cafe>, ServerError> {
    let health = registered! { endpoint:health_endpoint(), name:"cafe-health", summary:"Can the café accept requests?", allow_anonymous:true };
    let policy = pipeline! { set_header(http::HeaderName::from_static("x-cafe"),http::HeaderValue::from_static("neighborhood")); before |request| {begin_request(request)?;} on_exit(record_request_outcome); };
    let orders = router! {pipe_through:policy, on_error(error_reply); get(order_route()?,get_order); get("/health",health);};
    let cafe_routes = routes! {include(orders);};
    let binding = RequestBinding::new(|request: Request| {
        service! {
            env: move |root: &Cafe| {
            Arc::new(Cafe {
                telemetry: root.telemetry.clone(),
                json: root.json,
                cancellation: request.cancellation().clone(),
                customer: request
                    .headers()
                    .get(http::header::AUTHORIZATION)
                    .filter(|value| value.as_bytes() == b"Bearer cafe-demo-customer-7")
                    .map(|_| CustomerId(7)),
                request: request.clone(),
            })
        }, }
    });
    let server = server! {policy:ServerPolicy::new(body,host,CafeError::Http,error_reply),environment:binding,listen:std::net::SocketAddr::from(([127,0,0,1],0)),protocols:protocols,routes:cafe_routes}?;
    let propagation = effect_otel::Propagation::new(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
    Ok(effect_otel::server::install(
        server,
        propagation,
        |_| effect_otel::server::RequestSpan {
            name: Arc::from("cafe.http"),
            attributes: Vec::new(),
        },
        |_| effect_sync! { body: || (), },
        CafeError::Otel,
    ))
}
/// Native client support for this finite example; the caller selects method, fields and body.
pub(crate) async fn exchange(
    address: std::net::SocketAddr,
    request: &str,
) -> Result<String, ServerError> {
    let mut socket = tokio::net::TcpStream::connect(address)
        .await
        .map_err(ServerError::Io)?;
    socket
        .write_all(request.as_bytes())
        .await
        .map_err(ServerError::Io)?;
    let mut received = Vec::new();
    socket
        .read_to_end(&mut received)
        .await
        .map_err(ServerError::Io)?;
    String::from_utf8(received).map_err(|_| ServerError::Invalid("response UTF-8".into()))
}
/// Execute once at the outer host boundary, then drain providers and shut down caller-owned SDKs.
#[cfg(not(test))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let link = order_route()?.path(&OrderId(42))?;
    let chunk_bytes = NonZeroUsize::new(std::env::var("CAFE_CHUNK_BYTES")?.parse()?)
        .ok_or("CAFE_CHUNK_BYTES must be positive")?;
    let json_bytes = NonZeroUsize::new(std::env::var("CAFE_JSON_BYTES")?.parse()?)
        .ok_or("CAFE_JSON_BYTES must be positive")?;
    let telemetry = NonZeroUsize::new(std::env::var("CAFE_TELEMETRY_CAPACITY")?.parse()?)
        .ok_or("CAFE_TELEMETRY_CAPACITY must be positive")?;
    let drain = Duration::from_millis(std::env::var("CAFE_DRAIN_MS")?.parse()?);
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?,
    );
    let exported = opentelemetry_sdk::trace::InMemorySpanExporter::default();
    let traces = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_simple_exporter(exported.clone())
        .build();
    let metrics = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
    let logs = opentelemetry_sdk::logs::SdkLoggerProvider::builder().build();
    let root = environment(
        runtime.clone(),
        BorrowedProviders::new(&traces, &metrics, &logs),
        order_policy(json_bytes),
        TelemetryCapacity {
            active_spans: telemetry,
            metric_descriptors: telemetry,
            series_per_descriptor: telemetry,
        },
    )?;
    let host = describe(
        HttpProtocols::Http1,
        BodyPolicy {
            frames: NonZeroUsize::MIN,
            chunk_bytes,
        },
        HostPolicy { drain },
    )?;
    let execution=host.with_host(move |host| {let link=link.clone();effect_try_future! {
        once; body: move |_|async move {
        for media in ["text/html","application/json"] {let response=exchange(host.address,&format!("GET {link} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer cafe-demo-customer-7\r\nAccept: {media}\r\nConnection: close\r\n\r\n")).await?;use std::io::Write;writeln!(std::io::stdout().lock(), "{response}").map_err(ServerError::Io)?;}
        Ok::<_,ServerError>(())
    }, on_error: CafeError::Http, }});
    let exit = runtime.block_on(execution.run(root.clone()));
    runtime.block_on(drain_providers(&root));
    traces.force_flush()?;
    use std::io::Write;
    writeln!(
        std::io::stdout().lock(),
        "completed café spans: {}",
        exported.get_finished_spans()?.len()
    )?;
    logs.shutdown()?;
    metrics.shutdown()?;
    traces.shutdown()?;
    match exit {
        effect::Exit::Success(()) => Ok(()),
        other @ (effect::Exit::Fail(_)
        | effect::Exit::Die(_)
        | effect::Exit::Interrupted
        | effect::Exit::FinalizationFailed(_)) => Err(format!("café host: {other:?}").into()),
    }
}
