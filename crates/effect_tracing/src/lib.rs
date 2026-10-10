//! Tracing-backed logging and span adapters.
//!
//! [`TracingBackend`] uses a caller-supplied `tracing::Dispatch`, active-span
//! capacity, and `Diagnostics` service. Construct its logger and tracer, then
//! expose them through the application's `HasLogger` and `HasTracer`
//! capabilities. Effect carries span context through execution; this adapter
//! sends events to the selected dispatcher without installing a global
//! subscriber. Redaction runs before events reach the backend.

use effect::{
    Annotations, Defect, Diagnostics, LogLevel, Logger, SpanFinish, SpanId, SpanStart, Tracer,
};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
#[derive(Clone)]
/// Adapter that emits Effect telemetry through the `tracing` subscriber.
pub struct TracingBackend {
    /// Live native spans keyed by local Effect identity, never by OTel wire IDs.
    spans: Arc<Mutex<HashMap<SpanId, tracing::Span>>>,
    /// Caller-owned dispatch; this backend never installs a global subscriber.
    dispatch: tracing::Dispatch,
    /// Explicit maximum live span ownership retained by this adapter.
    capacity: NonZeroUsize,
    /// Best-effort telemetry failures stay separate from business outcomes.
    diagnostics: Diagnostics,
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
            spans: Arc::new(Mutex::new(HashMap::new())),
            dispatch,
            capacity,
            diagnostics,
        }
    }
    #[must_use]
    /// Count spans currently retained by this backend.
    /// # Returns
    /// Type: `usize`.
    pub fn active(&self) -> usize {
        self.spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
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
        let start = self.clone();
        let finish = self.clone();
        Tracer::new(
            self.diagnostics.clone(),
            redact,
            move |info| start.start(info, &start_native),
            move |info| finish.finish(info, &finish_native),
        )
    }

    /// Borrow a live native span for one checked synchronous operation.
    ///
    /// The map lock is released before the callback. A missing/closed identity
    /// returns `None`; this does not construct a replacement span or reinterpret
    /// local Effect IDs as distributed identities.
    pub fn with_native<T>(
        &self,
        id: &SpanId,
        action: impl FnOnce(&tracing::Span) -> T,
    ) -> Option<T> {
        let span = self
            .spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()?;
        Some(tracing::dispatcher::with_default(&self.dispatch, || {
            action(&span)
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
        Logger::new(minimum, self.diagnostics.clone(), redact, move |event| {
            let span = event.context.span.as_ref().and_then(|id| {
                backend
                    .spans
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(id)
                    .cloned()
            });
            tracing::dispatcher::with_default(&backend.dispatch, || match event.level {
                LogLevel::Debug => {
                    tracing::event!(parent: span.as_ref().and_then(tracing::Span::id), tracing::Level::DEBUG, message = %event.message, annotations = ?event.context.annotations)
                }
                LogLevel::Info => {
                    tracing::event!(parent: span.as_ref().and_then(tracing::Span::id), tracing::Level::INFO, message = %event.message, annotations = ?event.context.annotations)
                }
                LogLevel::Warn => {
                    tracing::event!(parent: span.as_ref().and_then(tracing::Span::id), tracing::Level::WARN, message = %event.message, annotations = ?event.context.annotations)
                }
                LogLevel::Error => {
                    tracing::event!(parent: span.as_ref().and_then(tracing::Span::id), tracing::Level::ERROR, message = %event.message, annotations = ?event.context.annotations)
                }
            });
            Ok(())
        })
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
        decorate: &impl Fn(&tracing::Span, &SpanStart) -> Result<(), Defect>,
    ) -> Result<Option<SpanId>, Defect> {
        let parent = info.parent.as_ref().and_then(|id| {
            self.spans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(id)
                .cloned()
        });
        let span = tracing::dispatcher::with_default(
            &self.dispatch,
            || tracing::span!(parent: parent.as_ref().and_then(tracing::Span::id), tracing::Level::INFO, stringify!(EffectSpan), name = %info.name, annotations = ?info.annotations, started = ?info.time, outcome = tracing::field::Empty, cleanup_failed = tracing::field::Empty, finished = tracing::field::Empty, otel.name = tracing::field::Empty, otel.kind = tracing::field::Empty),
        );
        tracing::dispatcher::with_default(&self.dispatch, || decorate(&span, info))?;
        let Some(id) = span.id().map(|id| SpanId::new(id.into_u64().to_string())) else {
            return Ok(None);
        };
        let mut spans = self
            .spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if spans.len() == self.capacity.get() {
            return Err(Defect::from(effect::LibraryDefect::ProviderCapacityFull));
        }
        let previous = spans.insert(id.clone(), span);
        drop(spans);
        drop(previous);
        Ok(Some(id))
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
        let span = self
            .spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&info.id);
        if let Some(span) = span {
            span.record(stringify!(outcome), tracing::field::debug(info.outcome));
            span.record(stringify!(cleanup_failed), info.cleanup_failed);
            span.record(stringify!(finished), tracing::field::debug(info.time));
            tracing::dispatcher::with_default(&self.dispatch, || decorate(&span, info))?;
        }
        Ok(())
    }
}

/// Thread-local native guards retained only for the duration of one poll.
struct NativePollScope {
    /// Entered span drops before the selected dispatcher is restored.
    _span: Option<tracing::span::EnteredSpan>,
    /// Previous thread dispatcher is restored after the span exits.
    _dispatch: tracing::dispatcher::DefaultGuard,
}

impl effect::PollScope for NativePollScope {}

impl effect::PollInstrumentation for TracingBackend {
    /// Enter the selected dispatch and this execution's native span on this thread.
    /// Owned guards avoid borrowing a map entry and cannot survive a poll boundary.
    fn enter(&self, trace: &effect::TraceContext) -> Box<dyn effect::PollScope> {
        let dispatch = tracing::dispatcher::set_default(&self.dispatch);
        let span = trace.span.as_ref().and_then(|id| {
            self.spans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(id)
                .cloned()
        });
        Box::new(NativePollScope {
            _span: span.map(tracing::Span::entered),
            _dispatch: dispatch,
        })
    }
}
