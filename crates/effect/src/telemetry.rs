use crate::internal::runtime::{self};
use crate::{Clock, Defect, Effect, Exit, HasClock, PrimaryOutcomeRef};
use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Annotations = BTreeMap<Arc<str>, Arc<str>>;
static EMPTY_ANNOTATIONS: Annotations = Annotations::new();
#[derive(Clone, Default)]
pub enum SharedAnnotations {
    #[default]
    Empty,
    Shared(Arc<Annotations>),
}
impl SharedAnnotations {
    #[must_use]
    pub fn new(annotations: Annotations) -> Self {
        if annotations.is_empty() {
            Self::Empty
        } else {
            Self::Shared(Arc::new(annotations))
        }
    }
}
impl AsRef<Annotations> for SharedAnnotations {
    fn as_ref(&self) -> &Annotations {
        match self {
            Self::Empty => &EMPTY_ANNOTATIONS,
            Self::Shared(annotations) => annotations,
        }
    }
}
impl std::ops::Deref for SharedAnnotations {
    type Target = Annotations;
    fn deref(&self) -> &Annotations {
        self.as_ref()
    }
}
impl std::fmt::Debug for SharedAnnotations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_ref(), f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SpanId(Arc<str>);
impl SpanId {
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Default, Debug)]
pub struct TraceContext {
    pub annotations: SharedAnnotations,
    pub span: Option<SpanId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}
pub struct LogEvent {
    pub level: LogLevel,
    pub message: Arc<str>,
    pub context: TraceContext,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticCapacity {
    Unbounded,
    Bounded(NonZeroUsize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticOverflow {
    DiscardOldest,
    DiscardNewest,
}
struct DiagnosticState {
    entries: VecDeque<Defect>,
    discarded: usize,
}
#[derive(Clone)]
pub struct Diagnostics {
    state: Arc<Mutex<DiagnosticState>>,
    capacity: DiagnosticCapacity,
    overflow: DiagnosticOverflow,
}
impl Diagnostics {
    #[must_use]
    pub fn new(capacity: DiagnosticCapacity, overflow: DiagnosticOverflow) -> Self {
        Self {
            state: Arc::new(Mutex::new(DiagnosticState {
                entries: VecDeque::new(),
                discarded: usize::default(),
            })),
            capacity,
            overflow,
        }
    }
    pub fn record(&self, defect: Defect) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let DiagnosticCapacity::Bounded(capacity) = self.capacity
            && state.entries.len() == capacity.get()
        {
            state.discarded = state.discarded.saturating_add(NonZeroUsize::MIN.get());
            match self.overflow {
                DiagnosticOverflow::DiscardOldest => {
                    drop(state.entries.pop_front());
                }
                DiagnosticOverflow::DiscardNewest => return,
            }
        }
        state.entries.push_back(defect);
    }
    #[must_use]
    pub fn drain(&self) -> Vec<Defect> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .drain(..)
            .collect()
    }
    #[must_use]
    pub fn discarded(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .discarded
    }
    fn isolate<T>(&self, callback: impl FnOnce() -> Result<T, Defect>) -> Option<T> {
        match catch_unwind(AssertUnwindSafe(callback)) {
            Ok(Ok(value)) => Some(value),
            Ok(Err(defect)) => {
                self.record(defect);
                None
            }
            Err(panic) => {
                self.record(Defect::from_panic(panic));
                None
            }
        }
    }
}
type Redact = Arc<dyn Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync>;
type WriteLog = Arc<dyn Fn(&LogEvent) -> Result<(), Defect> + Send + Sync>;
type StartSpan = Arc<dyn Fn(&SpanStart) -> Result<Option<SpanId>, Defect> + Send + Sync>;
type FinishSpan = Arc<dyn Fn(&SpanFinish) -> Result<(), Defect> + Send + Sync>;
#[derive(Clone)]
pub struct Logger {
    minimum: LogLevel,
    redact: Redact,
    write: WriteLog,
    diagnostics: Diagnostics,
}
impl Logger {
    #[must_use]
    pub fn new(
        minimum: LogLevel,
        diagnostics: Diagnostics,
        redact: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
        write: impl Fn(&LogEvent) -> Result<(), Defect> + Send + Sync + 'static,
    ) -> Self {
        Self {
            minimum,
            redact: Arc::new(redact),
            write: Arc::new(write),
            diagnostics,
        }
    }
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
    pub fn emit(&self, mut event: LogEvent) {
        if event.level < self.minimum {
            return;
        }
        if let Some(annotations) = self
            .diagnostics
            .isolate(|| (self.redact)(&event.context.annotations))
        {
            event.context.annotations = SharedAnnotations::new(annotations);
            let _ = self.diagnostics.isolate(|| (self.write)(&event));
        }
    }
}
pub trait HasLogger {
    fn logger(&self) -> &Logger;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanOutcome {
    Succeeded,
    Failed,
    Defected,
    Interrupted,
    Abandoned,
}
pub struct SpanStart {
    pub name: Arc<str>,
    pub parent: Option<SpanId>,
    pub annotations: SharedAnnotations,
    pub time: Duration,
}
pub struct SpanFinish {
    pub id: SpanId,
    pub outcome: SpanOutcome,
    pub cleanup_failed: bool,
    pub time: Duration,
}
#[derive(Clone)]
pub struct Tracer {
    redact: Redact,
    start: StartSpan,
    finish: FinishSpan,
    diagnostics: Diagnostics,
}
impl Tracer {
    #[must_use]
    pub fn new(
        diagnostics: Diagnostics,
        redact: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
        start: impl Fn(&SpanStart) -> Result<Option<SpanId>, Defect> + Send + Sync + 'static,
        finish: impl Fn(&SpanFinish) -> Result<(), Defect> + Send + Sync + 'static,
    ) -> Self {
        Self {
            diagnostics,
            redact: Arc::new(redact),
            start: Arc::new(start),
            finish: Arc::new(finish),
        }
    }
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
    fn begin(&self, mut start: SpanStart) -> Option<SpanId> {
        start.annotations = SharedAnnotations::new(
            self.diagnostics
                .isolate(|| (self.redact)(&start.annotations))?,
        );
        self.diagnostics.isolate(|| (self.start)(&start)).flatten()
    }
    fn end(&self, finish: SpanFinish) {
        let _ = self.diagnostics.isolate(|| (self.finish)(&finish));
    }
}
pub trait HasTracer {
    fn tracer(&self) -> &Tracer;
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Effect<A, E, R> {
    pub fn annotate(self, annotations: Annotations) -> Self {
        let annotations = SharedAnnotations::new(annotations);
        Self::new(move |env, context, finish| {
            let mut merged = context.trace.annotations.as_ref().clone();
            merged.extend(
                annotations
                    .iter()
                    .map(|(key, value)| (Arc::clone(key), Arc::clone(value))),
            );
            let mut context = context;
            context.trace.annotations = SharedAnnotations::new(merged);
            self.clone().execute(env, context, finish)
        })
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Effect<TraceContext, E, R> {
    pub fn trace_context() -> Self {
        Effect::ready(|_, context| Exit::Success(context.trace.clone()))
    }
}
impl<E: Send + 'static, R: HasLogger + Send + Sync + 'static> Effect<(), E, R> {
    pub fn log(level: LogLevel, message: impl Into<Arc<str>>) -> Self {
        let message = message.into();
        Self::ready(move |env, context| {
            // A provider accessor is instrumentation too; isolate its unwind.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                env.logger().emit(LogEvent {
                    level,
                    message: Arc::clone(&message),
                    context: context.trace.clone(),
                })
            }));
            Exit::Success(())
        })
    }
}
struct SpanGuard<R: HasTracer + HasClock + Send + Sync + 'static> {
    env: Arc<R>,
    id: Option<SpanId>,
}
impl<R: HasTracer + HasClock + Send + Sync + 'static> SpanGuard<R> {
    fn finish(&mut self, outcome: SpanOutcome, cleanup_failed: bool) {
        if let Some(id) = self.id.take() {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let tracer = self.env.tracer();
                if let Some(time) = tracer.diagnostics.isolate(|| Ok(self.env.clock().now())) {
                    tracer.end(SpanFinish {
                        id,
                        outcome,
                        cleanup_failed,
                        time,
                    });
                }
            }));
        }
    }
}
impl<R: HasTracer + HasClock + Send + Sync + 'static> Drop for SpanGuard<R> {
    fn drop(&mut self) {
        self.finish(SpanOutcome::Abandoned, false);
    }
}
impl<A: Send + 'static, E: Send + 'static, R: HasTracer + HasClock + Send + Sync + 'static>
    Effect<A, E, R>
{
    pub fn with_span(self, name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        Self::new(move |env, mut context, finish| {
            let id = catch_unwind(AssertUnwindSafe(|| {
                let tracer = env.tracer();
                let time = tracer.diagnostics.isolate(|| Ok(env.clock().now()))?;
                tracer.begin(SpanStart {
                    name: Arc::clone(&name),
                    parent: context.trace.span.clone(),
                    annotations: context.trace.annotations.clone(),
                    time,
                })
            }))
            .ok()
            .flatten();
            let mut guard = SpanGuard {
                env: Arc::clone(&env),
                id: id.clone(),
            };
            if id.is_some() {
                context.trace.span = id;
            }
            let publish_context = context.clone();
            self.clone().execute(
                env,
                context,
                runtime::continuation(move |outcome| {
                    let outcome = runtime::publish(outcome, &publish_context);
                    let status = match outcome.primary() {
                        PrimaryOutcomeRef::Succeeded => SpanOutcome::Succeeded,
                        PrimaryOutcomeRef::Failed(_) => SpanOutcome::Failed,
                        PrimaryOutcomeRef::Defected(_) => SpanOutcome::Defected,
                        PrimaryOutcomeRef::WasInterrupted => SpanOutcome::Interrupted,
                    };
                    guard.finish(status, matches!(outcome, Exit::FinalizationFailed(_)));
                    drop(guard);
                    runtime::finish(finish, outcome)
                }),
            )
        })
    }
}
