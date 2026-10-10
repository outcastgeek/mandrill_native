//! Fixed V07 cases: native context/signals, carriers, and provider ownership.
#![cfg(feature = "server")]

#[path = "../examples/cafe_telemetry/mod.rs"]
#[allow(
    dead_code,
    reason = "Shared café error variants also serve the runnable Server example"
)]
mod cafe_telemetry;
use cafe_telemetry::{Environment, TelemetryCapacity};
use effect::{
    Effect, Exit, LogLevel, MetricDescriptor, MetricDomain, MetricKind, MetricLabels, MetricName,
    MetricRecorder, MetricSample, MetricValue,
};
use effect_otel::{
    BorrowedProviders, NativeError, Otel, OtelContext, OwnedProviders, Propagation,
    ProviderFactories, ShutdownPolicy,
};
use opentelemetry::baggage::BaggageExt;
use opentelemetry::trace::{SpanContext, Status, TraceContextExt};
use opentelemetry_sdk::{
    logs::{InMemoryLogExporter, SdkLoggerProvider},
    metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider},
    trace::{InMemorySpanExporter, SdkTracerProvider},
};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

/// Two workers drive the finite concurrent-child case on one caller-owned reactor.
fn runtime() -> Result<Arc<tokio::runtime::Runtime>, std::io::Error> {
    Ok(Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?,
    ))
}

/// The suite uses one root and two concurrent children, three named metrics,
/// and one empty label set per metric; bounds follow those actual fixture counts.
fn telemetry_capacity() -> Result<TelemetryCapacity, Box<dyn std::error::Error>> {
    let child_names = ["first", "second"];
    let metric_names = ["cafe.orders", "cafe.subscriptions", "cafe.wait"];
    Ok(TelemetryCapacity {
        active_spans: NonZeroUsize::new(1 + child_names.len()).ok_or("empty span capacity")?,
        metric_descriptors: NonZeroUsize::new(metric_names.len())
            .ok_or("empty metric vocabulary")?,
        series_per_descriptor: NonZeroUsize::MIN,
    })
}

/// Discard only a success payload; typed failures, defects, interruption and the
/// complete ordered finalization carrier move unchanged into the unit exit.
fn discard_success<A, E>(exit: Exit<A, E>) -> Exit<(), E> {
    match exit {
        Exit::Success(_) => Exit::Success(()),
        Exit::Fail(error) => Exit::Fail(error),
        Exit::Die(defect) => Exit::Die(defect),
        Exit::Interrupted => Exit::Interrupted,
        Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
    }
}

/// Capture a native future's identity after suspension, then check the selected
/// blocking callback receives exactly that same request/span context.
fn native_child(name: &'static str) -> Effect<SpanContext, NativeError, Environment> {
    let future = Effect::from_future_with_env(|_, _| async {
        tokio::task::yield_now().await;
        opentelemetry::Context::current()
            .span()
            .span_context()
            .clone()
    });
    future
        .flat_map(|native: SpanContext| {
            let expected = native.clone();
            Effect::try_blocking(
                move |_| {
                    let actual = opentelemetry::Context::current()
                        .span()
                        .span_context()
                        .clone();
                    if actual == expected {
                        Ok(actual)
                    } else {
                        Err(NativeError::Parent(Arc::from("blocking context differs")))
                    }
                },
                std::convert::identity,
            )
        })
        .with_span(name)
}

/// Concurrent/nested native work restores its parent and exports typed signals
/// once through the selected native subscriber/SDK stack.
#[test]
fn parent_native_carrier_and_signal_mapping() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = runtime()?;
    let spans = InMemorySpanExporter::default();
    let logs = InMemoryLogExporter::default();
    let metrics = InMemoryMetricExporter::default();
    let traces = SdkTracerProvider::builder()
        .with_simple_exporter(spans.clone())
        .build();
    // Force-flush is the only collection trigger used by this finite fixture.
    let meter = SdkMeterProvider::builder()
        .with_reader(
            PeriodicReader::builder(metrics.clone())
                .with_interval(Duration::from_secs(3600))
                .build(),
        )
        .build();
    let logger = SdkLoggerProvider::builder()
        .with_simple_exporter(logs.clone())
        .build();
    let providers = BorrowedProviders::new(&traces, &meter, &logger);
    let environment = Arc::new(Environment::new(
        runtime.clone(),
        providers.clone(),
        telemetry_capacity()?,
    )?);
    let child_concurrency =
        NonZeroUsize::new(["first", "second"].len()).ok_or("empty child set")?;
    let propagation = Propagation::new(
        opentelemetry::propagation::TextMapCompositePropagator::new(vec![
            Box::new(opentelemetry_sdk::propagation::TraceContextPropagator::new()),
            Box::new(opentelemetry_sdk::propagation::BaggagePropagator::new()),
        ]),
    );
    let carrier = HashMap::from([
        (
            "traceparent".to_owned(),
            "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01".to_owned(),
        ),
        ("baggage".to_owned(), "cafe=neighborhood".to_owned()),
    ]);
    let parent = propagation.extract(&OtelContext::default(), &carrier)?;
    assert_eq!(
        parent
            .native()
            .baggage()
            .get("cafe")
            .map(ToString::to_string),
        Some("neighborhood".to_owned())
    );
    let expected_trace = parent.span_context().trace_id();
    let expected_parent = parent.span_context().span_id();
    let linked = parent.span_context();
    let source =
        Effect::from_exit_future_with_context(move |env: Arc<Environment>, context| async move {
            // Core owns sibling joining and retains whichever side actually failed.
            let children = Effect::all_concurrent(
                child_concurrency,
                vec![native_child("first"), native_child("second")],
            )
            .map(|spans| {
                assert_eq!(spans.len(), 2);
                if let Some((a, b)) = spans.first().zip(spans.get(1)) {
                    assert_ne!(a.span_id(), b.span_id());
                    assert_eq!(a.trace_id(), b.trace_id());
                }
            });
            let before = opentelemetry::Context::current()
                .span()
                .span_context()
                .clone();
            let result = context.run(children, env.clone()).await;
            if !matches!(result, Exit::Success(())) {
                return result;
            }
            let after = opentelemetry::Context::current()
                .span()
                .span_context()
                .clone();
            assert_eq!(before, after);
            let checked = context
                .run(
                    Otel::use_span(|span| {
                        use tracing_opentelemetry::OpenTelemetrySpanExt;
                        Ok(span.context().span().span_context().clone())
                    }),
                    env.clone(),
                )
                .await;
            match checked {
                Exit::Success(checked) => assert_eq!(checked, before),
                failed => return discard_success(failed),
            }
            context
                .run(
                    Otel::event_with(
                        "order.ready",
                        vec![
                            effect_otel::KeyValue::new("cups", 2_i64),
                            effect_otel::KeyValue::new("paid", true),
                        ],
                    )
                    .flat_map(|()| Otel::status(Status::Ok))
                    .flat_map(|()| Otel::count("cafe.orders", 1))
                    .flat_map(|()| Otel::log(LogLevel::Info, "pickup ready")),
                    env,
                )
                .await
        });
    let program = effect_otel::otel! { name: "order", parent: parent, links: vec![effect_otel::Link::with_context(linked)], effect: source };
    assert!(matches!(
        runtime.block_on(program.run(environment.clone())),
        Exit::Success(())
    ));
    meter.force_flush()?;
    traces.force_flush()?;
    logger.force_flush()?;
    let finished = spans.get_finished_spans()?;
    let root = finished
        .iter()
        .find(|span| span.name == "order")
        .ok_or("missing root span")?;
    assert_eq!(root.span_context.trace_id(), expected_trace);
    assert_eq!(root.parent_span_id, expected_parent);
    assert!(matches!(root.status, Status::Ok));
    assert!(
        root.links
            .links
            .iter()
            .any(|link| link.span_context.span_id() == expected_parent)
    );
    assert!(root.events.events.iter().any(|event| {
        event.name == "order.ready"
            && event
                .attributes
                .iter()
                .any(|kv| kv.key.as_str() == "paid" && kv.value == opentelemetry::Value::Bool(true))
    }));
    assert!(
        finished
            .iter()
            .filter(|span| span.name == "first" || span.name == "second")
            .all(|span| span.parent_span_id == root.span_context.span_id())
    );
    let emitted = logs.get_emitted_logs()?;
    assert_eq!(emitted.len(), 1);
    assert!(emitted.iter().all(|log| {
        log.record
            .trace_context()
            .is_some_and(|ctx| ctx.span_id == root.span_context.span_id())
    }));
    let exported = metrics.get_finished_metrics()?;
    assert!(exported.iter().flat_map(|batch| batch.scope_metrics()).flat_map(|scope| scope.metrics()).any(|metric| metric.name() == "cafe.orders" && metric.unit() == "{order}" && matches!(metric.data(), opentelemetry_sdk::metrics::data::AggregatedMetrics::U64(opentelemetry_sdk::metrics::data::MetricData::Sum(sum)) if sum.data_points().any(|point| point.value() == 1))));
    let mut outbound = HashMap::new();
    propagation.inject(
        &OtelContext::from_native(
            &opentelemetry::Context::new().with_remote_span_context(root.span_context.clone()),
        ),
        &mut outbound,
    )?;
    assert!(
        outbound
            .get("traceparent")
            .is_some_and(|value| value.contains(&root.span_context.span_id().to_string()))
    );
    // Dropping extension borrows never claims native provider shutdown.
    drop(providers);
    assert!(!spans.is_shutdown_called());
    assert!(!logs.is_shutdown_called());
    runtime.block_on(environment.tasks.stop().wait());
    runtime.block_on(environment.blocking.stop().wait());
    logger.shutdown()?;
    traces.shutdown()?;
    meter.shutdown()?;
    Ok(())
}

/// Exact signed gauges and durations keep distinct native numeric kinds/units.
#[test]
fn gauge_latency_precision_and_identity() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = runtime()?;
    let exporter = InMemoryMetricExporter::default();
    let traces = SdkTracerProvider::builder().build();
    let logs = SdkLoggerProvider::builder().build();
    let meter = SdkMeterProvider::builder()
        .with_reader(
            PeriodicReader::builder(exporter.clone())
                .with_interval(Duration::from_secs(3600))
                .build(),
        )
        .build();
    let env = Environment::new(
        runtime.clone(),
        BorrowedProviders::new(&traces, &meter, &logs),
        telemetry_capacity()?,
    )?;
    let gauge = MetricDescriptor::new(
        MetricName::new("cafe.subscriptions")?,
        MetricKind::Gauge,
        MetricDomain::General,
    );
    let latency = MetricDescriptor::new(
        MetricName::new("cafe.wait")?,
        MetricKind::Latency,
        MetricDomain::General,
    );
    env.otel.metrics().record(&MetricSample::new(
        gauge.clone(),
        MetricLabels::default(),
        MetricValue::Gauge(effect::GaugeValue::Integer(-2)),
    )?)?;
    env.otel.metrics().record(&MetricSample::new(
        latency,
        MetricLabels::default(),
        MetricValue::Latency(Duration::from_nanos(17)),
    )?)?;
    assert!(matches!(
        env.otel.metrics().record(&MetricSample::new(
            gauge,
            MetricLabels::default(),
            MetricValue::Gauge(effect::GaugeValue::Integer(i128::MAX))
        )?),
        Err(effect_otel::OtelMetricError::PrecisionLoss)
    ));
    meter.force_flush()?;
    let data = exporter.get_finished_metrics()?;
    assert!(data.iter().flat_map(|b| b.scope_metrics()).flat_map(|s| s.metrics()).any(|m| m.name() == "cafe.wait" && m.unit() == "ns" && matches!(m.data(), opentelemetry_sdk::metrics::data::AggregatedMetrics::U64(opentelemetry_sdk::metrics::data::MetricData::Histogram(histogram)) if histogram.data_points().any(|point| point.count() == 1 && point.sum() == 17))));
    assert!(data.iter().flat_map(|b| b.scope_metrics()).flat_map(|s| s.metrics()).any(|m| m.name() == "cafe.subscriptions" && matches!(m.data(), opentelemetry_sdk::metrics::data::AggregatedMetrics::I64(opentelemetry_sdk::metrics::data::MetricData::Gauge(gauge)) if gauge.data_points().any(|point| point.value() == -2))));
    logs.shutdown()?;
    traces.shutdown()?;
    meter.shutdown()?;
    Ok(())
}

/// Owned concurrent close joins one terminal result, and partial acquisition
/// failure shuts down the already acquired trace provider.
#[test]
fn owned_close_and_partial_acquisition_failure() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = runtime()?;
    let host_traces = SdkTracerProvider::builder().build();
    let host_meter = SdkMeterProvider::builder().build();
    let host_logs = SdkLoggerProvider::builder().build();
    let env = Arc::new(Environment::new(
        runtime.clone(),
        BorrowedProviders::new(&host_traces, &host_meter, &host_logs),
        telemetry_capacity()?,
    )?);
    let spans = InMemorySpanExporter::default();
    let logs = InMemoryLogExporter::default();
    let trace = spans.clone();
    let logger = logs.clone();
    let factories = ProviderFactories::new(
        move |builder| Ok(builder.with_simple_exporter(trace.clone()).build()),
        |builder| Ok(builder.build()),
        move |builder| Ok(builder.with_simple_exporter(logger.clone()).build()),
    );
    let policy = ShutdownPolicy::new(Duration::from_secs(5), true)?;
    let owned = match runtime.block_on(OwnedProviders::start(factories, policy).run(env.clone())) {
        Exit::Success(owned) => owned,
        other => return Err(format!("acquire failed: {:?}", discard_success(other)).into()),
    };
    let first = owned.close();
    let second = owned.close();
    let (a, b) =
        runtime.block_on(async { tokio::join!(first.run(env.clone()), second.run(env.clone())) });
    assert!(matches!(a, Exit::Success(())));
    assert!(matches!(b, Exit::Success(())));
    assert!(spans.is_shutdown_called());
    assert!(logs.is_shutdown_called());
    assert!(matches!(
        owned.borrowed().ensure_open(),
        Err(NativeError::ProviderClosed)
    ));
    let rollback_exporter = InMemorySpanExporter::default();
    let acquired = rollback_exporter.clone();
    let failed = ProviderFactories::new(
        move |builder| Ok(builder.with_simple_exporter(acquired.clone()).build()),
        |_| Err(NativeError::Parent(Arc::from("fixture meter acquisition"))),
        |builder| Ok(builder.build()),
    );
    let failed = runtime.block_on(OwnedProviders::start(failed, policy).run(env.clone()));
    assert!(matches!(failed, Exit::Fail(_)));
    assert!(rollback_exporter.is_shutdown_called());
    runtime.block_on(env.tasks.stop().wait());
    runtime.block_on(env.blocking.stop().wait());
    host_logs.shutdown()?;
    host_traces.shutdown()?;
    host_meter.shutdown()?;
    Ok(())
}
