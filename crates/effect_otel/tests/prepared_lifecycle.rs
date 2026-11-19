//! Exercise live native span drain and SDK cleanup on the independent provider.

#[path = "../../effect/tests/fixtures/shared/prepared_provider.rs"]
#[allow(
    dead_code,
    reason = "Sibling provider tests use the shared scalar working-set witness"
)]
mod fixture;

use effect::prepared_provider::ProviderWorkingSet;
use effect::{
    AdmissionCapacity, DiagnosticCapacity, DiagnosticOverflow, Diagnostics, Effect, Exit,
    HasBlockingExecutor, HasClock, HasTracer, PanicMessageOverflow, PreparedPanicConfig,
    PreparedPanicPool, Tracer, effect_future, span,
};
use effect_otel::{
    LogRouting, MetricPolicy, NativeError, OtelBackend, OtelContext, OtelMetricError,
    OtelMetricRecorder, PreparedOwnedProviders, PreparedProviderLifecycle, PreparedShutdownWorkers,
    ProviderFactories, Redaction, ShutdownPolicy,
};
use effect_tokio::{TokioBlocking, TokioClock};
use fixture::{NATIVE, REGION};
use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::trace::InMemorySpanExporter;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

struct SdkFixtureWork {
    _exporter: InMemorySpanExporter,
}

// SAFETY: This fixture queues one fixed-name span in one preconfigured simple
// exporter. Its owned exporter, SDK providers and borrowed reactor retain that
// native domain through the dedicated worker's report. No retry or user I/O is
// reachable from this workload.
unsafe impl ProviderWorkingSet for SdkFixtureWork {
    const CONTRACT: &'static str = "one fixture span and fixed simple exporter";
}

struct Startup(TokioBlocking);
impl HasBlockingExecutor for Startup {
    type Executor = TokioBlocking;

    fn blocking_executor(&self) -> &Self::Executor {
        &self.0
    }
}

struct SpanEnv {
    clock: TokioClock,
    tracer: Tracer,
}
impl HasClock for SpanEnv {
    type Clock = TokioClock;

    fn clock(&self) -> &Self::Clock {
        &self.clock
    }
}
impl HasTracer for SpanEnv {
    fn tracer(&self) -> &Tracer {
        &self.tracer
    }
}

#[test]
fn selected_close_waits_for_live_span_with_ordinary_blocking_stopped() -> Result<(), String> {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?,
    );
    let memory = fixture::memory().map_err(|error| format!("{error:?}"))?;
    let report = fixture::Diagnostics::prepare(&memory).map_err(|error| format!("{error:?}"))?;
    let panics = PreparedPanicPool::prepare(
        &memory,
        REGION,
        PreparedPanicConfig {
            slots: 7,
            bytes_per_message: 256.into(),
            overflow: PanicMessageOverflow::Utf8Prefix,
        },
    )
    .map_err(|error| format!("{error:?}"))?;
    let mut workers = PreparedShutdownWorkers::<SdkFixtureWork>::prepare(
        &memory,
        REGION,
        NonZeroUsize::MIN,
        NATIVE,
        &panics,
    )
    .map_err(|error| format!("{error:?}"))?;
    let observations = InMemorySpanExporter::default();
    let budget = report
        .budget(SdkFixtureWork {
            _exporter: observations.clone(),
        })
        .map_err(|error| format!("{error:?}"))?;
    let command = workers
        .reserve(budget)
        .map_err(|error| format!("{error:?}"))?;
    let exporter = observations.clone();
    let factories = ProviderFactories::new(
        move |builder| Ok(builder.with_simple_exporter(exporter.clone()).build()),
        |builder| Ok(builder.build()),
        |builder| Ok(builder.build()),
    );
    let lifecycle = PreparedProviderLifecycle::prepare(
        &memory,
        REGION,
        factories,
        NonZeroUsize::MIN.saturating_add(3),
        1,
        command,
    )
    .map_err(|error| format!("{error:?}"))?;
    let _memory = memory.seal();
    let diagnostics = Diagnostics::new(
        DiagnosticCapacity::Bounded(NonZeroUsize::MIN.saturating_add(3)),
        DiagnosticOverflow::DiscardNewest,
    );
    let startup = Arc::new(Startup(TokioBlocking::new(
        runtime.clone(),
        AdmissionCapacity::Bounded(NonZeroUsize::MIN),
        diagnostics.clone(),
    )));
    let policy =
        ShutdownPolicy::new(Duration::from_secs(5), true).map_err(|error| error.to_string())?;
    let owned = match runtime
        .block_on(PreparedOwnedProviders::start(lifecycle, policy).run(startup.clone()))
    {
        Exit::Success(owned) => owned,
        Exit::Fail(error) => return Err(error.to_string()),
        Exit::Die(defect) => return Err(defect.message().to_owned()),
        Exit::Interrupted | Exit::FinalizationFailed(_) => {
            return Err("prepared startup did not settle".into());
        }
    };
    runtime.block_on(startup.0.stop().wait());
    let providers = owned.borrowed();
    let scope = InstrumentationScope::builder("prepared-lifecycle-fixture").build();
    let dispatch = OtelBackend::dispatch(&providers, scope.clone(), LogRouting::EffectLogsOnly)
        .map_err(|error| error.to_string())?;
    let metrics = OtelMetricRecorder::new(
        providers.clone(),
        scope,
        MetricPolicy::new(
            NonZeroUsize::MIN,
            NonZeroUsize::MIN,
            |_| Err(OtelMetricError::InvalidDefinition),
            |labels| Ok(labels.clone()),
        ),
    )
    .map_err(|error| error.to_string())?;
    let backend = OtelBackend::new(
        dispatch,
        NonZeroUsize::MIN,
        diagnostics,
        providers.clone(),
        OtelContext::default(),
        Redaction::new(
            |annotations| Ok(annotations.clone()),
            |attributes| Ok(attributes.to_vec()),
            |message| Ok(Arc::from(message)),
        ),
        metrics,
    );
    let span_env = Arc::new(SpanEnv {
        clock: TokioClock::new(runtime.clone()),
        tracer: backend.tracer(),
    });
    let (ready, span_ready) = tokio::sync::oneshot::channel();
    let (release, span_release) = tokio::sync::oneshot::channel();
    let work: Effect<(), String, SpanEnv> = effect_future! { once; body: move |_| async move {
        let _ = ready.send(());
        let _ = span_release.await;
    }, };
    let work = span! { name: "prepared-test-held-span", effect: work };
    runtime.block_on(async {
        let span = tokio::spawn(async move { work.run(span_env).await });
        tokio::time::timeout(Duration::from_secs(5), span_ready)
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let first = owned.close::<()>();
        let mut first = std::pin::pin!(first.run(Arc::new(())));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut first)
                .await
                .is_err()
        );
        assert!(!observations.is_shutdown_called());
        assert!(matches!(
            providers.ensure_open(),
            Err(NativeError::ProviderClosed)
        ));
        release
            .send(())
            .map_err(|()| "live span ended early".to_owned())?;
        let span = span.await.map_err(|error| error.to_string())?;
        assert!(matches!(span, Exit::Success(())));
        let first = tokio::time::timeout(Duration::from_secs(5), &mut first)
            .await
            .map_err(|error| error.to_string())?;
        assert!(matches!(first, Exit::Success(())), "{first:?}");
        assert!(matches!(
            owned.close::<()>().run(Arc::new(())).await,
            Exit::Success(())
        ));
        Ok::<_, String>(())
    })?;
    workers.join();
    assert!(observations.is_shutdown_called());
    assert!(workers.worker_panics().payloads().is_empty());
    assert_eq!(report.deliveries(), 0);
    Ok(())
}

#[derive(Default)]
struct ExportGate {
    state: std::sync::Mutex<(bool, bool)>,
    changed: std::sync::Condvar,
}
impl ExportGate {
    fn enter(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.0 = true;
        self.changed.notify_all();
        drop(
            self.changed
                .wait_while(state, |state| !state.1)
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }

    fn wait_entered(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |state| !state.0)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.0
    }

    fn open(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .1 = true;
        self.changed.notify_all();
    }
}

#[test]
fn blocked_trace_delivery_keeps_native_queue_finite_and_recovers() -> Result<(), String> {
    use effect_otel::{
        OwnedProviders, PreparedBatchLimits, PreparedExport, PreparedExportBatch,
        PreparedExportCapacity, PreparedProviderLimits, Signal,
    };
    use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
    use opentelemetry_sdk::{
        Resource,
        error::OTelSdkError,
        metrics::Temporality,
        trace::{Sampler, SpanLimits},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?,
    );
    let memory = fixture::memory().map_err(|error| format!("{error:?}"))?;
    let gate = Arc::new(ExportGate::default());
    let entered = gate.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let sends = calls.clone();
    let export = Arc::new(
        PreparedExport::prepare(
            &memory,
            REGION,
            PreparedExportCapacity {
                batches: NonZeroUsize::MIN,
                bytes_per_batch: 8.into(),
            },
            |batch, bytes| {
                if let PreparedExportBatch::Traces {
                    records,
                    resource: _,
                } = batch
                    && !records.is_empty()
                {
                    bytes
                        .extend(b"trace")
                        .map_err(|error| OTelSdkError::InternalFailure(format!("{error:?}")))?;
                }
                Ok(())
            },
            move |signal, bytes| {
                if signal == Signal::Traces && bytes == b"trace" {
                    let previous = sends.fetch_add(1, Ordering::Relaxed);
                    if previous == 0 {
                        entered.enter();
                    }
                }
                Ok(())
            },
        )
        .map_err(|error| format!("{error:?}"))?,
    );
    let batch = PreparedBatchLimits {
        queue: NonZeroUsize::MIN.saturating_add(1),
        batch: NonZeroUsize::MIN,
        record_bytes: 512.into(),
        interval: Duration::from_secs(1),
    };
    let limits = PreparedProviderLimits {
        native_access: NonZeroUsize::MIN.saturating_add(1),
        spans: SpanLimits::default(),
        traces: batch,
        logs: batch,
        log_nesting: 4,
        metric_series: NonZeroUsize::MIN,
        metrics_per_export: NonZeroUsize::MIN,
        metric_interval: Duration::from_secs(60),
        temporality: Temporality::Cumulative,
    };
    let factories = ProviderFactories::prepared_export(
        limits,
        Resource::builder_empty().build(),
        Sampler::AlwaysOn,
        export.clone(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let _memory = memory.seal();
    let startup = Arc::new(Startup(TokioBlocking::new(
        runtime.clone(),
        AdmissionCapacity::Bounded(NonZeroUsize::MIN),
        Diagnostics::new(
            DiagnosticCapacity::Bounded(NonZeroUsize::MIN),
            DiagnosticOverflow::DiscardNewest,
        ),
    )));
    let policy =
        ShutdownPolicy::new(Duration::from_secs(5), true).map_err(|error| error.to_string())?;
    let owned =
        match runtime.block_on(OwnedProviders::start(factories, policy).run(startup.clone())) {
            Exit::Success(owned) => owned,
            Exit::Fail(error) => return Err(error.to_string()),
            Exit::Die(defect) => return Err(defect.message().to_owned()),
            Exit::Interrupted | Exit::FinalizationFailed(_) => {
                return Err("provider startup did not settle".into());
            }
        };
    let rollback = std::mem::DropGuard::new(gate.clone(), |gate| gate.open());
    let borrowed = owned.borrowed();
    let tracer = borrowed.tracer().tracer("bounded-export-fixture");
    tracer.start("first").end();
    assert!(gate.wait_entered());
    tracer.start("x".repeat(513)).end();
    for _ in 0..64 {
        tracer.start("queued").end();
    }
    assert_eq!(export.discarded_records(Signal::Traces), 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    drop(rollback);

    let close = runtime.block_on(owned.close().run(startup.clone()));
    assert!(matches!(close, Exit::Success(())), "{close:?}");
    let sent = calls.load(Ordering::Relaxed);
    assert!(
        (2..=3).contains(&sent),
        "bounded queue delivered {sent} batches"
    );
    assert_eq!(export.discarded_batches(Signal::Traces), 0);
    runtime.block_on(startup.0.stop().wait());
    Ok(())
}

#[test]
fn metric_registry_refuses_wide_drafts_and_keeps_existing_series() -> Result<(), String> {
    use effect::{
        MetricDescriptor, MetricDomain, MetricKind, MetricLabels, MetricName, MetricRecorder,
        MetricSample, MetricValue,
    };
    use effect_otel::{
        BorrowedProviders, MetricDefinition, NativeInstrumentKind, PreparedOtelMetricCapacity,
    };
    use opentelemetry_sdk::{
        logs::SdkLoggerProvider, metrics::SdkMeterProvider, trace::SdkTracerProvider,
    };

    let memory = fixture::memory().map_err(|error| format!("{error:?}"))?;
    let providers = BorrowedProviders::new(
        &SdkTracerProvider::builder().build(),
        &SdkMeterProvider::builder().build(),
        &SdkLoggerProvider::builder().build(),
    );
    let recorder = OtelMetricRecorder::new_prepared(
        providers,
        InstrumentationScope::builder("fixed-series-fixture").build(),
        MetricPolicy::new(
            NonZeroUsize::MIN,
            NonZeroUsize::MIN,
            |_| {
                Ok(MetricDefinition {
                    kind: NativeInstrumentKind::CounterU64,
                    unit: Arc::from(""),
                    latency_unit: None,
                })
            },
            |labels| Ok(labels.clone()),
        ),
        &memory,
        REGION,
        PreparedOtelMetricCapacity {
            drafts: NonZeroUsize::MIN,
            labels_per_series: NonZeroUsize::MIN,
            label_bytes_per_series: NonZeroUsize::MIN.saturating_add(3),
        },
    )
    .map_err(|error| error.to_string())?;
    let _memory = memory.seal();
    let descriptor = MetricDescriptor::new(
        MetricName::new("fixture.measurements").map_err(|error| error.to_string())?,
        MetricKind::Counter,
        MetricDomain::General,
    );
    let make = |label: &'static str| {
        let labels = MetricLabels::new([(Arc::from("n"), Arc::from(label))].into())
            .map_err(|error| error.to_string())?;
        MetricSample::new(descriptor.clone(), labels, MetricValue::Counter(1))
            .map_err(|error| error.to_string())
    };
    assert!(matches!(
        recorder.record(&make("wide")?),
        Err(OtelMetricError::CapacityReached)
    ));
    recorder
        .record(&make("one")?)
        .map_err(|error| error.to_string())?;
    assert!(matches!(
        recorder.record(&make("two")?),
        Err(OtelMetricError::CapacityReached)
    ));
    recorder
        .record(&make("one")?)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[derive(Debug)]
struct CarrierWriter {
    second: bool,
}
impl opentelemetry::propagation::TextMapPropagator for CarrierWriter {
    fn inject_context(
        &self,
        _: &opentelemetry::Context,
        sink: &mut dyn opentelemetry::propagation::Injector,
    ) {
        sink.set("first", "saved".into());
        if self.second {
            sink.set("second", "refused".into());
        }
    }

    fn extract_with_context(
        &self,
        base: &opentelemetry::Context,
        _: &dyn opentelemetry::propagation::Extractor,
    ) -> opentelemetry::Context {
        base.clone()
    }

    fn fields(&self) -> opentelemetry::propagation::text_map_propagator::FieldIter<'_> {
        opentelemetry::propagation::text_map_propagator::FieldIter::new(&[])
    }
}

#[test]
fn oversized_carrier_does_not_mutate_destination_and_slot_is_reusable() -> Result<(), String> {
    use effect_otel::{PreparedPropagationCapacity, PreparedPropagationCarriers, Propagation};

    let memory = fixture::memory().map_err(|error| format!("{error:?}"))?;
    let carriers = Arc::new(
        PreparedPropagationCarriers::prepare(
            &memory,
            REGION,
            PreparedPropagationCapacity {
                carriers: NonZeroUsize::MIN,
                fields: NonZeroUsize::MIN,
                values: NonZeroUsize::MIN,
                bytes: 20.into(),
            },
        )
        .map_err(|error| format!("{error:?}"))?,
    );
    let _memory = memory.seal();
    let mut headers =
        std::collections::HashMap::<String, String>::from([("existing".into(), "yes".into())]);
    let context = OtelContext::default();
    let refusing =
        Propagation::new(CarrierWriter { second: true }).with_prepared_carriers(carriers.clone());
    assert!(refusing.inject(&context, &mut headers).is_err());
    assert_eq!(headers.len(), 1);
    let accepted =
        Propagation::new(CarrierWriter { second: false }).with_prepared_carriers(carriers);
    accepted
        .inject(&context, &mut headers)
        .map_err(|error| error.to_string())?;
    assert_eq!(headers.get("first").map(String::as_str), Some("saved"));
    assert_eq!(headers.get("existing").map(String::as_str), Some("yes"));
    Ok(())
}
