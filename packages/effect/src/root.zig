//! Zig Effect: lazy typed work with environment-owned runtime dependencies.
//! Effects produce typed Exit values directly. Validation scope and deliberate
//! adaptations are recorded in docs/port-audit.md.
pub const Defect = @import("outcome.zig").Defect;
pub const Exit = @import("outcome.zig").Exit;
pub const Result = @import("outcome.zig").Result;
pub const OwnedSlice = @import("outcome.zig").OwnedSlice;
pub const PrimaryOutcome = @import("outcome.zig").PrimaryOutcome;
pub const CleanupFailure = @import("outcome.zig").CleanupFailure;
pub const Cancellation = @import("internal/runtime.zig").Cancellation;
pub const Effect = @import("effect.zig").Effect;
pub const EffectBuilder = @import("effect.zig").EffectBuilder;
pub const ResourceValue = @import("effect.zig").ResourceValue;
pub const ResourceBinding = @import("effect.zig").ResourceBinding;
pub const resource = @import("effect.zig").resource;
pub const Layer = @import("effect.zig").Layer;
pub const Stream = @import("effect.zig").Stream;
pub const StreamReader = @import("effect.zig").StreamReader;
pub const Chunk = @import("effect.zig").Chunk;
pub const Scope = @import("scope.zig").Scope;
pub const Async = @import("async.zig").Async;
pub const AsyncRun = @import("effect.zig").AsyncRun;
pub const Fiber = @import("effect.zig").Fiber;
pub const runAsync = @import("effect.zig").runAsync;
pub const Executor = @import("executor.zig").Executor;
pub const NativeIoExecutor = @import("executor.zig").NativeIo;
pub const ExecutionKind = @import("executor.zig").Kind;
pub const Deferred = @import("primitives.zig").Deferred;
pub const Latch = @import("primitives.zig").Latch;
pub const Ref = @import("primitives.zig").Ref;
pub const Queue = @import("primitives.zig").Queue;
pub const QueueError = @import("primitives.zig").QueueError;
pub const Schedule = @import("schedule.zig");
pub const Schema = @import("schema.zig").Schema;
pub const Clock = @import("services.zig").Clock;
pub const Random = @import("services.zig").Random;
pub const Logger = @import("services.zig").Logger;
pub const Tracer = @import("services.zig").Tracer;
pub const LogLevel = @import("services.zig").LogLevel;
pub const Annotation = @import("services.zig").Annotation;
pub const Redactor = @import("services.zig").Redactor;
pub const DiagnosticSink = @import("services.zig").DiagnosticSink;
pub const run = @import("effect.zig").run;

test {
    _ = @import("outcome.zig");
    _ = @import("effect.zig");
    _ = @import("proofs.zig");
}

pub const Cache = @import("cache.zig").Cache;
pub const CacheConfig = @import("cache.zig").Config;
pub const CacheKeyPolicy = @import("cache.zig").KeyPolicy;
pub const EffectCache = @import("effect.zig").EffectCache;
pub const Pool = @import("pool.zig").Pool;
pub const PoolConfig = @import("pool.zig").Config;
pub const EffectPool = @import("effect.zig").EffectPool;
pub const Http = @import("http.zig");
pub const Fetch = @import("effect.zig").Fetch;
pub const WebSocket = @import("websocket.zig");
pub const EffectWebSocket = @import("effect.zig").EffectWebSocket;
pub const EffectDeferred = @import("effect.zig").EffectDeferred;
pub const EffectLatch = @import("effect.zig").EffectLatch;
pub const EffectRef = @import("effect.zig").EffectRef;
pub const EffectQueue = @import("effect.zig").EffectQueue;
pub const Console = @import("services.zig").Console;
pub const NativeConsole = @import("services.zig").NativeConsole;
pub const EffectConsole = @import("effect.zig").EffectConsole;
pub const ManualClock = @import("manual_clock.zig").ManualClock;
pub const WeakCache = @import("weak_cache.zig").WeakCache;
pub const weak_cache_support = @import("weak_cache.zig").support;

pub const ExecutionJob = @import("executor.zig").Job;
pub const ExecutionCapabilities = @import("executor.zig").Capabilities;
pub const SpanId = @import("services.zig").SpanId;
pub const SpanStart = @import("services.zig").SpanStart;
pub const SpanFinish = @import("services.zig").SpanFinish;
pub const SpanOutcome = @import("services.zig").SpanOutcome;
pub const LogRecord = @import("services.zig").LogRecord;
pub const Annotations = @import("services.zig").Annotations;
pub const Parsed = @import("schema.zig").Parsed;
