//! Maya watches an oat latte move from accepted to preparing to ready.
//!
//! Run with explicit address, preparation milliseconds, SSE frame bytes, cursor
//! bytes, active span slots, metric descriptors, label sets per descriptor, and
//! a shared host-drain/SDK-shutdown timeout in positive milliseconds:
//! `cargo run -p effect-otel --features server --example watch_order_progress -- 127.0.0.1:8080 400 256 32 32 16 8 5000`.
//! Zero capacities or timeout fail argument parsing. `/orders/104/events` uses an inline
//! writer, `/orders/104/notifications` reuses an extracted writer, and
//! `/orders/104/progress` owns its writes in a body Effect. All three emit the
//! same accepted/preparing/ready sequence without collecting it.
//! Open a route with `curl -N`; send a `traceparent`/`baggage`
//! header to carry an existing distributed parent. The server writes each event
//! only after the consumer pulls the previous frame. Disconnect interrupts the
//! producer and its request scope unregisters the subscription before observation.
//! Providers use official in-memory exporters for this local story; replace the
//! factory builder hooks with your exporter configuration to send real telemetry.
//! The host owns SDK shutdown. Borrowed setup is documented in `cafe_telemetry`.

mod cafe_telemetry;
use cafe_telemetry::{CafeError, Environment, TelemetryCapacity};
use effect::{Effect, Exit, LogLevel, Release, Scope, Stream};
use effect_otel::{Otel, OwnedProviders, Propagation, ProviderFactories, ShutdownPolicy, otel};
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
    effect::effect! {
        Otel::tag("order.status", status)?;
        Otel::event(event)?;
        status.to_owned()
    }
}

/// Open subscriptions are real scope-owned registrations, not response construction counters.
fn order_updates(
    wait: Duration,
    subscriptions: Arc<AtomicUsize>,
) -> Stream<String, CafeError, Environment> {
    Stream::new(move |scope| {
        let opened = subscriptions.clone();
        let acquired = Effect::sync(move || {
            opened.fetch_add(1, Ordering::AcqRel);
            opened.clone()
        });
        let source: Stream<String, CafeError, Environment> = effect::stream! {
            Otel::event("order.accepted")?;
            yield "accepted".to_owned();
            Effect::sleep(wait)?;
            let preparing = otel! { name: "cafe.order.prepare", effect: publish_status("preparing", "barista.started") }?;
            yield preparing;
            Effect::sleep(wait)?;
            let ready = otel! { name: "cafe.order.ready", effect: publish_status("ready", "pickup.ready") }?;
            yield ready;
        };
        let source_scope = scope.clone();
        effect::effect! {
            let source_scope = source_scope.clone();
            let acquired = acquired.clone();
            let source = source.clone();
            source_scope.acquire(acquired, |registered| Release::from_effect(Effect::sync(move || {
                registered.fetch_sub(1, Ordering::AcqRel);
            })))?;
            source.open(source_scope)?
        }
    })
}

/// Inline D16 writing uses the core pull-driven source and awaited SSE/flush boundaries.
fn watch_order(
    wait: Duration,
    subscriptions: Arc<AtomicUsize>,
    sse: SsePolicy,
) -> Endpoint<CafeError, Environment> {
    endpoint! {
        let body = effect_server::stream! {
            source: order_updates(wait, subscriptions.clone()),
            write |update, writer| {
                let update: String = update;
                let writer: effect_server::BodyWriter<CafeError, Environment> = writer;
                // Transfer the payload once; only descriptions and sink handles are shared.
                let update_once: Effect<String, CafeError, Environment> = Effect::sync_once(move || update);
                otel! { name: "cafe.order.update", body {
                    let writer = writer.clone();
                    let update_once = update_once.clone();
                    let update = update_once?;
                    Otel::tag("order.id", "104")?;
                    writer.sse(SseRecord { event: Some("order-status".into()), id: None, retry: None, data: update }, sse)?;
                    effect::span! { name: "cafe.flush-status", effect: writer.flush() }?;
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
    effect::effect! {
        let frame = frame.clone();
        let writer = writer.clone();
        frame?;
        writer.flush()?;
    }
}

/// Extracted D16 callback keeps the same pull, write and subscription ownership.
fn watch_notifications(
    wait: Duration,
    subscriptions: Arc<AtomicUsize>,
    sse: SsePolicy,
) -> Endpoint<CafeError, Environment> {
    endpoint! {
        let body = effect_server::stream! {
            source: order_updates(wait, subscriptions.clone()),
            write: move |update, writer| write_status(update, writer, sse),
        };
        reply! { sse(body); }
    }
}

/// Body form runs the same scoped source; the Effect owns each awaited write/flush.
/// Core run_for_each releases the subscription on exhaustion or interruption.
fn watch_progress(
    wait: Duration,
    subscriptions: Arc<AtomicUsize>,
    sse: SsePolicy,
) -> Endpoint<CafeError, Environment> {
    endpoint! {
        let subscriptions = subscriptions.clone();
        let body = effect_server::stream! {
            body |writer| {
                let writer: BodyWriter<CafeError, Environment> = writer.clone();
                let subscriptions = subscriptions.clone();
                let updates = order_updates(wait, subscriptions);
                updates.run_for_each(move |update| write_status(update, writer.clone(), sse))?;
            }
        };
        reply! { sse(body); }
    }
}

/// A finite native host remains owned until Ctrl-C, then joins requests and SDK cleanup.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let address: std::net::SocketAddr =
        args.next().ok_or("provide the listener address")?.parse()?;
    let millis: u64 = args
        .next()
        .ok_or("provide preparation milliseconds")?
        .parse()?;
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
    let wait = Duration::from_millis(millis);
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
    let program: Effect<(), CafeError, Environment> = Scope::scoped(move |scope| {
        let factories = factories.clone();
        let provider_runtime = provider_runtime.clone();
        let watched = watched.clone();
        effect::effect! {
        let factories = factories.clone();
        let provider_runtime = provider_runtime.clone();
        let watched = watched.clone();
        let scope = scope.clone();
        let owned = OwnedProviders::acquire_with_error(scope, factories, shutdown, CafeError::Lifecycle)?;
        let providers = Effect::try_sync_once(move || owned.with(|owner| owner.borrowed()),
            |error| CafeError::Otel(effect_otel::NativeError::Native(error)))?;
        let environment = Effect::from_result_once(Environment::new(provider_runtime, providers, capacity))
            .map_error(|error| CafeError::Otel(effect_otel::NativeError::Native(effect::Defect::new(error))))?;
        let environment = Arc::new(environment);
        let selected = environment.clone();
        let events = watch_order(wait, watched.clone(), sse);
        let notifications = watch_notifications(wait, watched.clone(), sse);
        let progress = watch_progress(wait, watched, sse);
        let routes = router! {
            get("/orders/104/events", events);
            get("/orders/104/notifications", notifications);
            get("/orders/104/progress", progress);
        };
        // Each source awaits its writer, so one pending frame slot is sufficient;
        // the caller's encoded-frame limit also bounds each native body chunk.
        let policy = ServerPolicy::new(BodyPolicy { frames: NonZeroUsize::MIN, chunk_bytes: sse.frame_bytes },
            HostPolicy { drain }, CafeError::Server,
            |_| Effect::sync(|| reply! { text("Order tracking failed".into()); }));
        let server = Effect::from_result_once(server! {
            policy: policy,
            environment: move |request: effect_server::Request| {
                let mut projected = selected.as_ref().clone();
                projected.cancellation = request.cancellation().clone();
                Effect::sync(move || Arc::new(projected.clone()))
            },
            listen: address,
            protocols: HttpProtocols::Http1,
            routes: routes,
        }).map_error(CafeError::Server)?;
        let propagation = Propagation::new(opentelemetry::propagation::TextMapCompositePropagator::new(vec![Box::new(opentelemetry_sdk::propagation::TraceContextPropagator::new()), Box::new(opentelemetry_sdk::propagation::BaggagePropagator::new())]));
        let server = effect_otel::server::install(server, propagation,
            |_| effect_otel::server::RequestSpan { name: Arc::from("cafe.watch-order"), attributes: vec![] },
            |summary| Otel::log(LogLevel::Info, format!("order stream {:?}; cleanup_failed={}", summary.body, summary.cleanup_failed)), CafeError::Otel);
        // Explicit typed R handoff: host/request polls use the acquired native bridge.
        Effect::from_exit_future_with_context(move |_, context| {
            let server = server.clone();
            let environment = environment.clone();
            async move {
                context.run(server.with_host(|_| Effect::try_future_with_env(|_, _| async { tokio::signal::ctrl_c().await },
                    |error| CafeError::Server(effect_server::ServerError::Io(error)))), environment).await
            }
        })?;
        }
    });
    let exit = runtime.block_on(program.run(root.clone()));
    runtime.block_on(root.tasks.stop().wait());
    runtime.block_on(root.blocking.stop().wait());
    bootstrap_logs.shutdown()?;
    bootstrap_traces.shutdown()?;
    bootstrap_metrics.shutdown()?;
    match exit {
        Exit::Success(()) => Ok(()),
        other => Err(format!(
            "order host exited: {other:?}; active subscriptions={}",
            subscriptions.load(Ordering::Acquire)
        )
        .into()),
    }
}
