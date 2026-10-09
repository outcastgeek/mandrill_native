//! ## Generate the API documentation
//!
//! From the workspace root, run:
//!
//! ```sh
//! cargo doc -p effect -p effect-macros -p effect-native-runtime -p effect-rayon -p effect-tokio -p effect-tracing -p effect-app --no-deps
//! ```
//!
//! Open `target/doc/effect/index.html` in a browser. The same directory
//! contains the companion crate indexes. Add `--document-private-items` to
//! include internal types and helpers. Generating documentation does not run
//! tests or start a runtime.
//!
//! ## Read the API in this order
//!
//! 1. [`Effect`], [`Exit`], and [`Defect`] explain a lazy description, one
//!    execution's terminal outcome, and the distinction between an expected
//!    `E` error and a defect.
//! 2. [`effect!`], [`ResourceBinding`], [`Scope`], and [`Fiber`] explain Rust
//!    binding syntax, resource lifetime, child ownership, cancellation, and
//!    joining. The macro expands at compile time; the work runs later.
//! 3. Capability traits such as [`HasTaskSpawner`], [`HasCpuExecutor`], and
//!    [`HasClock`] show how one environment `R` supplies all runtime and
//!    application dependencies. Tokio tasks and Rayon CPU work may coexist
//!    in the same `R`; no provider is selected by the core.
//! 4. Feature modules explain their own authorities and guarantees:
//!    [`Schema`], [`persistence`], [`rpc`], [`reactivity`], [`mod@cluster`],
//!    [`mod@workflow`], [`eventlog`], and [`transactional`].
//!
//! `Effect<A, E, R>` is a reusable recipe: `A` is its success value, `E` its
//! expected application error, and `R` the environment shared with a run.
//! Calling [`Effect::run`] creates a future; polling it executes the recipe.
//! A `Fail(E)` can be handled by expected-error recovery, while defects,
//! interruption, and cleanup failures retain distinct exit channels.
//! A method's `R: Has...` bound identifies the capability it requires.
//! `Arc<R>` shares services without cloning application payloads. `Fn`
//! callbacks stored in a recipe can run again; `FnOnce` callbacks belong to
//! one run. Generic capability calls normally use static dispatch; stored
//! `dyn` callbacks use a local Rust vtable, which is never persisted.
//!
//! Rustdoc's `[src]` link on each item opens its implementation. Read a
//! feature module's contract before reading its individual types: for
//! example, an invalidation is not evidence of a durable write, and a memory
//! engine cannot promise recovery after process loss.

#![doc = include_str!("../README.md")]

extern crate self as effect;

pub use effect_macros::{
    activity, activity_definition, atom, cache, cluster, cluster_schedule, dependency_cache,
    durable_queue, entity, entity_resource, event, event_group, handled_event, hydration,
    persisted_queue, projection, reactive_bridge, reactive_derived, reactive_query,
    reactive_source, rpc_middleware, rpc_operation, rpc_protocol, rpc_stream_middleware,
    rpc_stream_operation, rpc_stream_protocol, service, singleton, span, stream, transaction,
    workflow, workflow_definition,
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
mod dependency_cache;
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
    Config, ConfigError, ConfigErrorKind, ConfigPath, ConfigPathError, ConfigSource, EffectConfig,
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
    AnyOfValue, ArrayConstraints, CheckedPattern, Codec, ConfiguredCodec, Decimal, DecimalError,
    DomainCodec, DuplicateFields, EffectSchema, IssueBudget, IssueSegment, JsonExactNumber,
    JsonRepresentation, JsonSchemaDocument, JsonValue, JsonWriter, JsonWriterPolicy,
    LiteralEnumValue, MultiplePatternPolicy, NativeField, NativeFields, NativeNil,
    NativeRecordBuilder, NumericConstraint, ObjectConstraints, ObjectRepresentation, ParseContext,
    ParseLimits, ParsePolicy, PatternPolicy, PatternProperty, PrefixElement, PrefixValue,
    PrimitiveShape, PureCodec, RecordBuilder, RecordField, RecordFields, RecordNil,
    RepresentationError, RepresentationLimits, RepresentationPrecision, Schema, SchemaAst,
    SchemaErrorMode, SchemaIssue, SchemaIssueKind, SchemaPolicy, SchemaProperty, SchemaReference,
    SchemaResource, SchemaShape, SequenceRepresentation, StoredCleanup, StoredDefect,
    StoredFinalization, StoredOutcome, StoredPrimary, StringConstraints, StringLengthUnit,
    StringPattern, TaggedEnumBuilder, UnknownFields, UnknownLiteralPolicy, ValueKind,
    export_json_schema, json_all_of, json_any_of, json_array, json_array_constraints, json_bool,
    json_bytes, json_decimal, json_exact_number, json_f64, json_i64, json_literal,
    json_literal_enum, json_null, json_nullable, json_object_constraints, json_one_of, json_option,
    json_prefix_rest, json_result, json_stable_key, json_string, json_string_constraints,
    json_string_pattern, json_tuple2, json_u64, json_utc_millis, native_array, native_map,
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
    pub use crate::internal::workflow::{
        Bindable, Workflow, bind, defer, defer_callback0, defer_callback1, defer_callback2, once,
        reusable, success,
    };
}

pub use stream::{Chunk, Stream, StreamBufferOptions, StreamContext, StreamReader};
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
    CallbackContention, CallbackDrop, CallbackOverflow, CallbackPubSub, CallbackPubSubOptions,
    CallbackPublish, PubSub, PubSubError, PubSubOptions, PubSubOverflow, PubSubShutdown,
    PublishReport, ReplayPolicy, Subscription, SubscriptionEvent,
};
pub use subscription_ref::{SubscriptionRef, SubscriptionSnapshot};
pub use synchronized_ref::SynchronizedRef;

pub use primary_key::{KeyDomain, KeyError, KeyNamespace, KeyParts, KeySegment, StableKey};
pub use request::{Request, RequestGroup, RequestKey};
pub use request_resolver::{HasRequestScope, RequestResolver, RequestResolverError, RequestScope};

pub use cache::{
    Cache, CacheAdmission, CacheCapacity, CacheConfig, CacheLookup, CacheObservation, CacheOptions,
    CacheTtl, SharedValue,
};
pub use dependency_cache::{
    DependencyCache, DependencyCacheError, DependencyCachePolicy, DependencyValue,
};

pub use weak_cache::{CachePartition, WeakCache, WeakCacheValue};

/// Typed native-service operation and resource mapping over the application environment.
mod service;
#[doc(hidden)]
pub use service::{ServiceBindable, ServiceBody, ServiceContext};
pub use service::{ServiceOperation, ServiceResource};
