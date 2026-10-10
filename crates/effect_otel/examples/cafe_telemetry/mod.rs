//! Caller-owned native providers for the café stream and finite V07 fixtures.
//! Borrowed mode keeps the three SDK originals in the caller and passes
//! `BorrowedProviders::new(&traces, &metrics, &logs)` to `Environment::new`.
//! Both modes also require caller-selected nonzero telemetry capacities.
//! Owned mode configures the same official builders with `ProviderFactories`,
//! acquires `OwnedProviders` in a core scope, and shares `owned.borrowed()`.
//! Neither mode installs a global subscriber or collector.

use effect::{
    AdmissionCapacity, Cancellation, DiagnosticCapacity, DiagnosticOverflow, Diagnostics,
    HasBlockingExecutor, HasCancellation, HasClock, HasTaskSpawner, HasTracer, Tracer,
};
use effect_otel::{
    BorrowedProviders, HasOtel, LatencyUnit, LogRouting, MetricDefinition, MetricPolicy,
    NativeInstrumentKind, OtelBackend, OtelContext, OtelMetricRecorder, Redaction,
};
use effect_tokio::{TokioBlocking, TokioClock, TokioTasks};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Example errors retain the native typed boundary that failed.
#[derive(Debug)]
pub enum CafeError {
    /// A checked OTel operation failed.
    Otel(effect_otel::NativeError),
    /// Owned acquisition/flush/shutdown retained its ordered failures.
    Lifecycle(effect_otel::LifecycleError),
    /// Server input or transport failed.
    Server(effect_server::ServerError),
}
impl std::fmt::Display for CafeError {
    /// Retain a safe category and its actual native cause.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Otel(error) => write!(f, "telemetry: {error}"),
            Self::Lifecycle(error) => write!(f, "provider lifecycle: {error}"),
            Self::Server(error) => write!(f, "server: {error}"),
        }
    }
}
/// Typed native failures remain inspectable through their café error variants.
impl std::error::Error for CafeError {}

/// Caller-selected storage bounds; none is substituted when the caller supplies zero.
#[derive(Clone, Copy)]
pub struct TelemetryCapacity {
    /// Maximum simultaneously retained native span identities.
    pub active_spans: NonZeroUsize,
    /// Maximum distinct metric descriptors retained by this recorder.
    pub metric_descriptors: NonZeroUsize,
    /// Maximum distinct label sets retained per metric descriptor.
    pub series_per_descriptor: NonZeroUsize,
}

/// Shared services keep request projections on the same clock and native span map.
#[derive(Clone)]
pub struct Environment {
    /// Caller-owned reactor survives all managed work and provider shutdown.
    pub runtime: Arc<tokio::runtime::Runtime>,
    /// Core task admission chosen explicitly by this example.
    pub tasks: TokioTasks,
    /// Core blocking executor used for native SDK lifecycle operations.
    pub blocking: TokioBlocking,
    /// One monotonic service shared by root and request projections.
    pub clock: Arc<TokioClock>,
    /// Core tracer and OTel backend use the same existing tracing span map.
    pub tracer: Tracer,
    /// Native bridge selected through R, with no global fallback.
    pub otel: OtelBackend,
    /// Request projection replaces only its cooperative token.
    pub cancellation: Cancellation,
}
impl Environment {
    /// Bind explicit native providers, runtime and storage bounds supplied by the caller.
    pub fn new(
        runtime: Arc<tokio::runtime::Runtime>,
        providers: BorrowedProviders,
        capacity: TelemetryCapacity,
    ) -> Result<Self, String> {
        let diagnostics = Diagnostics::new(
            DiagnosticCapacity::Unbounded,
            DiagnosticOverflow::DiscardOldest,
        );
        let tasks = TokioTasks::new(runtime.clone(), AdmissionCapacity::Unbounded)
            .map_err(|e| e.message().to_owned())?;
        let blocking = TokioBlocking::new(
            runtime.clone(),
            AdmissionCapacity::Unbounded,
            diagnostics.clone(),
        );
        let scope = opentelemetry::InstrumentationScope::builder("cafe.order-progress").build();
        let policy = MetricPolicy::new(
            capacity.metric_descriptors,
            capacity.series_per_descriptor,
            |descriptor| {
                let (kind, unit, latency_unit) = match descriptor.kind() {
                    effect::MetricKind::Counter => {
                        (NativeInstrumentKind::CounterU64, "{order}", None)
                    }
                    effect::MetricKind::Gauge => {
                        (NativeInstrumentKind::GaugeI64, "{subscription}", None)
                    }
                    effect::MetricKind::Latency => (
                        NativeInstrumentKind::HistogramU64,
                        "ns",
                        Some(LatencyUnit::Nanoseconds),
                    ),
                };
                Ok(MetricDefinition {
                    kind,
                    unit: Arc::from(unit),
                    latency_unit,
                })
            },
            |labels| Ok(labels.clone()),
        );
        let metrics = OtelMetricRecorder::new(providers.clone(), scope.clone(), policy)
            .map_err(|e| e.to_string())?;
        let dispatch = OtelBackend::dispatch(&providers, scope, LogRouting::EffectLogsOnly)
            .map_err(|e| e.to_string())?;
        let redaction = Redaction::new(|a| Ok(a.clone()), |a| Ok(a.to_vec()), |m| Ok(Arc::from(m)));
        let otel = OtelBackend::new(
            dispatch,
            capacity.active_spans,
            diagnostics,
            providers,
            OtelContext::default(),
            redaction,
            metrics,
        );
        Ok(Self {
            clock: Arc::new(TokioClock::new(runtime.clone())),
            runtime,
            tasks,
            blocking,
            tracer: otel.tracer(),
            otel,
            cancellation: Cancellation::new(),
        })
    }
}
impl HasClock for Environment {
    /// Request projections share the exact service origin.
    type Clock = TokioClock;
    /// Borrow the caller-selected monotonic provider.
    fn clock(&self) -> &TokioClock {
        &self.clock
    }
}
impl HasTracer for Environment {
    /// Ordinary span! and otel! use this same core tracer.
    fn tracer(&self) -> &Tracer {
        &self.tracer
    }
}
impl HasOtel for Environment {
    /// Explicit native provider bridge for this application R.
    fn otel(&self) -> &OtelBackend {
        &self.otel
    }
}
impl HasCancellation for Environment {
    /// Request-specific cancellation does not cancel siblings.
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}
impl HasTaskSpawner for Environment {
    /// One caller-owned Tokio executor drives managed children.
    type Tasks = TokioTasks;
    /// Borrow its explicit admission boundary.
    fn task_spawner(&self) -> &TokioTasks {
        &self.tasks
    }
}
impl HasBlockingExecutor for Environment {
    /// Native synchronous SDK cleanup runs on this selected provider.
    type Executor = TokioBlocking;
    /// Borrow the independent blocking capability.
    fn blocking_executor(&self) -> &TokioBlocking {
        &self.blocking
    }
}
impl effect_server::HasTokio for Environment {
    /// Native axum/Hyper polls use the caller's reactor.
    fn tokio(&self) -> &tokio::runtime::Handle {
        self.runtime.handle()
    }
}
impl effect_server::HasServerError for Environment {
    /// Retain native input/body failures in the application error type.
    type Error = CafeError;
    /// Map without flattening into a success response.
    fn server_error(&self, error: effect_server::ServerError) -> CafeError {
        CafeError::Server(error)
    }
}
