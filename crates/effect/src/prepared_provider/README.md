# Typed physical provider path

`PreparedProvider<F, T, D>` is an additive provider boundary, not a quota around
an ordinary spawn. Its submission authority is `PreparedSubmitter<F, T, D>`.
The ordinary `ScheduledTask`, `ScheduledJob`, `TaskSpawner`, CPU/blocking traits
and provider constructors keep their existing allocation/cancellation/Clone
semantics. They are **not** silently certified as prepared by this API.

Provider entry points:

- `effect_tokio::TokioTasks::prepare`: compatible cooperative fixed-worker task
  scheduler; retains the caller's multi-thread runtime and enters it separately
  on each worker for reactor access. No Tokio task is spawned.
- `effect_tokio::TokioBlocking::prepare`: dedicated fixed blocking workers,
  entering the supplied runtime on those workers. No `spawn_blocking` call.
- `effect_rayon::RayonCpu::prepare`: dedicated fixed CPU workers with concrete
  `SynchronousJob<C>` callbacks. No Rayon HeapJob, global pool or fallback.
- `effect_native_runtime::PreparedNativeTasks::prepare`: compatible task
  scheduler with lazy, worker-thread native future creation, existing native
  parent/service admission and exact prepared settlement-owner frames. No
  allocating native runtime submission. The native runtime owner must remain
  alive through native drain and prepared provider join.

## Order and ownership

1. Provision the backend using `PreparedMemory`, a region, capacity, explicit
   worker count/stack size and named `ProviderNativeEnvelope`. `storage_bytes`
   describes the concrete inline allocations, excluding region allocator
   headers/padding (visible in memory usage) and native/transitive storage.
   All queue cells, future cells, typed result/budget records, cancellation
   records, worker-handle and raw worker-failure arrays exist before submission.
   One Arc wake control per capacity cell and fixed thread/startup controls are
   created only at startup in the explicitly separate native envelope. Native
   allocator/OS overhead and platform OOM/abort behavior are not region proofs.
   Partial fallible thread/context startup stops and joins every created worker;
   raw context/join failures remain owned in the returned error.
2. Select `PreparedReuse` or `BoundedAllocation` in the supplied memory plan and
   seal it before execution. `memory_policy` exposes that choice. Under either
   policy the backend reuses its fixed typed records and never grows its queue
   or uses System/native spawn as a fallback. In bounded-allocation mode a
   working-set lease may explicitly authorize fallible allocation from its
   bounded domain; that is not a claim of startup-only allocation for that work.
3. `reserve` checks out the actual future/result/cancellation/control ownership
   before expensive capture construction. Refusal is `ProviderRefusal`, not the
   workflow's domain error. Native reservation also acquires exact settlement
   frame storage before native parent/service permits. Failure rolls back all
   partial ownership. Reservation is acceptance: stop rejects later reservations
   but an already accepted preparation may hand off once, and drain waits for
   its handoff or rollback/destruction.
4. `submit` first obtains `ProviderBudget<D>`: the actual transitive working-set
   lease plus THREE concrete pre-admitted diagnostic event leases. Only then
   does it invoke the capture/future factory, outside provider metadata locks.
   D is installed in the unique unpublished entry before factory invocation;
   the factory borrows it without holding the metadata gate. Factory unwind
   therefore uses the entry's payload/descendant/footer rollback, not stack-drop
   of D ahead of queued partial captures. Factories themselves and any
   already-owned input remain caller/native-domain work; a caller must not construct expensive captures before `reserve` and
   then claim they were admitted by it. No pointer layout accounts their
   reachable input/output storage. `ProviderWorkingSet` is an explicit unsafe
   native storage/lease contract, not a compiler allocation-freedom inference.
5. Futures are placed once into stable region-owned cells. A worker owns the
   sole queue work record while polling; Pending returns that record to the
   fixed cooperative queue, not a worker blocked waiting on one future. Wakes
   use startup controls. Old externally retained wake handles can cause only
   spurious polls, not accesses to an old future/generation. Concrete futures
   are never moved after pinning and are dropped in place outside queue/result
   locks before publication, including on unwind. Prepared execution-owner
   frames use the existing FlatDrop lane. Native `create` runs on the worker,
   after worker-local runtime entry, and initializes its future in place.
6. `PreparedReceipt` separates caller ownership from execution ownership. Its
   drop requests cooperative cancellation; it cannot free an accepted queued
   or running future. A callback ignoring cancellation continues to retain all
   actual work storage until it returns. `SynchronousJob<C>` is the raw typed
   cooperative callback boundary; domain cancellation/cleanup is represented
   by its callback's result, not invented by the provider. Ordinary Effect job
   cancellation semantics remain unchanged on the ordinary path.
7. `settled(&mut receipt)` provides one notification registration. Caller Waker
   clone/drop/allocation behavior belongs to the explicit native working-set
   contract; clone, replacement drop and completion wake run outside provider
   metadata locks. `result` borrows the result while holding its state guard.
   `retain_result` yields `PreparedOutput`; fallible sharing copies only
   prepared owner metadata, never T or its pointees. No detached T is returned.
   Result, notification and working-set backing stay alive through the final
   consumer and payload destruction, even after execution/native drains finish.
8. `stop` closes new reservations; `join` waits for accepted preparation and
   actual task return/notification/settlement, not for result consumers to drop.
   Call it from the explicit synchronous shutdown owner, not from its own
   worker or while holding work that must settle. Provider drop requests stop
   but does not block/discard accepted work; workers retain their storage/context
   until settlement. `worker_panics` retains raw worker/context failures for
   owner inspection rather than silently discarding JoinHandle errors.

## Once-only escaped-output transfer

`settled` above observes result publication; its notification may arrive at the
final boundary. The additive final flag is published after future destruction,
result publication, execution-owner retirement and occupied queue/running-count
retirement, but BEFORE the final waiter notification. The worker retains its
entry/D through that notification and only then retires its entry owner. The
flag is separate from cancellation and remains in the actual prepared control
cell; it does not assert that the entry/D have already been destroyed.

A retained output can call nonblocking `try_take` only after that final flag and
a unique-entry check. Other output aliases, a live receipt or worker ownership
refuse recoverably. The settlement is not moved: the exclusive
`PreparedOwnedProviderResult` still borrows it from the original entry gate.
`transfer` requires a `PreparedOutputOwnerReservation` from the originating
run's pre-business hook and returns `PreparedOutputOwnerView` of the actual
installed provider lease. This view is an output consumer acquired during
reservation, not a bare ProviderSettlement<T>. It permits only borrowed
`result().settlement()` access. Concurrent original-output destruction cannot
release its entry/D. No safe public extraction or owned callback strips T/D.

Final provider-entry retirement has explicit payload/footer/release phases:
future/result/waiter and their prepared descendants, then D and its descendants,
then entry and the actual retained cancellation/control cell. This works even
inside an active prepared drain or one unwind. It is not a proof based on field
declaration order. Intrusive node and queued owner/control metadata are in
startup entry storage. The core [output-origin contract](../internal/runtime/OUTPUT_OWNERS.md)
describes the postorder/FIFO traversal and counted handoff.

For an unpublished projected/Layer environment, the [construction contract](../internal/runtime/CONSTRUCTION.md)
provides actual pre-business authority: PreparedProviderConstruction owns the
real unpublished physical submission, D installed in that entry before the
outer graph factory, a private non-observing retention guard and three diagnostic events;
ConstructionProvider<K,KEY> claims that actual typed per-invocation site rather
than invoking the not-yet-published R getter. Direct preparations likewise use
an actual typed prepared indirection cell before native budget callbacks, never
a heap Box or metadata fallback. Foreign construction is explicit and selected
through available root R services; a key/type alone is not a provider.

Generated scheduling and canonical scoped fork/join/race use this SAME backend;
see [SCHEDULING.md](../internal/runtime/SCHEDULING.md). Their private, pre-reserved
retirement frame releases D only after all future/result/waiter descendants,
then releases an immutable prefix of actual source origins and the child node.
This keeps abandoned accepted work and unobserved results charged without
storing a permanent counted output consumer inside its own root. Public result
access, unsafe native working-set obligations and diagnostic policy are unchanged.

Awaited graph observations use a final-settlement poll distinct from publication.
The worker now retains its entry/D through its single notification after actual
execution-owner and queue retirement. No result writes occur after that final
flag; only private graph-owned receipts without public aliases can use the
corresponding private result handoff. Existing external unique extraction checks
remain stricter, and all three pre-admitted diagnostic sites remain explicit.

A separate crate-private engine handoff supplies the unchanged business A without
exposing it through public extraction: `effect_prepare! { provider_result; ... }`
is a sealed result-only description, not scheduler wiring. It consumes the
finally-settled unique view, pre-admits its actual entry/D origin and temporary
consumer, installs D before taking A, then calls only the existing private
Finish<A,E> of the same live invocation. PreparedInvocation owns its output
consumer through the complete drive. The temporary consumer is released after
that continuation call, never stored in its own root outcome/origin. Non-Clone A
therefore reaches the ordinary typed graph continuation with D still attached;
public transfer/views remain borrowed and consumer-coupled. Full authority and
rollback details are in OUTPUT_OWNERS.md.

`try_take` is not an async final-settlement waiter or a new spin/timeout policy;
its refusal preserves ownership for a later properly scheduled attempt. General
graph provider/fork/join wiring remains W05-14. Arbitrary T pointees and Clone
implementations still obey D's explicit native ownership contract: clones that
escape an observer must carry their own backing/budget, not assume a borrowed
callback intercepted those clones. No native pointer layout is promoted to a
transitive allocation proof.

## Results and failure domains

T is preserved, including all five `Exit<A, E>` families and `E = !`.
Preexecution refusal and working-set acquisition errors are separate Results;
no domain error value is fabricated. Poll/callback/future-destructor unwind is
retained as `ProviderSettlement::Panicked` without string copying or Defect
formatting. Ordinary Rust double-panic/abort behavior is not overridden.

There are three separately pre-admitted provider side-channel sites per
operation: completion waiter wake, worker-side final payload retirement, and
execution-owner retirement. These each execute once, not once per poll. An
already published outcome is not replaced. `PreparedDiagnosticLease` places a
concrete caller-owned `ProviderDiagnosticSinkLease` into an exact typed prepared
frame. Delivery moves the raw payload AND its actual pre-admitted event owner
into the explicitly selected native sink domain until final acknowledgment.
Unused leases return their backing; neither delivery nor unused-lease drop may
panic or acquire late/unbounded storage under that unsafe contract. Sink owner
loss, retention/overflow and native payload backing are the caller's explicit
contract, not a new implicit diagnostic policy. An unsupported sink must fail
reservation/configuration before the factory runs. Arbitrary sink/Waker/user
Drop code is not certified allocation-free by its pointer size.

This boundary does not implement general prepared string diagnostics, timers,
app/server/SQL/graph DSL conversion or automatically migrate ordinary erased
Effect CPU/fork graphs. Those integrations retain their own plan requirements.
No runtime/concurrency/performance validation is claimed during the current
checks-and-source-only implementation phase.
