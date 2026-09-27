use effect::{
    Annotations, Defect, Diagnostics, LogLevel, Logger, SpanFinish, SpanId, SpanStart, Tracer,
};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
#[derive(Clone)]
pub struct TracingBackend {
    spans: Arc<Mutex<HashMap<SpanId, tracing::Span>>>,
    dispatch: tracing::Dispatch,
    capacity: NonZeroUsize,
    diagnostics: Diagnostics,
}
impl TracingBackend {
    #[must_use]
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
    pub fn active(&self) -> usize {
        self.spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
    #[must_use]
    pub fn tracer(
        &self,
        redact: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
    ) -> Tracer {
        let start = self.clone();
        let finish = self.clone();
        Tracer::new(
            self.diagnostics.clone(),
            redact,
            move |info| start.start(info),
            move |info| {
                finish.finish(info);
                Ok(())
            },
        )
    }
    #[must_use]
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
    fn start(&self, info: &SpanStart) -> Result<Option<SpanId>, Defect> {
        let parent = info.parent.as_ref().and_then(|id| {
            self.spans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(id)
                .cloned()
        });
        let span = tracing::dispatcher::with_default(
            &self.dispatch,
            || tracing::span!(parent: parent.as_ref().and_then(tracing::Span::id), tracing::Level::INFO, stringify!(EffectSpan), name = %info.name, annotations = ?info.annotations, started = ?info.time, outcome = tracing::field::Empty, cleanup_failed = tracing::field::Empty, finished = tracing::field::Empty),
        );
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
    fn finish(&self, info: &SpanFinish) {
        let span = self
            .spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&info.id);
        if let Some(span) = span {
            span.record(stringify!(outcome), tracing::field::debug(info.outcome));
            span.record(stringify!(cleanup_failed), info.cleanup_failed);
            span.record(stringify!(finished), tracing::field::debug(info.time));
        }
    }
}
