# Source contracts and Rust adaptations

Behavioral reference: `mandrill_net/effect`, revision
`a1f874b7e993d77472ff9f5c992ea73653e81a28`. The mapping below describes
Rust APIs and deliberate adaptations of the source contracts.

| Source contract | Rust API or adaptation |
| --- | --- |
| Types.fs: typed outcomes and cleanup failures | `Exit`, `PrimaryOutcome`, `CleanupFailure`, `Finalization`, opaque `Defect` |
| Interpreter.fs: lazy reusable description, typed trampoline | reusable Arc recipes and typed FnOnce continuations per run |
| Effect.fsi: succeed/fail/die/sync/defer | `succeed`, `fail`, `fail_with`, `die`, `sync`, `defer` |
| Effect.fsi: trySync and task/result adapters | `try_sync`, `from_future`, `try_future`; Rust Result errors are explicitly mapped, panics remain defects |
| Combinators.fs: map/flatMap/zip/all/forEach | `map`, `flat_map`, `zip`, `all`, `for_each`; sequential and short-circuiting |
| Combinators.fs: expected-error recovery and conversions | `map_error`, `catch_all`, `either`/`result`, `option`, `sandbox`, `from_result`, `from_option` |
| Combinators.fs: service/provide/provideSome | `service`, `provide`, `provide_some` |
| EffectBuilder.fs: Return/Bind/Delay/Zero/Combine | procedural `effect!` with Rust statements, whole-step Effect `?`, tail values, top-level returns, patterns, and unit blocks |
| EffectBuilder.fs value-producing branches | `if`/`else if`, `match` with patterns/guards, and unlabeled blocks |
| EffectBuilder.fs While / sequential iteration | `while_loop`; existing `for_each` handles traversal |
| Interpreter.fs: cancellation checkpoints and raw/publication distinction | sequential execution and cooperative futures |
| Interpreter.fs: runTask/runTaskWith | lazy Future-returning `run` and `run_cancellable`; future owns recipe/environment independently of description borrow; sole external configuration is R, cancellable boundary requires HasCancellation |
| Runtime cancellation notifications | executor-independent broadcast Cancellation |
| Deep descriptions and continuation teardown | Rust-specific iterative destruction; no interpretation-time Any success/error downcasts or unsafe value stack |
| Cooperative executor handoff | return control between typed transitions using standard wakeups; no arbitrary batching quantum; CPU-time bounds and scheduler fairness are not promised |
| No magic values / Rust defect identities | `DefectKind` distinguishes custom diagnostics, caught panics, and typed `LibraryDefect`; broadcast and package lookup follow documented contracts |
| Interpreter.fs protected execution and Bracket.fs finalization | `uninterruptible`, `on_interrupt`, `ensuring`, `ensuring_with`, ordered cleanup accumulation |
| Resource.fs / Bracket.fs acquisition ownership | `ResourceBinding`, invalidatable `ResourceValue`, single-use `Release`, borrowed `PrimaryOutcomeRef` |
| EffectBuilder.fs resource-aware Bind | safe sealed macro dispatch for Effect/ResourceBinding |
| Rust observer/driver ownership | `owned_run` / `owned_run_cancellable`, `RunDriver` / `RunObserver` |
| Scope.fs dynamic registration / concurrent close | `Scope::scoped`, `acquire`, `add_finalizer`, `add_release`, `close`, `closed`; the owner retains canonical E while closers share status |
| Fiber.fs scoped children / repeat observation | Scope::fork, Fiber owned join/await_exit/interrupt, shared await_exit_shared/interrupt_shared, status-only settled, SharedExit borrowed inspection |
| Linked child cancellation | Cancellation::child, iterative descendant request and ancestor destruction; child token comes from active interpreter context |
| TaskSpawner environment capability | unsealed TaskSpawner/HasTaskSpawner with ownership-aware ScheduledTask and Scope::fork |
| Parallel.fs race | `race` commits first terminal selection in child completion, cancels/joins the loser, inherits winner protection and keeps loser cleanup failures |
| Parallel.fs allConcurrent / Combinators.fs forEachConcurrent | explicit NonZeroUsize limit, fixed bounded workers, ordered `all_concurrent` / `for_each_concurrent`; `_shared` variants reuse Arc slices, transformation borrows selected input |
| Deferred.fs first-write completion / Facilities.fs full outcomes | Deferred make/new, owned complete/succeed/fail, fresh complete_with, await_value, repeatable await_shared/poll_shared |
| Latch.fs / Facilities.fs independent waiters | payload-free Latch make/new/open/await_open |
| Ref.fs / Concurrency.fs atomic updates | AtomicRef make_with/new, borrowed with/modify/update, in-place modification, set_with; explicitly copying make/set/get |
| Queue.fs FIFO admission / Concurrency.fs queue workflows | AsyncQueue make/new, explicit Unbounded/Bounded/Rendezvous, owned offer/fresh offer_with/explicit offer_cloned, take/size, typed Drain/Discard shutdown |
| CPU/blocking jobs and provider admission | CpuExecutor/BlockingExecutor, contextual/reusable/owned job variants, ProviderAdmission and driver-aware drains |
| Optional runtimes and native completion | effect-tokio tasks/blocking/clock, effect-rayon CPU, effect-native-runtime tasks/request linking/full completion |
| Clock.fs / TestClock.fs / Random.fs / Console.fs | Duration-based Clock/TestClock, owned tick/frontier acknowledgement, validated UnitInterval, Random/Console capabilities |
| Schedule.fs | unfold/recurs/spaced/exponential/up_to/jitter/intersect, retry/repeat/structured timeout |
| Logger.fs / Tracer.fs | explicit R logger/tracer/clock, immutable annotations, parent contexts, spans/outcome recording, bounded diagnostics |
| Schema.fs | reusable typed borrowed JSON decoders, optional decoder, lazy decode/parse with explicit syntax mapping |
| Layer.fs | make_with/from_effect/scoped/build/merge/provide/provide_to |
| Stream.fs | from_pull/from_iterable/from_async_iterable/map_effect/run_for_each/run_collect, explicit disposal and optional limit |
| Http.fs / Fetch.fs | R transport, typed request/options/status, owned scoped responses, text/fetch_text |
| WebSocket.fs / WebSocketTypes.fs | R transport, typed frames/messages/limits/errors, independent FIFO direction gates, fragmented reads, close/abort/disposal |
| Pool.fs | explicit capacity/idle TTL, make/acquire/lease inspection/release/discard/set_capacity/exists_idle/ensure_size/shutdown |
| Cache.fs | explicit equality/TTL/retention/failure policy, shared in-flight get, invalidate/shutdown and shared A/E |
| WeakCache.fs | weak key plus weak association/consumer ownership, typed partitions, get/invalidate/shutdown |
| Production application integration | effect-app actual file workflow and workspace-native boundary |


## Deliberate adaptations and limits

- The source's `'a/'e/'r` are type variables; Rust spells the same relationship
  `Effect<A, E, R>`. The Rust API is sendable and owns an `Arc<R>`.
- R is the sole application/runtime provider boundary. Rust expresses the
  source's separate clock/token run parameters and direct .NET worker calls
  through environment capabilities. Private continuation/cancellation state does not
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
  loss is explicitly reported as a defect, not successful cleanup. Scoped children
  join admitted work; native cancellation translation and vendor provider drains
  retain accepted work through settlement.
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
  failures for the scope owner.
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
  and classification are const-capable. Runtime trampoline boxing/dispatch,
  Arc atomics, completion locks, and cancellation registries incur runtime costs.
  Concurrent execution additionally retains shared source/factory/batch state,
  result slots, worker join futures and per-item execution contexts. Fixed
  workers avoid per-input task submission. Borrowing the evaluator context
  avoids an extra context clone solely to inspect completion protection.
- Deferred follows the same owned/shared adaptation as Fiber: await_value moves
  once; shared observation retains a canonical complete Exit without A/E Clone
  or Sync bounds. Active views prevent owned transfer without changing data;
  unique ownership later permits it. Consumed outcomes report a typed defect.
  The source's repeated Task result becomes shared observation without implicit
  payload copying. Completion descriptions move their one supplied candidate once and
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
- SharedExit uses the producer-independent OutcomeStatus classification;
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
  cleanup to a departed observer or await release. QueueCapacity::from_limit supports
  const contexts; dynamic occupancy/cancellation remain runtime work.
- run/run_cancellable futures own recipe/environment, with explicit precise
  capture rather than Rust 2024's implicit description-borrow capture. The future
  can outlive local and borrowed descriptions without an allocation beyond the
  owned recipe handle. The language
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
- The boxed typed interpreter allocates recipes and continuations and uses
  dynamic dispatch. See [RUNTIMES.md](RUNTIMES.md) for ownership and execution costs.
