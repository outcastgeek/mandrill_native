#![doc = include_str!("../README.md")]

extern crate self as effect;

pub use effect_macros::{
    activity, activity_definition, entity, event, event_group, projection, reactive_bridge,
    reactive_derived, reactive_query, reactive_source, rpc_middleware, rpc_operation, rpc_protocol,
    rpc_stream_middleware, rpc_stream_operation, rpc_stream_protocol, transaction, workflow,
    workflow_definition,
};

mod admission;
mod cache;
mod cancellation;
mod clock;
pub mod cluster;
mod config;
mod console;
mod cron;
mod defect;
mod deferred;
mod duration;
pub mod encoding;
pub mod eventlog;
mod execution;
mod exit;
mod fiber;
pub mod http;
mod internal;
mod job;
mod latch;
mod layer;
mod metric;
mod owned_run;
mod parallel;
mod partitioned_semaphore;
pub mod persistence;
mod pool;
mod primary_key;
#[path = "effect.rs"]
mod program;
mod pub_sub;
mod queue;
mod random;
pub mod reactivity;
mod reference;
mod request;
mod request_resolver;
mod resource;
pub mod rpc;
mod schedule;
mod schema;
mod scope;
mod semaphore;
mod sink;
mod stream;
mod subscription_ref;
mod synchronized_ref;
mod telemetry;
pub mod transactional;
mod weak_cache;
pub mod websocket;
pub mod workflow;

pub use cancellation::Cancellation;
pub use clock::{Clock, ClockError, ClockFuture, ClockTick, HasClock, TestClock, TestClockAdvance};
pub use config::{
    Config, ConfigError, ConfigErrorKind, ConfigPath, ConfigPathError, ConfigSource,
    HasConfigSource, MemoryConfigSource,
};
pub use console::{Console, ConsoleChannel, HasConsole, StandardConsole};
pub use cron::{
    CalendarOccurrence, CalendarTimeError, CalendarTransitionPolicy, CalendarZone, CivilDateTime,
    Cron, CronBudgetKind, CronEvalError, CronEvalErrorKind, CronFieldKind, CronParseError,
    CronParseErrorKind, CronSearchBudget, DayMatch, FixedOffsetZone, GapPolicy, LocalResolution,
    MissedOccurrencePolicy, OccurrencePhase, OverlapPolicy,
};
pub use defect::{Defect, DefectKind, LibraryDefect};
pub use deferred::Deferred;
pub use duration::{Deadline, HasWallClock, TestWallClock, TimeError, UtcMillis, WallClock};
pub use effect_macros::{effect, persistable, schema, schema_enum};
pub use execution::{HasCancellation, HasTaskSpawner, ScheduledTask, TaskSpawner};
pub use exit::{CleanupFailure, Exit, Finalization, PrimaryOutcome, PrimaryOutcomeRef};
pub use fiber::{Fiber, FiberStatus, OutcomeStatus, SharedExit};
pub use job::{BlockingExecutor, CpuExecutor, HasBlockingExecutor, HasCpuExecutor, ScheduledJob};
pub use latch::Latch;
pub use layer::Layer;
pub use metric::{
    GaugeValue, HasMetricRecorder, MemoryMetricRecorder, MetricAggregate, MetricCapacity,
    MetricDescriptor, MetricDomain, MetricError, MetricKind, MetricLabels, MetricName,
    MetricRecorder, MetricSample, MetricSeries, MetricValue,
};
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
pub use schema::{
    Codec, Decimal, DecimalError, DuplicateFields, EffectSchema, IssueSegment, JsonSchemaDocument,
    JsonValue, ParseLimits, ParsePolicy, PrimitiveShape, PureCodec, RecordBuilder, RecordField,
    RecordFields, RecordNil, RepresentationError, RepresentationLimits, RepresentationPrecision,
    Schema, SchemaAst, SchemaErrorMode, SchemaIssue, SchemaIssueKind, SchemaProperty,
    SchemaReference, SchemaResource, SchemaShape, StoredCleanup, StoredDefect, StoredFinalization,
    StoredOutcome, StoredPrimary, StringLengthUnit, StringPattern, TaggedEnumBuilder,
    UnknownFields, ValueKind, export_json_schema, json_array, json_bool, json_bytes, json_decimal,
    json_f64, json_i64, json_literal, json_null, json_nullable, json_option, json_result,
    json_stable_key, json_string, json_string_pattern, json_tuple2, json_u64, json_utc_millis,
    stored_outcome_codec, value_kind,
};
pub use scope::{Scope, ScopeCloseStatus};
pub use semaphore::{Semaphore, SemaphoreError, SemaphorePermit};
pub use sink::Sink;

/// Safe construction bridge for procedural macro expansion.
#[doc(hidden)]
pub mod __private {
    pub use crate::Exit;
    pub use crate::internal::macro_future::{AsyncBindable, MacroContext, async_workflow};
    pub use crate::internal::workflow::{Bindable, Workflow, bind, defer, reusable, success};
}

pub use stream::{Chunk, Stream, StreamBufferOptions, StreamReader};
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

pub use partitioned_semaphore::PartitionedSemaphore;
pub use pool::{Pool, PoolCapacity, PoolEndStatus, PoolLease};

pub use pub_sub::{
    PubSub, PubSubError, PubSubOptions, PubSubOverflow, PubSubShutdown, PublishReport,
    ReplayPolicy, Subscription, SubscriptionEvent,
};
pub use subscription_ref::{SubscriptionRef, SubscriptionSnapshot};
pub use synchronized_ref::SynchronizedRef;

pub use primary_key::{KeyDomain, KeyError, KeyNamespace, KeyParts, KeySegment, StableKey};
pub use request::{Request, RequestGroup, RequestKey};
pub use request_resolver::{HasRequestScope, RequestResolver, RequestResolverError, RequestScope};

pub use cache::{Cache, CacheCapacity, CacheConfig, SharedValue};

pub use weak_cache::{CachePartition, WeakCache, WeakCacheValue};
