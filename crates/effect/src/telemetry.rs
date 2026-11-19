//! Tracing, logging, and diagnostic capability contracts.

use crate::internal::runtime::{self};
use crate::{Clock, Defect, Effect, Exit, HasClock, PrimaryOutcomeRef};
use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod prepared_trace;
mod prepared_diagnostics;
pub use prepared_diagnostics::{
    PreparedDiagnostic, PreparedDiagnosticCapacity, PreparedDiagnosticError,
    PreparedDiagnosticSnapshot, PreparedDiagnostics,
};
pub use prepared_trace::{PreparedTraceCapacity, PreparedTraceContext, PreparedTracePool};

/// Type alias for annotations in telemetry.
pub type Annotations = BTreeMap<Arc<str>, Arc<str>>;
static EMPTY_ANNOTATIONS: Annotations = Annotations::new();
/// Immutable shared annotations carried through execution context; Debug reports only entry count and never exposes annotation values.
#[derive(Clone, Default)]
pub enum SharedAnnotations {
    #[default]
    /// No value is present.
    Empty,
    /// Other observers still retain a shared view of the outcome. Contains `Arc<Annotations>`.
    Shared(Arc<Annotations>),
}
impl SharedAnnotations {
    #[must_use]
    /// Construct a `SharedAnnotations` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `annotations` (`Annotations`): The annotations supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(annotations: Annotations) -> Self {
        if annotations.is_empty() {
            Self::Empty
        } else {
            Self::Shared(Arc::new(annotations))
        }
    }
}
impl AsRef<Annotations> for SharedAnnotations {
    ///
    /// # Returns
    /// Type: `&Annotations`.
    fn as_ref(&self) -> &Annotations {
        match self {
            Self::Empty => &EMPTY_ANNOTATIONS,
            Self::Shared(annotations) => annotations,
        }
    }
}
impl std::ops::Deref for SharedAnnotations {
    /// Type alias for target in telemetry.
    type Target = Annotations;
    ///
    /// # Returns
    /// Type: `&Annotations`.
    fn deref(&self) -> &Annotations {
        self.as_ref()
    }
}
impl std::fmt::Debug for SharedAnnotations {
    ///
    /// # Parameters
    /// - `f` (`&mut std::fmt::Formatter<'_>`): The f supplied to this operation.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedAnnotations")
            .field("entries", &self.len())
            .finish()
    }
}

#[derive(Clone)]
/// Stable identity used by telemetry operations.
pub struct SpanId(SpanIdText);

#[derive(Clone)]
enum SpanIdText {
    Shared(Arc<str>),
    Decimal([u8; 20]),
}
impl SpanId {
    #[must_use]
    /// Construct a `SpanId` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `id` (`impl Into<Arc<str>>`): Stable identity of the selected record or operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(SpanIdText::Shared(id.into()))
    }
    /// Retain a native numeric span identity in line, with the same textual
    /// equality and hash as an equivalent shared decimal identity.
    #[must_use]
    pub fn from_u64(id: u64) -> Self {
        let mut number = std::fmt::NumBuffer::new();
        let text = id.format_into(&mut number);
        let mut bytes = [0; 20];
        bytes.split_at_mut(text.len()).0.copy_from_slice(text.as_bytes());
        Self(SpanIdText::Decimal(bytes))
    }

    #[must_use]
    /// Borrow the textual representation without allocating a new string.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn as_str(&self) -> &str {
        match &self.0 {
            SpanIdText::Shared(text) => text,
            SpanIdText::Decimal(text) => {
                str::from_utf8(text).unwrap_or_default().trim_end_matches('\0')
            }
        }
    }
}
impl std::borrow::Borrow<str> for SpanId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
impl PartialEq for SpanId {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for SpanId {}
impl std::hash::Hash for SpanId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(self.as_str(), state);
    }
}
impl std::fmt::Debug for SpanId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("SpanId").field(&self.as_str()).finish()
    }
}
#[derive(Clone, Default, Debug)]
/// The active span identity and annotations carried with a running Effect and its children.
pub struct TraceContext {
    /// Immutable metadata propagated with this operation. Type: `SharedAnnotations`.
    pub annotations: SharedAnnotations,
    /// Active parent span, if one exists. Type: `Option<SpanId>`.
    pub span: Option<SpanId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// Severity used to filter application log events before they reach a backend.
pub enum LogLevel {
    /// Debug-level diagnostic detail.
    Debug,
    /// Informational application event.
    Info,
    /// Warning that may need operator attention.
    Warn,
    /// The value is an expected application error.
    Error,
}
/// A message, level, and captured trace context passed to the configured logger.
pub struct LogEvent {
    /// Severity used to filter and emit this log event. Type: `LogLevel`.
    pub level: LogLevel,
    /// Text emitted by the logger. Type: `Arc<str>`.
    pub message: Arc<str>,
    /// Captured span and annotation context. Type: `TraceContext`.
    pub context: TraceContext,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for telemetry.
pub enum DiagnosticCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The policy for retaining diagnostics when a bounded diagnostic buffer is full.
pub enum DiagnosticOverflow {
    /// Discard the oldest buffered diagnostic to retain the new one.
    DiscardOldest,
    /// Discard the incoming diagnostic and retain older entries.
    DiscardNewest,
}
/// Typed diagnostic state used by telemetry.
struct DiagnosticState {
    entries: VecDeque<Defect>,
    discarded: usize,
}
#[derive(Clone)]
/// A separately retained channel for instrumentation defects that must not replace an application outcome.
pub struct Diagnostics {
    state: Arc<Mutex<DiagnosticState>>,
    capacity: DiagnosticCapacity,
    overflow: DiagnosticOverflow,
}
impl Diagnostics {
    #[must_use]
    /// Construct a `Diagnostics` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `capacity` (`DiagnosticCapacity`): The caller-supplied capacity bound.
    /// - `overflow` (`DiagnosticOverflow`): Behavior when the configured buffer capacity is full.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Retain a diagnostic defect under this buffer’s capacity and overflow policy.
    ///
    /// # Parameters
    /// - `defect` (`Defect`): The defect supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
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
    /// Wait for accepted work to settle and collect retained outcomes.
    ///
    /// # Returns
    /// Type: `Vec<Defect>`.
    pub fn drain(&self) -> Vec<Defect> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .drain(..)
            .collect()
    }
    #[must_use]
    /// Count diagnostic events discarded after the buffer reached capacity.
    ///
    /// # Returns
    /// Type: `usize`.
    pub fn discarded(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .discarded
    }
    ///
    /// # Type parameters
    /// - `T`: Stored or transferred value type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce() -> Result<T, Defect>`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Option<T>`.
    /// Returns the value when present, or `None`.
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

fn isolate_diagnostic<T>(
    ordinary: &Diagnostics,
    prepared: Option<&PreparedDiagnostics>,
    callback: impl FnOnce() -> Result<T, Defect>,
) -> Option<T> {
    match prepared {
        Some(prepared) => prepared.isolate(callback),
        None => ordinary.isolate(callback),
    }
}
/// Type alias for redact in telemetry.
type Redact = Arc<dyn Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync>;
/// Type alias for write log in telemetry.
type WriteLog = Arc<dyn Fn(&LogEvent) -> Result<(), Defect> + Send + Sync>;
/// Type alias for start span in telemetry.
type StartSpan = Arc<dyn Fn(&SpanStart) -> Result<Option<SpanId>, Defect> + Send + Sync>;
/// Type alias for finish span in telemetry.
type FinishSpan = Arc<dyn Fn(&SpanFinish) -> Result<(), Defect> + Send + Sync>;
#[derive(Clone)]
/// An environment-supplied logger that filters, redacts, and forwards events to its backend.
pub struct Logger {
    minimum: LogLevel,
    redact: Redact,
    write: WriteLog,
    diagnostics: Diagnostics,
    prepared_diagnostics: Option<PreparedDiagnostics>,
}
impl Logger {
    #[must_use]
    /// Construct a `Logger` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `minimum` (`LogLevel`): Lower bound enforced by this policy.
    /// - `diagnostics` (`Diagnostics`): Separate sink for instrumentation or provider failures.
    /// - `redact` (`impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static`): The redact supplied to this operation.
    /// - `write` (`impl Fn(&LogEvent) -> Result<(), Defect> + Send + Sync + 'static`): The write supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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
            prepared_diagnostics: None,
        }
    }

    /// Route logging instrumentation failures to the selected regional sink.
    #[must_use]
    pub fn with_prepared_diagnostics(mut self, diagnostics: PreparedDiagnostics) -> Self {
        self.prepared_diagnostics = Some(diagnostics);
        self
    }

    /// Borrow the selected regional instrumentation sink, when configured.
    #[must_use]
    pub const fn prepared_diagnostics(&self) -> Option<&PreparedDiagnostics> {
        self.prepared_diagnostics.as_ref()
    }
    #[must_use]
    /// # Returns
    /// Type: `&Diagnostics`.
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
    /// Filter and forward a log event while retaining backend defects as diagnostics.
    ///
    /// # Parameters
    /// - `event` (`LogEvent`): Typed event identity and payload submitted to the journal.
    ///
    /// # Returns
    /// Type: `()`.
    pub fn emit(&self, mut event: LogEvent) {
        if event.level < self.minimum {
            return;
        }
        if let Some(annotations) = isolate_diagnostic(
            &self.diagnostics,
            self.prepared_diagnostics.as_ref(),
            || (self.redact)(&event.context.annotations),
        )
        {
            event.context.annotations = SharedAnnotations::new(annotations);
            let _ = isolate_diagnostic(&self.diagnostics, self.prepared_diagnostics.as_ref(), || (self.write)(&event));
        }
    }
}
/// Selects the logging capability supplied by an application environment.
pub trait HasLogger {
    /// # Returns
    /// Type: `&Logger`.
    fn logger(&self) -> &Logger;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The terminal status recorded when a tracing span finishes.
pub enum SpanOutcome {
    /// The operation completed successfully.
    Succeeded,
    /// The operation completed with a failure.
    Failed,
    /// The operation ended in a defect outside its expected-error channel.
    Defected,
    /// Execution was interrupted.
    Interrupted,
    /// The owner was lost before the operation settled.
    Abandoned,
}
/// The name, parent, and context needed to begin a span.
pub struct SpanStart {
    /// The stable name identifying this value. Type: `Arc<str>`.
    pub name: Arc<str>,
    /// Identity of the parent operation or span. Type: `Option<SpanId>`.
    pub parent: Option<SpanId>,
    /// Immutable metadata propagated with this operation. Type: `SharedAnnotations`.
    pub annotations: SharedAnnotations,
    /// Time associated with this transition. Type: `Duration`.
    pub time: Duration,
}
/// The span identity and terminal status delivered to a tracing backend.
pub struct SpanFinish {
    /// Stable identity of this record. Type: `SpanId`.
    pub id: SpanId,
    /// The recorded outcome of this operation. Type: `SpanOutcome`.
    pub outcome: SpanOutcome,
    /// Whether protected cleanup reported a failure. Type: `bool`.
    pub cleanup_failed: bool,
    /// Time associated with this transition. Type: `Duration`.
    pub time: Duration,
}
#[derive(Clone)]
/// An environment-supplied span service that records begin and end events.
pub struct Tracer {
    redact: Option<Redact>,
    start: StartSpan,
    finish: FinishSpan,
    diagnostics: Diagnostics,
    prepared_diagnostics: Option<PreparedDiagnostics>,
}
impl Tracer {
    #[must_use]
    /// Construct a `Tracer` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `diagnostics` (`Diagnostics`): Separate sink for instrumentation or provider failures.
    /// - `redact` (`impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static`): The redact supplied to this operation.
    /// - `start` (`impl Fn(&SpanStart) -> Result<Option<SpanId>, Defect> + Send + Sync + 'static`): The start supplied to this operation.
    /// - `finish` (`impl Fn(&SpanFinish) -> Result<(), Defect> + Send + Sync + 'static`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        diagnostics: Diagnostics,
        redact: impl Fn(&Annotations) -> Result<Annotations, Defect> + Send + Sync + 'static,
        start: impl Fn(&SpanStart) -> Result<Option<SpanId>, Defect> + Send + Sync + 'static,
        finish: impl Fn(&SpanFinish) -> Result<(), Defect> + Send + Sync + 'static,
    ) -> Self {
        Self {
            diagnostics,
            prepared_diagnostics: None,
            redact: Some(Arc::new(redact)),
            start: Arc::new(start),
            finish: Arc::new(finish),
        }
    }
    #[must_use]
    /// # Returns
    /// Type: `&Diagnostics`.
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Use a backend start that acquires its own storage before redacting.
    /// The callback receives the original annotations and must apply its
    /// selected redaction before sending any metadata to a native sink.
    #[must_use]
    pub fn new_with_start(
        diagnostics: Diagnostics,
        start: impl Fn(&SpanStart) -> Result<Option<SpanId>, Defect> + Send + Sync + 'static,
        finish: impl Fn(&SpanFinish) -> Result<(), Defect> + Send + Sync + 'static,
    ) -> Self {
        Self {
            diagnostics,
            prepared_diagnostics: None,
            redact: None,
            start: Arc::new(start),
            finish: Arc::new(finish),
        }
    }

    /// Route span instrumentation failures to the selected regional sink.
    #[must_use]
    pub fn with_prepared_diagnostics(mut self, diagnostics: PreparedDiagnostics) -> Self {
        self.prepared_diagnostics = Some(diagnostics);
        self
    }

    /// Borrow the selected regional instrumentation sink, when configured.
    #[must_use]
    pub const fn prepared_diagnostics(&self) -> Option<&PreparedDiagnostics> {
        self.prepared_diagnostics.as_ref()
    }

    fn isolate<T>(&self, callback: impl FnOnce() -> Result<T, Defect>) -> Option<T> {
        isolate_diagnostic(&self.diagnostics, self.prepared_diagnostics.as_ref(), callback)
    }
    ///
    /// # Parameters
    /// - `start` (`SpanStart`): The start supplied to this operation.
    ///
    /// # Returns
    /// Type: `Option<SpanId>`.
    /// Returns the value when present, or `None`.
    fn begin(&self, mut start: SpanStart) -> Option<SpanId> {
        if let Some(redact) = &self.redact {
            start.annotations = SharedAnnotations::new(
                self.isolate(|| redact(&start.annotations))?,
            );
        }
        self.isolate(|| (self.start)(&start)).flatten()
    }
    ///
    /// # Parameters
    /// - `finish` (`SpanFinish`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `()`.
    fn end(&self, finish: SpanFinish) {
        let _ = self.isolate(|| (self.finish)(&finish));
    }
}
/// Selects the tracing capability supplied by an application environment.
pub trait HasTracer {
    /// # Returns
    /// Type: `&Tracer`.
    fn tracer(&self) -> &Tracer;
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Effect<A, E, R> {
    /// Return a value with the supplied annotations added to its context.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `annotations` (`Annotations`): The annotations supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Capture the current span and annotations for propagation to later work.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn trace_context() -> Self {
        Effect::ready(|_, context| Exit::Success(context.trace.clone()))
    }
}
impl<E: Send + 'static, R: HasLogger + Send + Sync + 'static> Effect<(), E, R> {
    /// Describe one log event with its level and message.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `HasLogger + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `level` (`LogLevel`): The level supplied to this operation.
    /// - `message` (`impl Into<Arc<str>>`): Text or payload transferred to the selected consumer.
    ///
    /// # Returns
    /// Type: `Self`.
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
/// Typed span guard used by telemetry.
///
/// # Type parameters
/// - `R`: Environment type. Bound or type: `HasTracer + HasClock + Send + Sync + 'static`.
struct ActiveSpan {
    tracer: Tracer,
    id: SpanId,
    started: Duration,
}

struct SpanGuard<R: HasTracer + HasClock + Send + Sync + 'static> {
    env: Arc<R>,
    live: Option<ActiveSpan>,
}
impl<R: HasTracer + HasClock + Send + Sync + 'static> SpanGuard<R> {
    ///
    /// # Parameters
    /// - `outcome` (`SpanOutcome`): Complete terminal outcome, including expected errors and cleanup failures.
    /// - `cleanup_failed` (`bool`): The cleanup failed supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    fn finish(&mut self, outcome: SpanOutcome, cleanup_failed: bool) {
        if let Some(ActiveSpan { tracer, id, started }) = self.live.take() {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                // A clock failure is diagnostic. Retirement must still reach
                // the same backend even when it cannot get a fresh time sample.
                let time = tracer.isolate(|| Ok(self.env.clock().now())).unwrap_or(started);
                tracer.end(SpanFinish { id, outcome, cleanup_failed, time });
            }));
        }
    }
}
impl<R: HasTracer + HasClock + Send + Sync + 'static> Drop for SpanGuard<R> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        self.finish(SpanOutcome::Abandoned, false);
    }
}
impl<A: Send + 'static, E: Send + 'static, R: HasTracer + HasClock + Send + Sync + 'static>
    Effect<A, E, R>
{
    /// Run this work inside a named tracing span and record its outcome.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bound: `HasTracer + HasClock + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn with_span(self, name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        Self::new(move |env, mut context, finish| {
            let live = catch_unwind(AssertUnwindSafe(|| {
                let tracer = env.tracer();
                let time = tracer.isolate(|| Ok(env.clock().now()))?;
                let id = tracer.begin(SpanStart {
                    name: Arc::clone(&name),
                    parent: context.trace.span.clone(),
                    annotations: context.trace.annotations.clone(),
                    time,
                })?;
                Some(ActiveSpan { tracer: tracer.clone(), id, started: time })
            }))
            .ok()
            .flatten();
            if let Some(span) = &live {
                context.trace.span = Some(span.id.clone());
            }
            let mut guard = SpanGuard { env: Arc::clone(&env), live };
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
