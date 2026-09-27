#![doc = include_str!("../README.md")]

extern crate self as effect;

mod admission;
mod cache;
mod cancellation;
mod clock;
mod console;
mod defect;
mod deferred;
mod execution;
mod exit;
mod fiber;
pub mod http;
mod internal;
mod job;
mod latch;
mod layer;
mod owned_run;
mod parallel;
mod pool;
#[path = "effect.rs"]
mod program;
mod queue;
mod random;
mod reference;
mod resource;
mod schedule;
mod schema;
mod scope;
mod stream;
mod telemetry;
mod weak_cache;
pub mod websocket;

pub use cancellation::Cancellation;
pub use clock::{Clock, ClockError, ClockFuture, ClockTick, HasClock, TestClock, TestClockAdvance};
pub use console::{Console, ConsoleChannel, HasConsole, StandardConsole};
pub use defect::{Defect, DefectKind, LibraryDefect};
pub use deferred::Deferred;
pub use effect_macros::effect;
pub use execution::{HasCancellation, HasTaskSpawner, ScheduledTask, TaskSpawner};
pub use exit::{CleanupFailure, Exit, Finalization, PrimaryOutcome, PrimaryOutcomeRef};
pub use fiber::{Fiber, FiberStatus, OutcomeStatus, SharedExit};
pub use job::{BlockingExecutor, CpuExecutor, HasBlockingExecutor, HasCpuExecutor, ScheduledJob};
pub use latch::Latch;
pub use layer::Layer;
pub use owned_run::{RunDriver, RunObserver};
pub use program::Effect;
pub use queue::{AsyncQueue, QueueCapacity, QueueError, QueueShutdown};
pub use random::{HasRandom, Random, RandomFn, UnitInterval};
pub use reference::AtomicRef;
pub use resource::{Release, ResourceBinding, ResourceValue};
pub use schedule::{
    CombineSampling, Deterministic, GrowthFactor, NonnegativeFactor, Randomized, Schedule,
    ScheduleDecision, ScheduleSampling,
};
pub use schema::{JsonValue, Schema};
pub use scope::{Scope, ScopeCloseStatus};

/// Safe construction bridge for procedural macro expansion.
#[doc(hidden)]
pub mod __private {
    pub use crate::Exit;
    pub use crate::internal::macro_future::{AsyncBindable, MacroContext, async_workflow};
    pub use crate::internal::workflow::{Bindable, Workflow, bind, defer, reusable, success};
}

pub use stream::{Chunk, Stream, StreamReader};
pub use telemetry::{
    Annotations, DiagnosticCapacity, DiagnosticOverflow, Diagnostics, HasLogger, HasTracer,
    LogEvent, LogLevel, Logger, SharedAnnotations, SpanFinish, SpanId, SpanOutcome, SpanStart,
    TraceContext, Tracer,
};

pub use admission::{
    AdmissionCapacity, AdmissionError, AdmissionPermit, AdmittedJob, AdmittedTask,
    ProviderAdmission, ProviderDrain,
};

pub use http::{
    FetchFailure, HasHttpTransport, HttpBody, HttpHeaders, HttpMethod, HttpRequest, HttpResponse,
    HttpStatus, HttpTransport, RequestOptions, TransportFuture,
};

pub use websocket::{
    HasWebSocketTransport, InvalidWebSocketMessage, WebSocket, WebSocketCloseInfo,
    WebSocketConnectOptions, WebSocketConnection, WebSocketError, WebSocketFragmentKind,
    WebSocketFrame, WebSocketLimits, WebSocketMessage, WebSocketOperation, WebSocketReceive,
    WebSocketTransport,
};

pub use pool::{Pool, PoolCapacity, PoolEndStatus, PoolLease};

pub use cache::{Cache, CacheCapacity, CacheConfig, SharedValue};

pub use weak_cache::{CachePartition, WeakCache, WeakCacheValue};
