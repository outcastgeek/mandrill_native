//! Zig Effect: lazy typed work with environment-owned runtime dependencies.
//! `Effect(A, E, R)` describes work producing `A`, failing with `E`, and
//! requiring caller-defined environment `R`. `R` may supply an allocator,
//! `std.Io`, and selected async or parallel engines. `run` and `runAsync`
//! expose all terminal outcomes through `Exit(A, E)`; callers switch on its
//! success, typed failure, defect, interruption, and cleanup variants.
//! Builder modules define schemas, persistence, RPC, reactivity, cluster,
//! workflow, event log, and transactional compositions at comptime.
//! Zig expresses these generic contracts through `comptime` parameters and
//! checked function signatures, rather than a named trait declaration.
//! Runtime-substitutable services expose explicit function-pointer tables;
//! callers choose implementations and keep their provider state alive for
//! the lifetime documented by each capability.
//!
//! ## Start with one effect
//! `A` is the success type, `E` is the expected application-error type, and
//! `R` is the caller's environment type. Constructing an effect describes work;
//! `run(&env, program)` executes it. The environment is supplied once at that
//! boundary. In particular, its allocator and I/O are not separate arguments
//! to each operation. This function shows the types and execution boundary:
//! ```zig
//! const std = @import("std");
//! const fx = @import("effect");
//! const AppError = enum { invalid };
//! const Env = struct { allocator: std.mem.Allocator, io: std.Io };
//!
//! fn execute(env: *const Env, value: u32) fx.Exit(u32, AppError) {
//!     const program = fx.Effect(u32, AppError, Env).succeed(value);
//!     return fx.run(env, program);
//! }
//! ```
//! `execute` returns `Exit(u32, AppError)` directly. Switch on `.success` for
//! `A`, `.fail` for `E`, `.interrupted` for cancellation, `.die` for a runtime
//! defect, and `.finalization_failed` for release failures that retain the
//! original outcome. Effect failures do not need Zig `try`/`catch`. Call
//! `Exit.deinit` for an owning result; its documented ownership contract
//! distinguishes library-managed payloads from ordinary application values.
//!
//! ## Compose and execute
//! `EffectBuilder(E, R)` supplies comptime-checked `bind`, loops, finalizers,
//! and scoped resource binding. `Effect.map` changes a success value;
//! `flatMap` chooses the next effect. `runAsync` returns an `AsyncRun` handle:
//! join it for an `Exit` or deinitialize it to cancel and drain. A `Scope`
//! owns resources and child fibers until cleanup. Select `Executor` providers
//! through `R` for async, CPU, or blocking work; the provider's advertised
//! capabilities must cover the requested work. Captures, `R`, and providers
//! must live until all accepted work is joined and scopes are closed.
//!
//! ## Find a feature
//! Begin with `Schema`/`SchemaBuilder` for typed representation, then use the
//! feature family you need: `Persistable`/`PersistedQueue` for persistence,
//! `RpcSession`/`RpcProtocolBuilder` for RPC, `AtomBuilder` for reactivity,
//! `ClusterEntityBuilder` for clustering, `WorkflowBuilder` for workflows,
//! `EventBuilder` for event logs, or `TransactionBuilder` for transactional
//! memory. `Stream`, `Schedule`, `Clock`, `Logger`, and `Tracer` provide the
//! shared streaming, scheduling, time, and observability contracts. Open a
//! declaration in the generated docs to see its parameters, generic types,
//! return value, ownership, and provider/vtable requirements.
//!
//! Generate this API reference with `zig build docs` from the package
//! directory, then open `zig-out/docs/index.html`. The runnable consumer is
//! `examples/port.zig`. Open the named declarations above to see their
//! signatures, provider contracts, failure variants, and ownership rules.
/// Unexpected failure distinct from the application error type.
pub const Defect = @import("outcome.zig").Defect;
/// Terminal success, typed failure, defect, interruption, or cleanup failure.
pub const Exit = @import("outcome.zig").Exit;
/// Pure two-channel result; effect execution returns `Exit` instead.
pub const Result = @import("outcome.zig").Result;
/// Owns the array storage only. Element ownership follows the producing operation's contract; resources
/// belong in scopes rather than raw arrays.
pub const OwnedSlice = @import("outcome.zig").OwnedSlice;
/// Primary success, typed failure, defect, or interruption without finalizer details.
pub const PrimaryOutcome = @import("outcome.zig").PrimaryOutcome;
/// Typed release failure, release defect, or interrupted finalizer.
pub const CleanupFailure = @import("outcome.zig").CleanupFailure;
/// Shared cancellation token for an Effect branch; request is observable by registered child operations.
pub const Cancellation = @import("internal/runtime.zig").Cancellation;
/// Lazy program specialized by success `A`, error `E`, and environment `R`.
pub const Effect = @import("effect.zig").Effect;
/// Contextual role adapters returning the exact existing Effect type.
pub const Dsl = @import("dsl.zig").Dsl;
/// Source-driven inference, typed captures, selectors, and pipeline stages.
pub const dsl = @import("dsl.zig");
/// Comptime sequencing helpers for effects sharing `E` and `R`.
pub const EffectBuilder = @import("effect.zig").EffectBuilder;
/// Scoped resource value paired with its release contract.
pub const ResourceValue = @import("effect.zig").ResourceValue;
/// Typed acquisition/release binding consumed by `EffectBuilder.using`.
pub const ResourceBinding = @import("effect.zig").ResourceBinding;
/// Build an acquisition/release binding for scoped use.
pub const resource = @import("effect.zig").resource;
/// Scoped service construction and provision.
pub const Layer = @import("effect.zig").Layer;
/// Typed, lazy stream of effectful elements.
pub const Stream = @import("effect.zig").Stream;
/// Scoped Stream/PubSub bridge. The caller owns the bounded bus and chooses its capacity, replay window
/// and overload policy in the same environment. Subscribe before starting producers when every item
/// must be observed: replay is an explicit bus policy, not an implicit stream buffer. With
/// backpressure, publishers wait only while an active subscriber protects the oldest item; cancellation
/// interrupts that wait. A closed bus lets a reader drain retained items before `end`. Publisher
/// failures stay on the returned producer Effect, so its owner must join that Effect as well as
/// consuming the stream; an end marker alone never means the producer succeeded.
pub const StreamPubSub = @import("stream_pubsub.zig");
/// Typed consumer that folds a stream into one result.
pub const Sink = @import("sink.zig").Sink;
/// Inferred stateful pure/effectful result folds on the ordinary Effect kernel.
pub const SinkBuilder = @import("sink_builder.zig");
/// Bounded, incremental NDJSON records. Each returned record owns its bytes; the transport may release
/// a fed chunk immediately. The JSON value itself is decoded by the caller's typed Schema codec.
pub const Ndjson = @import("encoding/ndjson.zig");
/// Versioned binary envelope for a schema codec's binary representation. Wire layout (v1): version:u8,
/// schema-name-length:u32 big endian, payload-length:u32 big endian, UTF-8 schema name, codec payload.
/// The schema codec owns the payload grammar; this module owns identity, framing and limits shared by
/// event replication and stored outcomes.
pub const SchemaBinary = @import("encoding/schema_binary.zig");
/// Schema-derived binary payloads for explicitly supported Zig value shapes. The comptime type is the
/// representation schema: fields use declaration order; scalars are fixed-width big endian; optionals
/// carry a presence byte; slices carry a u32 count; enum tags use u32; arrays have their type-level
/// fixed count. A caller-supplied stable schema name identifies that exact shape on the wire. Changing
/// a field type/order requires a new name or a caller migration.
pub const TypedBinary = @import("encoding/typed_binary.zig");
/// Scoped pull reader for a typed Stream.
pub const StreamReader = @import("effect.zig").StreamReader;
/// Typed stream chunk or chunk container.
pub const Chunk = @import("effect.zig").Chunk;
/// Scope for resource finalizers and child lifetimes.
pub const Scope = @import("scope.zig").Scope;
/// Native command boundary. Construction starts the command inline. A pending command owns all provider
/// resources until its single join owner completes.
pub const Async = @import("async.zig").Async;
/// Handle returned by `runAsync`; join or deinitialize pending work.
pub const AsyncRun = @import("effect.zig").AsyncRun;
/// Structured child computation.
pub const Fiber = @import("effect.zig").Fiber;
/// Start an effect with the caller’s environment and return its async handle.
pub const runAsync = @import("effect.zig").runAsync;
/// Caller-selected execution engine capability for concurrency and parallelism.
pub const Executor = @import("executor.zig").Executor;
/// Consumer-owned adapter. Capabilities describe the supplied implementation, not std.Io in general: an
/// event loop cannot claim independent CPU workers.
pub const NativeIoExecutor = @import("executor.zig").NativeIo;
/// Execution lane requested by an Effect. Providers must only accept lanes covered by their advertised
/// `Capabilities`.
pub const ExecutionKind = @import("executor.zig").Kind;
/// One-shot completion primitive carrying typed success or failure.
pub const Deferred = @import("primitives.zig").Deferred;
/// One-shot synchronization gate with a typed failure channel.
pub const Latch = @import("primitives.zig").Latch;
/// Concurrent mutable reference with typed operations.
pub const Ref = @import("primitives.zig").Ref;
/// Bounded or unbounded typed work queue primitive.
pub const Queue = @import("primitives.zig").Queue;
/// `QueueError` cases: `shutdown`.
pub const QueueError = @import("primitives.zig").QueueError;
/// I/O-aware permit gate with a caller-selected limit and explicit release.
pub const Semaphore = @import("semaphore.zig").Semaphore;
/// Typed permit-gate error for invalid capacity, shutdown, or release.
pub const SemaphoreError = @import("semaphore.zig").Error;
/// Partitions exist only while an acquisition or waiter owns them. Unrelated keys use independent
/// semaphores; the map gate protects only lifecycle and never remains held while a permit waits or user
/// work runs.
pub const PartitionedSemaphore = @import("semaphore.zig").PartitionedSemaphore;
/// Typed key-validation or semaphore failure from a keyed permit gate.
pub const PartitionedSemaphoreError = @import("semaphore.zig").PartitionError;
/// Typed publication/subscription bus with explicit capacity policy.
pub const PubSub = @import("pub_sub.zig").PubSub;
/// `Overflow` cases: `backpressure`, `drop_oldest`, `drop_newest`, `reject`.
pub const PubSubOverflow = @import("pub_sub.zig").Overflow;
/// Reference that publishes committed value changes to subscribers.
pub const SubscriptionRef = @import("subscription_ref.zig").SubscriptionRef;
/// Typed recurrence decisions and schedule composition over injected time.
pub const Schedule = @import("schedule.zig");
/// Typed schema decoding and encoding through Effects.
pub const Schema = @import("schema.zig").Schema;
/// Runtime-selected clock. Its borrowed `userdata` and `vtable` must remain valid while effects using
/// this service are running. `SleepResult` keeps interruption separate from the successful sleep
/// outcome.
pub const Clock = @import("services.zig").Clock;
/// Runtime-selected random source. `sample_fn` receives borrowed `userdata` and returns a sample as
/// `f64`; the provider chooses its implementation.
pub const Random = @import("services.zig").Random;
/// Provider-backed logger with level filtering, redaction, and defect reporting. The provider context
/// must outlive each `write_fn` call.
pub const Logger = @import("services.zig").Logger;
/// Provider-backed tracing lifecycle. Its vtable must implement begin, end, and abandon so a failed
/// export cannot orphan a native span.
pub const Tracer = @import("services.zig").Tracer;
/// `LogLevel` cases: `debug`, `info`, `warning`, `err`.
pub const LogLevel = @import("services.zig").LogLevel;
/// Key/value diagnostic field carried into logging and tracing callbacks.
pub const Annotation = @import("services.zig").Annotation;
/// `Redactor` is a callback interface with `userdata: ?*anyopaque`, `redact_fn: *const fn (?*anyopaque,
/// Annotation) ?Annotation`.
pub const Redactor = @import("services.zig").Redactor;
/// Consumer-supplied defect reporting hook. Messages passed to `report_fn` are borrowed; the sink
/// copies text it wants to retain.
pub const DiagnosticSink = @import("services.zig").DiagnosticSink;
/// Run a lazy effect with its `R` environment and return `Exit(A, E)`.
pub const run = @import("effect.zig").run;

test {
    _ = @import("outcome.zig");
    _ = @import("effect.zig");
    _ = @import("proofs.zig");
}

/// Keyed cache with caller-selected storage and expiry policy.
pub const Cache = @import("cache.zig").Cache;
/// Capacity and expiry configuration for Cache.
pub const CacheConfig = @import("cache.zig").Config;
/// Hash, equality, and retention contract for cache keys.
pub const CacheKeyPolicy = @import("cache.zig").KeyPolicy;
/// Effect wrapper for shared, scoped cache loading.
pub const EffectCache = @import("effect.zig").EffectCache;
/// The pool owns resources. Leases borrow them exclusively until their first disposition completes.
/// Scope finalization also destroys lease bookkeeping.
pub const Pool = @import("pool.zig").Pool;
/// Typed config record; release owned state with `deinit`.
pub const PoolConfig = @import("pool.zig").Config;
/// Effect wrapper for scoped pool acquisition and release.
pub const EffectPool = @import("effect.zig").EffectPool;
/// Managed HTTP requests over a caller-provided transport and I/O environment.
pub const Http = @import("http.zig");
/// Managed HTTP request Effect over a caller-supplied transport.
pub const Fetch = @import("effect.zig").Fetch;
/// Managed WebSocket session operations over a caller-provided transport.
pub const WebSocket = @import("websocket.zig");
/// Managed WebSocket Effects over a caller-supplied transport.
pub const EffectWebSocket = @import("effect.zig").EffectWebSocket;
/// Effect operations for the Deferred completion primitive.
pub const EffectDeferred = @import("effect.zig").EffectDeferred;
/// Effect operations for the Latch synchronization primitive.
pub const EffectLatch = @import("effect.zig").EffectLatch;
/// Effect operations for the concurrent Ref primitive.
pub const EffectRef = @import("effect.zig").EffectRef;
/// Effect operations for the typed Queue primitive.
pub const EffectQueue = @import("effect.zig").EffectQueue;
/// Caller-supplied console capability. Both hooks take borrowed message bytes and return typed `Exit`
/// values rather than throwing Zig errors.
pub const Console = @import("services.zig").Console;
/// Console adapter using caller-provided output and error writers.
pub const NativeConsole = @import("services.zig").NativeConsole;
/// Effect operations backed by the environment’s Console capability.
pub const EffectConsole = @import("effect.zig").EffectConsole;
/// An explicitly advanced clock for deterministic schedulers and applications. Sleeping native
/// operations remain cancellable through the injected I/O.
pub const ManualClock = @import("manual_clock.zig").ManualClock;
/// Weakly retained cache entries with caller-controlled cleanup.
pub const WeakCache = @import("weak_cache.zig").WeakCache;
/// Weak cache adapters and retention support operations.
pub const weak_cache_support = @import("weak_cache.zig").support;

/// Borrowed callback state and its optional cancellation token. An accepted executor owns the job until
/// its handle has been joined and destroyed.
pub const ExecutionJob = @import("executor.zig").Job;
/// Which independent, CPU, and blocking lanes this provider actually supplies.
pub const ExecutionCapabilities = @import("executor.zig").Capabilities;
/// Trace span identifier carried across log and tracing callbacks.
pub const SpanId = @import("services.zig").SpanId;
/// Borrowed input to `Tracer.begin`: parent, span name, annotations, and optional timestamp.
pub const SpanStart = @import("services.zig").SpanStart;
/// Input to `Tracer.end`: span id, primary outcome, cleanup status, and optional timestamp.
pub const SpanFinish = @import("services.zig").SpanFinish;
/// Primary span outcome: success, failure, defect, or interruption.
pub const SpanOutcome = @import("services.zig").SpanOutcome;
/// Borrowed log payload containing severity, text, redacted annotations, and optional span id.
pub const LogRecord = @import("services.zig").LogRecord;
/// A provider iterates redacted annotations. This avoids allocating a second annotation map merely to
/// export it. Returned text is borrowed through export.
pub const Annotations = @import("services.zig").Annotations;
/// Decoded value retaining the source document and its ownership.
pub const Parsed = @import("schema.zig").Parsed;
/// Stable, namespaced identities shared by persisted and routed work. The wire form is versioned, typed
/// and length delimited. It is never a process hash or an ambiguous concatenation of application
/// strings.
pub const PrimaryKey = @import("primary_key.zig");
/// Owned, structured codec failures. Field names in paths are copied so an issue can outlive the JSON
/// document or frame from which it was produced.
pub const SchemaIssue = @import("schema/schema_issue.zig");
/// Immutable structural metadata. The AST describes representations and constraints; it is not a
/// witness that can construct an arbitrary Zig type. Typed constructors, projections and codecs supply
/// that separate proof.
pub const SchemaAst = @import("schema/schema_ast.zig");
/// Comptime record schema builder and field descriptors.
pub const SchemaBuilder = @import("schema/schema_builder.zig");
/// A record is unavailable until its fields and policy are supplied. Every field constructor itself
/// requires the matching codec and, for defaults, the typed default callback before this stage can
/// finish. Staged record builder. `fields` must pair every Zig field with a typed codec, and `policy`
/// must be supplied before `build` exposes the schema. Allocating decode/encode operations take their
/// allocator from `R`.
pub const SchemaRecordBuilder = @import("schema/schema_builder.zig").Builder;
/// Named tagged unions with compile-time checked Zig constructors and tags.
pub const VariantSchema = @import("schema/variant_schema.zig");
/// Bounded typed JSON arrays composed from pure or structural child codecs.
pub const ArraySchema = @import("schema/array_schema.zig");
/// Option is an explicit tagged representation, distinct from JSON null and from an absent record
/// field.
pub const OptionCodec = @import("schema/option_codec.zig");
/// Fixed-arity tuples compose pure codecs without allocating or erasing any element's decoded,
/// represented, or ownership type.
pub const TupleCodec = @import("schema/tuple_codec.zig");
/// Lazy boxed structural composition for recursive Zig application types. A schema factory is evaluated
/// at comptime only when an operation uses it; the box breaks the otherwise infinite type-level
/// expansion.
pub const RecursiveSchema = @import("schema/recursive_schema.zig");
/// Bounded, lazy preparation of dynamic schema graphs. Static typed codecs do not enter this path;
/// runtime definitions are checked once per definition set.
pub const SchemaPreparation = @import("schema/internal/preparation.zig");
/// Encoded-shape metadata attached to codecs rather than inferred from Zig application types. Only
/// known built-in codecs get built-in witnesses.
pub const SchemaRepresentation = @import("schema/internal/representation.zig");
/// Bounded JSON Schema export for declared encoded representations. This is metadata translation, not a
/// decoder or a reflection-based codec fallback. Exact definitions reject policies the dialect cannot
/// express; conservative definitions may omit only restrictions that would otherwise overconstrain.
pub const JsonSchema = @import("schema/json_schema.zig");
/// Bidirectional codec interface specialized at comptime. The decoder and encoder are statically typed
/// functions; each constructs a lazy Effect, and decoding and encoding may require different caller
/// environments.
pub const Codec = @import("schema/codec.zig").Codec;
/// Pure callbacks never acquire a runtime service or introduce an implicit native error channel. They
/// can still report structured root issues.
pub const PureCodec = @import("schema/codec.zig").PureCodec;
/// Primitive JSON codecs. Structural codecs compose these without changing the distinction between
/// decoded application values and encoded JSON data.
pub const JsonCodec = @import("schema/json_codec.zig");
/// Validated decimal text representation without lossy floating-point conversion.
pub const DecimalText = @import("schema/numeric_codec.zig").DecimalText;
/// Rejects signs on nonnegative values, leading zeroes and any other spelling that does not round-trip
/// to the one stable decimal representation.
pub const IntegerCodec = @import("schema/numeric_codec.zig").integer;
/// Fixed-scale decimal arithmetic keeps the exact signed coefficient. Scale is part of the Zig type and
/// therefore part of the schema identity.
pub const FixedDecimal = @import("schema/numeric_codec.zig").FixedDecimal;
/// Fixed-scale decimal text representation.
pub const FixedDecimalText = @import("schema/numeric_codec.zig").FixedDecimalText;
/// Codec for a fixed-scale decimal representation.
pub const FixedDecimalCodec = @import("schema/numeric_codec.zig").fixedDecimal;
/// Wall-clock instants are encoded as canonical signed epoch nanoseconds. This never substitutes a
/// process-local monotonic timestamp for durable data.
pub const WallInstantCodec = @import("schema/numeric_codec.zig").wall_instant;
/// Owned bytes containing validated Base64 text; release them with `deinit`.
pub const Base64Text = @import("schema/base64_codec.zig").Base64Text;
/// Codec for base64-encoded byte values.
pub const Base64Codec = @import("schema/base64_codec.zig").codec;
/// Versioned, data-only Exit representation. Payload codecs supply the bytes for typed successes and
/// failures; this layer preserves terminal tags, defect diagnostics, interruption and ordered cleanup
/// failures. It never persists process pointers, functions, scopes or live runtime objects.
pub const StoredOutcome = @import("schema/outcome_codec.zig");
/// Codec for typed success/failure/defect/interruption outcomes.
pub const TypedOutcomeCodec = @import("schema/typed_outcome_codec.zig").PayloadCodec;
/// Decoded durable terminal outcome with owned payloads.
pub const DecodedStoredOutcome = @import("schema/typed_outcome_codec.zig").DecodedExit;
/// Typed persistence error channel combining operation and provider failures.
pub const PersistenceError = @import("persistence/persistence_error.zig").Error;
/// Typed RPC failure channel combining protocol, middleware, session, and domain errors.
pub const RpcError = @import("rpc/rpc_error.zig").Error;
/// Typed cluster operation and provider failure channel.
pub const ClusterError = @import("cluster/cluster_error.zig").Error;
/// `OperationError` is a tagged choice.
pub const WorkflowOperationError = @import("workflow/workflow_error.zig").OperationError;
/// Typed completed workflow outcome stored by the engine.
pub const WorkflowCompleted = @import("workflow/workflow_error.zig").Completed;
/// Typed event-log operation and provider failure channel.
pub const EventLogError = @import("eventlog/event_log_error.zig").Error;
/// Typed transaction operation and domain failure channel.
pub const TransactionError = @import("transactional/transaction_error.zig").Error;
/// A domain may contain many cells of T. Distinct domains remain isolated, including when their values
/// happen to have the same Zig type.
pub const MemoryTransaction = @import("transactional/memory_transaction.zig").Domain;
/// In-memory transaction success, failure, or retry decision.
pub const TransactionDecision = @import("transactional/memory_transaction.zig").Decision;
/// Completed asynchronous operation outcome and interruption status.
pub const AsyncResult = @import("reactivity/async_result.zig").AsyncResult;
/// Revisioned invalidation subscriptions with caller-owned I/O and allocator.
pub const InvalidationRegistry = @import("reactivity/invalidation_registry.zig").Registry;
/// Typed invalidation error from the reactive registry.
pub const InvalidationError = @import("reactivity/invalidation_registry.zig").Error;
/// Subscription notice: a changed revision or a requirement to resynchronize.
pub const InvalidationNotice = @import("reactivity/invalidation_registry.zig").Notice;
/// Observable query state exposed to reactive subscribers.
pub const ReactiveQueryState = @import("reactivity/query.zig").State;
/// Typed failure channel for reactive query evaluation.
pub const ReactiveQueryError = @import("reactivity/query.zig").Error;
/// A query is a compile-time pairing of its evaluator and selected state. It uses the caller's R for
/// all engines, allocator, I/O and services.
pub const ReactiveQueryRefresh = @import("reactivity/query.zig").refresh;
/// Retains the previous result while a claimed refresh runs. The injected clock makes expiry
/// deterministic, and the existing scoped permit makes a refresh waiting behind another evaluation
/// cancellable.
pub const ReactiveQueryRefreshIfStale = @import("reactivity/query.zig").refreshIfStale;
/// The caller runs this Effect in a scoped fiber and joins it on disposal. Notifications received while
/// evaluating are coalesced by refresh's dirty bit; an event gap forces the same conservative refresh
/// path.
pub const ReactiveQueryPump = @import("reactivity/query.zig").pump;
/// Deadline that either never expires or becomes due at a wall-time instant.
pub const Deadline = @import("deadline.zig").Deadline;
/// Signed Unix nanosecond timestamp used by injected clock and deadline capabilities.
pub const WallInstant = @import("deadline.zig").WallInstant;
/// Retained absolute deadlines use an injected wall clock. Local waiting continues to use the caller's
/// monotonic clock/Io; replay never reads an ambient process clock or reconstructs a monotonic
/// timestamp as wall time.
pub const WallClock = @import("deadline.zig").WallClock;
/// Neutral metric descriptions and recording. Names, units and observation kinds are definition data;
/// exporters and backend configuration stay outside the core package.
pub const Metric = @import("metric.zig");
/// Schema-backed configuration starts from a provider-neutral byte source. Source failures, missing
/// entries and schema-invalid entries are distinct; secret values are never embedded in diagnostics by
/// this contract.
pub const Config = @import("config.zig");
/// Nonnegative nanosecond duration used by time and scheduling APIs.
pub const Duration = @import("deadline.zig").Duration;
/// Providers return owned read buffers allocated from R. They copy borrowed write buffers before
/// committing or retaining them. The callback constructs an Effect; the existing kernel then owns any
/// suspension/cancellation.
pub const KeyValueStore = @import("persistence/key_value_store.zig").KeyValueStore;
/// `WriteAcknowledgement` cases: `committed`, `possibly_committed`.
pub const WriteAcknowledgement = @import("persistence/key_value_store.zig").WriteAcknowledgement;
/// A successful compareExchange is linearizable for this key, including its revision and expiry update.
/// Implementations unable to uphold that contract cannot construct a conforming conditional capability.
pub const ConditionalStore = @import("persistence/key_value_store.zig").ConditionalStore;
/// In-memory implementation of persistence capabilities.
pub const MemoryPersistence = @import("persistence/memory_persistence.zig").MemoryPersistence;
/// Schema-backed persistence definition with stable identity and codecs.
pub const Persistable = @import("persistence/persistable.zig").Persistable;
/// Staged persisted-Exit definition. A codec, schema version, wire limits, key/expiry selectors, store,
/// and codec environments are required by `build`. The resulting operation uses the provider supplied
/// through `R`.
pub const PersistableBuilder = @import("persistence/persistence_builder.zig").Builder;
/// Typed result of reading a persisted value.
pub const PersistedResultRead = @import("persistence/persistable.zig").ReadResult;
/// Cache backed by a caller-supplied persistence capability.
pub const PersistedCache = @import("persistence/persisted_cache.zig").PersistedCache;
/// Typed result of reading the persisted cache.
pub const PersistedCacheRead = @import("persistence/persisted_cache.zig").Read;
/// Typed persisted-cache operation and provider failure.
pub const PersistedCacheFailure = @import("persistence/persisted_cache.zig").Failure;
/// Provider interface for rate-limit state and feedback.
pub const RateLimitStore = @import("persistence/rate_limiter.zig").Store;
/// Versioned rate-limit rule whose revision separates incompatible policies.
pub const RateLimitPolicy = @import("persistence/rate_limiter.zig").Policy;
/// Admission uses the current capacity in a fixed window. Feedback changes that capacity under the same
/// store gate as admissions. Every policy number is chosen by the application; the core has no tuning
/// constants.
pub const AdaptiveRateLimit = @import("persistence/rate_limiter.zig").Adaptive;
/// The producer assigns increasing sequence numbers within one key/policy revision. Duplicate or
/// reordered observations never undo newer feedback.
pub const RateLimitFeedback = @import("persistence/rate_limiter.zig").Feedback;
/// Feedback application result distinguishing a fresh update from a stale one.
pub const RateLimitFeedbackResult = @import("persistence/rate_limiter.zig").FeedbackResult;
/// In-memory implementation of rate-limit state.
pub const MemoryRateLimiter = @import("persistence/memory_rate_limiter.zig").MemoryRateLimiter;
/// A provider must atomically deduplicate an offer, fence claims by owner, generation and attempt, and
/// keep raw corrupt payloads inspectable. A plain key/value get/set provider does not satisfy these
/// transitions. Only a recoverable implementation can promise restart-stable accepted work.
pub const PersistedQueue = @import("persistence/persisted_queue.zig").Store;
/// In-memory provider for the persisted-queue contract.
pub const MemoryPersistedQueue = @import("persistence/memory_persisted_queue.zig").MemoryPersistedQueue;
/// This is an ordered partial-progress capability, not an atomic multi-key transaction. Failure of one
/// item cannot be represented as whole-batch success; the report records the failure at its exact
/// index. Borrowed input slices remain valid until the returned Effect completes.
pub const BatchStore = @import("persistence/batch_store.zig").BatchStore;
/// Bounded, transport-neutral RPC envelopes. Framing adds only the outer length prefix; operation
/// identity, correlation and application metadata remain independent of HTTP or any other transport.
pub const RpcEnvelope = @import("rpc/envelope.zig");
/// Frames are complete, length-prefixed values; send does not borrow its input after the Effect
/// completes. An implementation must never report `accepted` before it has copied or taken ownership of
/// the bytes. Receive returns bytes allocated using the caller-supplied allocator in R.
pub const RpcSession = @import("rpc/session.zig").Session;
/// Server-side one-chunk window. The next chunk cannot be sent until its sequence is acknowledged;
/// cancellation closes the window permanently.
pub const RpcStreamWindow = @import("rpc/session.zig").StreamWindow;
/// Caller-owned, process-local serializable transaction domain. A short commit gate validates read
/// versions and publishes staged writes together; retries wait on observed changes without holding the
/// gate across suspension.
pub const TransactionDomain = @import("transactional/transaction.zig").Domain;
/// Success, typed failure, retry, or defect from atomic evaluation.
pub const TransactionAtomicDecision = @import("transactional/transaction.zig").Decision;
/// Transactional reference read and written through a journal.
pub const TransactionRef = @import("transactional/transaction.zig").Ref;
/// Comptime transactional descriptions composed against a journal.
pub const TransactionBuilder = @import("transactional/transaction_builder.zig");
/// Bounded transactional chunk of typed elements.
pub const TransactionChunk = @import("transactional/tx_chunk.zig").Chunk;
/// Transactional bounded queue.
pub const TransactionQueue = @import("transactional/tx_queue.zig").Queue;
/// Transactional one-shot completion cell.
pub const TransactionDeferred = @import("transactional/tx_deferred.zig").Deferred;
/// Permit counter whose acquire and release participate in one transaction journal.
pub const TransactionSemaphore = @import("transactional/tx_semaphore.zig").Semaphore;
/// Transactional lock keyed by a caller-defined owner type.
pub const TransactionReentrantLock = @import("transactional/tx_reentrant_lock.zig").ReentrantLock;
/// Transactional bounded priority queue with a comptime comparator.
pub const TransactionPriorityQueue = @import("transactional/tx_priority_queue.zig").PriorityQueue;
/// Transactional hash map whose changes commit atomically.
pub const TransactionHashMap = @import("transactional/tx_hash_map.zig").HashMap;
/// Transactional set whose changes commit atomically.
pub const TransactionHashSet = @import("transactional/tx_hash_set.zig").HashSet;
/// Transactional publication/subscription state.
pub const TransactionPubSub = @import("transactional/tx_pub_sub.zig").PubSub;
/// Transactional reference with observable committed changes.
pub const TransactionSubscriptionRef = @import("transactional/tx_subscription_ref.zig").SubscriptionRef;
/// The committed version remains available even if local notification fails. Repeating from the last
/// observed version repairs a missed notification.
pub const TransactionAwaitAndInvalidate = @import("transactional/observation.zig").awaitAndInvalidate;
/// Stream of committed transactional changes after a revision.
pub const TransactionChangesAfter = @import("transactional/stream.zig").changesAfter;
/// Typed failure when observing transactional revisions.
pub const TransactionObservationError = @import("transactional/observation.zig").Error;
/// Typed event identity, schema, and version definition.
pub const EventDefinition = @import("eventlog/event.zig").Event;
/// Comptime event definitions and codecs.
pub const EventBuilder = @import("eventlog/event_builder.zig");
/// Closed group of typed event definitions.
pub const EventGroup = @import("eventlog/event.zig").Group;
/// Typed replay over authoritative journal pages. Pure folds are separate from event handlers and never
/// publish a checkpoint on partial failure.
pub const EventProjection = @import("eventlog/projection.zig");
pub const EventProjectionCases = @import("eventlog/projection_cases.zig");
/// Data-only replica protocol. A batch carries journal cursors and original event identities;
/// notifications are never treated as history authority.
pub const EventReplicaMessage = @import("eventlog/event_log_message.zig");
/// Replaceable replication, cursor, session and protection boundaries. None constructs a transport,
/// clock, crypto backend or persistence engine.
pub const EventReplicaRemote = @import("eventlog/event_log_remote.zig");
/// Compaction requires a provider-defined replay proof. The core never assumes a projection checkpoint
/// can replace history for every consumer.
pub const EventCompaction = @import("eventlog/event_log_remote.zig").Compaction;
/// Conflict facts are borrowed for the duration of a handler Effect. A handler may consult application
/// history or request a snapshot, but the importer never fabricates a universal event merge from
/// timestamps.
pub const EventReplicaConflict = @import("eventlog/event_log_remote.zig").Conflict;
/// Resolution is an application-defined typed value. The core deliberately does not interpret it as
/// permission to rewrite journal history; any replacement must pass the same authoritative journal
/// contracts as append.
pub const EventReplicaConflictHandler = @import("eventlog/event_log_remote.zig").ConflictHandler;
/// In-memory replica cursor implementation.
pub const MemoryEventReplicaCursor = @import("eventlog/memory_replica_cursor.zig").MemoryReplicaCursor;
/// Imports encoded replica events with typed validation.
pub const EventReplicaImporter = @import("eventlog/replication.zig").Importer;
/// The caller supplies the encoded import operation and a registry-local key. A failed notification
/// reports its already committed cursor; a caller can retry the same import or read the journal/cursor
/// to reconstruct observation.
pub const EventImportAndInvalidate = @import("eventlog/observation.zig").importAndInvalidate;
/// Typed error while observing event-log changes.
pub const EventObservationError = @import("eventlog/observation.zig").Error;
/// Typed reason an event-log observer must invalidate or resynchronize.
pub const EventObservationCause = @import("eventlog/observation.zig").ObservationCause;
/// Invalidate only after a local append is acknowledged committed. If the notification fails, the typed
/// error retains the committed result, which can be reconciled from the journal without repeating the
/// handler.
pub const EventAppendAndInvalidate = @import("eventlog/observation.zig").appendAndInvalidate;
/// Typed error while appending and invalidating observers.
pub const EventAppendObservationError = @import("eventlog/observation.zig").AppendObservationError;
/// Typed, versioned workflow activity definition.
pub const WorkflowActivity = @import("workflow/activity.zig").Activity;
/// Closed registry of versioned workflow activities.
pub const WorkflowActivityRegistry = @import("workflow/activity.zig").Registry;
/// Typed, versioned workflow definition and handler.
pub const Workflow = @import("workflow/workflow.zig").Workflow;
/// The type carries payload, success and domain-error identity through client code; the process-local
/// handler remains separate from the durable handle.
pub const WorkflowHandle = @import("workflow/workflow.zig").Handle;
/// Closed registry of workflow definitions.
pub const WorkflowRegistry = @import("workflow/workflow.zig").Registry;
/// Comptime workflow definition and step builder.
pub const WorkflowBuilder = @import("workflow/workflow_builder.zig").WorkflowBuilder;
/// Pairs a stable activity identity and payload/outcome codecs with an ordinary typed Effect body. The
/// workflow engine records its completed Exit so replay does not rerun a completed activity.
pub const WorkflowActivityBuilder = @import("workflow/activity_builder.zig").ActivityBuilder;
/// A completed record is decoded instead of rerunning its body. An absent record permits execution;
/// only an acknowledged record is replayable. `possibly_recorded` remains explicit uncertainty for
/// caller read-back.
pub const WorkflowRunActivity = @import("workflow/activity_execution.zig").runRecorded;
/// Previously recorded activity Exit consulted during replay.
pub const WorkflowRecordedActivity = @import("workflow/activity_execution.zig").Recorded;
/// Typed failure while running or replaying an activity.
pub const WorkflowActivityRunError = @import("workflow/activity_execution.zig").RunError;
/// Durable compensation metadata and journal boundary. A compensation is a recorded workflow action,
/// never an automatic rollback of an external side effect. Uncertain recordings require lookup before
/// another attempt.
pub const WorkflowCompensation = @import("workflow/compensation.zig");
/// Replay a completed attempt or execute and record its complete Exit before returning it. An ambiguous
/// record remains typed uncertainty; callers must look it up before retrying a possibly completed
/// external compensation. The supplied program must use a stable external idempotency key derived from
/// the attempt identity whenever it performs an external side effect; recording the Exit cannot make
/// that side effect exactly once.
pub const WorkflowRunCompensation = @import("workflow/compensation.zig").runRecorded;
/// Run the highest-ordinal outstanding compensation. Repeated calls process an execution in reverse
/// registration order. The journal page remains owned through the dispatched Effect, including
/// cancellation and cleanup. A failed compensation stays in the returned completed Exit for the caller
/// to combine with the original workflow failure; dispatch must record before this step can report
/// completion.
pub const WorkflowRunNextCompensation = @import("workflow/compensation.zig").runNextPending;
/// Drain registered compensations in reverse order. A failed, interrupted or defected compensation is
/// returned as an owned Exit so the original workflow Exit can remain visible in `Resolution`. A
/// dispatcher that reports success without making journal progress is rejected instead of looping
/// forever.
pub const WorkflowRunAllCompensations = @import("workflow/compensation.zig").runAllPending;
/// In-memory compensation journal provider.
pub const MemoryWorkflowCompensation = @import("workflow/memory_compensation.zig").MemoryCompensation;
/// Stable branch identities and recorded durable fan-out/race decisions. Worker execution remains with
/// the caller's Effect parallel capability; this journal only supplies replay authority for completed
/// branches.
pub const WorkflowDurableParallel = @import("workflow/durable_parallel.zig");
/// In-memory journal for durable parallel branches.
pub const MemoryWorkflowParallelJournal = @import("workflow/memory_parallel_journal.zig").MemoryParallelJournal;
/// Workflow queue composition over Persistence.PersistedQueue and a typed durable-deferred result. This
/// module owns no second queue or delivery authority. An ambiguous offer remains explicit for caller
/// reconciliation.
pub const WorkflowDurableQueue = @import("workflow/durable_queue.zig");
/// Typed client for submitting and observing workflows.
pub const WorkflowProxy = @import("workflow/workflow_proxy.zig").Proxy;
/// Reference with serialized read-modify-write operations.
pub const SynchronizedRef = @import("synchronized_ref.zig").SynchronizedRef;
/// Validated nanosecond duration used by the scheduling helpers.
pub const CheckedDuration = @import("duration.zig").Duration;
/// Pure Gregorian recurring-calendar evaluation. A zone capability supplies civil-time resolution,
/// including gaps and any number of overlap candidates; no process timezone or bundled timezone
/// database is consulted.
pub const Cron = @import("cron.zig");
/// Typed request payload or request record for this operation.
pub const Request = @import("request.zig").Request;
/// Typed batched request resolver with scoped shared loading.
pub const RequestResolver = @import("request_resolver.zig").Resolver;
/// Compose event handling with append and observer invalidation.
pub const EventLogHandleAndAppend = @import("eventlog/event_log.zig").handleAndAppend;
/// Encode the event payload before invoking the handler, then retain its owned bytes through the
/// journal acknowledgement. The adapter receives the definition's checked payload codec and can
/// serialize its representation using the caller's R; no codec or I/O implementation is fixed by core.
pub const EventLogHandleAndAppendEncoded = @import("eventlog/event_log.zig").handleAndAppendEncoded;
/// Typed entity definition and handler for routed messages.
pub const ClusterEntity = @import("cluster/entity.zig").Entity;
/// Closed registry of typed clustered entities.
pub const ClusterEntityRegistry = @import("cluster/entity.zig").Registry;
/// Comptime clustered entity definition and handler builder.
pub const ClusterEntityBuilder = @import("cluster/entity_builder.zig").EntityBuilder;
/// A conforming implementation linearizes acquire, renew and release for each entity; generations never
/// repeat. It must not execute user handlers while holding its ownership gate. An ambiguous
/// acknowledgement is surfaced for read-back rather than assumed successful.
pub const ClusterOwnership = @import("cluster/cluster_capabilities.zig").Ownership;
/// In-memory cluster ownership and fencing provider.
pub const MemoryClusterOwnership = @import("cluster/memory_cluster.zig").MemoryOwnership;
/// Scoped entity resource acquisition and release.
pub const ClusterEntityResource = @import("cluster/entity_resource.zig").EntityResource;
pub const ClusterEntityResourcePlan = @import("cluster/entity_resource_plan.zig");
/// Run work under a fenced singleton lease.
pub const ClusterWithSingletonLease = @import("cluster/singleton.zig").withLease;
/// Versioned recurring cluster schedule with explicit calendar and search policy.
pub const ClusterSchedule = @import("cluster/cluster_schedule.zig").Schedule;
/// Typed client for sending requests to a clustered entity.
pub const ClusterEntityProxy = @import("cluster/entity_proxy.zig").Proxy;
/// Mutable atom reference with subscription state.
pub const AtomRef = @import("reactivity/atom_ref.zig").AtomRef;
/// Bounded graph of reactive dependencies, synchronized with caller-supplied I/O.
pub const ReactiveDependencyGraph = @import("reactivity/internal/dependency_graph.zig").Graph;
/// Explicit limits on graph nodes and dependencies per node.
pub const ReactiveGraphLimits = @import("reactivity/internal/dependency_graph.zig").Limits;
/// Restricted actual-read programs with atomic generation/revision publication.
pub const AtomProgram = @import("reactivity/atom_program.zig");
/// Registry of typed atom definitions and their subscription state.
pub const AtomRegistry = @import("reactivity/atom_registry.zig").Registry;
/// Checked registry, graph, and invalidation failures.
pub const AtomRegistryError = @import("reactivity/atom_registry.zig").Error;
/// Typed reactive atom definition.
pub const Atom = @import("reactivity/atom.zig").Atom;
/// Comptime reactive atom definition.
pub const AtomBuilder = @import("reactivity/atom_builder.zig").AtomBuilder;
/// Atom write result distinguishing committed state from notification loss.
pub const AtomMutation = @import("reactivity/atom.zig").Mutation;
/// Encoded reactive state transfer. Snapshots contain stable identity and schema-versioned bytes, never
/// tasks, subscriptions, services or closures.
pub const ReactiveHydration = @import("reactivity/hydration.zig");
/// RPC-backed query and mutation composition. Network/session behavior remains with the caller's RPC
/// client; this module only connects typed observations to the scoped atom registry after a successful
/// mutation.
pub const ReactiveRpc = @import("reactivity/atom_rpc.zig");
/// Persisted atom composition keeps storage acknowledgement distinct from local invalidation. A local
/// notice is only a hint; persisted state remains authoritative after ambiguous writes or lost
/// notifications.
pub const ReactivePersistence = @import("reactivity/atom_persistence.zig");
/// Provider contract for durable deferred completions.
pub const WorkflowDurableDeferred = @import("workflow/durable_deferred.zig").Store;
/// A signal's result schema is part of its compile-time identity. Its execution-specific token cannot
/// be constructed with an unrelated result type or silently rebound to a different schema version.
pub const WorkflowSignal = @import("workflow/durable_deferred.zig").Signal;
/// A recoverable provider must commit link, completion and acknowledgement transitions atomically
/// within its own authority. The volatile memory implementation exercises the same contract without
/// claiming disk safety.
pub const WorkflowChildStore = @import("workflow/child_store.zig").Store;
/// Durable parent/child execution relationship.
pub const WorkflowChildLink = @import("workflow/child_store.zig").Link;
/// Recorded child execution awaiting a terminal outcome.
pub const WorkflowChildPending = @import("workflow/child_store.zig").Pending;
/// In-memory child-workflow relationship store.
pub const MemoryWorkflowChildStore = @import("workflow/memory_child_store.zig").MemoryChildStore;
/// Child completion and parent wake composition. The child store retains the encoded Exit independently
/// of the wake attempt. A failed or lost wake is reconciled from pending completions; acknowledgement
/// is a separate step performed only after the parent consumes the recorded reason.
pub const WorkflowChildWake = @import("workflow/child_wake.zig");
/// In-memory durable-deferred provider.
pub const MemoryWorkflowDurableDeferred = @import("workflow/memory_durable_deferred.zig").MemoryDurableDeferred;
/// Provider contract for durable workflow timers.
pub const WorkflowDurableClock = @import("workflow/durable_clock.zig").Store;
/// In-memory durable-clock provider.
pub const MemoryWorkflowDurableClock = @import("workflow/memory_durable_clock.zig").MemoryDurableClock;
/// Define a unary RPC operation from three codecs. Each codec must satisfy `codec_contract.require` at
/// comptime; the decoded types become the request, success, and domain-error channels of the operation.
pub const RpcUnary = @import("rpc/protocol.zig").Unary;
/// Streaming uses its own descriptor and typed handler shape. A stream chunk cannot be registered as
/// the unary success of an otherwise similar request. Each codec must satisfy `codec_contract.require`
/// at comptime.
pub const RpcStreaming = @import("rpc/protocol.zig").Streaming;
/// Borrowed application metadata and correlation for one invocation. Encoded handlers receive it from
/// their request envelope; direct calls use `empty`.
pub const RpcContext = @import("rpc/protocol.zig").Context;
/// Only completed operation/handler pairs enter this static registry. It rejects duplicate names at
/// declaration, before any request is dispatched.
pub const RpcProtocol = @import("rpc/protocol.zig").Protocol;
/// Comptime RPC protocol builder with typed operations and framing.
pub const RpcProtocolBuilder = @import("rpc/protocol_builder.zig").ProtocolBuilder;
/// Typed RPC payload codec for the selected environment.
pub const RpcWireCodec = @import("rpc/wire_codec.zig").WireCodec;
/// Registry of active RPC session correlations.
pub const RpcSessionRegistry = @import("rpc/internal/session.zig").Registry;
/// One suspended provider receive, then bounded frame/envelope decoding and delivery. A caller-owned
/// registry remains alive until this Effect joins.
pub const RpcReceiveOne = @import("rpc/internal/session.zig").receiveOne;
/// A separate bounded registry keeps streaming responses distinct from unary Deferred values. One
/// pending call owns a bounded frame queue and sequence.
pub const RpcStreamRegistry = @import("rpc/internal/session.zig").StreamRegistry;
/// Receive one framed streaming RPC event.
pub const RpcReceiveStreamOne = @import("rpc/internal/session.zig").receiveStreamOne;
/// A single receiver dispatches both unary and streaming replies. Separate receive loops on the same
/// provider could steal one another's frames.
pub const RpcReceiveAnyOne = @import("rpc/internal/session.zig").receiveAnyOne;
/// Typed client for unary RPC operations.
pub const RpcUnaryClient = @import("rpc/client.zig").UnaryClient;
/// Typed client for streaming RPC operations.
pub const RpcStreamClient = @import("rpc/stream_client.zig").StreamClient;
/// Server dispatcher for framed streaming RPC calls.
pub const RpcStreamServer = @import("rpc/stream_server.zig").StreamServer;
/// Server dispatcher for framed unary RPC calls.
pub const RpcUnaryServer = @import("rpc/server.zig").UnaryServer;
/// Builds a closed encoded server from typed handler/codec bindings. The generated routes and runtime
/// dispatch remain private to this package.
pub const RpcProtocolServer = @import("rpc/server.zig").ProtocolServer;
/// Typed middleware wrapping an RPC service.
pub const RpcMiddleware = @import("rpc/middleware.zig").Middleware;
/// Typed middleware and domain failure union.
pub const RpcMiddlewareFailure = @import("rpc/middleware.zig").Failure;
/// Derives the reference union representation from the two typed JSON case codecs. The case tags and
/// field names are one stable wire contract.
pub const RpcMiddlewareErrorSchema = @import("rpc/middleware.zig").errorSchema;
/// Retains the operation identity, request/success codecs and key extractor; only the domain-error
/// codec is extended with the middleware case.
pub const RpcMiddlewareOperation = @import("rpc/middleware.zig").operation;
/// Incremental length-prefixed RPC frame parser with bounded buffering.
pub const RpcFramer = @import("rpc/framing.zig").Framer;
/// Caller-selected bounds for individual RPC frames and buffered bytes.
pub const RpcFramingLimits = @import("rpc/framing.zig").Limits;
/// A journal implementation may be volatile or retained. This interface promises ordered append/read
/// and explicit uncertainty, not physical durability. A stronger projection/checkpoint atomic
/// capability is separate.
pub const EventJournal = @import("eventlog/event_journal.zig").Journal;
/// Replay events from a journal before following live changes.
pub const EventJournalCatchUp = @import("eventlog/journal_stream.zig").catchUp;
/// This is data accepted by a journal provider. Both slices remain borrowed until the returned Effect
/// settles; a provider copies what it retains.
pub const EventProposal = @import("eventlog/event_journal.zig").Proposal;
/// Sequence zero is the position before the first event; committed events have consecutive positive
/// sequences. `readAfter` excludes this cursor.
pub const EventCursor = @import("eventlog/event_journal.zig").Cursor;
/// Caller-selected limits for a bounded event-journal page.
pub const EventReadPolicy = @import("eventlog/event_journal.zig").ReadPolicy;
/// Owned event page with a continuation cursor and a `more` flag.
pub const EventReadPage = @import("eventlog/event_journal.zig").ReadPage;
/// Journal acknowledgement of an accepted event append.
pub const EventAppendAcknowledgement = @import("eventlog/event_journal.zig").AppendAcknowledgement;
/// Encoded journal entry with stable tag, event identity, and replay cursor.
pub const EventEntry = @import("eventlog/event_journal.zig").Entry;
/// In-memory event journal provider.
pub const MemoryEventJournal = @import("eventlog/memory_event_journal.zig").MemoryEventJournal;
/// A compare-and-publish must validate the prior cursor and publish the next encoded projection state
/// and cursor indivisibly. The caller does not infer this guarantee from a key/value store or from a
/// notification channel. Missing/compacted journal history remains a typed EventLog failure.
pub const AtomicProjectionCheckpoint = @import("eventlog/projection_checkpoint.zig").AtomicCheckpointStore;
/// Owned state and cursor read from an atomic projection checkpoint store.
pub const ProjectionCheckpoint = @import("eventlog/projection_checkpoint.zig").Checkpoint;
/// Expected cursor revision used by atomic projection publication.
pub const ProjectionCheckpointRevision = @import("eventlog/projection_checkpoint.zig").Revision;
/// Atomic publication acknowledgement, including conflict and uncertainty.
pub const ProjectionCheckpointCommit = @import("eventlog/projection_checkpoint.zig").CommitResult;
/// In-memory projection checkpoint provider.
pub const MemoryProjectionCheckpoint = @import("eventlog/memory_projection_checkpoint.zig").MemoryProjectionCheckpoint;
/// Logical identities and ownership data, independent of physical topology.
pub const ClusterTypes = @import("cluster/cluster_types.zig");
/// A provider must atomically deduplicate acceptance by request identity and reject a reused identity
/// with different request bytes or definition. Claim and completion are fenced by a current ownership
/// proof for this exact entity as well as generation and attempt. A claim lease cannot outlive that
/// ownership lease. Replacing or releasing ownership and publishing a completion share one
/// linearization boundary; a stale completion cannot commit merely because nobody has claimed the
/// message under a new owner. Saved streaming chunks are ordered from sequence one and fenced by the
/// same claim as final completion. Duplicate sequence/bytes are idempotent; conflicting bytes or a gap
/// are rejected. Reads use a caller bound and acknowledgement advances monotonically. Replies and
/// unacknowledged chunks remain inspectable until retention permits pruning; pruning never removes
/// unfinished or unacknowledged work. Observer cancel does not cancel accepted execution. `recoverable`
/// is a stronger provider promise tested by conformance; the in-memory implementation is volatile.
pub const ClusterMessageStore = @import("cluster/message_store.zig").MessageStore;
/// Caller-selected message claim count and lease deadline.
pub const ClusterClaimPolicy = @import("cluster/message_store.zig").ClaimPolicy;
/// Chunk sequences start after zero and advance contiguously for each request. A provider records a
/// chunk before reporting it to a streaming observer.
pub const ClusterSavedChunk = @import("cluster/message_store.zig").SavedChunk;
/// Caller-selected bounds for chunk count and encoded bytes.
pub const ClusterChunkPolicy = @import("cluster/message_store.zig").ChunkPolicy;
/// Owned page of saved reply chunks with next sequence and continuation flag.
pub const ClusterChunkPage = @import("cluster/message_store.zig").ChunkPage;
/// Typed result of reading saved reply chunks.
pub const ClusterChunkRead = @import("cluster/message_store.zig").ChunkRead;
/// `ChunkWrite` cases: `recorded`, `already_recorded`, `stale_claim`, `possibly_recorded`.
pub const ClusterChunkWrite = @import("cluster/message_store.zig").ChunkWrite;
/// `ChunkAck` cases: `acknowledged`, `already_acknowledged`, `missing`, `unknown_sequence`,
/// `uncertain`.
pub const ClusterChunkAck = @import("cluster/message_store.zig").ChunkAck;
/// Owned message accepted for processing under an ownership lease.
pub const ClusterClaimedMessage = @import("cluster/message_store.zig").ClaimedMessage;
/// `Completion` is a tagged choice.
pub const ClusterMessageCompletion = @import("cluster/message_store.zig").Completion;
/// In-memory cluster message-store provider.
pub const MemoryClusterMessageStore = @import("cluster/memory_message_store.zig").MemoryMessageStore;
/// Run a typed handler and save its full Exit under the claim's ownership fence. A stale or uncertain
/// completion remains an explicit store result. The claim selected from R must remain alive until this
/// Effect completes.
pub const ClusterServeClaim = @import("cluster/runner.zig").serveClaim;
/// The caller retains the claimed batch and pointer array until the returned Effect and all its
/// children have joined. Concurrency is supplied by the caller and admitted through Effect's selected
/// execution capabilities. Each output corresponds to the same input position, including stale and
/// uncertain fenced completions.
pub const ClusterServeClaims = @import("cluster/runner.zig").serveClaims;
/// Claim at most the caller's policy limit, run each accepted request, and stop at a fenced or
/// uncertain completion. The claimed page is retained by structured acquisition until every handler and
/// store operation has joined. Use `serveClaims` when a caller wants bounded parallel execution of a
/// page.
pub const ClusterClaimAndServe = @import("cluster/runner.zig").claimAndServe;
/// Progress report for a bounded cluster-message processing batch.
pub const ClusterBatchProgress = @import("cluster/runner.zig").BatchProgress;
/// Typed failure while running clustered entities.
pub const ClusterRunnerError = @import("cluster/runner.zig").Error;
/// Workflow engine backed by cluster messaging.
pub const ClusterWorkflowEngine = @import("cluster/cluster_workflow_engine.zig").ClusterWorkflowEngine;
/// Cluster message selecting a workflow run or control operation.
pub const ClusterWorkflowCommand = @import("cluster/cluster_workflow_engine.zig").Command;
/// Typed reply to a cluster workflow command.
pub const ClusterWorkflowReply = @import("cluster/cluster_workflow_engine.zig").Reply;
/// Data-only workflow identities and states. Code is registered separately; execution and activity
/// histories never contain live functions or handles.
pub const WorkflowTypes = @import("workflow/workflow_types.zig");
/// The provider copies borrowed submission/control buffers before retaining them. Returned records are
/// owned by the caller's allocator from R. A recordActivity transition is atomic for its execution,
/// identity, attempt and activation fence. `possibly_recorded` requires lookup before retrying an
/// external side effect; it must never masquerade as `recorded`. `cancel_terminal` records a
/// cooperative cancellation request for running work and reaches a terminal state at a fenced
/// activity/suspension boundary or after lease recovery. `interrupt_unsafe` may fence it immediately
/// while external effects still run. Providers must keep these operations distinct.
pub const WorkflowEngine = @import("workflow/workflow_engine.zig").WorkflowEngine;
/// Typed workflow-engine operation and provider failure.
pub const WorkflowEngineError = @import("workflow/workflow_engine.zig").Error;
/// In-memory workflow-engine provider.
pub const MemoryWorkflowEngine = @import("workflow/memory_workflow_engine.zig").MemoryWorkflowEngine;

/// Native record/object/sequence contracts with shared-runtime composition.
pub const NativeCodec = @import("schema/native_codec.zig");
/// Static required/optional/default native field declarations; representation and ownership are checked at build time.
pub const NativeField = NativeCodec;
/// Staged native record builder with independent decoding/encoding environments and caller object contract.
pub const NativeRecordBuilder = NativeCodec.Builder;
/// Representation-generic domain failures, independent pure/effectful directions and typed issue ownership mappings.
pub const DomainCodec = @import("schema/domain_codec.zig");
/// Exact decimal token ownership/arithmetic and bounded arbitrary-width integer conversion, without f64 rounding.
pub const JsonNumberCodec = @import("schema/json_number_codec.zig");
/// Typed Unicode/enum/array/prefix/object validation with explicit caller-selected pattern capabilities.
pub const JsonConstraints = @import("schema/json_constraints.zig");
/// Owning oneOf/anyOf/allOf candidates with immutable metadata and invocation-local parse state.
pub const JsonAlternatives = @import("schema/json_alternatives.zig");
/// std.json-backed bounded writer and checked tree-free Effect adapters; successful bytes own R.allocator.
pub const JsonWriter = @import("schema/json_writer.zig");
/// Version-one owned diagnostic wire codec with explicit input, JSON and related-issue bounds.
pub const SchemaIssueCodec = @import("schema/issue_codec.zig");
/// Require actual public pure-direction evidence; service-requiring codecs cannot masquerade as direct Result codecs.
pub const requirePure = @import("schema/codec.zig").requirePure;
/// Scoped dependency-aware cache sharing the existing caller runtime and generation engine.
pub const DependencyCache = @import("effect.zig").DependencyCache;
/// Finite retention/admission and per-entry dependency limits with explicit ownership callbacks.
pub const DependencyCacheConfig = @import("dependency_cache.zig").Config;
/// Owning loader payload and individually retained dependency metadata.
pub const DependencyCacheLoaded = @import("dependency_cache.zig").Loaded;
/// Caller equality/retain/release contract for dependency identities and their pointed-to storage.
pub const DependencyPolicy = @import("dependency_cache.zig").DependencyPolicy;
/// Provenance chosen before waiting: retained, shared in-flight or newly loaded generation.
pub const CacheObservation = @import("cache.zig").CacheObservation;
/// Value observation plus provenance; marked payload ownership is cloned/released independently.
pub const CacheLookup = @import("cache.zig").CacheLookup;
/// Scoped finite-capacity callback publication with explicit nonblocking overflow policy; acceptance is not subscriber delivery.
pub const CallbackPubSub = @import("pub_sub.zig").CallbackPubSub;
/// Pure byte-representation schema adapter with checked direct execution and explicit framing bounds.
pub const PureSchemaBinary = @import("encoding/schema_binary.zig").Pure;

/// Independent JSON input, preparation and output limits.
pub const JsonSurface = @import("schema/json_surface.zig");
/// Require public imported payload ownership evidence.
pub const requireOwnedPayload = @import("outcome.zig").requireOwned;
/// Bounds selected by a framed RPC session's caller.
pub const RpcSessionPolicy = @import("rpc/session.zig").Policy;
