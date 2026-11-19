//! Tracing-backed logging and span adapters.
//!
//! [`TracingBackend`] uses a caller-supplied `tracing::Dispatch`, active-span
//! capacity, and `Diagnostics` service. Construct its logger and tracer, then
//! expose them through the application's `HasLogger` and `HasTracer`
//! capabilities. Effect carries span context through execution; this adapter
//! sends events to the selected dispatcher without installing a global
//! subscriber. Redaction runs before events reach the backend.

use effect::memory::{MemoryError, PreparedMemory, RegionId};
use effect::{
    AdmissionPermit, Annotations, Defect, Diagnostics, Effect, HasClock, HasTracer, LogLevel, Logger,
    PollInstrumentation, PreparedAdmissionPermit, PreparedDiagnostics, PreparedPollHook,
    PreparedTraceContext, SharedAnnotations, SpanFinish, SpanId, SpanStart, Tracer,
};
mod prepared;
pub use prepared::{PreparedSpanAnnotations, PreparedSpanStart, PreparedTracingCapacity};
use prepared::{PreparedSpans, SpanOwner, capacity_defect};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use tracing::{event, span};
#[derive(Clone)]
/// Adapter that emits Effect telemetry through the `tracing` subscriber.
pub struct TracingBackend {
    /// Live native spans keyed by local Effect identity, never by OTel wire IDs.
    spans: Arc<Spans>,
    /// Caller-owned dispatch; this backend never installs a global subscriber.
    dispatch: tracing::Dispatch,
    /// Explicit maximum live span ownership retained by this adapter.
    capacity: NonZeroUsize,
    /// Best-effort telemetry failures stay separate from business outcomes.
    diagnostics: Diagnostics,
    prepared_diagnostics: Option<PreparedDiagnostics>,
}
enum Spans {
    Ordinary(Mutex<HashMap<SpanId, Arc<LiveSpan>>>),
    Prepared(PreparedSpans),
}

/// Provider admission retained through the tracked native span's destruction.
pub enum SpanAdmission {
    /// Admission from an ordinary provider.
    Ordinary(AdmissionPermit),
    /// Admission using startup-prepared provider controls.
    Prepared(PreparedAdmissionPermit),
}

struct LiveSpan {
    // Closing the last native span may submit its export synchronously. Keep
    // lifecycle admission until after that native destructor has returned.
    native: tracing::Span,
    _retirement: Option<SpanAdmission>,
}

struct StartData<T> {
    value: T,
    retirement: Option<SpanAdmission>,
}

enum SpanLoan {
    Ordinary(Arc<LiveSpan>),
    Prepared(SpanOwner),
}

impl SpanLoan {
    fn native(&self) -> Option<&tracing::Span> {
        match self {
            Self::Ordinary(span) => Some(&span.native),
            Self::Prepared(span) => span.native(),
        }
    }
}

impl TracingBackend {
    #[must_use]
    /// Retain the selected dispatcher and bound the number of tracked spans.
    /// # Parameters
    /// - `dispatch` (`tracing::Dispatch`): The dispatch supplied to this operation.
    /// - `capacity` (`NonZeroUsize`): The caller-supplied capacity bound.
    /// - `diagnostics` (`Diagnostics`): Separate sink for instrumentation or provider failures.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        dispatch: tracing::Dispatch,
        capacity: NonZeroUsize,
        diagnostics: Diagnostics,
    ) -> Self {
        Self {
            spans: Arc::new(Spans::Ordinary(Mutex::new(HashMap::new()))),
            dispatch,
            capacity,
            diagnostics,
            prepared_diagnostics: None,
        }
    }
    /// Install physical span records and metadata before the region is sealed.
    /// The caller's native subscriber retains its own independent lifecycle.
    pub fn prepare(
        dispatch: tracing::Dispatch,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedTracingCapacity,
        diagnostics: Diagnostics,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            spans: Arc::new(Spans::Prepared(PreparedSpans::prepare(
                memory, region, capacity,
            )?)),
            dispatch,
            capacity: capacity.spans,
            diagnostics,
            prepared_diagnostics: None,
        })
    }

    /// Select prepared diagnostics for best-effort instrumentation failures.
    #[must_use]
    pub fn with_prepared_diagnostics(mut self, diagnostics: PreparedDiagnostics) -> Self {
        self.prepared_diagnostics = Some(diagnostics);
        self
    }

    #[must_use]
    /// Count spans currently retained by this backend.
    /// # Returns
    /// Type: `usize`.
    pub fn active(&self) -> usize {
        match self.spans.as_ref() {
            Spans::Ordinary(spans) => spans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            Spans::Prepared(spans) => spans.active(),
        }
    }

    fn find(&self, id: &str) -> Option<SpanLoan> {
        match self.spans.as_ref() {
            Spans::Ordinary(spans) => spans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(id)
                .cloned()
                .map(SpanLoan::Ordinary),
            Spans::Prepared(spans) => spans.lookup(id).map(SpanLoan::Prepared),
        }
    }

    #[must_use]
    /// Build a tracer that redacts annotations before emitting spans.
    /// # Parameters
    /// - `redact` (`impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static`): The redact supplied to this operation.
    ///
    /// # Returns
    /// Type: `Tracer`.
    pub fn tracer(
        &self,
        redact: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
    ) -> Tracer {
        self.tracer_with_hooks(redact, |_, _| Ok(()), |_, _| Ok(()))
    }

    /// Decorate the same native spans used by ordinary `span!`.
    ///
    /// `start_native` runs before the span is entered or exposed, allowing a
    /// distributed parent to be attached before SDK span construction. The end
    /// callback observes the full Effect outcome before the last retained handle
    /// is released. Hook failures enter diagnostics through the core tracer.
    /// Neither callback may retain ambient guards across asynchronous work.
    #[must_use]
    pub fn tracer_with_hooks(
        &self,
        redact: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
        start_native: impl Fn(&tracing::Span, &SpanStart) -> Result<(), Defect> + Send + Sync + 'static,
        finish_native: impl Fn(&tracing::Span, &SpanFinish) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
    ) -> Tracer {
        self.tracer_with_admission(redact, || Ok(None), start_native, finish_native)
    }

    /// Keep the admitted provider operation live through all tracked native
    /// span borrows, including the actual native close/export submission.
    #[must_use]
    pub fn tracer_with_retained_hooks(
        &self,
        redact: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
        admit: impl Fn() -> Result<SpanAdmission, Defect> + Send + Sync + 'static,
        start_native: impl Fn(&tracing::Span, &SpanStart) -> Result<(), Defect> + Send + Sync + 'static,
        finish_native: impl Fn(&tracing::Span, &SpanFinish) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
    ) -> Tracer {
        self.tracer_with_admission(
            redact,
            move || admit().map(Some),
            start_native,
            finish_native,
        )
    }

    fn tracer_with_admission(
        &self,
        redact: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
        admit: impl Fn() -> Result<Option<SpanAdmission>, Defect> + Send + Sync + 'static,
        start_native: impl Fn(&tracing::Span, &SpanStart) -> Result<(), Defect> + Send + Sync + 'static,
        finish_native: impl Fn(&tracing::Span, &SpanFinish) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
    ) -> Tracer {
        let start = self.clone();
        let finish = self.clone();
        let tracer = match self.spans.as_ref() {
            Spans::Ordinary(_) => Tracer::new(
                self.diagnostics.clone(),
                redact,
                move |info| start.start(info, &admit, &start_native),
                move |info| finish.finish(info, &finish_native),
            ),
            Spans::Prepared(_) => Tracer::new_with_start(
                self.diagnostics.clone(),
                move |info| {
                    start.start_prepared(
                        info,
                        |annotations, output| {
                            let redacted = redact(annotations)?;
                            for (key, value) in &redacted {
                                output.push(key, value).map_err(capacity_defect)?;
                            }
                            Ok(SpanStart {
                                name: info.name.clone(),
                                parent: info.parent.clone(),
                                annotations: SharedAnnotations::new(redacted),
                                time: info.time,
                            })
                        },
                        |_, redacted| {
                            Ok(StartData {
                                value: redacted,
                                retirement: admit()?,
                            })
                        },
                        |span, _, redacted| start_native(span, &redacted),
                    )
                },
                move |info| finish.finish(info, &finish_native),
            ),
        };
        self.tracer_diagnostics(tracer)
    }

    /// Reserve actual span storage before redaction and separately stage any
    /// native integration data before constructing or decorating the span.
    /// The redactor writes to a fixed destination, and checked refusal skips
    /// all subscriber work. Integration captures remain the caller's domain.
    pub fn tracer_with_prepared_hooks<T>(
        &self,
        redact: impl Fn(&Annotations, &mut PreparedSpanAnnotations<'_>) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
        before_start: impl Fn(&PreparedSpanStart<'_>) -> Result<T, Defect> + Send + Sync + 'static,
        start_native: impl Fn(&tracing::Span, &PreparedSpanStart<'_>, T) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
        finish_native: impl Fn(&tracing::Span, &SpanFinish) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
    ) -> Tracer {
        self.tracer_with_prepared_admission(
            redact,
            move |info| {
                Ok(StartData {
                    value: before_start(info)?,
                    retirement: None,
                })
            },
            start_native,
            finish_native,
        )
    }

    /// Retain the provider permit with the same physical record as the prepared
    /// native span. The staging value remains scoped to its start callback.
    pub fn tracer_with_prepared_retained_hooks<T>(
        &self,
        redact: impl Fn(&Annotations, &mut PreparedSpanAnnotations<'_>) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
        before_start: impl Fn(&PreparedSpanStart<'_>) -> Result<(SpanAdmission, T), Defect>
        + Send
        + Sync
        + 'static,
        start_native: impl Fn(&tracing::Span, &PreparedSpanStart<'_>, T) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
        finish_native: impl Fn(&tracing::Span, &SpanFinish) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
    ) -> Tracer {
        self.tracer_with_prepared_admission(
            redact,
            move |info| {
                let (permit, value) = before_start(info)?;
                Ok(StartData {
                    value,
                    retirement: Some(permit),
                })
            },
            start_native,
            finish_native,
        )
    }

    fn tracer_with_prepared_admission<T>(
        &self,
        redact: impl Fn(&Annotations, &mut PreparedSpanAnnotations<'_>) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
        before_start: impl Fn(&PreparedSpanStart<'_>) -> Result<StartData<T>, Defect>
        + Send
        + Sync
        + 'static,
        start_native: impl Fn(&tracing::Span, &PreparedSpanStart<'_>, T) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
        finish_native: impl Fn(&tracing::Span, &SpanFinish) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
    ) -> Tracer {
        let start = self.clone();
        let finish = self.clone();
        self.tracer_diagnostics(Tracer::new_with_start(
            self.diagnostics.clone(),
            move |info| {
                start.start_prepared(info, &redact, |info, ()| before_start(info), &start_native)
            },
            move |info| finish.finish(info, &finish_native),
        ))
    }

    fn tracer_diagnostics(&self, tracer: Tracer) -> Tracer {
        match &self.prepared_diagnostics {
            Some(diagnostics) => tracer.with_prepared_diagnostics(diagnostics.clone()),
            None => tracer,
        }
    }

    /// Borrow a live native span for one checked synchronous operation.
    ///
    /// The map lock is released before the callback. A missing/closed identity
    /// returns `None`; this does not construct a replacement span or reinterpret
    /// local Effect IDs as distributed identities. Adapter admission covers
    /// the native borrow. An explicit native clone that escapes the callback
    /// follows the caller's native lifecycle contract.
    pub fn with_native<T>(
        &self,
        id: &SpanId,
        action: impl FnOnce(&tracing::Span) -> T,
    ) -> Option<T> {
        self.with_native_text(id.as_str(), action)
    }

    /// Borrow a span by its checked textual identity, as carried by a prepared trace.
    pub fn with_native_text<T>(
        &self,
        id: &str,
        action: impl FnOnce(&tracing::Span) -> T,
    ) -> Option<T> {
        let owner = self.find(id)?;
        let span = owner.native()?;
        Some(tracing::dispatcher::with_default(&self.dispatch, || {
            action(span)
        }))
    }

    /// Borrow the selected dispatch for native subscriber composition.
    #[must_use]
    pub fn dispatch(&self) -> &tracing::Dispatch {
        &self.dispatch
    }
    #[must_use]
    /// Build a level-filtered logger that redacts annotations before emission.
    /// # Parameters
    /// - `minimum` (`LogLevel`): Lower bound enforced by this policy.
    /// - `redact` (`impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static`): The redact supplied to this operation.
    ///
    /// # Returns
    /// Type: `Logger`.
    pub fn logger(
        &self,
        minimum: LogLevel,
        redact: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
    ) -> Logger {
        let backend = self.clone();
        let logger = Logger::new(minimum, self.diagnostics.clone(), redact, move |event| {
            let parent = event
                .context
                .span
                .as_ref()
                .and_then(|id| backend.find(id.as_str()));
            let span = parent.as_ref().and_then(SpanLoan::native);
            tracing::dispatcher::with_default(&backend.dispatch, || match event.level {
                LogLevel::Debug => {
                    event!(parent: span.and_then(tracing::Span::id), tracing::Level::DEBUG, message = %event.message, annotations = ?event.context.annotations)
                }
                LogLevel::Info => {
                    event!(parent: span.and_then(tracing::Span::id), tracing::Level::INFO, message = %event.message, annotations = ?event.context.annotations)
                }
                LogLevel::Warn => {
                    event!(parent: span.and_then(tracing::Span::id), tracing::Level::WARN, message = %event.message, annotations = ?event.context.annotations)
                }
                LogLevel::Error => {
                    event!(parent: span.and_then(tracing::Span::id), tracing::Level::ERROR, message = %event.message, annotations = ?event.context.annotations)
                }
            });
            Ok(())
        });
        match &self.prepared_diagnostics {
            Some(diagnostics) => logger.with_prepared_diagnostics(diagnostics.clone()),
            None => logger,
        }
    }
    ///
    /// # Parameters
    /// - `info` (`&SpanStart`): The info supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Option<SpanId>, Defect>`.
    fn start(
        &self,
        info: &SpanStart,
        admit: &impl Fn() -> Result<Option<SpanAdmission>, Defect>,
        decorate: &impl Fn(&tracing::Span, &SpanStart) -> Result<(), Defect>,
    ) -> Result<Option<SpanId>, Defect> {
        let Spans::Ordinary(spans) = self.spans.as_ref() else {
            return Err(capacity_defect(MemoryError::UnsupportedRunContract));
        };
        let retirement = admit()?;
        let parent = info.parent.as_ref().and_then(|id| self.find(id.as_str()));
        let native = self.make_span(parent.as_ref(), &info.name, &info.annotations, info.time);
        tracing::dispatcher::with_default(&self.dispatch, || decorate(&native, info))?;
        let Some(id) = native.id().map(|id| SpanId::from_u64(id.into_u64())) else {
            return Ok(None);
        };
        let mut spans = spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if spans.len() == self.capacity.get() {
            return Err(Defect::from(effect::LibraryDefect::ProviderCapacityFull));
        }
        let previous = spans.insert(
            id.clone(),
            Arc::new(LiveSpan {
                native,
                _retirement: retirement,
            }),
        );
        drop(spans);
        drop(previous);
        Ok(Some(id))
    }

    fn start_prepared<T, D>(
        &self,
        info: &SpanStart,
        redact: impl FnOnce(&Annotations, &mut PreparedSpanAnnotations<'_>) -> Result<T, Defect>,
        before: impl FnOnce(&PreparedSpanStart<'_>, T) -> Result<StartData<D>, Defect>,
        decorate: impl FnOnce(&tracing::Span, &PreparedSpanStart<'_>, D) -> Result<(), Defect>,
    ) -> Result<Option<SpanId>, Defect> {
        let Spans::Prepared(spans) = self.spans.as_ref() else {
            return Err(capacity_defect(MemoryError::UnsupportedRunContract));
        };
        let mut vacancy = spans.reserve()?;
        let redacted = vacancy.write(info, redact)?;
        let info = vacancy.info(info);
        let StartData { value, retirement } = before(&info, redacted)?;
        let parent = info.parent.and_then(|id| self.find(id.as_str()));
        let native = self.make_span(parent.as_ref(), info.name, &info, info.time);
        tracing::dispatcher::with_default(&self.dispatch, || decorate(&native, &info, value))?;
        let Some(id) = native.id().map(|id| SpanId::from_u64(id.into_u64())) else {
            return Ok(None);
        };
        spans.insert(
            &id,
            vacancy.publish(
                id.clone(),
                LiveSpan {
                    native,
                    _retirement: retirement,
                },
            ),
        )?;
        Ok(Some(id))
    }

    fn make_span(
        &self,
        parent: Option<&SpanLoan>,
        name: &str,
        annotations: &dyn std::fmt::Debug,
        time: std::time::Duration,
    ) -> tracing::Span {
        let parent = parent
            .and_then(SpanLoan::native)
            .and_then(tracing::Span::id);
        tracing::dispatcher::with_default(
            &self.dispatch,
            || span!(parent: parent, tracing::Level::INFO, stringify!(EffectSpan), name = %name, annotations = ?annotations, started = ?time, outcome = tracing::field::Empty, cleanup_failed = tracing::field::Empty, finished = tracing::field::Empty, otel.name = tracing::field::Empty, otel.kind = tracing::field::Empty),
        )
    }

    ///
    /// # Parameters
    /// - `info` (`&SpanFinish`): The info supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn finish(
        &self,
        info: &SpanFinish,
        decorate: &impl Fn(&tracing::Span, &SpanFinish) -> Result<(), Defect>,
    ) -> Result<(), Defect> {
        let owner = match self.spans.as_ref() {
            Spans::Ordinary(spans) => spans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&info.id)
                .map(SpanLoan::Ordinary),
            Spans::Prepared(spans) => spans.remove(&info.id).map(SpanLoan::Prepared),
        };
        if let Some(span) = owner.as_ref().and_then(SpanLoan::native) {
            span.record(stringify!(outcome), tracing::field::debug(info.outcome));
            span.record(stringify!(cleanup_failed), info.cleanup_failed);
            span.record(stringify!(finished), tracing::field::debug(info.time));
            tracing::dispatcher::with_default(&self.dispatch, || decorate(span, info))?;
        }
        Ok(())
    }
}

/// Run a typed program inside a named span and its selected native poll instrumentation.
///
/// The environment supplies the tracer and clock. The instrumentation is selected
/// lazily from that same environment and applies to span start, body and finish.
/// Each native poll guard restores its caller's context on completion or unwind.
pub fn with_span<A, E, R>(
    name: impl Into<Arc<str>>,
    program: Effect<A, E, R>,
    select: impl Fn(&R) -> Arc<dyn PollInstrumentation> + Send + Sync + 'static,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasTracer + HasClock + Send + Sync + 'static,
{
    effect::effect! {
        poll_instrumentation;
        program: effect::span! { name: name, effect: program, },
        select: select,
    }
}

/// Thread-local native guards retained only for the duration of one poll.
#[doc(hidden)]
pub struct NativePollScope {
    // Leave the span before releasing its real owner or restoring the dispatcher.
    _span: Option<tracing::span::EnteredSpan>,
    _owner: Option<SpanLoan>,
    _dispatch: tracing::dispatcher::DefaultGuard,
}

impl effect::PollScope for NativePollScope {}

impl TracingBackend {
    /// Enter this run's native span and retain the concrete per-poll guard.
    #[doc(hidden)]
    pub fn enter_poll_scope(&self, trace: &effect::TraceContext) -> NativePollScope {
        self.enter_native(trace.span.as_ref().map(SpanId::as_str))
    }

    fn enter_native(&self, id: Option<&str>) -> NativePollScope {
        let dispatch = tracing::dispatcher::set_default(&self.dispatch);
        let owner = id.and_then(|id| self.find(id));
        let span = owner.as_ref().and_then(SpanLoan::native).cloned();
        NativePollScope {
            _span: span.map(tracing::Span::entered),
            _owner: owner,
            _dispatch: dispatch,
        }
    }
}

impl effect::PollInstrumentation for TracingBackend {
    /// Enter the selected dispatch and this execution's native span on this thread.
    fn enter(&self, trace: &effect::TraceContext) -> Box<dyn effect::PollScope> {
        Box::new(self.enter_poll_scope(trace))
    }
}

impl PreparedPollHook for TracingBackend {
    type Guard<'a> = NativePollScope;
    fn enter<'a>(&'a self, trace: Option<&'a PreparedTraceContext>) -> Self::Guard<'a> {
        self.enter_native(trace.and_then(PreparedTraceContext::span))
    }
}
