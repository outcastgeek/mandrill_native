//! Native span mapping, poll contexts and lazy typed telemetry operations.

use crate::{BorrowedProviders, OtelContext, OtelMetricRecorder};
use effect::{
    Annotations, Defect, Diagnostics, Effect, HasClock, HasTracer, LogLevel, Logger,
    PollInstrumentation, PollScope, SpanId, SpanOutcome, TraceContext, Tracer, effect,
    effect_try_sync, service,
};
use effect_tracing::TracingBackend;
use opentelemetry::baggage::BaggageExt;
use opentelemetry::trace::TraceContextExt;
use opentelemetry::{KeyValue, Value};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use tracing::event;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::prelude::*;

/// Checked failures retain provider diagnostics without exposing application data.
#[derive(Clone, Debug)]
pub enum NativeError {
    /// No local span is active in this Effect execution.
    NoActiveSpan,
    /// The native SDK layer is absent or rejected parent attachment.
    Parent(Arc<str>),
    /// An owned provider is closing/closed and rejects new native access.
    ProviderClosed,
    /// A selected native operation failed or panicked.
    Native(Defect),
}

impl NativeError {
    /// Preserve an actual native defect; describe structural rejections safely.
    pub(crate) fn into_defect(self) -> Defect {
        match self {
            Self::Native(defect) => defect,
            other @ (Self::NoActiveSpan | Self::Parent(_) | Self::ProviderClosed) => {
                Defect::new(other.to_string())
            }
        }
    }
}

impl std::fmt::Display for NativeError {
    /// Structural error text contains no attributes, payloads or credentials.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoActiveSpan => f.write_str("no active native telemetry span"),
            Self::Parent(error) => write!(f, "native parent rejected: {error}"),
            Self::ProviderClosed => f.write_str("telemetry provider is closed"),
            Self::Native(error) => write!(f, "native telemetry operation: {}", error.message()),
        }
    }
}
impl std::error::Error for NativeError {}

/// Redact core string annotations while retaining checked callback failures.
type AnnotationsRedactor = dyn Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync;
/// Redact native typed attributes without converting their values to strings.
type AttributesRedactor = dyn Fn(&[KeyValue]) -> Result<Vec<KeyValue>, Defect> + Send + Sync;
/// Redact application log text before the native log bridge observes it.
type MessageRedactor = dyn Fn(&str) -> Result<Arc<str>, Defect> + Send + Sync;

/// Caller-selected redaction callbacks, invoked before any Effect metadata export.
#[derive(Clone)]
pub struct Redaction {
    /// Core string annotations remain separate from typed native attributes.
    annotations: Arc<AnnotationsRedactor>,
    /// Typed values can be removed/replaced without debug-string conversion.
    attributes: Arc<AttributesRedactor>,
    /// Application log text has an explicit redaction boundary as well.
    message: Arc<MessageRedactor>,
}

impl Redaction {
    /// Supply all redaction decisions; no SQL/payload/credential serializer is used.
    #[must_use]
    pub fn new(
        annotations: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
        attributes: impl Fn(&[KeyValue]) -> Result<Vec<KeyValue>, Defect> + Send + Sync + 'static,
        message: impl Fn(&str) -> Result<Arc<str>, Defect> + Send + Sync + 'static,
    ) -> Self {
        Self {
            annotations: Arc::new(annotations),
            attributes: Arc::new(attributes),
            message: Arc::new(message),
        }
    }
}

/// Deliberate native event routing prevents equivalent records in two signals.
#[derive(Clone, Copy)]
pub enum LogRouting {
    /// Native tracing events go to logs; typed `Otel::event` goes only to traces.
    NativeEventsAsLogs,
    /// Native events go to traces; only the Effect log target reaches logs.
    EffectLogsOnly,
}

/// Immutable metadata applied before SDK span construction.
#[derive(Clone)]
pub struct SpanOptions {
    /// The SDK span kind; protocol values come from the native OTel contract.
    pub kind: opentelemetry::trace::SpanKind,
    /// Explicit remote/detached parent overrides the inherited parent when selected.
    pub parent: Option<OtelContext>,
    /// Caller-selected typed metadata is redacted before the span reaches the SDK.
    pub attributes: Vec<KeyValue>,
    /// Distributed links retain native flags/tracestate and their typed attributes.
    pub links: Vec<opentelemetry::trace::Link>,
}

impl Default for SpanOptions {
    /// Internal is the OTel protocol's ordinary business-span kind.
    /// No operational bounds, export policy or runtime is selected by this value.
    fn default() -> Self {
        Self {
            kind: opentelemetry::trace::SpanKind::Internal,
            parent: None,
            attributes: Vec::new(),
            links: Vec::new(),
        }
    }
}

/// Pending options travel in an explicit poll attachment for exactly one child span.
#[derive(Clone)]
struct PendingOptions {
    /// Shared immutable metadata; sharing copies no application payload.
    options: Arc<SpanOptions>,
}
/// Private marker distinguishes an explicit provider bridge attachment from ambient host data.
#[derive(Clone)]
struct BridgeAttachment;

/// One provider bridge over the existing native tracing span map.
#[derive(Clone)]
pub struct OtelBackend {
    /// Tracing owns local spans; this extension supplies no second lifetime engine.
    tracing: TracingBackend,
    /// Caller-owned providers and their lifecycle admission state.
    providers: BorrowedProviders,
    /// Explicit detached base parent and baggage for this environment generation.
    context: OtelContext,
    /// Redaction precedes SDK operations and core annotation emission.
    redaction: Redaction,
    /// Metrics use the native meter and explicit identity/capacity policies.
    metrics: OtelMetricRecorder,
    /// Telemetry failures never replace the business exit for concise operations.
    diagnostics: Diagnostics,
}

impl OtelBackend {
    /// Build over a supplied dispatch with the appropriate official native layers.
    /// `capacity` bounds the existing tracing map; metric bounds live in `metrics`.
    /// The providers are retained but never implicitly shut down by this backend.
    #[must_use]
    pub fn new(
        dispatch: tracing::Dispatch,
        capacity: NonZeroUsize,
        diagnostics: Diagnostics,
        providers: BorrowedProviders,
        context: OtelContext,
        redaction: Redaction,
        metrics: OtelMetricRecorder,
    ) -> Self {
        Self {
            tracing: TracingBackend::new(dispatch, capacity, diagnostics.clone()),
            providers,
            context,
            redaction,
            metrics,
            diagnostics,
        }
    }

    /// Compose official trace and log layers with explicit event routing.
    ///
    /// The supplied SDK tracer/provider already contains the caller's samplers,
    /// resources and processors. Custom subscribers remain usable through `new`.
    /// Native tracing events are an explicit native escape: redact their fields
    /// at the application's emission boundary or its selected subscriber layer.
    pub fn dispatch(
        providers: &BorrowedProviders,
        scope: opentelemetry::InstrumentationScope,
        routing: LogRouting,
    ) -> Result<tracing::Dispatch, NativeError> {
        let _permit = providers.admit()?;
        catch_unwind(AssertUnwindSafe(|| {
            use opentelemetry::trace::TracerProvider;
            let trace = tracing_opentelemetry::layer()
                .with_tracer(providers.tracer().tracer_with_scope(scope));
            let trace = trace.with_filter(tracing_subscriber::filter::filter_fn(move |metadata| {
                metadata.is_span()
                    || matches!(routing, LogRouting::EffectLogsOnly)
                        && metadata.target() != "effect::log"
            }));
            let logs = opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(
                providers.logger(),
            );
            let logs = logs.with_filter(tracing_subscriber::filter::filter_fn(move |metadata| {
                metadata.is_event()
                    && (matches!(routing, LogRouting::NativeEventsAsLogs)
                        || metadata.target() == "effect::log")
            }));
            tracing::Dispatch::new(tracing_subscriber::registry().with(trace).with(logs))
        }))
        .map_err(|panic| NativeError::Native(Defect::from_panic(panic)))
    }

    /// Create the core tracer used by both ordinary `span!` and `otel!`.
    /// Native SDK timestamps are used; core monotonic durations are never epoch times.
    #[must_use]
    pub fn tracer(&self) -> Tracer {
        let redaction = self.redaction.clone();
        let start = self.clone();
        let finish = self.clone();
        self.tracing.tracer_with_hooks(
            move |a| (redaction.annotations)(a),
            move |span, info| {
                if span.is_disabled() {
                    return Ok(());
                }
                let _permit = start.providers.admit().map_err(NativeError::into_defect)?;
                let current = opentelemetry::Context::current();
                let options = current
                    .get::<PendingOptions>()
                    .map(|o| Arc::clone(&o.options));
                let inherited = if current.get::<BridgeAttachment>().is_some() {
                    OtelContext::from_native(&current)
                } else {
                    let trace = TraceContext {
                        annotations: info.annotations.clone(),
                        span: info.parent.clone(),
                    };
                    OtelContext::from_native(&start.native_context(&trace))
                };
                // Strip pending bridge metadata before SDK inheritance so nested
                // ordinary spans cannot inherit another span's kind/attributes/links.
                let parent = options
                    .as_ref()
                    .and_then(|o| o.parent.as_ref())
                    .unwrap_or(&inherited)
                    .native()
                    .clone();
                span.set_parent(parent)
                    .map_err(|error| Defect::new(error.to_string()))?;
                span.record("otel.name", info.name.as_ref());
                let kind = options
                    .as_ref()
                    .map_or(opentelemetry::trace::SpanKind::Internal, |o| o.kind.clone());
                span.record("otel.kind", kind_name(&kind));
                // Core annotations have already passed the core redactor.
                for (key, value) in info.annotations.iter() {
                    span.set_attribute(key.to_string(), value.to_string());
                }
                if let Some(options) = options {
                    for value in (start.redaction.attributes)(&options.attributes)? {
                        span.set_attribute(value.key, value.value);
                    }
                    for link in &options.links {
                        span.add_link_with_attributes(
                            link.span_context.clone(),
                            (start.redaction.attributes)(&link.attributes)?,
                        );
                    }
                }
                Ok(())
            },
            move |span, info| {
                // Outcome classification contains no typed business error/defect payload.
                span.set_attribute("effect.outcome", outcome_name(info.outcome));
                span.set_attribute("effect.cleanup_failed", info.cleanup_failed);
                if info.cleanup_failed || !matches!(info.outcome, SpanOutcome::Succeeded) {
                    span.set_status(opentelemetry::trace::Status::error(outcome_name(
                        info.outcome,
                    )));
                }
                // A user-set status is preserved on successful completion.
                finish
                    .providers
                    .ensure_open()
                    .map_err(NativeError::into_defect)
            },
        )
    }

    /// Build a redacting core logger whose events route to the official log bridge.
    /// Log events never implicitly serialize their application error or payload.
    #[must_use]
    pub fn logger(&self, minimum: LogLevel) -> Logger {
        let redact = self.redaction.clone();
        let backend = self.clone();
        Logger::new(
            minimum,
            self.diagnostics.clone(),
            move |a| (redact.annotations)(a),
            move |event| {
                let _permit = backend
                    .providers
                    .admit()
                    .map_err(NativeError::into_defect)?;
                let message = (backend.redaction.message)(&event.message)?;
                let _guard = backend.enter_event(&event.context);
                match event.level {
                    LogLevel::Debug => {
                        event!(target: "effect::log", tracing::Level::DEBUG, message = %message, annotations = ?event.context.annotations.as_ref())
                    }
                    LogLevel::Info => {
                        event!(target: "effect::log", tracing::Level::INFO, message = %message, annotations = ?event.context.annotations.as_ref())
                    }
                    LogLevel::Warn => {
                        event!(target: "effect::log", tracing::Level::WARN, message = %message, annotations = ?event.context.annotations.as_ref())
                    }
                    LogLevel::Error => {
                        event!(target: "effect::log", tracing::Level::ERROR, message = %message, annotations = ?event.context.annotations.as_ref())
                    }
                }
                Ok(())
            },
        )
    }

    /// Scope a different explicit base context while sharing native service handles.
    #[must_use]
    pub fn with_context(&self, context: OtelContext) -> Self {
        Self {
            context,
            ..self.clone()
        }
    }

    /// Borrow the diagnostic sink used by concise best-effort operations.
    #[must_use]
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Borrow the metric recorder without constructing a new instrument registry.
    #[must_use]
    pub fn metrics(&self) -> &OtelMetricRecorder {
        &self.metrics
    }

    /// Perform a checked synchronous native operation while the local span is live.
    pub fn with_native<A>(
        &self,
        trace: &TraceContext,
        action: impl FnOnce(&tracing::Span) -> Result<A, NativeError>,
    ) -> Result<A, NativeError> {
        let _permit = self.providers.admit()?;
        let id = trace.span.as_ref().ok_or(NativeError::NoActiveSpan)?;
        catch_unwind(AssertUnwindSafe(|| {
            self.tracing.with_native(id, |span| {
                if !span.context().span().span_context().is_valid() {
                    return Err(NativeError::NoActiveSpan);
                }
                action(span)
            })
        }))
        .map_err(|panic| NativeError::Native(Defect::from_panic(panic)))?
        .ok_or(NativeError::NoActiveSpan)?
    }

    /// Preserve current local identity when live, otherwise retain the explicit base.
    fn native_context(&self, trace: &TraceContext) -> opentelemetry::Context {
        let native = trace
            .span
            .as_ref()
            .and_then(|id| self.tracing.with_native(id, OpenTelemetrySpanExt::context))
            .unwrap_or_else(|| self.context.native().clone());
        let inherited = native.baggage().iter().map(|(key, (value, metadata))| {
            opentelemetry::baggage::KeyValueMetadata::new(
                key.clone(),
                value.clone(),
                metadata.clone(),
            )
        });
        let selected = self
            .context
            .native()
            .baggage()
            .iter()
            .map(|(key, (value, metadata))| {
                opentelemetry::baggage::KeyValueMetadata::new(
                    key.clone(),
                    value.clone(),
                    metadata.clone(),
                )
            });
        native.with_baggage(inherited.chain(selected))
    }

    /// Respect an explicit detached/log bridge attachment without capturing an unrelated span.
    /// A captured core log context with another local ID still uses its own native map entry.
    fn enter_event(&self, trace: &TraceContext) -> Box<dyn PollScope> {
        let current = opentelemetry::Context::current();
        let selected = self.native_context(trace);
        let same = current.span().span_context() == selected.span().span_context();
        if current.get::<BridgeAttachment>().is_some() && (trace.span.is_none() || same) {
            let tracing = self.tracing.enter(trace);
            Box::new(OtelPollScope {
                _context: current.attach(),
                _tracing: tracing,
            })
        } else {
            self.enter(trace)
        }
    }

    /// Isolate concise operation failures, including panics in user redaction.
    fn diagnose(&self, action: impl FnOnce() -> Result<(), NativeError>) {
        match catch_unwind(AssertUnwindSafe(action)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => self.diagnostics.record(error.into_defect()),
            Err(panic) => self.diagnostics.record(Defect::from_panic(panic)),
        }
    }
}

/// Guards restore OTel first, then native span/dispatch before a poll returns.
struct OtelPollScope {
    /// The native context guard is thread-bound and never retained in a future.
    _context: opentelemetry::ContextGuard,
    /// Existing tracing adapter's span and dispatcher guards.
    _tracing: Box<dyn PollScope>,
}
impl PollScope for OtelPollScope {}
/// Empty guard returned after a diagnostic-only native instrumentation failure.
struct NoopPollScope;
impl PollScope for NoopPollScope {}
impl PollInstrumentation for OtelBackend {
    /// Enter this execution's native span and immutable distributed context.
    fn enter(&self, trace: &TraceContext) -> Box<dyn PollScope> {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let native = self.native_context(trace).with_value(BridgeAttachment);
            let tracing = self.tracing.enter(trace);
            Box::new(OtelPollScope {
                _context: native.attach(),
                _tracing: tracing,
            }) as Box<dyn PollScope>
        }));
        match result {
            Ok(guard) => guard,
            Err(panic) => {
                self.diagnostics.record(Defect::from_panic(panic));
                Box::new(NoopPollScope)
            }
        }
    }
}

/// One span's configuration attachment; nested spans cannot inherit pending options.
struct SpanInstrumentation {
    /// Environment-selected bridge shared for the execution.
    backend: OtelBackend,
    /// Parent local ID selects the pre-start transition only.
    parent: Option<SpanId>,
    /// Configuration is immutable and shared, without payload clones.
    options: Arc<SpanOptions>,
}
impl PollInstrumentation for SpanInstrumentation {
    /// Attach pending metadata only when the new span is being created.
    fn enter(&self, trace: &TraceContext) -> Box<dyn PollScope> {
        if trace.span != self.parent {
            return self.backend.enter(trace);
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            let native = self
                .backend
                .native_context(trace)
                .with_value(BridgeAttachment)
                .with_value(PendingOptions {
                    options: Arc::clone(&self.options),
                });
            let tracing = self.backend.tracing.enter(trace);
            Box::new(OtelPollScope {
                _context: native.attach(),
                _tracing: tracing,
            }) as Box<dyn PollScope>
        }));
        match result {
            Ok(guard) => guard,
            Err(panic) => {
                self.backend.diagnostics.record(Defect::from_panic(panic));
                Box::new(NoopPollScope)
            }
        }
    }
}

/// Selects the OTel bridge through the single application environment `R`.
/// Implementations expose `otel().tracer()` as the same environment's core tracer.
pub trait HasOtel: HasTracer + HasClock {
    /// Borrow the selected bridge; no global provider discovery occurs.
    fn otel(&self) -> &OtelBackend;
}

/// Typed Effect operations; concise signals are lazy and diagnostic-only.
pub struct Otel;
impl Otel {
    /// Install the environment's poll bridge around ordinary Effects and `span!`.
    /// This preserves the interpreter and typed exit, including ordered cleanup.
    pub fn instrument<A, E, R>(program: Effect<A, E, R>) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        program.with_poll_instrumentation(|env| Arc::new(selected_backend(env)))
    }

    /// Lower the convenience DSL to the exact `Effect::with_span` lifetime path.
    /// Metadata is shared configuration; business values/errors need no Clone.
    pub fn span<A, E, R>(
        name: impl Into<Arc<str>>,
        options: SpanOptions,
        program: Effect<A, E, R>,
    ) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        let name = name.into();
        let options = Arc::new(options);
        Effect::<TraceContext, E, R>::trace_context().flat_map(move |trace| {
            let parent = trace.span;
            let options = Arc::clone(&options);
            program
                .clone()
                .with_span(Arc::clone(&name))
                .with_poll_instrumentation(move |env| {
                    Arc::new(SpanInstrumentation {
                        backend: selected_backend(env),
                        parent: parent.clone(),
                        options: Arc::clone(&options),
                    })
                })
        })
    }

    /// Capture detached flags/tracestate/baggage from the current poll attachment.
    /// Use inside `instrument`, `otel!`, or `envelope`; the native thread guard is
    /// supplied by the environment's bridge and never used to discover a provider.
    pub fn current_context<E, R>() -> Effect<OtelContext, E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        effect! {
            let trace = Effect::<TraceContext, E, R>::trace_context()?;
            service! {
                env |env: &R| {
                    let backend = env.otel();
                    match catch_unwind(AssertUnwindSafe(|| {
                        let current = opentelemetry::Context::current();
                        if current.get::<BridgeAttachment>().is_some() {
                            OtelContext::from_native(&current)
                        } else {
                            OtelContext::from_native(&backend.native_context(&trace))
                        }
                    })) {
                        Ok(context) => context,
                        Err(panic) => {
                            backend.diagnostics.record(Defect::from_panic(panic));
                            backend.context.clone()
                        }
                    }
                },
            }?
        }
    }

    /// Attach a detached context for a lexical boundary without changing `R`.
    /// The inherited local span is cleared for this detached boundary; the caller's
    /// local context and instrumentation are restored by core continuation tags.
    pub fn envelope<A, E, R>(context: OtelContext, program: Effect<A, E, R>) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        effect! {
        bind;
        program: Effect::<TraceContext, E, R>::trace_context(),
        body: move |mut trace| {
                   trace.span = None;
                   let context = context.clone();
                   program
                       .clone()
                       .with_poll_instrumentation(move |env| {
                           Arc::new(env.otel().with_context(context.clone()))
                       })
                       .with_trace_context(trace)
               },
        }
    }

    /// Scope baggage updates while retaining the same live local span.
    /// Native W3C baggage validation/bounds apply; entries are explicit propagation
    /// metadata and the caller owns their disclosure/redaction policy.
    pub fn with_baggage<A, E, R>(
        entries: Vec<opentelemetry::baggage::KeyValueMetadata>,
        program: Effect<A, E, R>,
    ) -> Effect<A, E, R>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        let entries = Arc::new(entries);
        effect! {
        bind;
        program: Self::current_context(),
        body: move |context| {
                   let next = context.with_baggage(entries.iter().cloned());
                   program.clone().with_poll_instrumentation(move |env| {
                       Arc::new(env.otel().with_context(next.clone()))
                   })
               },
        }
    }

    /// Run a checked native callback; missing layers/live spans are typed errors.
    pub fn use_span<A, R>(
        action: impl Fn(&tracing::Span) -> Result<A, NativeError> + Send + Sync + 'static,
    ) -> Effect<A, NativeError, R>
    where
        A: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        let action = Arc::new(action);
        effect! {
            let action = Arc::clone(&action);
            let trace = Effect::<TraceContext, NativeError, R>::trace_context()?;
            let backend = service! { env |env: &R| { env.otel().clone() }, }?;
            effect_try_sync! {
                body { backend.with_native(&trace, |span| action(span)) },
            }?
        }
    }

    /// Perform native provider access as a checked lazy Effect.
    /// The callback receives host-owned handles, with lifecycle admission checked.
    pub fn use_providers<A, R>(
        action: impl Fn(&BorrowedProviders) -> Result<A, NativeError> + Send + Sync + 'static,
    ) -> Effect<A, NativeError, R>
    where
        A: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        let action = Arc::new(action);
        effect! {
            let action = Arc::clone(&action);
            let providers = service! { env |env: &R| { env.otel().providers.clone() }, }?;
            effect_try_sync! {
                body {
                    let _permit = providers.admit()?;
                    catch_unwind(AssertUnwindSafe(|| action(&providers)))
                        .map_err(|panic| NativeError::Native(Defect::from_panic(panic)))?
                },
            }?
        }
    }

    /// Add a typed string/bool/i64/f64 attribute when this statement is reached.
    pub fn attribute<E, R>(
        key: impl Into<opentelemetry::Key>,
        value: impl Into<Value>,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        let values = Arc::new(vec![KeyValue::new(key, value)]);
        best_effort(move |backend, trace| {
            backend.with_native(trace, |span| {
                for value in (backend.redaction.attributes)(&values).map_err(NativeError::Native)? {
                    span.set_attribute(value.key, value.value);
                }
                Ok(())
            })
        })
    }

    /// String shorthand for the typed attribute operation, with lazy emission.
    pub fn tag<E, R>(
        key: impl Into<opentelemetry::Key>,
        value: impl Into<String>,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        Self::attribute(key, value.into())
    }
    /// Emit a native boolean without string conversion.
    pub fn tag_bool<E, R>(key: impl Into<opentelemetry::Key>, value: bool) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        Self::attribute(key, value)
    }
    /// Emit the SDK's exact signed 64-bit integer representation.
    pub fn tag_int<E, R>(key: impl Into<opentelemetry::Key>, value: i64) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        Self::attribute(key, value)
    }
    /// Emit a native floating point attribute, preserving numeric kind.
    pub fn tag_number<E, R>(key: impl Into<opentelemetry::Key>, value: f64) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        Self::attribute(key, value)
    }

    /// Emit a trace event only, without also creating an equivalent log record.
    pub fn event<E, R>(name: impl Into<Arc<str>>) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        Self::event_with(name, Vec::new())
    }
    /// Emit typed redacted event attributes with SDK time, never core epoch guesses.
    pub fn event_with<E, R>(
        name: impl Into<Arc<str>>,
        attributes: Vec<KeyValue>,
    ) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        let name = name.into();
        let attributes = Arc::new(attributes);
        best_effort(move |backend, trace| {
            backend.with_native(trace, |span| {
                span.add_event(
                    name.to_string(),
                    (backend.redaction.attributes)(&attributes).map_err(NativeError::Native)?,
                );
                Ok(())
            })
        })
    }

    /// Apply an explicitly selected native status; success does not overwrite it.
    pub fn status<E, R>(status: opentelemetry::trace::Status) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        best_effort(move |backend, trace| {
            backend.with_native(trace, |span| {
                let status = match &status {
                    opentelemetry::trace::Status::Unset => opentelemetry::trace::Status::Unset,
                    opentelemetry::trace::Status::Ok => opentelemetry::trace::Status::Ok,
                    opentelemetry::trace::Status::Error { description } => {
                        opentelemetry::trace::Status::error(
                            (backend.redaction.message)(description)
                                .map_err(NativeError::Native)?
                                .to_string(),
                        )
                    }
                };
                span.set_status(status);
                Ok(())
            })
        })
    }

    /// Link another distributed operation with typed attributes and native identity.
    pub fn link<E, R>(link: opentelemetry::trace::Link) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        best_effort(move |backend, trace| {
            backend.with_native(trace, |span| {
                span.add_link_with_attributes(
                    link.span_context.clone(),
                    (backend.redaction.attributes)(&link.attributes)
                        .map_err(NativeError::Native)?,
                );
                Ok(())
            })
        })
    }

    /// Submit a core metric sample as diagnostic-only telemetry at this position.
    /// Explicit checked metrics remain available through `MetricRecorder`.
    pub fn record_metric<E, R>(sample: effect::MetricSample) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        let sample = Arc::new(sample);
        best_effort(move |backend, _| {
            use effect::MetricRecorder;
            backend
                .metrics
                .record(&sample)
                .map_err(|error| NativeError::Native(Defect::new(error.to_string())))
        })
    }

    /// Best-effort unsigned count using the selected metric policy for its units.
    /// Name validation and emission occur only when reached; native registry
    /// compatibility/cardinality constraints are unchanged. This short form has
    /// no labels; use `record_metric` for an explicit core descriptor/label set.
    pub fn count<E, R>(name: impl Into<Arc<str>>, value: u64) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        let name = name.into();
        best_effort(move |backend, _| {
            use effect::MetricRecorder;
            let name = effect::MetricName::new(Arc::clone(&name))
                .map_err(|error| NativeError::Native(Defect::new(error.to_string())))?;
            let descriptor = effect::MetricDescriptor::new(
                name,
                effect::MetricKind::Counter,
                effect::MetricDomain::General,
            );
            let sample = effect::MetricSample::new(
                descriptor,
                effect::MetricLabels::default(),
                effect::MetricValue::Counter(value),
            )
            .map_err(|error| NativeError::Native(Defect::new(error.to_string())))?;
            backend
                .metrics
                .record(&sample)
                .map_err(|error| NativeError::Native(Defect::new(error.to_string())))
        })
    }

    /// Emit a redacted application log through the official appender only.
    pub fn log<E, R>(level: LogLevel, message: impl Into<Arc<str>>) -> Effect<(), E, R>
    where
        E: Send + 'static,
        R: HasOtel + Send + Sync + 'static,
    {
        let message = message.into();
        best_effort(move |backend, trace| {
            backend.logger(level).emit(effect::LogEvent {
                level,
                message: Arc::clone(&message),
                context: trace.clone(),
            });
            Ok(())
        })
    }
}

/// Preserve a lexical detached parent/baggage attachment when selecting a nested
/// bridge. Without this snapshot, a nested `otel!` would fall back to the original
/// environment's base after replacing an enclosing `envelope` instrumentation.
fn selected_backend<R: HasOtel>(env: &R) -> OtelBackend {
    let current = opentelemetry::Context::current();
    if current.get::<BridgeAttachment>().is_some() {
        env.otel().with_context(OtelContext::from_native(&current))
    } else {
        env.otel().clone()
    }
}

/// Lift one best-effort signal lazily with the current execution's context.
fn best_effort<E, R>(
    action: impl Fn(&OtelBackend, &TraceContext) -> Result<(), NativeError> + Send + Sync + 'static,
) -> Effect<(), E, R>
where
    E: Send + 'static,
    R: HasOtel + Send + Sync + 'static,
{
    let action = Arc::new(action);
    effect! {
        let action = Arc::clone(&action);
        let trace = Effect::<TraceContext, E, R>::trace_context()?;
        service! {
            env |env: &R| {
                let backend = env.otel();
                backend.diagnose(|| action(backend, &trace));
            },
        }?
    }
}

/// OTel protocol spellings recognized by tracing-opentelemetry's special field.
fn kind_name(kind: &opentelemetry::trace::SpanKind) -> &'static str {
    match kind {
        opentelemetry::trace::SpanKind::Internal => "internal",
        opentelemetry::trace::SpanKind::Server => "server",
        opentelemetry::trace::SpanKind::Client => "client",
        opentelemetry::trace::SpanKind::Producer => "producer",
        opentelemetry::trace::SpanKind::Consumer => "consumer",
    }
}
/// Safe structural outcome metadata, with no error/credential formatting.
fn outcome_name(outcome: SpanOutcome) -> &'static str {
    match outcome {
        SpanOutcome::Succeeded => "succeeded",
        SpanOutcome::Failed => "failed",
        SpanOutcome::Defected => "defected",
        SpanOutcome::Interrupted => "interrupted",
        SpanOutcome::Abandoned => "abandoned",
    }
}
