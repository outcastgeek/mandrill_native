# Effect operation contracts

Public entry point: `src/root.zig`. All Effect-returning operations preserve a
direct `Exit(A, E)` completion boundary. Runtime dependencies are selected through
R, including bound/projected environments. Provider configuration belongs to the
consumer. Pointer lifetimes and unique ownership remain explicit Zig contracts.

## Core, composition, and environments

| Operations | Contract |
| --- | --- |
| `succeed`, `fail`, `die`, `interrupted` | Lazy ordinary outcomes. Owning A/E are rejected by reusable succeed/fail; use a lazy factory. Defects and interruption stay separate from E. |
| `service`, `sync`, `fromOutcome` | Callback runs only when reached, anew per run. `service`/`sync` infer/check callback results at comptime. Outcome factories can transfer newly owning payloads. |
| `fromResult`, `fromOption` | Adapt explicit typed results/options; no hidden native error channel. Owning reusable values follow constructor restrictions. |
| `trySync`, `tryIo`, `tryNative` | Map declared native error unions inside the Effect boundary. `tryIo` uses R.io; native adapters suspend through injected I/O. They do not catch Zig panics. |
| `fromBorrowed`, `fromCancellableBorrowed` | Capture storage stays borrowed through execution and cleanup. Cancellable callbacks receive the active branch token. |
| `fromNativeBorrowed`, `fromNativeCancellableBorrowed`, `fromAsync`, `fromAsyncBorrowed` | Start a native operation when reached, retain a single join owner, forward cancellation without concurrently joining the native future, then destroy provider state. R supplies allocator/io. |
| `fromOwned`, `flatMapOwned` | Allocate runtime capture/continuation descriptions through R. `flatMapOwned` consumes its mutable source even on construction failure. Result descriptions are uniquely owned and reusable while alive. Pointer fields in capture records remain borrowed. |
| `borrow`, `deinit` | Borrowing does not transfer descriptor ownership. Description deinit destroys its originating allocation chain iteratively. Initial run descriptions are borrowed; descriptors returned by factories/continuations transfer unless explicitly borrowed. |
| `map`, `flatMap`, builder `bind` | Comptime callback/output inference. Bind callbacks accept `fn(A) Out` or `fn(A,*const R) Out`; environment-aware callbacks receive the current R automatically. Ordinary continuations run only after successful publication and cancellation checking. Error/environment mismatches require explicit adaptation. Static receivers are comptime-known; their runtime inputs need not be. |
| `catchAll`, `mapError`, `either`/`result`, `option`, `sandbox` | Recover only the selected channel. `mapError` maps typed errors including cleanup records. `sandbox` explicitly maps defects. Interruption/failed cleanup are not silently converted by expected-error recovery. Owning payloads are consumed or cloned according to the outcome contract. Conversion/copy OOM is a typed defect. |
| `zip`, `all`, `forEach`, `forEachDiscard` | Sequential, ordered evaluation; failure skips later work and releases already owned output. Input slices remain borrowed. A transform receiving a cloned owning input owns that clone. |
| `provide`, `provideSome` | Borrow a full R or project a larger environment. Full provision yields `Effect(A,E,void)` while retaining provided runtime capabilities. Inherited capabilities remain available to internal deferred work. Bound environment storage must outlive execution. |
| `EffectBuilder` | `succeed`, `fail`, `returnFrom`, `zero`, `delay`, `combine`, `whileLoop`, `forLoop`, `yieldEffect`, `tryFinally`, `bind`, `using` use the same kernel. Resource Bind installs ownership; Using of its ResourceValue reuses it. No public interpreter context or imitation F# syntax parser. |

`Exit` owns its cleanup chain and marked payloads. `clone` produces an independent
owner or a typed OOM defect while preserving the source. `observe` additionally
records whether copying succeeded, so a failed observation does not suppress
unobserved diagnostics. Every owning Exit copy must be deinitialized once.
Unmarked pointers/slices/application structures keep ordinary Zig ownership.

## Lifetimes, async, and execution

| Operations | Contract |
| --- | --- |
| `acquireRelease`, `resource`, `scopeAcquire`, `scoped` | Successful raw acquisition is owned before cancellation publication. Body callbacks may also accept `*const R`, including builder resource bindings. Body construction is skipped after late cancellation; protected release runs exactly once. Scoped acquisitions expose borrowed handles; owning payloads are held behind a resource pointer. |
| `Scope`, `closeScope` | Reverse registration release; shared close waits for one drain, including suspended cleanup. Registration into a closing scope releases its newly acquired resource. Cleanup storage is reserved before admission, preserving primary outcome and ordered failures without allocating during release. |
| `uninterruptible`, `onInterrupt` | Lexical protection/interrupt cleanup. Protected failure survives a later cancellation checkpoint. External work must still complete/join; masking is not permission to abandon it. |
| `run`, `runAsync`, `AsyncRun.join/request/deinit` | Run borrows the root description/environment. runAsync performs the immediate prefix, then owns its continuation. Request forwards cancellation; join returns an owning observation; deinit cancels/drains and destroys the handle. Serial ownership of join/deinit is required. |
| `fork`, `joinFiber`, `awaitFiberExit`, `interruptFiber` | Scope owns the child and captured environment/annotation storage. Observations are independently cloned; interrupted/failed-copy observers do not mark the child observed. Scope close reports unobserved failures and drains child work. |
| `dispatch`, `onCpu`, `onBlocking`, borrowed variants | The environment selects the executor. Borrowed variants carry runtime data separately from comptime callback/type metadata. Captures stay alive until join. Unsupported/saturated admission rejects before starting work. |
| `race`, `timeout`, `timeoutFrom` | Completion arbitration owns both branches. The winner publishes only after the loser is cancelled and drained; retained loser cleanup contributes to the outcome. Timeout uses the selected clock and an explicit typed error factory. |
| `allConcurrent`, `forEachConcurrent` | Explicit positive admission bound, completion-driven replenishment, ordered output. First terminal outcome stops admission and drains accepted work. Rejection after partial admission also joins admitted work. |
| `Executor`, `NativeIoExecutor` | Accepted callback runs once and has a single join/destroy owner. Request is nonblocking/thread-safe. Independent, CPU, and blocking capabilities are explicit. NativeIo requires genuine `std.Io.concurrent` progress and an explicit maximum_owned limit. No inline fallback. |

Provider/capture/environment objects must outlive all joined children and scope
cleanup. Caller allocators used concurrently must support that concurrency.
Pool shutdown waits for active leases; release/discard them or close their scopes
to let shutdown complete. Already completed lease bookkeeping can outlive pool
destruction and still close safely. A released lease value or disposed raw
resource pointer must never be reused. These contracts cannot be proved by Zig's type system.

## Application services and facilities

| Facility | Contract |
| --- | --- |
| `Layer.succeed/fromEffect/scoped/build/merge/provide/provideTo` | Typed service records/borrowed handles. Scoped services own their acquisition; partial merge/provision failures close constructed services in reverse order. Every build has fresh acquisition state. Runtime services remain ordinary R fields. |
| `Schema.fromDecoder/fromOutcomeDecoder/fromOptionDecoder/decode/parse` | Explicit typed JSON decoders. Decode borrows the supplied JSON value; parse returns `Parsed(A)`, owning its JSON document and any marked decoded payload. Document-backed views remain valid until owner destruction. Decoder errors must own data or borrow independent storage. Syntax/validation/defect channels remain distinct. |
| `Schedule.recurs/spaced/exponential/upTo/jitter/intersect/unfold` | Concrete state machines with comptime-known input/output/callback types. Durations, factors, samples, and overflow are validated. `retry`/`retryFrom` select ordinary typed failures; `repeat`/`repeatFrom` select success. Policy state is fresh per run. |
| `Clock`, `ManualClock`, `Random` | Injected time/randomness. ManualClock advances explicitly and removes interrupted sleepers. Native adapters use consumer I/O. Clock handles borrowing an I/O value require that value to stay alive. |
| `withSpan`/`withSpanFrom`, `annotate`, `log`/`logFrom`, `Tracer`, `Logger` | Lexical lineage/annotations restored after scope exit, carried across workers/fibers, and owned where storage escapes. Redaction occurs through the provider's iterator. Exporter/logger defects go to the diagnostic sink and do not replace application outcomes. Sink capacity/overflow is consumer policy. |
| `Deferred`, `EffectDeferred`, `Latch`, `EffectLatch` | Complete once; waiters own events and synchronized cancellation registrations. Completion owns an accepted Exit; rejected completion remains caller-owned. Poll/wait results are independently owning clones. Latch opens once. |
| `Ref`, `EffectRef` | Locked borrowed/copyable state, get/set/update/modify. Owning stored payloads are rejected without an ownership adapter. Modify infers result type and validates replacement type at comptime. |
| `Queue`, `EffectQueue` | Explicit bounded, unbounded (null), or rendezvous (zero) capacity. FIFO waiter transfer, backpressure, synchronized cancellation. A committed transfer wins over late cancellation. Offer consumes a value only on success; the Effect wrapper discards rejected factory-produced owning values. Shutdown optionally drains buffered values; nondraining shutdown disposes owned values. |
| `Stream.fromPull/fromIterable/fromAsyncIterable/open/mapEffect/runCollect/runForEach` | Scope owns acquisition/iterator release. Readers own pull descriptions; pulls return owned/borrowed chunks or end. Collect uses an explicit optional limit and releases partial results on failure. Async iterator operations are joined before iterator disposal. Repeated executions acquire fresh iterators. |
| `Fetch.request/text/fetchText`, `Http.NativeClient` | Scope owns acquired responses even after late cancellation or status rejection. Request/status acceptance/body limits are consumer policy. Response body/cache owners clone independently; native readers are cancelled/drained before disposal. Native HTTP uses the supplied std.http.Client, allocator/io, and transfer buffers. Text returns body bytes; no automatic charset transcoding. |
| `EffectWebSocket` | Typed native transport in R; limits and connect options selected from R. Send/receive serialization, UTF-8 validation and bounded fragment assembly. Close races handshake/clock and drains both; timeout retains handshake cleanup. Managed connection owns native abort/disposal, including failed initialization. Adapter abort must unblock outstanding operations. No bundled WebSocket protocol engine. |
| `Cache`, `EffectCache` | Captured environment/providers, explicit key retain/release/equality and failure-cache policy. Shared in-flight load; cancelling one observer leaves shared load alive. Explicit TTL/capacity/inflight admission. Invalidate/shutdown drain affected work and preserve unobserved cleanup; failed result copies do not suppress diagnostics. |
| `Pool`, `EffectPool` | Captured factory environment. FIFO borrowers, exclusive leases, validation/TTL on acquisition, explicit capacity and population target. Factories committed to borrowers participate in population observation. Resource disposal runs outside the ownership lock. Lease release/discard is idempotent; lease scope destroys bookkeeping. Shutdown waits for committed work and leases and retains diagnostics. |
| `Console`, `NativeConsole`, `EffectConsole` | Injected output/error writers; no process-global output destination installed. Native write failures stay inside typed Exit. |
| `WeakCache` | Unsupported GC ephemeron contract, rejected with a deliberate compiler diagnostic. No ordinary map is advertised as a weak cache. |
