# Zig Effect implementation audit

Updated: 2026-09-27. The implementation plan is complete. Native
verification is recorded below. This is an initial port, with bounded evidence;
module coverage does not assert full F# stress-suite parity or production
validation on every backend/target.

## Authority and delivery

Behavioral authority: `mandrill_net/effect`, revision `a1f874b`, rechecked with
no changes under its Effect directory. Plan:
`scratch/plans/effect-zig/implementation-plan.md`. Destination: `packages/effect`.
No unrelated workspace files, F# source, or Git metadata were changed.

The public signature is exactly `Effect(A,E,R)`. R owns application/runtime
services, including allocator, I/O, execution providers, clock/randomness, and
observability. Execution returns the five-channel typed Exit directly. Native
error unions, allocation failure, and provider rejection are adapted inside this
boundary. There is no public interpreter Context or separate allocator/io
argument on Effect methods.

Implementation modules:

| Module | Delivered responsibilities |
| --- | --- |
| `effect.zig`, `internal/runtime.zig` | Lazy descriptors, one iterative typed continuation kernel, ordinary/raw-exit binding, static specialization, runtime captures, provision, builder, resources, concurrency, layers, streams, and facility adapters |
| `outcome.zig` | Typed success/error/defect/interruption/finalization outcomes, ordered owning cleanup chains, marked payload lifecycle, safe independent observations |
| `scope.zig` | Acquisition ownership, protected reverse finalization, shared closure and late registration |
| `async.zig`, `executor.zig`, `internal/native_operation.zig` | Single-owner native future progress/join, nonblocking cancellation forwarding, explicit provider capabilities/admission, native I/O adapter |
| `schedule.zig`, `manual_clock.zig` | Concrete schedule state, retry/repeat support, deterministic cancellable time |
| `schema.zig`, `services.zig` | Typed JSON decode/parse owners, time/random/tracing/logging/redaction/console provider contracts |
| `primitives.zig` | Deferred, latch, inferred Ref.modify, bounded/unbounded/rendezvous queue ownership |
| `cache.zig`, `pool.zig` | Captured service lifetimes, shared loading, explicit policies, leases, population observation, cancellation/drain and shutdown diagnostics |
| `http.zig`, `websocket.zig` | Managed response/socket lifetimes, real std.http.Client adapter, typed socket transport, bounded body/messages and structured cleanup |
| `weak_cache.zig`, `internal/diagnostics.zig` | Explicit unsupported ephemeron contract and named presentation diagnostics |

The selected public contracts are in [api-contracts.md](api-contracts.md).
Examples are [examples/port.zig](../examples/port.zig) and the external consumer
[proofs/facilities.zig](../proofs/facilities.zig).

## Executed native evidence

Compiler: Zig 0.16.0. Target: aarch64 macOS. Native I/O comes from consumer
`std.process.Init.io` or Zig's native test runtime. The F# suite was inspected as
behavioral authority; it was not executed as a Zig parity suite.

```sh
zig build test example compile-fail fmt codegen provider-test --summary all
zig build test example compile-fail fmt provider-test -Doptimize=ReleaseSafe --summary all
```

Debug and ReleaseSafe each pass **85 tests**, the external example, formatting,
**16 intentional compiler rejections**, and leak-detection self-checks. Debug
also passes all **six ReleaseFast codegen comparisons**. The final builds
report 31/31 successful Debug steps and 26/26 successful ReleaseSafe steps.
The test target contains 13 module tests, one void-binding test, 14 runtime
binding/lifecycle tests, one boundary-value codegen test, 35 facility tests,
six concurrency-contract tests, three mixed-pipeline tests and 12 additional
lifecycle-contract tests. `provider-test` selects all four facility/provider
suites; `leak-check` can also run independently. Each native test binary runs
with Zig's standard runner inside a test-only 120-second subprocess watchdog.
Assertions, allocator leak reports and failing process statuses are preserved.

| Contract | Executed evidence |
| --- | --- |
| Three parameters/direct typed Exit | External consumers compose declared E/R and switch all five outcomes; invalid error/environment/callback/finalizer combinations reject |
| Laziness and runtime injection | Static descriptions call no user work at construction; reruns use distinct environments and injected allocator/io; provider snapshots preserve this run's provider identity |
| Provision | Full and projected environments retain runtime capabilities, including cancellation and deferred runtime-owned binding under void |
| Static specialization | Map, unary/environment-aware binds, two/eight binds and typed recovery match the complete optimized handwritten LLVM bodies through the ordinary API |
| Runtime ownership/depth | 100,000 owned binds execute repeatedly and destroy iteratively; 100,000 deferred steps and 10,000 nested resource brackets avoid recursive execution/destruction overflow |
| Acquisitions/cleanup | Late successful acquisition owns before cancellation; body is skipped and release runs once; reverse release and ordered typed cleanup survive frame-admission OOM |
| Scopes and protected work | Two asynchronous close callers stay pending until shared protected virtual-clock cleanup completes; timeout cancels and drains sleepers before publishing |
| Native async | runAsync suspends native waits, forwards cancellation, drains native futures, and removes interrupted virtual sleepers; typed native error adapters also compile and execute |
| Parallelism/provider composition | Four synchronous bodies run on four distinct OS threads; two synchronous racers both cross a barrier. An external executor has no I/O field. CPU/blocking captures select independent engines; nested saturated admission rejects without starting child work. Fifty inputs replenish as individual workers finish, preserve order and respect bound two. |
| Partial rejection | A later rejected submission drains accepted work; provider ownership counters end at zero |
| Fibers | Scope-owned child execution and independently owning observations; observation-copy failure preserves the original owning outcome; unobserved protected child failure is retained on close |
| Layers/schema | Typed provisioning, merge failure releases partial services, chained layers; parse owns document views, validates typed syntax/shape failures, and sweeps allocation failures; borrowed option decode is exercised |
| Scheduling | Fresh recurrence state, selected outcome channels, virtual retry/repeat timing, backoff/jitter validation, elapsed bounds and custom concrete state |
| Tracing/logging | Span parentage across workers, annotation restoration/redaction, exporter/logger isolation and diagnostics; 10,000 nested spans preserve lineage, close in reverse order and return allocated storage to zero |
| Primitives | Scoped deferred, inferred atomic modification, rendezvous queue, owned buffered-value disposal on shutdown, latch completion once |
| Streams | Synchronous/asynchronous iterators, owned mapped chunks, collect limits and independent reruns; typed mapper failure retains disposal failure. Cancellation joins a pending async move and waits for protected native disposal before publishing. |
| HTTP | Managed status/body/transport cleanup and a real loopback std.http.Server/std.http.Client request through supplied allocator/io |
| WebSocket | Fragmented UTF-8 assembly, native disposal; virtual close timeout aborts/drains the handshake and retains its cleanup failure |
| Cache/pool | Shared inflight load survives one observer's cancellation; TTL begins at settle, LRU evicts settled entries, and reentrant clock/equality closure/invalidation is rechecked. FIFO pool waiters skip cancellation; pending disposal retains capacity; stale disposition cannot retire a later lease; validation/cleanup failures survive shared shutdown. Population observation, lease bookkeeping lifetime and allocation-position sweeps also pass. |
| Integrated application | Schema + scoped services + tracing + bounded CPU/blocking work + retry; invalid input starts no worker and valid work completes cleanup/spans |
| Required mixed pipeline | Native file fetch -> CPU parse -> bounded concurrent native file lookup + CPU score + blocking file lookup -> join -> native file save. Three distinct engines compose through R. Success, typed branch failure, malformed input and parent cancellation all release the session and join admitted work. |
| Race arbitration/fault retention | A typed failure wins against a waiting competitor; loser release, cancellation-request and every join defect remain ordered cleanup records, and all work finishes before publication |

Negative consumers are compiled as executables and must fail for their intended
diagnostic. They cover wrong A/E/R, allocator/finalizer types, missing runtime
capabilities, const ownership transfer, reusable owning values, owning layer
services, invalid Ref replacement type, mutable/wrong environment continuation,
WeakCache, and incomplete Exit handling.
No unrelated import/build error is accepted as a passing rejection.

## Comptime, UX, and costs

Known callbacks, type relationships, field projections, adapters, schedule state
layouts, resource/finalizer types, and callback outputs specialize at comptime.
`map`, `flatMap`, builder binding, and Ref.modify infer redundant result types.
Resource-aware Bind/Using share acquisition ownership. Loop/finally forms use
one kernel and preserve its cancellation/cleanup semantics.

`flatMap`, `flatMapOwned`, acquire/release bodies and resource builder Bind accept
either `fn(A) Out` or `fn(A,*const R) Out`. This removes the need to duplicate R
inside successful application values when constructing a dependent owning
description. Parameter validation and adapter selection happen at comptime;
the environment-aware static path also folds to the handwritten instructions.

The builder consumer and explicit flatMap composition select the same typed
continuations. Consumer examples do not provide execution state or duplicate
runtime services. Runtime input is carried separately from static metadata:
borrowed leaf/CPU/blocking captures need no capture allocation; fromOwned and
flatMapOwned allocate uniquely owned runtime description nodes through R.
Those nodes and runtime/native continuation frames have explicit lifecycle
costs. Static composition has no alternate public optimization API.

Optimized LLVM entry-body instruction counts from the installed compiler:

| Proof | Effect instructions | Handwritten instructions |
| --- | --- | --- |
| map / unary bind / environment-aware bind / two binds / eight binds | 2 each | 2 each |
| typed recovery | 5 | 5 |

The gate compares entire normalized instruction bodies, not only these counts.
A failing allocator separately checks static composition admission. These are
representative static-path proofs; they do not imply zero work/allocations at
native, dynamic description, resource, collection, or observation boundaries.

One uncached native run reported ~7 s / 792 MB maximum RSS compiling the external
Debug facility suite and ~30 s / 1 GB compiling ReleaseSafe. The ReleaseFast
codegen object took ~2 s. The final ReleaseSafe facility compile reported
31 s / ~1 GB RSS; the additional lifecycle suite reported 9 s / 392 MB RSS.
The final emitted assembly/IR text sizes are 221,266 / 272,012 bytes;
these are whole proof-artifact text sizes, not executable size or per-operation
costs. Numbers depend on compiler/host/cache and are recorded as observations,
not budgets or tuning policies.

### Sustained mixed-workload measurement

The executable fixture uses seed `0xeffec7`, 64 fresh requests, real temporary
file operations, three distinct OS-thread executors, and explicit traversal
bound two. The CPU/blocking providers do not implement I/O. The measured interval
includes request preparation, execution and result verification; these are
integration-fixture rates, not a production throughput promise or speed gate.

| Measurement | Debug | ReleaseSafe |
| --- | --- | --- |
| Elapsed / completed runs per second | 172,324,125 ns / 371.4 | 36,821,167 ns / 1,738.1 |
| Allocations per complete pipeline run | 28 | 28 |
| Peak live allocations / bytes | 18 / 7,552 | 18 / 7,552 |
| Peak owned jobs: async / CPU / blocking | 3 / 1 / 1 | 3 / 1 / 1 |
| Started / completed / joined jobs | 384 / 384 / 384 | 384 / 384 / 384 |
| After each run: live allocations, bytes, jobs, cancellation registrations | 0 | 0 |

The async count includes the root and two traversal workers; active application
queries peak at two. These providers reject saturation instead of accumulating
an internal work queue. Admission rejection and FIFO queued borrowers are tested
separately. Results are consumed before releasing the scoped session. Retained
cleanup chains have explicit Exit ownership and are destroyed by the deep
resource/outcome tests, including allocation-failure paths.

Static map/bind construction and execution require zero allocations in the
rejecting-allocator proofs. Borrowed captures allocate no description box.
Runtime `fromOwned` and `flatMapOwned` reserve one concrete typed description
node each; the mixed workflow creates three such dependent binding nodes.
Resource/async/provider frames, output arrays and native handles are actual
runtime ownership boundaries and are included in the measured 28 allocations.

## Leak and failure guarantees exercised

Allocating tests use `std.testing.allocator`, directly or beneath tracking and
failing allocators. Zig's standard test runner checks every test for leaks.
Allocation-free proof paths use a rejecting allocator. Deep descriptions,
retained metadata, resource admission, schema parsing, cache/pool construction,
and lifecycle cases include failure sweeps/repeated runs and complete teardown.
The concurrent failure allocator wrapper serializes bookkeeping while preserving
the leak-detecting backing allocator.

The test target also runs isolated clean and deliberately leaking fixtures in
Debug and ReleaseSafe. Assertions pass in both fixtures; only the deliberate
leak must fail its native build with the intended leak diagnostic. Thus passing
assertions cannot hide a leak. Native handle counts, resource counters, cancelled
sleepers and bounded active-worker counts supplement allocated/freed byte totals.

Cleanup ownership is reserved before admission. Release bodies that may wait
suspend through the existing kernel without allocating a new cleanup task.
Cache/pool manager metadata is reserved before work; unobserved shutdown results
move into retained diagnostics rather than dropping them on observer-copy OOM.
A failed observer copy remains a defect for that observer and does not mark the
source failure observed. WebSocket timeout transfers the losing cleanup chain
without a new allocation.

## Values, policies, and deliberate adaptations

Production tuning choices are consumer policy: engine admission, traversal
concurrency, collection limits, queue capacity, cache capacity/TTL/inflight/key
ownership/error caching, pool capacity/idle lifetime, HTTP transfer/body limits,
and WebSocket buffer/message/close timeout. Null/zero modes have documented
semantics rather than substituted tuning defaults.

Structural storage sizes derive from types and outcome shapes. Cleanup prefixes
reserve the primary/first-cleanup contribution; remaining links already exist.
Reference counts start with their actual owner; empty state/count and identity
transformations follow their mathematical/lifecycle contracts. Overflow uses
native type bounds and checked arithmetic. Protocol/field identifiers are the
native URI/HTTP/socket or typed-environment contracts. Dispatch uses enums/types;
diagnostic strings live in a named presentation catalog. Test/example literals
are illustrative input, not production defaults.

Omitted clock services adapt the supplied I/O's awake/monotonic clock, with the
I/O value owned/borrowed at the corresponding lifetime. No global allocator,
I/O backend, worker runtime, or telemetry sink is installed. Empty annotations
and absent optional observability preserve the application's ordinary result.

Deliberate Zig adaptations and evidence boundaries:

- Zig does not catch panics as F# exceptions. Native error-union adapters and
  explicit decoder/defect factories provide typed outcomes.
- Owning application payloads opt into a checked clone/deinit marker contract;
  arbitrary unmarked user aggregates retain Zig ownership rules. Zig cannot
  enforce unique moves, borrowed pointer lifetimes, allocator thread safety,
  or correct provider callbacks. Internal struct fields are unsupported API,
  although Zig cannot hide every field the way F# sealed/private types can.
- Independent Exit observations clone owning data/cleanup. Copy or typed-error
  conversion can return OOM; type-changing conversion cannot retain unmappable
  source E inside target F. Original outcomes are preserved on observation OOM.
- Managed HTTP text returns owned bytes without automatic charset transcoding.
  Native WebSocket behavior is supplied through the typed transport interface;
  no new protocol engine or fixed external runtime is bundled.
- WeakCache is deliberately unsupported because GC ephemeron ownership is absent.
- No cross-target runtime claim, exhaustive source stress-suite claim, telemetry
  SDK integration claim, or production mixed-I/O/CPU throughput claim is made.
  Broader release validation should use these contracts and executable gates;
  it does not require restarting the completed implementation work.

## Completion audit against the implementation plan

The required implementation stages and facilities are complete. The following
maps the plan's gates to concrete source and executed evidence; the plan's
original requirements remain the authority. Compatibility claims stay limited
to the installed compiler, target and tested consumer providers above.

| Plan gate | Delivered source and evidence |
| --- | --- |
| Representation, authoring and lifetime feasibility | `effect.zig`, `internal/runtime.zig`, `outcome.zig`; external example, runtime-binding/provision/capture tests, valid resource builder and paired compiler rejections |
| Sequential kernel | One iterative Step/typed continuation driver; 100,000 binds/deferred iterations, fresh reruns, typed recovery, cancellation checkpoints and originating allocator destruction |
| First-brick native I/O | Native/error adapters, `async.zig`, `internal/native_operation.zig`; supplied I/O recording, native wait cancellation, single native-handle owner, real loopback HTTP and mixed file pipeline |
| Resources/scopes | `scope.zig` and generated resource frames; 10,000 brackets, admission OOM sweeps, late acquisition, protected cleanup, reverse release, shared close, closed-scope registration and unobserved child failure |
| Structured concurrency | `executor.zig`, scoped Fiber/race/traversal adapters; four synchronous workers, two racers, fifty ordered replenished inputs, partial rejection, nested saturation, retained request/join/loser cleanup faults |
| Independent runtime providers | One R with allocator, I/O and three distinct external engines; the executor itself needs no I/O service. Pipeline success/failure/cancellation joins all admitted work; no exclusive runtime mode or hidden backend |
| Dependency layers | Generated typed layer builds/merge/provision; fresh builds, partial-construction cleanup, reverse release and scoped integrated application |
| Schema | `schema.zig`; typed syntax/validation/defect results, explicit Parsed owner, borrowed decoding, allocation failures and invalid-input short circuit |
| Schedule/time/randomness | Concrete generic schedule state, `manual_clock.zig`, service adapters; fresh retry/repeat state, selected channels, virtual sleep cancellation, custom state, intersection/backoff/jitter/overflow validation |
| Tracing/logging | Typed provider callbacks and lexical generated adapters; worker lineage, redaction/restoration, exporter isolation and 10,000 nested spans with complete teardown |
| Deferred/latch/ref/queue | `primitives.zig` and scoped adapters; completion/observation ownership, failed-copy retention, cancellable waiters, inferred Ref results, rendezvous and owned buffer disposal |
| Streams | Generated typed readers/chunks/iterators; sync/async acquisition, collection limits, reusable mapped owned chunks, mapper/disposal failure retention, pending move cancellation and joined protected disposal |
| Managed HTTP/WebSocket | `http.zig`, `websocket.zig` and typed Effect adapters; response ownership, native loopback request, status/body limits, UTF-8 fragment assembly and drained close timeout retaining cleanup |
| Cache/pool | `cache.zig`, `pool.zig`; shared load/population, TTL/LRU, reentrant callbacks, observer cancellation, FIFO leases, disposal capacity, shared shutdown, validation/cleanup retention and failure sweeps |
| WeakCache decision | Explicit compiler rejection of unavailable GC ephemeron semantics, as permitted by the plan; no ordinary owning map masquerades as a weak cache |
| Comptime and consumer UX | Concrete A/E/R/frame/callback/schedule/decoder types; inferred outputs, automatic resource ownership, environment-aware continuation form, 16 targeted negative consumers and six complete optimized-body comparisons |
| Bounded resource/performance evidence | Static allocation rejection, dynamic node failure sweeps, sustained 64-run mixed workload with per-run allocations, peak live bytes/jobs, no queued provider backlog and zero retained run storage/registrations/jobs |
| Leak/test infrastructure | Native Debug + ReleaseSafe standard runners, backed tracking/failure allocators, deliberate-leak self-checks, full resource/provider teardown and bounded subprocess risk probes |
| Documentation/encapsulation | Supported public API is `root.zig`; contracts, examples, source adaptations and costs are documented. Consumers provide R and handle Exit, without driving interpreter state. |

### Comptime and production value provenance by facility

| Facility | Compile-time work | Runtime data/policy and structural justification |
| --- | --- | --- |
| Core/Exit/builder | Success/error/environment types, typed callback arity/results, continuation and capture layouts, provision projections | Byte-sized outcome/delivery discriminants are native representation units; cleanup prefixes derive from the exact primary/first-record tuple. Static composition folds away; genuinely runtime descriptions own one concrete node. |
| Resources/async/executors | Finalizer types, frame fields and provider callback adapters | Resource/native lifetime and cancellation coordination need runtime state. Provider capabilities and admission limits come from R. No global allocator, I/O implementation or worker policy is installed. |
| Layers/schema | Dependency record/result types, compatibility checks and decoder adapters | Acquisitions/document/input storage are real runtime data. Schema field names are consumer declarations; HTTP/JSON identifiers follow their protocol contracts. |
| Scheduling/observability | Concrete schedule state and known randomness selection, typed lexical annotation/span/log adapters | Delays/factors/recurrences/samples and sink retention are explicit consumer policy. Clock fallback uses the supplied I/O's awake clock. Span names/messages are supplied presentation data. |
| Primitives/streams | Stored/element/error types, inferred Ref result/replacement type, iterator/transform frame types | Capacity, backpressure and collect limits are explicit. Empty/zero/rendezvous states, owner counts and singleton tuple sizes have lifecycle/type semantics; no generous fixed production buffer is selected. |
| HTTP/WebSocket/cache/pool | Typed transport/key/value/resource callbacks and exact ownership metadata | Transfer/message/key/TTL/inflight/pool limits and failure caching are explicit policies. UTF-8/protocol bounds come from native contracts, arithmetic uses native checked limits, and dispatch uses typed operations. |
| Diagnostics/WeakCache | Named diagnostic catalog and deliberate unsupported compiler contract | Diagnostic text never selects execution behavior. Unsupported ephemeron ownership is rejected rather than replaced by an arbitrary runtime fallback. |

Example/test values, workload seed, barrier participant counts and subprocess
watchdog duration are fixture inputs. They are not production defaults.
