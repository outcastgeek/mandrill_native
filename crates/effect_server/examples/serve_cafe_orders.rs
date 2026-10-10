//! A finite café HTTP/1 consumer; needs the `http1` feature. Run with
//! `CAFE_CHUNK_BYTES=4096 CAFE_JSON_BYTES=1024 CAFE_TELEMETRY_CAPACITY=8 CAFE_DRAIN_MS=3000 cargo run -p effect-server --example serve_cafe_orders`.
//! The caller chooses the bounded frame size and graceful-drain deadline. The finite run
//! applies the supplied telemetry count to spans, metric descriptors and series per descriptor.
//! prints HTTP 200 HTML and JSON responses for order 42, then releases the host and providers.
//! The shared café telemetry support exports the authorized load spans locally; no collector is needed.
//! The public demonstration bearer identifies customer 7 on a loopback listener; it is an
//! explicit local policy, while the Identity consumer demonstrates native credential validation.
//! Business steps run in endpoint DSLs; only the outer owned host executes Effect.
//! The pure fixture catalog projects `R = ()` and `Infallible` explicitly into the café's
//! request environment/error channel; absence and authorization remain separate domain failures.

#[path = "../../effect_otel/examples/cafe_telemetry/mod.rs"]
pub mod cafe_telemetry;
use cafe_telemetry::TelemetryCapacity;
use effect::{
    Cancellation, Effect, HasBlockingExecutor, HasCancellation, HasClock, HasTaskSpawner,
    HasTracer, JsonWriterPolicy, ParsePolicy,
};
use effect_otel::{BorrowedProviders, HasOtel, Otel, OtelBackend, otel};
use effect_server::{
    BodyPolicy, Endpoint, HasRequest, HasTokio, HostPolicy, HtmlContent, HttpProtocols, HttpStatus,
    JsonWriterCodec, Reply, Request, RequestBinding, ResourceValidators, Route, Scalar, Server,
    ServerError, ServerPolicy, endpoint, pipeline, registered, reply, representations, route,
    router, routes, server,
};
#[cfg(not(test))]
use std::time::Duration;
use std::{num::NonZeroUsize, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
/// Caller-owned Tokio I/O and scheduling capabilities beside application request state.
pub(crate) struct Cafe {
    /// Existing café provider composition, shared by root and request projections.
    telemetry: cafe_telemetry::Environment,
    /// Caller-supplied JSON budget and explicit record parse policy.
    json: ParsePolicy,
    /// Each selected request has its own cancellation source.
    cancellation: Cancellation,
    /// Actual body/metadata owner for this execution.
    request: Request,
    /// Trusted result of the explicit local demonstration policy.
    customer: Option<CustomerId>,
}
impl HasTaskSpawner for Cafe {
    /// Reuse the existing café task provider.
    type Tasks = effect_tokio::TokioTasks;
    /// Borrow the selected independent-child scheduler.
    fn task_spawner(&self) -> &Self::Tasks {
        &self.telemetry.tasks
    }
}
impl HasBlockingExecutor for Cafe {
    /// JSON native writes run on the existing independent blocking provider.
    type Executor = effect_tokio::TokioBlocking;
    /// Borrow the same provider retained through host drain.
    fn blocking_executor(&self) -> &Self::Executor {
        &self.telemetry.blocking
    }
}
impl HasClock for Cafe {
    /// Request projections preserve the existing monotonic origin.
    type Clock = effect_tokio::TokioClock;
    /// Borrow the selected native clock.
    fn clock(&self) -> &Self::Clock {
        &self.telemetry.clock
    }
}
impl HasTracer for Cafe {
    /// Both request and domain spans share the existing OTel-backed tracer.
    fn tracer(&self) -> &effect::Tracer {
        &self.telemetry.tracer
    }
}
impl HasOtel for Cafe {
    /// Borrow the selected native bridge without global discovery.
    fn otel(&self) -> &OtelBackend {
        &self.telemetry.otel
    }
}
impl effect_server::HasServerError for Cafe {
    /// JSON/input failures retain the café HTTP error channel.
    type Error = CafeError;
    /// Map the checked native cause once.
    fn server_error(&self, error: ServerError) -> CafeError {
        CafeError::Http(error)
    }
}
impl HasTokio for Cafe {
    /// Borrow the runtime kept alive by main through host drain.
    fn tokio(&self) -> &tokio::runtime::Handle {
        self.telemetry.runtime.handle()
    }
}
impl HasCancellation for Cafe {
    /// Select the cancellation authority belonging to this execution.
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}
impl HasRequest for Cafe {
    /// Preserve the real native request and its checked one-body owner.
    fn request(&self) -> &Request {
        &self.request
    }
}
/// Root capability state uses caller-owned providers and explicit telemetry/JSON storage bounds.
pub(crate) fn environment(
    runtime: Arc<tokio::runtime::Runtime>,
    providers: BorrowedProviders,
    json: ParsePolicy,
    capacity: TelemetryCapacity,
) -> Result<Arc<Cafe>, String> {
    let cancellation = Cancellation::new();
    Ok(Arc::new(Cafe {
        telemetry: cafe_telemetry::Environment::new(runtime, providers, capacity)?,
        json,
        request: Request::from_native(
            http::Request::new(axum::body::Body::empty()),
            cancellation.clone(),
        ),
        cancellation,
        customer: None,
    }))
}
/// The caller selects JSON bytes; structural bounds describe this record's three scalar fields.
pub(crate) fn order_policy(bytes: NonZeroUsize) -> ParsePolicy {
    ParsePolicy {
        limits: effect::ParseLimits {
            bytes,
            depth: NonZeroUsize::new(2).expect("record and scalar"),
            nodes: NonZeroUsize::new(4).expect("record plus three fields"),
            collection: NonZeroUsize::new(3).expect("three fields"),
            issues: NonZeroUsize::MIN,
        },
        errors: effect::SchemaErrorMode::First,
        unknown_fields: effect::UnknownFields::Reject,
        duplicate_fields: effect::DuplicateFields::Reject,
    }
}
/// Drain the reused scheduling providers before the caller shuts down its native SDK originals.
pub(crate) async fn drain_providers(root: &Cafe) {
    root.telemetry.tasks.stop().wait().await;
    root.telemetry.blocking.stop().wait().await;
}
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
    Effect::sync(move || {
        (id.0 == 42).then(|| CoffeeOrder {
            id,
            owner: CustomerId(7),
            drink: "Oat latte".into(),
        })
    })
}
/// Absence becomes the café's typed not-found case, without erasing transport or authorization errors.
fn require_order(order: Option<CoffeeOrder>) -> Effect<CoffeeOrder, CafeError, Cafe> {
    Effect::from_result_once(order.ok_or(CafeError::NotFound))
}
/// Extracted renderer owns its input once after representation selection.
fn render_order_html(order: CoffeeOrder) -> Effect<HtmlContent, CafeError, Cafe> {
    Effect::sync_once(move || HtmlContent::text(format!("Order {}: {}", order.id.0, order.drink)))
}
/// Schema fields retain direct writer witnesses without cloning the owned order or building a DOM.
fn order_json(policy: ParsePolicy) -> Result<JsonWriterCodec<CoffeeOrder>, ServerError> {
    let id = effect::json_u64().transform(|id| Ok(OrderId(id)), |id: &OrderId| Ok(id.0));
    let owner = effect::json_u64().transform(
        |owner| Ok(CustomerId(owner)),
        |owner: &CustomerId| Ok(owner.0),
    );
    let codec=effect::schema! {struct CoffeeOrder {id:id,owner:owner,drink:effect::json_string()} policy:policy}.map_err(ServerError::Schema)?;
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
    Effect::from_result_once(if customer == Some(order.owner) {
        Ok(())
    } else {
        Err(CafeError::Unauthorized)
    })
}
/// Domain tracing encloses the moved load result and authorization under one request context.
fn get_order(id: OrderId) -> Endpoint<CafeError, Cafe> {
    endpoint! {
        let customer=Effect::service(|env:&Cafe| env.customer)?;
        let order=otel! {name:"cafe.order.load",body {
            let loaded=load_order(id).map_error(|never|match never {}).provide_some(|_:&Cafe|())?;
            let order=require_order(loaded)?;
            authorize_order(customer,&order)?;
            Otel::tag("order.id",order.id.0.to_string())?;
            Otel::event("order.loaded")?;
            order
        }}?;
        let policy=Effect::service(|env:&Cafe|env.json)?;
        let choices=Effect::from_result_once(order_representations(policy).map_err(CafeError::Http))?;
        reply! { status:HttpStatus::Ok, conditional_get(ResourceValidators {exists:true,etag:Some("\"order-42\"".parse().expect("literal ETag")),last_modified:None}); negotiate(choices,order); }
    }
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
    Effect::sync_once(
        move || reply! {status:HttpStatus::new(status).expect("body status"), text(message);},
    )
}
/// An extracted reusable Effect constructs the readiness reply only when the endpoint runs.
fn health_reply() -> Effect<Reply<CafeError, Cafe>, CafeError, Cafe> {
    Effect::sync(|| reply! {text(String::from("ready"));})
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
    effect::Release::from_effect(Otel::event_with(
        "cafe.request.finished",
        vec![effect_otel::KeyValue::new("cafe.outcome", status)],
    ))
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
        Effect::service(move |root: &Cafe| {
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
        })
    });
    let server = server! {policy:ServerPolicy::new(body,host,CafeError::Http,error_reply),environment:binding,listen:"127.0.0.1:0".parse::<std::net::SocketAddr>().expect("loopback"),protocols:protocols,routes:cafe_routes}?;
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
        |_| Effect::sync(|| ()),
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
    let execution=host.with_host(move |host| {let link=link.clone();Effect::try_future_once(move |_|async move {
        for media in ["text/html","application/json"] {let response=exchange(host.address,&format!("GET {link} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer cafe-demo-customer-7\r\nAccept: {media}\r\nConnection: close\r\n\r\n")).await?;println!("{response}");}
        Ok::<_,ServerError>(())
    },CafeError::Http)});
    let exit = runtime.block_on(execution.run(root.clone()));
    runtime.block_on(drain_providers(&root));
    traces.force_flush()?;
    println!(
        "completed café spans: {}",
        exported.get_finished_spans()?.len()
    );
    logs.shutdown()?;
    metrics.shutdown()?;
    traces.shutdown()?;
    match exit {
        effect::Exit::Success(()) => Ok(()),
        other => Err(format!("café host: {other:?}").into()),
    }
}
