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
| `Stream.fromPull/fromIterable/fromAsyncIterable/fromAsyncIterableEffect/open/mapEffect/runCollect/runForEach` | Scope owns acquisition/iterator release. Readers own pull descriptions; pulls return owned/borrowed chunks or end. Collect uses an explicit optional limit and releases partial results on failure. Async iterator operations are joined before iterator disposal. Repeated executions acquire fresh iterators. |
| `Fetch.request/text/fetchText`, `Http.NativeClient` | Scope owns acquired responses even after late cancellation or status rejection. Request/status acceptance/body limits are consumer policy. Response body/cache owners clone independently; native readers are cancelled/drained before disposal. Native HTTP uses the supplied std.http.Client, allocator/io, and transfer buffers. Text returns body bytes; no automatic charset transcoding. |
| `EffectWebSocket` | Typed native transport in R; limits and connect options selected from R. Send/receive serialization, UTF-8 validation and bounded fragment assembly. Close races handshake/clock and drains both; timeout retains handshake cleanup. Managed connection owns native abort/disposal, including failed initialization. Adapter abort must unblock outstanding operations. No bundled WebSocket protocol engine. |
| `Cache`, `EffectCache` | Captured environment/providers, explicit key retain/release/equality and failure-cache policy. Shared in-flight load; cancelling one observer leaves shared load alive. Explicit TTL/capacity/inflight admission. Invalidate/shutdown drain affected work and preserve unobserved cleanup; failed result copies do not suppress diagnostics. |
| `Pool`, `EffectPool` | Captured factory environment. FIFO borrowers, exclusive leases, validation/TTL on acquisition, explicit capacity and population target. Factories committed to borrowers participate in population observation. Resource disposal runs outside the ownership lock. Lease release/discard is idempotent; lease scope destroys bookkeeping. Shutdown waits for committed work and leases and retains diagnostics. |
| `Console`, `NativeConsole`, `EffectConsole` | Injected output/error writers; no process-global output destination installed. Native write failures stay inside typed Exit. |
| `WeakCache` | Unsupported GC ephemeron contract, rejected with a deliberate compiler diagnostic. No ordinary map is advertised as a weak cache. |

## Reference-update capabilities

The [2026-10-06 reference update](reference-update-2026-10-06.md) records the
fixed F1–F8, T1–T10 and Z1–Z5 dispositions and verification results.

| Operations | Contract |
| --- | --- |
| `Codec.validate`, `DomainCodec.validate`, `PureCodec.validateDirect` | Execute encoding once, discard successful representations once, and preserve the encoding environment and typed error. Pure validation needs no Effect frame. |
| `requirePure`, `JsonCodec.requireWriter` | Require actual public capability witnesses. AST shape, method names and service-free type names alone are insufficient. Direct writing is preserved only by implementations that provide it. |
| `NativeRecordBuilder`, `NativeField`, `NativeCodec.sequence/array/map/option` | Representation-generic construction with separate direction environments. Required/optional/defaulted fields are checked against named Zig metadata. Default representations decode only when absent. Borrowed default outputs must own their storage. Child effects run on the shared trampoline; no nested run or JSON conversion is required. Map contracts own their entry/uniqueness semantics. |
| `DomainCodec.ofSchema/optional/array/map` | Preserve application errors with independently supplied decode/encode issue mappings. Mapping callbacks own their input issues. Pure schema mappings and their optional compositions keep checked direct execution. |
| `JsonNumberCodec.exact`, `JsonConstraints`, `JsonAlternatives` | Exact decimal tokens, Unicode-scalar limits, explicit bounded pattern capabilities, enum unknown policies, prefix/rest and unique arrays, property-name/count/pattern/additional checks. Every matching object pattern applies. oneOf requires one success; anyOf retains original representation and typed matches; allOf merges compatible encodings and rejects conflicts. |
| `JsonConstraints.withLimits`, `JsonWriter` | Positive independent output-byte, JSON-depth, value-byte and integer-digit limits. The structural adapter is an explicit generic fallback; the writer capability uses the supported direct path. Output owns the caller allocator. |
| `SchemaIssueCodec` | Versioned owning issue envelopes with owned field/property/alternative paths, constraint operands, match indices, limit details and related failures. Caller limits bound input bytes, JSON depth, issue depth/count and output. |
| `DependencyCache` | Scoped finite retention and explicit positive in-flight/dependency limits; typed capacity failures, generation-selection observations, nonloading tryGet, copied replacements/dependencies and conservative invalidation of unfinished generations. Policy callbacks and cancel/join execute outside cache locks. Borrowed inputs remain alive until join. |
| `CallbackPubSub` | Finite capacity and nonblocking overflow policies only. A boolean reports source acceptance, not subscriber delivery. Release subscriptions before shutdown/deinit. |
| `EventReplicaRemote.bindAuthorized` | Authorize the borrowed identity before invoking session binding. Rejection skips binding; identity and provider capabilities remain alive through join. |

A caller may supply an arena with a thread-safe backing allocator for concurrent allocation. Zig 0.17 arena allocation supports concurrent callers when its backing allocator is thread-safe. Reset and deinit are exclusive lifetime operations: perform them only after every Effect, I/O task, executor callback, provider and result borrowing or owning that arena has joined and been released.

`JsonSurface.Limits` separates UTF-8 input bytes/value bytes/JSON depth, schema preparation nodes/edges/depth, issue count, integer digits, and output writer limits. `withLimits` with this selector exposes `parseBytesDirect` and `parseBytes` returning an owning `Parsed(A)`. Writer-only selectors retain the DOM adapter and do not claim an input-document byte budget. Direct writer evidence selects tree-free record and variant encoding; schemas without that evidence use checked representation fallback.

`WorkflowTypes.ResumeRequest` carries `expected_revision`; providers compare it with the stored checkpoint revision atomically before changing suspended/running state. Poll returns the observed revision in those states. `resumeExecution` observes then submits the fenced request; custom callers can use `resumeChecked`. Cluster resume commands carry the same request. For a failed-run retry, pass the observed completed reply bytes as `expected_terminal_reply` and its `revision`; the provider checks both atomically and removes that terminal checkpoint before queuing. Plain resume never replays a completed run. Previously recorded activities remain available for typed replay. A stale resume returns `stale_revision` without mutation.

RPC normal EOF calls `StreamRegistry.stopIntake`, preserving accepted frames for draining and rejecting new registrations. Receive returns `end_of_input` so the driver stops reading. Abnormal close and scope release discard unread buffered frames and retain existing cancellation behavior.

Known primitive JSON codecs carry explicit encoded-shape pointers. `JsonCodec.refine` and `transform` retain their child shape and named executable checks; `asCodec` and domain issue mappings retain that same immutable metadata. Exact JSON Schema export rejects checks with unknown portable semantics; conservative export keeps the underlying shape. Arbitrary callbacks receive no inferred shape witness.


JSON parsing uses Zig 0.17 `std.json.Scanner` and `parseFromSlice`; bounded writing
uses `std.json.Stringify` over a checked `std.Io.Writer` sink. Effect keeps the
caller’s byte/depth/digit policies, typed issues, owned outputs, exact decimals and
schema-to-JSON-Schema translation. It does not provide a second JSON syntax or
escaping implementation. Container-local std emitters preserve caller depths
above std’s fixed 256-level debug grammar stack. Standard short control escapes
and finite-float spelling may differ while denoting the same JSON value; exact
`number_string` tokens are retained verbatim.

Source API comments document units, inclusive bounds, borrowing, destruction,
direction environments and callback ownership. Complete public-module examples
remain in `proofs/reference-update.zig` and `tests/capabilities/consumer/src/main.zig`.

## Inferred composition and domain adapters

| Entry | Contract |
| --- | --- |
| `dsl.program.captured(env, inputs, nodes)` | Moves one runtime input record into the existing capture owner. Reserved binding `inputs` provides its inferred pointer to callbacks. Native-only descriptions reuse immediate lowering; effectful descriptions have independent initialized slots per run. Resources register with the existing Scope. No capture callback runs at construction. Construction failure consumes initialized inputs. The description remains alive through joined execution. |
| `dsl.streams.from/borrowed/scoped` | Infer item/resource/E/R from exact Effect roles. Scope is the acquisition/release authority. Borrowed configuration retains original pointer qualifiers and its caller lifetime; scoped configuration is disposed after reader work and protected cleanup. Returned Effect bodies remain ordinary composition. |
| `dsl.streams.append/bind/branch/yieldEffect/unfold/loop` | Exact compatible Stream descriptions only. Children open on demand, each reader has independent state, and an exhausted child closes before the next opens. Cancellation drains pending pulls before release. Native loop conditions/advances are explicit; effectful selection uses unfold. |
| `dsl.streams.map/mapError/project/consume` | Capture configuration once for the body's runtime stream. Error mapping includes source scope cleanup failures. Projection changes the required execution environment explicitly; it does not replace the source resource owner. Consume delegates to fold and preserves early stop/close. |
| `SinkBuilder.fold(...).runWith(env, source)` | Runtime source description is retained by the generated scope. State, pending transitions, final-result transfers and unconsumed chunk contents follow the same native fold authority as static run. |
| `TransactionBuilder.context(E)` | E is the unresolved domain anchor. Ref value types come from canonical cell handles, and map/bind/zip/orElse retain the native journal and rollback/retry boundary. Atomic execution returns the exact existing Effect. Native callback purity and source pointer validity remain native obligations. |
| `dsl.activation.acquire/use/bind/bindCaptured/branch/run` | Only exact generated activation plans compose. Native resource providers, token/fence and scope remain authoritative. Check token generation before acquisition/use; each activation creates fresh resources. Native provider mutations still fence their commit. The fence guard is not held during user work. Partial/late acquisition and primary-plus-release failures use existing protected cleanup. Known ResourceValue wrappers cannot escape as root results or persisted data. |
| `dsl.entity/entityDefinition/event/eventDefinition` | Codec evidence determines application payload/result/error channels. Handler signatures determine R, with an explicit anchor only for missing evidence. Native definition version and identity validators remain authoritative. |
| `dsl.eventCase/events.caseWith/projection` | Cases associate one completed native event with its pure `void`/`Result(void,F)` fold. Duplicate/foreign cases and incompatible state/fold types reject at construction. Contextual decoders require their own directional environment projection when R differs. Unknown JSON duplicate policy needs an explicit policy; conflicting codec metadata rejects. |
| `projection.replayPage` | Borrows input state/page until join, clones private state, and decodes/folds in native cursor order. Suspended decoding retains its parse document. Failure/interruption disposes partial state without publishing a cursor. Atomic checkpoint publication remains a distinct native provider operation. |
| `dsl.persistence.payloadCodec/from/scoped/declared` | Channels derive from exact native codec callbacks; decode and encode environments stay independent. Version, limits, PrimaryKey bytes and expiry are explicit application policy. Scoped keys/configuration outlive native provider work; outcome ownership and write uncertainty stay native. |
| `dsl.queues.scoped/declared` | Requires canonical PersistedQueue and PureSchemaBinary evidence, native guarantee, explicit key/offer identity, visibility, retention, owner/generation and claim policy. Offer/claim/decode/retry/acknowledge return exact Effects. Decoded results retain claimed bytes through payload cleanup and final shared-owner disposal, including OOM. Payload code owns its declared resources; arbitrary unmarked native pointers keep their original lifetime obligations. |

Every effectful boundary above returns the exact existing `Effect(A,E,R)`.
Descriptors, pure codecs/folds, and definitions remain inert/pure domain values.
No adapter inserts a root evaluation to connect domains. Native root runners and
provider/domain-owned child execution retain their existing execution contracts.

The executable `examples/dsl` recipes cover native file/CPU/blocking work,
structured orders, scoped cache/fold consumption, managed unary/streaming RPC,
recorded business-key workflow traversal, transactional withdrawal, tracked
prices, and activation-owned projection checkpoints. `composition.zig` connects
these existing Effect boundaries with explicit error/environment adapters. It
keeps RPC consumption inside the activation bracket through checkpoint and
reader completion. Each recipe handles the root Exit and releases its result.

`WorkflowBuilder.forEach(env, inputs, roles)` admits only the builder's Step or
OwnedStep returned by `body`. Its explicit name/version/execution and selected
business key determine replay identity, independently of array reorder or
allocation address. Static duplicate keys are compilation errors; dynamic keys
are validated before any child is admitted. `activityCaptured` infers the native
engine, activity channels and recording codec from its roles. Both return a
consuming OwnedStep; `intoEffect` is their ordinary Effect boundary. Raw Effects
remain confined to native recorded activity handlers. Providers still determine
volatility/recoverability, retry attempts, compensation order, and uncertainty.

Known `single_thread` retained captures cannot enter concurrent traversal. Types
that expose retained configuration reject at compilation. Effect descriptions
retain this evidence in their existing owner chain after type erasure and reject
at runtime before worker admission. This check does not infer synchronization
for arbitrary borrowed pointers or prove behavior of native callback code.
