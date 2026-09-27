# Source contracts and implementation status

Behavioral reference: `mandrill_net/effect`, revision
`a1f874b7e993d77472ff9f5c992ea73653e81a28`. The source subtree was clean when
rechecked during this implementation. Status describes the current Rust API;
it is not a claim of complete source parity or production readiness.

| Source contract | Rust API / status | Evidence |
| --- | --- | --- |
| Types.fs: typed outcomes and cleanup failures | Implemented: `Exit`, `PrimaryOutcome`, `CleanupFailure`, `Finalization`, opaque `Defect` | `core::error_mapping_preserves_every_finalization_error`; primary/first/rest representation retains ordered nonempty cleanup failures |
| Interpreter.fs: lazy reusable description, typed trampoline | Implemented with reusable Arc recipes and typed FnOnce continuations per run | `core::reusable_macro_moves_non_clone_values_and_defers_all_statements`; `depth::deep_graphs_in_subprocess` |
| Effect.fsi: succeed/fail/die/sync/defer | Implemented: `succeed`, `fail`, `fail_with`, `die`, `sync`, `defer` | `core`, `async_boundary`, and `depth`; stored values have documented cloning bounds |
| Effect.fsi: trySync and task/result adapters | Adapted: `try_sync`, `from_future`, `try_future`; Rust Result errors are explicitly mapped, panics remain defects | `async_boundary`: fresh factories, inline sync, Result mapping, factory/poll/continuation panic capture |
| Combinators.fs: map/flatMap/zip/all/forEach | Implemented: `map`, `flat_map`, `zip`, `all`, `for_each`; sequential and short-circuiting | `core::sequential_traversal_is_ordered_lazy_and_short_circuits_callback_creation`; deep bind/defer cases |
| Combinators.fs: expected-error recovery and conversions | Implemented: `map_error`, `catch_all`, `either`/`result`, `option`, `sandbox`, `from_result`, `from_option` | `core` outcome tests; external typing fixture rejects unmapped error families |
| Combinators.fs: service/provide/provideSome | Implemented: `service`, `provide`, `provide_some` | `core::typed_environment_and_projections`; compiler fixtures reject wrong environment/projection |
| EffectBuilder.fs: Return/Bind/Delay/Zero/Combine | Adapted: procedural `effect!` with Rust statements, whole-step Effect `?`, tail values, top-level returns, patterns, and unit blocks | `core` macro tests and `typing` renamed external consumer; safe opaque workflow bridge |
| EffectBuilder.fs value-producing branches | Adapted: `if`/`else if`, `match` with patterns/guards, and unlabeled blocks | `branches` covers non-Clone values, selected-branch laziness, failure short circuit, resource release, pure Rust mutation, and branch defects; external compiler cases cover supported syntax and accepted nested returns/assignments plus `macro_control` loop/resource cases |
| EffectBuilder.fs While / sequential iteration | Adapted: `while_loop`; existing `for_each` handles traversal | `iteration` covers lazy guards/factories, expected errors, cancellation, reruns, and panic boundaries; subprocess executes 100,000 iterations; `macro_control` proves loop syntax, labeled exits, local ownership and cancellation |
| Interpreter.fs: cancellation checkpoints and raw/publication distinction | Implemented for sequential execution and cooperative futures | `core` skip tests; `async_boundary` settlement test; `environments` root-token preservation and independent runs |
| Interpreter.fs: runTask/runTaskWith | Adapted: lazy Future-returning `run` and `run_cancellable`; future owns recipe/environment independently of description borrow; sole external configuration is R, cancellable boundary requires HasCancellation | Portable fixtures; compiler fixtures reject missing HasCancellation and accept futures outliving local/borrowed descriptions; lazy accessor, panic and cross-thread queue race tests |
| Runtime cancellation notifications | Adapted to executor-independent broadcast Cancellation | 32-waiter/late-waiter test and 256 request/registration races with explicit wake observations |
| Deep descriptions and continuation teardown | Rust-specific iterative destruction; no interpretation-time Any success/error downcasts or unsafe value stack | Subprocess tests for 100,000 binds/defer/error mappings, early failure, pre-cancel, unrun/abandoned graphs, capture counts, and TLS teardown |
| Cooperative executor handoff | Rust adaptation: return control between typed transitions using standard wakeups; no arbitrary batching quantum | `core::cooperative_yield_allows_sibling_progress` observes sibling progress before a ready chain completes; tests await explicit entry/suspension; CPU-time bounds and scheduler fairness are not promised |
| No magic values / Rust defect identities | Rust adaptation: `DefectKind` distinguishes custom diagnostics, caught panics, and typed `LibraryDefect`; broadcast and package lookup follow documented contracts | `defects` verifies message impersonation is impossible, non-string unwind classification, consumed observers, and identity retention through finalization/error mapping; `effect-macros` source-policy tests inventory production literals and detect reintroduced budgets, generated literals, string state, and string-only defects; external `defect_kind` consumer |
| Interpreter.fs protected execution and Bracket.fs finalization | Implemented: `uninterruptible`, `on_interrupt`, `ensuring`, `ensuring_with`, ordered cleanup accumulation | `protection` tests cover pre-cancelled protected work, masked continuation, awaited finalizers, borrowed non-Clone primary errors, and release defects |
| Resource.fs / Bracket.fs acquisition ownership | Adapted: `ResourceBinding`, invalidatable `ResourceValue`, single-use `Release`, borrowed `PrimaryOutcomeRef` | `resources`: lazy reruns, late success before cancellation, reverse release order, awaited masking, body/release factory defects, observer loss, and closed handles; typing rejects escaping references; depth tests include 10,000 nested brackets |
| EffectBuilder.fs resource-aware Bind | Implemented through safe sealed macro dispatch for Effect/ResourceBinding | `resources::macro_resource_bindings_scope_the_remaining_workflow_and_release_in_reverse_order`; external `resource_macro` compiler fixture |
| Rust observer/driver ownership | Adapted: `owned_run` / `owned_run_cancellable`, `RunDriver` / `RunObserver` | `owned_run` tests cover non-Clone/non-Sync results, late observation, observer loss, and owner-loss defects |
| Scope.fs dynamic registration / concurrent close | Adapted: `Scope::scoped`, `acquire`, `add_finalizer`, `add_release`, `close`, `closed`; the owner retains canonical E while closers share status | `scopes`: reverse order, non-Clone/non-Sync errors, awaited shared close, 32 registration/close races, late registration/acquisition, retained environments, escaped handles, owner/driver loss, and release defects; subprocess includes 10,000 nested scopes |
| Fiber.fs scoped children / repeat observation | Adapted: Scope::fork, Fiber owned join/await_exit/interrupt, shared await_exit_shared/interrupt_shared, status-only settled, SharedExit borrowed inspection | fibers: non-Clone/non-Sync outcomes, shared pointer identity/exact destruction/unique transfer, repeated interruption, protected unobserved error, cancelled waiter, awaited release, loss/rejection/getter panic/pre-cancel, late registration and independent synchronous-worker progress |
| Linked child cancellation | Adapted: Cancellation::child, iterative descendant request and ancestor destruction; child token comes from active interpreter context | linked_cancellation: local requests, dropped intermediate handle, linked waiter broadcast and registration/request races; subprocess includes 100,000 linked propagation and unrequested destruction |
| TaskSpawner environment capability | Implemented public unsealed TaskSpawner/HasTaskSpawner with ownership-aware ScheduledTask and Scope::fork | Working external independent-thread provider on neutral default/disabled-default and Tokio hosts; typing accepts non-Clone shared outcomes and rejects missing capability/escaping borrow. Optional vendor adapters are verified by the mixed external consumer |
| Parallel.fs race | Adapted: `race` commits first terminal selection in child completion, cancels/joins the loser, inherits winner protection and keeps loser cleanup failures | `parallel` proves independent synchronous progress, failure winning, awaited loser release, root versus nested-fiber protection, right winner while coordinator is paused, and admitted-driver loss |
| Parallel.fs allConcurrent / Combinators.fs forEachConcurrent | Adapted: explicit NonZeroUsize limit, fixed bounded workers, ordered `all_concurrent` / `for_each_concurrent`; `_shared` variants reuse Arc slices, transformation borrows selected input | `parallel`: exact worker admissions, active bound, ordered non-Clone/non-Sync outputs, repeated input pointer identity, lazy short circuit, cancellation through release, empty/rejected/lost work, cleanup order despite blocked primary Drop and projected provider preserving root cancellation; external compiler/portable cases |
| Deferred.fs first-write completion / Facilities.fs full outcomes | Adapted: Deferred make/new, owned complete/succeed/fail, fresh complete_with, await_value, repeatable await_shared/poll_shared | `synchronization`: complete terminal categories/ordered cleanup, cancellation/abandonment isolation, first-write races, broadcast, wake callback outside gate, non-Clone/non-Sync pointer identity/exact destruction/unique transfer, active/consumed ownership and factory/inspection/destruction defects; external consumers |
| Latch.fs / Facilities.fs independent waiters | Adapted: payload-free Latch make/new/open/await_open | `synchronization`: persistent/idempotent opening, cancelled/abandoned/surviving/late waiters, pre-cancelled opening, all-waiter broadcast; same portable case on all three hosts |
| Ref.fs / Concurrency.fs atomic updates | Adapted: AtomicRef make_with/new, borrowed with/modify/update, in-place modification, set_with; explicitly copying make/set/get | `synchronization`: fresh lazy state, non-Clone/non-Sync data/results, unchanged buffer allocation, exact source 10,000 updates on 16 fixed workers, callback/mutation/destruction boundaries, old Drop outside gate; compiler rejects escaping reference and non-Clone get |
| Queue.fs FIFO admission / Concurrency.fs queue workflows | Adapted: AsyncQueue make/new, explicit Unbounded/Bounded/Rendezvous, owned offer/fresh offer_with/explicit offer_cloned, take/size, typed Drain/Discard shutdown | `queue`: non-Clone/non-Sync FIFO/backpressure, targeted cancellation/drop, committed-transfer races, stale slot identity, all-waiter wake, shutdown/drain, destruction/re-entrant wake boundaries and scoped child joining; external compiler/portable cases |
| CPU/blocking jobs and provider admission | Implemented: CpuExecutor/BlockingExecutor, contextual/reusable/owned job variants, ProviderAdmission and driver-aware drains | 17 `jobs` tests; mixed vendor fixture checks rejection, queued/running cancellation, panic/expected errors, subtree overrides, load and capacity-one reuse |
| Optional runtimes and native completion | Adapted: effect-tokio tasks/blocking/clock, effect-rayon CPU, effect-native-runtime tasks/request linking/full completion | Separate mixed external consumer uses all adapters together and native owner shutdown; default/disabled-default dependency guards |
| Clock.fs / TestClock.fs / Random.fs / Console.fs | Adapted: Duration-based Clock/TestClock, owned tick/frontier acknowledgement, validated UnitInterval, Random/Console capabilities | `services` virtual deadlines/FIFO/cancel/drop/overflow, retry/jitter; explicit service policies and const/missing-capability compiler cases |
| Schedule.fs | Implemented: unfold/recurs/spaced/exponential/up_to/jitter/intersect, retry/repeat/structured timeout | `services` deterministic channel/timing/invalid-policy checks; jitter capability enforced during compilation |
| Logger.fs / Tracer.fs | Adapted: explicit R logger/tracer/clock, immutable annotations, parent contexts, spans/outcome recording, bounded diagnostics | `services` child/CPU propagation, restoration, redaction and instrumentation isolation; effect-tracing cross-thread dispatcher/parent/retirement integration |
| Schema.fs | Adapted: reusable typed borrowed JSON decoders, optional decoder, lazy decode/parse with explicit syntax mapping | `services` laziness/reuse, domain/syntax errors and downstream short circuit; actual application's invalid-input tests; non-Clone compiler consumer |
| Layer.fs | Adapted: make_with/from_effect/scoped/build/merge/provide/provide_to | `services` scoped dependency use/release; builds are lazy and owned values move |
| Stream.fs | Adapted: from_pull/from_iterable/from_async_iterable/map_effect/run_for_each/run_collect, explicit disposal and optional limit | `services` moved non-Sync values, limits, reusable readers and awaited disposal |
| Http.fs / Fetch.fs | Adapted: R transport, typed request/options/status, owned scoped responses, text/fetch_text | `managed` closes success/rejection/decode failure, preserving expected primary plus disposal defect; no global .NET client or selected network SDK |
| WebSocket.fs / WebSocketTypes.fs | Adapted: R transport, typed frames/messages/limits/errors, independent FIFO direction gates, fragmented reads, close/abort/disposal | `managed` assembly, message bounds and virtual close timeout joining/disposal; explicit provider abort/settlement contract |
| Pool.fs | Adapted: explicit capacity/idle TTL, make/acquire/lease inspection/release/discard/set_capacity/exists_idle/ensure_size/shutdown | `managed` independent population, slot ownership, validation/TTL/capacity, exact disposal and abandoned-owner lease invalidation/recovery drain; full errors retained by owning scopes |
| Cache.fs | Adapted: explicit equality/TTL/retention/failure policy, shared in-flight get, invalidate/shutdown and shared A/E | `managed` shared producers, identity, TTL/LRU, rejection cause, waiter isolation, root joining, observed-cleanup retirement and owner-loss admission/recovery |
| WeakCache.fs | Adapted: weak key plus weak association/consumer ownership, typed partitions, get/invalidate/shutdown | `managed` settled value-to-key retention, separate partitions and rejection through live associations after owner loss; documented weaker retention than tracing-GC ephemerons |
| Production application integration | Implemented: effect-app actual file workflow and workspace-native boundary | 3 external application tests cover commit/disposal, invalid input, exhausted I/O retry, cancellation, child provider failure, instrumentation and exhaustive cleanup reporting |


## Deliberate adaptations and limits

- The source's `'a/'e/'r` are type variables; Rust spells the same relationship
  `Effect<A, E, R>`. The initial implementation is sendable and owns an `Arc<R>`.
- R is the sole application/runtime provider boundary. The source's separate
  clock/token run parameters and direct .NET worker calls are being adapted to
  environment capabilities. Private continuation/cancellation state does not
  configure a second runtime.
- Rust execution is lazy until the returned future is polled. The caller
  chooses the host that polls it. Cancellation access is lazy as well.
- Reusable Result/Option/value constructors clone stored payloads; factories
  and macro-local values support non-Clone results/errors. Captured inputs must
  be reproduced or shared explicitly. Projection constructs a fresh environment
  value; it does not clone arbitrary success/error payloads.
- Native Result errors and unwind panics occupy different channels. Catching
  panic captures a diagnostic message and does not suppress the panic hook.
  `DefectKind::CallbackPanic` also identifies non-string unwinds. Built-in failures
  have `LibraryDefect` identities, independent of their human diagnostic text;
  equality includes that identity. Custom diagnostics cannot impersonate a
  library failure by copying its message.
- Dropping a plain run future abandons execution. An owned driver retained by
  the host survives loss of its observer and completes bracket release. Driver
  loss is explicitly reported as a defect, not successful cleanup. Scoped child
  joining, native cancellation translation, and vendor provider drains are implemented.
  The supplied runtime owner still retains shutdown authority.
- Resource values require Send + Sync; no resource/error Clone bound is imposed.
  Escaping handles are invalidated, borrowed references cannot escape access,
  and FnOnce Release factories can consume owned native resources.
- Scope handles have only E as a type parameter, like the reference. Registered
  finalizers capture their R environment; this is private ownership state, not
  a second provider configuration boundary. Scope close acknowledgements do not
  clone owned cleanup errors; the enclosing scoped effect publishes those once.
- Fiber ownership is registered before task submission; this closes the source's
  post-submit registration gap at the Rust provider boundary. Rejected/never-
  started tasks do not add a duplicate loss failure; accepted loss is observable.
  Scopes still join admitted work. A provider that retains rejected work or never
  drives accepted work violates the capability contract; the core cannot make
  an arbitrary provider progress or await cleanup from Drop.
- The reference shares Task outcomes. Rust owned joins move A/E once; repeatable
  observations share one Arc<Mutex<Exit<A,E>>> allocated on demand. Non-Sync
  payloads require guarded borrowing, without adding Sync/Clone bounds. Shared
  inspection cannot return an escaping reference. Owned transfer with active
  views is rejected without altering the data; unique ownership later permits it.
  Observed errors are handled; status-only waiting or cancelled observers leave
  failures for the scope owner. Mixed-provider/load evidence is recorded in the external fixture and RUNTIMES.md.
- Bounded traversal validates positivity through NonZeroUsize, rather than the
  source's runtime rejection of invalid integer limits. Worker count derives
  from that explicit operation policy and actual input length. Shared input
  collections reuse their allocation; selected callbacks borrow inputs and
  cannot return Effects retaining the temporary borrow. Output/error ownership
  has no Clone or Sync requirement. Owned Vec variants move into shared storage
  once; no input-vector cloning is performed on subsequent runs.
- Like Parallel.fs, race selects the first terminal result, rather than the first
  success. Ordinary losing/secondary primary failures are discarded; their
  finalization failures are retained. Selection commits at branch completion,
  and traversal cleanup groups commit before discarded primary destructors run
  outside the gate. Race propagates winner protection; fresh branch/worker
  origins inherit the enclosing fiber-boundary flag. Every admitted child is
  joined through cleanup. Unlike the .NET source, rejected/lost Rust driver
  ownership has an explicit typed infrastructure outcome; loss remains visible
  even when it is not the selected primary failure.
- Macro lowering/type checking is compile-time work. Typed defect construction
  and classification are const-capable; external const-context consumers prove
  evaluation. Runtime trampoline boxing/dispatch, Arc atomics, completion locks,
  and cancellation registries remain costs to measure, not zero-overhead claims.
  Concurrent execution additionally retains shared source/factory/batch state,
  result slots, worker join futures and per-item execution contexts. Fixed
  workers avoid per-input task submission. Borrowing the evaluator context
  avoids an extra context clone solely to inspect completion protection.
- Deferred follows the same owned/shared adaptation as Fiber: await_value moves
  once; shared observation retains a canonical complete Exit without A/E Clone
  or Sync bounds. Active views prevent owned transfer without changing data;
  unique ownership later permits it. Consumed outcomes report a typed defect.
  The source's repeated Task result is not implemented through implicit payload
  copying. Completion descriptions move their one supplied candidate once and
  return false on repeat; complete_with instead creates fresh candidates. A
  pre-cancelled completion leaves its candidate available. Polling is explicitly
  shared, and pending waiter cancellation/drop does not alter completion.
- Latch uses a persistent flag/event instead of a unit-payload Deferred wrapper;
  it preserves first-opening/broadcast/waiter isolation while removing the payload
  allocation and mutex. Its state is executor-independent, with no provider field.
- AtomicRef callbacks inspect current state by borrow and return owned results/
  replacements. This adapts the source's GC value passing without requiring
  stored data to be Clone. Compound callbacks serialize under the value mutex.
  Replaced values and rejected Deferred candidates are dropped outside gates;
  their caught destruction panics report defects after the committed state.
  In-place/interior mutation can survive panic, as in ordinary Rust; no rollback
  is claimed. Copying constructors/setters/snapshots remain explicit opt-ins.
- SharedExit now uses the producer-independent OutcomeStatus classification;
  FiberStatus remains an alias of that same enum. Classification/status helpers
  stay const-capable. Live mutex/event state and Arc allocation require runtime
  work; no compile-time evaluation of live synchronization is claimed.
- Queue transfers owned Send payloads, without Clone or Sync. Unlike a GC-held
  reusable source value, offer consumes its one candidate on first started
  execution; repeating reports QueueOfferConsumed. Pre-cancellation retains it;
  offer_with produces fresh candidates and offer_cloned explicitly copies.
  Requested pending nodes cannot match even when their drivers are unpolled.
  Cancellation/drop removes only uncommitted nodes, never reversing a transfer.
  Drain keeps only committed buffer data; Discard removes it too. Earlier
  pending cancellation retains raw Interrupted through shutdown. Safe indexed
  links use actual slot positions, embedded free links and retained Arc identity
  to prevent stale guards from removing a reused slot. Unlink is constant-time;
  insertion is amortized and normal reuse retains high-water storage. Shutdown
  retires pending storage; discard retires the buffer. Nodes/events exist only
  for pending operations. Typed registrations select offer/take behavior through
  generics and move the same node handle into the guard/wait future.
  State commits before wake/destruction outside the gate; caught panics append
  ordered cleanup defects to the active operation. Abandoned Drop cannot publish
  cleanup to a departed observer or await release. QueueCapacity::from_limit is
  tested in const contexts; dynamic occupancy/cancellation remain runtime work.
- run/run_cancellable futures own recipe/environment, with explicit precise
  capture rather than Rust 2024's implicit description-borrow capture. External
  compiler cases prove the future can outlive local and borrowed descriptions;
  the cross-thread queue race exercises it. This public lifetime correction
  adds no allocation beyond the existing owned recipe handle. The language
  rule is documented in the
  [edition guide](https://doc.rust-lang.org/edition-guide/rust-2024/rpit-lifetime-capture.html).
- Driver abandonment invalidates acquired handles but cannot await release.
  Scope observers wake with Failed; owned-run observers receive a defect.
- Simple macro inputs move through single-use continuations. Imperative branches,
  loops, nested returns and let-else lower to owned Rust async state machines.
  Each actual iteration cooperates, and lexical releases are awaited at normal,
  break, continue and return/failure boundaries. Closure/async/item/other-macro
  inputs retain their own syntax boundaries. Native futures have explicit adapters.
- Time uses checked Rust Duration values; nonfinite float policy/sample values
  are rejected. Jitter capability is static Schedule metadata, not another Effect
  backend parameter. Virtual advancement follows continuations until their next
  native wait/outcome; local macro bind completion does not acknowledge early.
- Scoped objects retain their construction R; this is resource ownership, not
  another provider configuration channel. Cache outcomes share typed handles;
  weak associations use the explicitly documented Rust retention contract.
- HTTP/WebSocket SDKs remain consumer supplied through R. No unsolicited client,
  reactor, F# ABI export, or tracing-GC parity claim is introduced. The integrated
  application uses actual Tokio file I/O and native owned completion.
- The boxed typed interpreter has substantial measured per-step overhead.
  Source safety/ownership and bounded lifecycle checks do not establish a
  production performance SLO. See RUNTIMES.md and the retained measurement output.


## Verification scope

The isolated `runtime_neutral` fixture tests both default and disabled default
features; `check_portability.py` rejects vendor packages in production/build
dependencies for both configurations. `tokio_consumer` runs the same portable
definitions on a separate host, including owned execution, resource release,
dynamic scopes, and value-producing branches. All three configurations also run
an independent-thread custom provider through public APIs with owned/shared
non-Clone outcomes. Shared bounded descriptions, borrowed non-Clone inputs and
races also run on all three hosts. These prove portable scoped-fiber/concurrent
integration. A fourth shared conformance group verifies non-Clone reference
state, Deferred shared/owned failure observation and structured latch waiting.
A fifth group moves queue payloads through a scoped rendezvous child and verifies
buffered drain/shutdown through public APIs on all three hosts. A sixth group covers CPU/blocking jobs. The separate mixed_tokio_rayon fixture
uses actual Tokio tasks/blocking/timers, Rayon pools, subtree overrides, native
ownership, cancellation, admission, failures and bounded load/drainage.

The implementation plan includes debug/release tests, external compiler
consumers, doctests, Clippy, formatting, workspace checking, and later per-adapter
gates. The workspace formatting check currently reports an unrelated difference
in `crates/native_interop/src/lib.rs`; it is kept separate from Effect changes.
Record current command results in the implementation ledger rather than reusing
historical F# aggregate test counts as Rust evidence.
