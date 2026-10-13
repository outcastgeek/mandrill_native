//! Replay a ticket's recorded status timeline to the customer's phone, with native tracing.
//!
//! Supply a UTF-8 CSV with no header: `elapsed_ms,status`, in increasing time order.
//! Accepted statuses: `accepted`, `preparing`, `ready`. For example, rows
//! `0,accepted`, `400,preparing`, `800,ready` produce those three SSE updates.
//! Run: `cargo +1.99.0 run -p effect-otel --features server --example watch_order_progress -- 127.0.0.1:8080 HISTORY.csv 256 32 32 16 8 5000 inline`.
//! Read: `curl -N http://127.0.0.1:8080/orders/104/events`. The final argument
//! selects one writer (`inline`, `extracted`, or `body`) for the same route.
//! The timeline is input data; this example does not prepare physical drinks.
//! Frame/cursor bytes, active spans, metric descriptors, series and drain/shutdown
//! milliseconds are positive caller-selected limits. Disconnect releases the
//! subscription. Ctrl-C drains the host and shuts down its owned native exporters.
//! Optional traceparent/baggage headers carry the caller's distributed context.

use effect::{
    effect_future, effect_map_error, effect_scope, effect_stream_for_each, effect_try_future,
};
mod cafe_telemetry;
use cafe_telemetry::{CafeError, Environment, TelemetryCapacity};
use effect::span;
use effect::stream;
use effect::{Effect, Exit, LogLevel, Stream, effect, effect_scope_acquire};
use effect_otel::effect_sync;
use effect_otel::effect_try_sync;
use effect_otel::{Otel, OwnedProviders, Propagation, ProviderFactories, ShutdownPolicy, otel};
use effect_server::stream as server_stream;
use effect_server::{
    BodyPolicy, BodyWriter, Endpoint, HostPolicy, HttpProtocols, ServerPolicy, SsePolicy,
    SseRecord, endpoint, reply, router, server,
};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

/// Reusable lazy status recipe: annotate and emit only when reached, then move
/// the status into the stream. Errors and native context remain those of the caller.
fn publish_status(
    status: &'static str,
    event: &'static str,
) -> Effect<String, CafeError, Environment> {
    effect! {
        Otel::tag("order.status", status)?;
        Otel::event(event)?;
        status.to_owned()
    }
}

/// Open subscriptions are real scope-owned registrations, not response construction counters.
fn order_updates(
    history: Arc<[StatusRecord]>,
    subscriptions: Arc<AtomicUsize>,
) -> Stream<String, CafeError, Environment> {
    stream! {
        open: move |scope| {
        let opened = subscriptions.clone();
        let acquired = effect_sync! {
            body {
            opened.fetch_add(1, Ordering::AcqRel);
            opened.clone()
            },
        };
        let timeline = history.clone();
        let source: Stream<String, CafeError, Environment> = stream! {
            let timeline = timeline.clone();
            let mut previous = Duration::ZERO;
            for record in timeline.iter() {
                Effect::sleep(record.elapsed - previous)?;
                previous = record.elapsed;
                let status = record.status;
                let update = otel! { name: "cafe.order.recorded-status", effect: publish_status(status.label(), status.event()) }?;
                yield update;
            }
        };
        let source_scope = scope.clone();
        effect! {
            let source_scope = source_scope.clone();
            let acquired = acquired.clone();
            let source = source.clone();
            effect_scope_acquire! {
                scope: source_scope.clone(), acquire: acquired,
                release |registered: Arc<AtomicUsize>| {
                    effect_sync! { body { registered.fetch_sub(1, Ordering::AcqRel); }, }?;
                },
            }?;
            source.open(source_scope)?
        }
    }, }
}

/// Inline writing uses the core pull-driven source and awaited SSE/flush boundaries.
fn watch_order(
    history: Arc<[StatusRecord]>,
    subscriptions: Arc<AtomicUsize>,
    sse: SsePolicy,
) -> Endpoint<CafeError, Environment> {
    endpoint! {
        let body = server_stream! {
            source: order_updates(history.clone(), subscriptions.clone()),
            write |update, writer| {
                let update: String = update;
                let writer: effect_server::BodyWriter<CafeError, Environment> = writer;
                // Transfer the payload once; only descriptions and sink handles are shared.
                let update_once: Effect<String, CafeError, Environment> = effect_sync! {
                    once; body { update },
                };
                otel! {
                    name: "cafe.order.update", body {
                    let writer = writer.clone();
                    let update_once = update_once.clone();
                    let update = update_once?;
                    Otel::tag("order.id", "104")?;
                    writer.sse(SseRecord { event: Some("order-status".into()), id: None, retry: None, data: update }, sse)?;
                    span! { name: "cafe.flush-status", effect: writer.flush() }?;
                }}?;
            }
        };
        reply! { sse(body); }
    }
}

/// Reusable typed writer recipe: move one status into a frame, then await flush.
fn write_status(
    update: String,
    writer: BodyWriter<CafeError, Environment>,
    sse: SsePolicy,
) -> Effect<(), CafeError, Environment> {
    let frame = writer.sse(
        SseRecord {
            event: Some("order-status".into()),
            id: None,
            retry: None,
            data: update,
        },
        sse,
    );
    effect! {
        let frame = frame.clone();
        let writer = writer.clone();
        frame?;
        writer.flush()?;
    }
}

/// Extracted writer callback keeps the same pull, write and subscription ownership.
fn watch_notifications(
    history: Arc<[StatusRecord]>,
    subscriptions: Arc<AtomicUsize>,
    sse: SsePolicy,
) -> Endpoint<CafeError, Environment> {
    endpoint! {
        let body = server_stream! {
            source: order_updates(history.clone(), subscriptions.clone()),
            write: move |update, writer| write_status(update, writer, sse),
        };
        reply! { sse(body); }
    }
}

/// Body form runs the same scoped source; the Effect owns each awaited write/flush.
/// Core run_for_each releases the subscription on exhaustion or interruption.
fn watch_progress(
    history: Arc<[StatusRecord]>,
    subscriptions: Arc<AtomicUsize>,
    sse: SsePolicy,
) -> Endpoint<CafeError, Environment> {
    endpoint! {
        let subscriptions = subscriptions.clone(); let history = history.clone();
        let body = server_stream! {
            body |writer| {
                let writer: BodyWriter<CafeError, Environment> = writer.clone();
                let subscriptions = subscriptions.clone(); let history = history.clone();
                let updates = order_updates(history.clone(), subscriptions);
                effect_stream_for_each! { stream: updates, body: move |update| write_status(update, writer.clone(), sse), }?;
            }
        };
        reply! { sse(body); }
    }
}

/// Caller-selected writer form; only one endpoint is mounted in each run.
#[derive(Clone, Copy)]
enum WriterMode {
    /// Write each update in the stream callback.
    Inline,
    /// Reuse the extracted typed frame writer.
    Extracted,
    /// The body Effect owns traversal and its writes.
    Body,
}
/// One actual recorded state and its elapsed timestamp.
struct StatusRecord {
    /// Time since this ticket's recorded acceptance.
    elapsed: Duration,
    /// State reported by the stored timeline.
    status: Status,
}
/// Stored vocabulary, separate from the runtime or tracing configuration.
#[derive(Clone, Copy)]
enum Status {
    /// Counter accepted the ticket.
    Accepted,
    /// Preparation started.
    Preparing,
    /// Cups reached pickup.
    Ready,
}
impl Status {
    /// Customer-facing status value in the SSE record.
    fn label(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Preparing => "preparing",
            Self::Ready => "ready",
        }
    }
    /// Native span event describes the supplied recorded phase.
    fn event(self) -> &'static str {
        match self {
            Self::Accepted => "order.accepted",
            Self::Preparing => "barista.started",
            Self::Ready => "pickup.ready",
        }
    }
}
/// Reject malformed or reordered input before creating request subscriptions.
fn read_history(path: &str) -> Result<Arc<[StatusRecord]>, Box<dyn std::error::Error>> {
    let csv = std::fs::read_to_string(path)?;
    let mut history = Vec::<StatusRecord>::new();
    for line in csv.lines() {
        let (elapsed, status) = line.split_once(',').ok_or("expected elapsed_ms,status")?;
        let elapsed = Duration::from_millis(elapsed.parse()?);
        let status = match status {
            "accepted" => Status::Accepted,
            "preparing" => Status::Preparing,
            "ready" => Status::Ready,
            _ => return Err("unknown recorded status".into()),
        };
        if history
            .last()
            .is_some_and(|previous| previous.elapsed > elapsed)
        {
            return Err("recorded times must not go backwards".into());
        }
        history.push(StatusRecord { elapsed, status });
    }
    if history.is_empty() {
        return Err("provide at least one recorded status".into());
    }
    Ok(history.into())
}
/// A finite native host remains owned until Ctrl-C, then joins requests and SDK cleanup.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let address: std::net::SocketAddr =
        args.next().ok_or("provide the listener address")?.parse()?;
    let history = read_history(&args.next().ok_or("provide recorded history CSV")?)?;
    let sse = SsePolicy {
        frame_bytes: args
            .next()
            .ok_or("provide maximum SSE frame bytes")?
            .parse()?,
        cursor_bytes: args
            .next()
            .ok_or("provide maximum SSE cursor bytes")?
            .parse()?,
    };
    let capacity = TelemetryCapacity {
        active_spans: args
            .next()
            .ok_or("provide maximum active span slots")?
            .parse()?,
        metric_descriptors: args
            .next()
            .ok_or("provide maximum metric descriptors")?
            .parse()?,
        series_per_descriptor: args
            .next()
            .ok_or("provide maximum label sets per descriptor")?
            .parse()?,
    };
    let drain_millis: NonZeroU64 = args
        .next()
        .ok_or("provide positive host-drain/SDK-shutdown milliseconds")?
        .parse()?;
    let drain = Duration::from_millis(drain_millis.get());
    let writer = match args.next().as_deref() {
        Some("inline") => WriterMode::Inline,
        Some("extracted") => WriterMode::Extracted,
        Some("body") => WriterMode::Body,
        _ => return Err("choose inline, extracted or body writer".into()),
    };
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let shutdown = ShutdownPolicy::new(drain, true)?;
    // Two workers are required by the selected independent-child task provider.
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?,
    );
    let traces = opentelemetry_sdk::trace::InMemorySpanExporter::default();
    let logs = opentelemetry_sdk::logs::InMemoryLogExporter::default();
    let metrics = opentelemetry_sdk::metrics::InMemoryMetricExporter::default();
    let trace_hook = traces.clone();
    let log_hook = logs.clone();
    let factories = ProviderFactories::new(
        move |builder| Ok(builder.with_simple_exporter(trace_hook.clone()).build()),
        move |builder| {
            Ok(builder
                .with_reader(
                    opentelemetry_sdk::metrics::PeriodicReader::builder(metrics.clone()).build(),
                )
                .build())
        },
        move |builder| Ok(builder.with_simple_exporter(log_hook.clone()).build()),
    );
    // Initial host services are borrowed originals. Owned providers are acquired
    // below before serving; this bootstrap does not shut down any host service.
    let bootstrap_traces = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let bootstrap_metrics = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
    let bootstrap_logs = opentelemetry_sdk::logs::SdkLoggerProvider::builder().build();
    let root = Arc::new(Environment::new(
        runtime.clone(),
        effect_otel::BorrowedProviders::new(&bootstrap_traces, &bootstrap_metrics, &bootstrap_logs),
        capacity,
    )?);
    let subscriptions = Arc::new(AtomicUsize::new(0));
    let watched = subscriptions.clone();
    let provider_runtime = runtime.clone();
    let program: Effect<(), CafeError, Environment> = effect_scope! {
    body: move |scope| {
            let factories = factories.clone();
            let provider_runtime = provider_runtime.clone();
            let watched = watched.clone();
            let history = history.clone();
            effect! {
            let factories = factories.clone();
            let provider_runtime = provider_runtime.clone();
            let watched = watched.clone(); let history = history.clone();
            let scope = scope.clone();
            let owned = OwnedProviders::acquire_with_error(scope, factories, shutdown, CafeError::Lifecycle)?;
            let providers = effect_try_sync! {
                once;
                on_error |error| { CafeError::Otel(effect_otel::NativeError::Native(error)) },
                body { owned.with(|owner| owner.borrowed()) },
            }?;
            let environment = effect_map_error! { program: effect_try_sync! { once; body: { let __dsl_result = Environment::new(provider_runtime, providers, capacity); move || __dsl_result }, on_error: ::core::convert::identity, }, map: |error| CafeError::Otel(effect_otel::NativeError::Native(effect::Defect::new(error))), }?;
            let environment = Arc::new(environment);
            let selected = environment.clone();
            let history = history.clone();
            let events = match writer {
                WriterMode::Inline => watch_order(history, watched, sse),
                WriterMode::Extracted => watch_notifications(history, watched, sse),
                WriterMode::Body => watch_progress(history, watched, sse),
            };
            let routes = router! { get("/orders/104/events", events); };
            // Each source awaits its writer, so one pending frame slot is sufficient;
            // the caller's encoded-frame limit also bounds each native body chunk.
            let policy = ServerPolicy::new(BodyPolicy { frames: NonZeroUsize::MIN, chunk_bytes: sse.frame_bytes },
                HostPolicy { drain }, CafeError::Server,
                |_| effect_sync! { body: || reply! { text("Order tracking failed".into()); }, });
            let server = effect_map_error! {
                program: effect_try_sync! {
                once; body: {
                let __dsl_result = server! {
                policy: policy,
                environment: move |request: effect_server::Request| {
                    let mut projected = selected.as_ref().clone();
                    projected.cancellation = request.cancellation().clone();
                    effect_sync! { body: move || Arc::new(projected.clone()), }
                },
                listen: address,
                protocols: HttpProtocols::Http1,
                routes: routes,
            }; move || __dsl_result }, on_error: ::core::convert::identity, }, map: CafeError::Server, }?;
            let propagation = Propagation::new(opentelemetry::propagation::TextMapCompositePropagator::new(vec![Box::new(opentelemetry_sdk::propagation::TraceContextPropagator::new()), Box::new(opentelemetry_sdk::propagation::BaggagePropagator::new())]));
            let server = effect_otel::server::install(server, propagation,
                |_| effect_otel::server::RequestSpan { name: Arc::from("cafe.watch-order"), attributes: vec![] },
                |summary| Otel::log(LogLevel::Info, format!("order stream {:?}; cleanup_failed={}", summary.body, summary.cleanup_failed)), CafeError::Otel);
            // Explicit typed R handoff: host/request polls use the acquired native bridge.
            effect_future! {
                exit_with_context; body: move |_, context| {
                let server = server.clone();
                let environment = environment.clone();
                async move {
                    context.run(server.with_host(|_| effect_try_future! { with_env; body: |_, _| async { tokio::signal::ctrl_c().await }, on_error: |error| CafeError::Server(effect_server::ServerError::Io(error)), }), environment).await
                }
            }, }?;
            }
        },
    };
    let exit = runtime.block_on(program.run(root.clone()));
    runtime.block_on(root.tasks.stop().wait());
    runtime.block_on(root.blocking.stop().wait());
    bootstrap_logs.shutdown()?;
    bootstrap_traces.shutdown()?;
    bootstrap_metrics.shutdown()?;
    match exit {
        Exit::Success(()) => {
            assert_eq!(subscriptions.load(Ordering::Acquire), 0);
            use std::io::Write;
            writeln!(
                std::io::stdout().lock(),
                "Order timeline host closed; all subscriptions released."
            )?;
            Ok(())
        }
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            Err(format!(
                "order host exited: {other:?}; active subscriptions={}",
                subscriptions.load(Ordering::Acquire)
            )
            .into())
        }
    }
}
