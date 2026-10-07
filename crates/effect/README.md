# Effect

Effect is a foundation for robust, maintainable, type-safe Rust applications.
`Effect<A, E, R>` describes a lazy, reusable workflow with a success value `A`,
an expected error `E`, and an environment `R` containing its dependencies.

The implementation includes typed errors, dependency injection, structured
concurrency, scheduling, tracing, and JSON schema validation. It provides the
procedural `effect!` macro, native future/owned callback adapters, resources and
scopes, fibers and bounded parallel composition, synchronization, injectable
services, layers/streams, managed transport contracts, pools/caches, optional
Tokio/Rayon/native/tracing providers, and an integrated application. R carries
all selected providers together. [PORTING.md](PORTING.md) maps source contracts;
[RUNTIMES.md](RUNTIMES.md) describes ownership, provider limits and execution costs.


## A consumer workflow

```rust
use effect::{Cancellation, Effect, Exit, HasCancellation, effect};
use std::sync::Arc;

struct Application {
    prefix: Arc<str>,
    cancellation: Cancellation,
}

impl HasCancellation for Application {
    fn cancellation(&self) -> &Cancellation { &self.cancellation }
}

fn greeting() -> Effect<String, String, Application> {
    effect! {
        let prefix = Effect::service(|env: &Application| Arc::clone(&env.prefix))?;
        let name = Effect::try_future(
            |_cancel| async { Ok::<_, String>(String::from("Rust")) },
            |error| error,
        )?;
        format!("{prefix}, {name}!")
    }
}

let env = Arc::new(Application {
    prefix: Arc::from("Hello"),
    cancellation: Cancellation::new(),
});
let program = greeting();

// This executor is used only to drive the example; Effect does not select one.
let outcome = futures_executor::block_on(program.run_cancellable(Arc::clone(&env)));
assert_eq!(outcome, Exit::Success(String::from("Hello, Rust!")));

assert!(env.cancellation.request());
assert_eq!(futures_executor::block_on(program.run_cancellable(env)), Exit::Interrupted);
```

`effect!` uses ordinary Rust statements. Within a whole binding initializer,
expression statement, or tail expression, `operation()?` executes an Effect and
propagates every non-success outcome. A `ResourceBinding` operand instead
acquires a scoped resource and arranges release around the remaining workflow.
A tail value or `return value;` completes that workflow successfully. Ordinary
Rust `for`, `while`, `loop`, labeled `break`/`continue`, `let-else`, nested returns,
patterns, and mutable local state are supported. Effect steps use `?`; Result
and Option require the explicit adapters. Simple workflows use typed
continuations; richer control flow lowers to a Rust async state machine. Neither
path introduces an F# syntax dialect or a nightly Try implementation.

Resource bindings release at their lexical block boundary, including loop
iterations and break/continue exits. Returning or failing drains the remaining
root scope. Each actual loop iteration cooperates with the executor and observes
cancellation; no numeric batch policy is embedded in the macro.

Closures, async blocks, items, and other macro invocations retain their own Rust
syntax boundaries. Bind an Effect result to a local before passing it to another
macro, such as `assert!`. Native futures enter through `from_future` or its
explicit error/environment/owned variants. Pure per-run prefixes may clone Arc
or Effect handles explicitly when a reusable recipe needs to move them into
its next step; application payloads are moved.

## Values, captures, and reuse

Construction and cloning execute no user callbacks. Every run creates fresh
continuations and futures. Values created within a run can move through its
bindings without `Clone`:

```rust
use effect::{Effect, effect};
use std::sync::Arc;

struct Document(Arc<str>, String); // No Clone implementation.
let source: Arc<str> = Arc::from("content");
let program: Effect<Document, (), ()> = effect! {
    let source = Arc::clone(&source); // Explicit per-run capture.
    let suffix = Effect::sync(|| String::from("!"))?;
    Document(source, suffix)
};
let outcome = futures_executor::block_on(program.run(Arc::new(())));
assert!(matches!(outcome, effect::Exit::Success(Document(text, suffix)) if &*text == "content" && suffix == "!"));
```

Reusable stored callbacks are `Fn + Send + Sync + 'static`; per-run macro binds
are `FnOnce`. Share captured services through `Arc` or reproduce owned inputs
explicitly. `succeed`, `fail`, and stored Result/Option adapters clone their
payload on execution. `sync`, `fail_with`, and factories create fresh non-Clone
values. The core uses `A/E: Send + 'static` and `R: Send + Sync + 'static`.
The public API requires sendable execution; it has no local, thread-affine variant.

## Scoped children and shared outcomes

`Scope::fork` selects the task provider through `R: HasTaskSpawner`. Providers
drive whole children independently, including synchronous callbacks on separate
workers. The scope owns each child before submission, cancels and joins it at
close, and retains unobserved failures. Awaiting an outcome marks it observed;
checking `settled()` only reports status. Losing an accepted task driver reports
a typed defect. Cooperative cancellation awaits settlement and cleanup.

`join` and `await_exit` move the owned outcome once. Repeated observations use
`await_exit_shared` or `interrupt_shared`, retaining the same data without
`A: Clone`, `E: Clone`, or Sync bounds on payloads:

```rust
use effect::{Defect, Effect, Exit, HasTaskSpawner, Scope, SharedExit};

struct Document(Vec<u8>); // No Clone implementation.
struct LoadError; // No Clone implementation.

fn load<R>() -> Effect<SharedExit<Document, LoadError>, LoadError, R>
where R: HasTaskSpawner + Send + Sync + 'static {
    Scope::scoped(|scope| {
        scope.fork(Effect::sync(|| Document(Vec::new())))
            .flat_map(|fiber| fiber.await_exit_shared())
    })
}

fn byte_count(view: &SharedExit<Document, LoadError>) -> Result<Option<usize>, Defect> {
    view.with(|exit| match exit {
        Exit::Success(document) => Some(document.0.len()),
        Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
    })
}
```

Shared handles may outlive scope closure; the shared storage contains only the
outcome and status, with no additional environment/provider reference.
`with` borrows under a data lock and rejects escaping
references. Its synchronous callback must not re-enter inspection of the same
data. An owned join while shared views exist reports `FiberOutcomeShared`;
after those views are dropped it can take the canonical data. A subsequent
owned observation reports `FiberOutcomeConsumed`. Observed errors are not
reported again as unobserved failures when the scope closes.

Prefer moves and borrowing. `Arc::clone` shares an allocation but still performs
reference-count bookkeeping. Shared outcome storage is allocated only on shared
observation; the owned path transfers the existing value. The interpreter still
allocates recipes/continuations and uses dynamic dispatch, and child completion
uses locks and notifications. No zero-overhead claim is made. Macro syntax and
type/capability checks happen at compilation; live effects execute lazily at
runtime. `Defect::library` and `kind` support const contexts for static typed
conditions.

## Bounded traversal and races

`all_concurrent` and `for_each_concurrent` use an explicit `NonZeroUsize` limit
and schedule `min(limit, input length)` independent workers through R. Each
worker reserves one input at a time; a terminal failure stops reservations and
cancels the worker group. The operation joins every worker and its cleanup before
returning. Results retain input order without result/error cloning. Empty inputs
submit no tasks. This workflow limit is separate from a provider's physical
worker/admission limits and from CPU/blocking job admission.

The traversal callback borrows immutable input data rather than copying it.
Owned vectors are moved into shared collection storage once. Use
`all_concurrent_shared` / `for_each_concurrent_shared` to retain an existing
`Arc<[T]>` allocation directly. Borrowed inputs require Send + Sync; produced
values and errors require only Send, and neither requires Clone. Captures in
the returned reusable Effect must still have valid independent ownership.

```rust
use effect::{Effect, HasTaskSpawner};
use std::{cell::Cell, num::NonZeroUsize, sync::Arc};

struct Document(Vec<u8>); // No Clone.
struct Size(Cell<usize>); // Neither Clone nor Sync.
struct ReadError;

fn sizes<R>(limit: NonZeroUsize, documents: Arc<[Document]>)
    -> Effect<Vec<Size>, ReadError, R>
where R: HasTaskSpawner + Send + Sync + 'static {
    Effect::for_each_concurrent_shared(limit, documents, |document| {
        let size = document.0.len();
        Effect::sync(move || Size(Cell::new(size)))
    })
}
```

`left.race(right)` selects the first terminal outcome, including Fail or Die.
Selection commits at branch completion, so later polling order cannot replace
the winner. It cancels and joins the loser, retaining its cleanup failures.
Like the reference, ordinary losing primary errors are discarded; failed
cleanup remains visible. Accepted-task ownership loss is additionally retained
as a typed defect. Submission rejection aborts admission and joins work already
owned. Both operations preserve the active cancellation hierarchy through
environment projection; R still selects each subtree's task provider.

Input storage, ordered result slots, completion gates, and per-worker join state
are runtime costs. Sharing avoids per-run input copies and per-input task
submission, but does not remove allocation, reference counting or locking.

## Deferred completion, latches, and atomic references

`Deferred<A, E>` accepts the first complete `Exit<A, E>` and broadcasts to
waiters. `await_value` transfers that outcome once through the Effect channels;
`await_shared` and `poll_shared` expose a repeatable `SharedExit` with borrowed
inspection. Polling returns None while pending. Cancelling or dropping a pending
waiter affects only that waiter. These operations need no task provider and
work with any compatible host; cancellable execution still reads only R.

`complete`, `succeed`, and `fail` move a supplied non-Clone outcome on their
first execution. Re-running that completion recipe returns false. A pre-cancelled
run leaves its candidate available. `complete_with` creates a fresh candidate
per run, including unsuccessful attempts; only the first write is accepted.
Rejected candidates are destroyed outside the completion gate. `make` creates
a fresh Deferred lazily per run, while `new` creates a handle immediately.

Shared views retain the same success/error/cleanup payloads and outlive the
Deferred handle. An owned wait with active shared views reports
`DeferredOutcomeShared` without changing their data; after views are dropped,
unique ownership permits transfer. Subsequent owned/shared observation reports
`DeferredOutcomeConsumed`. `OutcomeStatus` classifies a shared exit; `FiberStatus`
names that same classification for fibers. Shared inspection explicitly handles
the stored outcome, including its errors. No payload cloning reconciles observers.

`Latch` is a persistent opening: `open` returns true once, and `await_open`
waits without consuming that opening. Cancellation/abandonment does not open it.
It stores a flag and broadcast event, without a payload outcome mutex.

`AtomicRef<T>` provides atomic compound operations under a mutex, with T: Send
and no blanket Clone/Sync requirement. `with` inspects a borrowed snapshot,
`modify` returns an owned result/replacement, and `modify_in_place` can reuse
existing storage. Replaced values are destroyed outside the value gate.
Callbacks are synchronous under that gate and must not re-enter the same
reference or return escaping borrows. Replacement commits only after its
callback returns. In-place/interior mutation follows Rust's normal semantics:
a panic can leave changes already performed. Panics become callback defects;
there is no rollback guarantee for those mutations.

```rust
use effect::{AtomicRef, Deferred, Effect, Exit, effect};
use std::{cell::Cell, sync::Arc};

struct Counter(Cell<usize>); // Neither Clone nor Sync.

let program: Effect<usize, ()> = effect! {
    let counter = AtomicRef::make_with(|| Counter(Cell::new(0)))?;
    counter.modify_in_place(|value| value.0.set(value.0.get() + 1))?;
    let current = counter.with(|value| value.0.get())?;
    let completed = Deferred::make()?;
    completed.succeed(current)?;
    completed.await_value()?
};
assert_eq!(futures_executor::block_on(program.run(Arc::new(()))), Exit::Success(1));
```

`make_with` / `set_with` produce fresh owned reference state without copying.
The stored-value `make` / `set` and owned `get` explicitly require Clone and
perform a copy; choose them when that ownership is intended. These facilities
still allocate shared state/recipes, perform ownership atomics and acquire
gates. Shared Deferred storage is allocated only when requested. Constant
syntax/types/classification can be resolved at compilation; live state and
cancellation require runtime work.

## FIFO queues and shutdown

`AsyncQueue<T>` moves Send payloads without requiring Clone or Sync. Choose
`QueueCapacity::Bounded(NonZeroUsize)`, `Unbounded`, or `Rendezvous` explicitly.
A bounded queue admits a waiting offer when a take frees space; unbounded queues
buffer values; rendezvous queues commit only when an offer and take meet.
Both buffered values and pending peers retain FIFO order. `size` counts only
committed buffer entries. `make` creates a fresh queue lazily per run; `new`
creates one immediately. The queue itself requires no runtime capability.

```rust
use effect::{AsyncQueue, Effect, Exit, QueueCapacity, QueueError, QueueShutdown, effect};
use std::{cell::Cell, sync::Arc};

struct Message(Cell<usize>); // Neither Clone nor Sync.

fn round_trip(size: usize) -> Effect<usize, QueueError> {
    effect! {
        let queue = AsyncQueue::make(QueueCapacity::Unbounded)?;
        queue.offer(Message(Cell::new(size)))?;
        let Message(value) = queue.take()?;
        queue.shutdown(QueueShutdown::Drain)?;
        value.get()
    }
}
assert_eq!(futures_executor::block_on(round_trip(7).run(Arc::new(()))), Exit::Success(7));
```

`offer(value)` moves one candidate on its first started execution; reuse reports
`LibraryDefect::QueueOfferConsumed`. A pre-cancelled execution leaves the candidate
available. `offer_with` makes fresh candidates on each run, while `offer_cloned`
explicitly copies a stored payload. Values created inside a per-run `effect!`
workflow can move into an owned offer without copying them.

Cancelling or dropping a pending operation removes its actual waiting node.
Matching peers skip requested nodes even while those drivers are unpolled.
Already committed transfers persist through later cancellation/abandonment;
final publication can report interruption after a take consumed its value.
The queue never rolls that value back or silently redelivers it.

`shutdown(QueueShutdown::Drain)` rejects pending operations/future offers with
the expected `QueueError::Shutdown` and retains only committed buffered values.
Takes consume that buffer, then fail with Shutdown. `Discard` also removes the
buffer; a later discard can clear a previously draining queue. Earlier pending
cancellation retains its interruption, including the raw release reason.
Nodes complete under the admission gate, then their notifications and discarded
payload destruction run outside it. Caught wake/destruction panics are retained
as ordered cleanup defects on the active operation. Abandonment cannot publish
fallible cleanup to a departed caller or perform asynchronous release in Drop.

Pending lists use safe indexed FIFO links with retained allocation identity,
constant-time unlink and free links embedded in vacant slots. Reusing a slot
cannot let an old guard remove a new peer. Insertion may grow storage; waiter
completion/event allocations exist only for pending operations. Buffer/pending
storage retains its observed high-water capacity for reuse; shutdown retires
pending storage and discard retires the buffer. Mutexes, node/queue Arc ownership,
events, work vectors, boxing and contention incur runtime costs. Typed offer/
take registrations share one node handle, avoiding an additional observation
clone and runtime role choice. `QueueCapacity::from_limit` supports actual const
contexts; negative capacity is unrepresentable in its unsigned input.

## Typed outcomes and dependencies

The complete `Exit<A, E>` is `Success`, `Fail`, `Die`, `Interrupted`, or
`FinalizationFailed`. Expected errors remain application types. `catch_all`
recovers ordinary `Fail`; `either`/`result` and `option` convert only expected
failures. `sandbox` maps a top-level defect explicitly. `map_error` transforms
expected errors retained in primary and cleanup outcomes as well as `Fail`.
Unwinding user panics are defects; aborting panics cannot be caught.

`Defect::kind()` supplies the semantic identity. Library failures use
`LibraryDefect`; caught unwinds use `DefectKind::CallbackPanic`; application or
adapter diagnostics constructed with `Defect::new` use `DefectKind::Custom`.
`message()` is human-readable presentation. Equality includes the identity,
so matching diagnostic text cannot impersonate another kind of failure.

```rust
use effect::{Defect, DefectKind, LibraryDefect};

let expired = Defect::from(LibraryDefect::ResourceScopeEnded);
assert_eq!(expired.kind(), DefectKind::Library(LibraryDefect::ResourceScopeEnded));
assert_ne!(expired, Defect::new(expired.message()));
```

All application dependencies and runtime providers belong in `R`. There is no
public run-options provider registry or second execution-services environment.
`service` accesses an environment, `provide` closes over a supplied environment,
and `provide_some` projects a parent environment for a subtree. Projection
preserves the current run's cancellation state, including when the projected
environment has a different external cancellation source. It does not begin a
new run. Independent executions can use the same recipe with different environments.

Runtime capability interfaces will ship with their working operations and
adapters. The planned task, CPU, blocking, clock, and I/O capabilities compose
together in `R`, allowing mixed async and parallel workflows. Their concrete
providers can come from Tokio, Rayon, or consumer implementations. Current
portable execution does not establish that these later operations exist.

## Running and cancellation

`run(env)` returns a sendable standard future and needs no cancellation
capability. It creates a fresh internal request source and intentionally does
not inspect an environment's optional traits. `run_cancellable(env)` requires
`R: HasCancellation`, reads that capability lazily when polled, and passes its
request source to native future factories. `Cancellation` itself implements
`HasCancellation`, so it can also serve as a minimal environment.

Both futures own their recipe handle and environment and can outlive the
borrowed description. Their explicit Rust 2024 `use<A, E, R>` bounds prevent
capturing that borrow in the opaque future type; see the
[edition guide on precise capture](https://doc.rust-lang.org/edition-guide/rust-2024/rpit-lifetime-capture.html).
This shares the existing recipe allocation and does not copy result/error data.

Cancellation is cooperative. A pre-cancelled run starts no ordinary callbacks;
a request between success and a bind skips the continuation. A pending native
future receives the token and is awaited until it settles. Non-cooperative work
can delay cancellation. Final publication changes ordinary success/failure to
interruption when requested, while defects and retained cleanup failures survive.

Request cancellation and keep awaiting to observe settlement. Dropping the
run future abandons execution; it does not perform cancel-and-join or promise
asynchronous cleanup. The owned-run boundary below separates the driver from
observation when the host must continue execution after observer loss.

## Resource ownership and owned execution

Declare acquisition and release together. The release callback receives the
owned resource and a borrowed primary-outcome view, so it requires no cloning
of either the resource or an expected error. `Release` is a single-use cleanup
description: its sync and future factories can consume the acquired resource.

```rust
use effect::{Effect, Exit, Release, ResourceBinding, effect};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

let released = Arc::new(AtomicUsize::new(0));
let witness = Arc::clone(&released);
let program: Effect<usize, (), ()> = effect! {
    let witness = Arc::clone(&witness);
    let resource = ResourceBinding::new(
        Effect::sync(|| String::from("document")),
        move |value, _reason| {
            let witness = Arc::clone(&witness);
            Release::from_future(move |masked| async move {
                assert!(!masked.is_requested());
                drop(value);
                let _ = witness.fetch_add(1, Ordering::SeqCst);
            })
        },
    )?;
    let length = resource.access(String::len)?;
    length
};

let (driver, observer) = program.owned_run(Arc::new(()));
// The host owns and polls the driver. It can retain it even if observation ends.
futures_executor::block_on(driver);
assert_eq!(futures_executor::block_on(observer), Exit::Success(8));
assert_eq!(released.load(Ordering::SeqCst), 1);
```

Ownership is installed after raw acquisition success and before checking
pending cancellation. The body is skipped when cancelled, but acquired values
still release. Release invalidates the access handle, runs with cancellation
masked, and is awaited before publishing the complete outcome. Nested bindings
release in reverse order. Every cleanup failure is retained alongside the
primary outcome. `ensuring` awaits protected cleanup for every started outcome;
`ensuring_with` can inspect its borrowed primary outcome. `uninterruptible` masks a started operation; a pre-cancelled
run still starts no work. `on_interrupt` awaits protected cleanup only when the
primary outcome is interrupted.

`ResourceValue::access` produces an Effect performing short synchronous access.
It reports a closed handle as a defect. `with` provides direct synchronous
access returning `Result`; neither allows a borrowed reference to escape.
Cloning or returning the access handle does not extend resource lifetime.
Copy owned data while accessing the resource; do not block, await release, or
duplicate a native ownership obligation inside the access callback. Resource
values currently require `Send + Sync`, without a `Clone` requirement.

`owned_run` and `owned_run_cancellable` return a `RunDriver` and a single-consumer
`RunObserver`. The host must poll the driver through settlement; the observer
receives the exit, including non-Clone/non-Sync payloads. Dropping the observer
does not stop the driver. Cancelling an owned run uses the same source in `R`.
Dropping the driver early reports an ownership-loss defect to its observer and
invalidates resource handles. It cannot run asynchronous cleanup in `Drop`.
These APIs start no hidden task and configure no executor. Scheduled owners
follow with the structured-concurrency increment.

## Dynamic resource scopes

`Scope::scoped` creates a fresh scope on each run. `acquire` declares ownership
before work starts, while `add_finalizer` and `add_release` register reusable
effects or fresh single-use releases. Each registration retains its environment;
one scope can include finalizers registered under different provided environments.

```rust
use effect::{Effect, Exit, Release, Scope, effect};
use std::sync::Arc;

let program: Effect<usize, String> = Scope::scoped(|scope| {
    let acquisition = scope.acquire(
        Effect::sync(|| String::from("scoped")),
        |value| Release::from_future(move |masked| async move {
            assert!(!masked.is_requested());
            drop(value);
        }),
    );
    effect! {
        let acquisition = acquisition.clone();
        let handle = acquisition?;
        let size = handle.access(String::len)?;
        if size > 0 { Effect::succeed(size)? } else { 0 }
    }
});
assert_eq!(futures_executor::block_on(program.run(Arc::new(()))), Exit::Success(6));
```

The scope awaits finalizers in reverse registration order even after a release
fails. It invalidates acquired handles and publishes every cleanup failure with
the body's primary outcome. A registration racing with close either joins that
drain or releases immediately and reports a closing-scope defect.

`scope.close()` joins one shared drain; concurrent callers receive
`ScopeCloseStatus::Clean` or `Failed`. The enclosing `scoped` effect owns and
publishes the actual typed errors exactly once. This deliberate Rust adaptation
avoids requiring `E: Clone + Sync` merely to share completion. A close status
does not recover or consume those errors. `scope.closed().await` observes the
same settlement, including after the enclosing effect has taken its outcome.
An early close ends resource lifetimes before the rest of the body continues.

The first closer's driver owns the drain. Retain its owned-run driver through
settlement. Losing a scope owner or close driver wakes observers with `Failed`,
without claiming successful cleanup or attempting async work in `Drop`.
Finalizers must not await their own scope's close: that would join themselves.

`sync` runs on the thread polling the interpreter. The interpreter returns
control to its host between typed transitions using standard `Poll`/`Waker`;
synchronous callbacks
must still return promptly. `zip`, `all`, and `for_each` are sequential, and
traversal creates subsequent callbacks only after preceding success.
`while_loop` checks a lazy guard and creates a fresh body per admitted iteration;
failure or cancellation prevents later guards and bodies from starting.
This cooperative handoff does not bound a callback's CPU time or guarantee host
scheduler fairness. Scheduling policy and worker providers belong in R. There
is no hard-coded batch count for ready interpreter transitions.

The implementation admits no arbitrary policy values. Fixed values require a
documented semantic basis; consumer policies are explicit and runtime providers
belong in R. Built-in failure identities and lifecycle states use Rust types.
Broadcast notification uses event-listener's all-listeners contract, and macro
package lookup uses the Cargo package identity. The source-inventory guard flags
new fixed numeric literals and unreviewed string-expression sites for review.

The core and macro have no Tokio, Rayon, or native-runtime production/build
dependencies. `from_future` accepts standard futures, but a wrapped Tokio
socket/timer still needs compatible infrastructure. Effect supplies neither a
reactor nor a hidden executor. Applications keep their host runtime alive for
the whole execution.

## Validation

To check the core and procedural macro, including external consumer portability,
run these commands from the workspace root:

```sh
cargo test -p effect -p effect-macros --all-targets
cargo test -p effect -p effect-macros --doc
cargo clippy -p effect -p effect-macros --all-targets -- -D warnings
python3 crates/effect/scripts/check_portability.py
```

## Scheduling, validation, and ordinary workflow control flow

Clock and randomness are environment capabilities. Deterministic schedules need
`HasClock`; jitter adds a compile-time `HasRandom` requirement. All delays,
recurrence counts, jitter bounds, and timeout errors are supplied by the caller.
Retry selects ordinary expected errors; repeat selects successes. Defects,
interruption, and finalization failures keep their original channels. Retry is
an explicit decision at an operation boundary whose idempotency the application
understands.

```rust
use effect::{Effect, Exit, Schedule, TestClock};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
let attempts = Arc::new(AtomicUsize::new(0));
let count = Arc::clone(&attempts);
let operation: Effect<usize, usize, TestClock> = Effect::try_sync(move || {
    let attempt = count.fetch_add(1, Ordering::SeqCst);
    if attempt == 0 { Err(attempt) } else { Ok(attempt) }
}, |error| error);
let program = operation.retry(Schedule::recurs(1));
# futures_executor::block_on(async {
assert_eq!(program.run(Arc::new(TestClock::default())).await, Exit::Success(1));
# });
```

`TestClock::advance` is FIFO at invocation and follows registered deadlines,
including new timers registered by a resumed Effect continuation. Register the
initial sleeper by driving its program before advancing. Virtual advancement
awaits continuation progress to the next native wait or outcome; it does not
represent CPU preemption or completion of unrelated application work.

`Schema` decodes borrowed JSON into owned values or structured application
errors. Parsing maps syntax errors separately. No derivation system or extra
format is implied. A reusable schema can be shared across boundaries:

```rust
use effect::{effect, Effect, Exit, Schema};
use std::sync::Arc;
#[derive(Debug)] enum Validation { Syntax, ExpectedInteger }
let schema = Schema::from_decoder(|json| json.as_i64().ok_or(Validation::ExpectedInteger));
let program: Effect<i64, Validation> = effect! {
    let schema = schema.clone();
    let mut total = 0;
    for text in ["20", "22"] {
        let value = schema.parse(text, |_| Validation::Syntax)?;
        total = Effect::sync_once(move || total + value)?;
    }
    total
};
# futures_executor::block_on(async {
assert!(matches!(program.run(Arc::new(())).await, Exit::Success(42)));
# });
```

`Layer` constructs dependencies within a scope and provides their environment
to a subtree. `Stream` owns its reader in a scope, moves chunks and items, and
awaits explicitly supplied native disposal. Collection limits are caller policy.
Neither facility requires payload cloning.

## Runtime providers and managed services

The optional `effect-tokio`, `effect-rayon`, `effect-native-runtime`, and
`effect-tracing` crates supply providers that coexist in R. CPU and blocking
operations dispatch through separate capabilities; they always join accepted
work after cancellation. Single-use constructors move non-Clone/non-Sync captures;
reusable factories produce fresh values. Context-aware job variants carry trace
metadata explicitly. They introduce no global backend mode.

Logger/Tracer services filter and redact before calling configured backends.
Span parents and immutable annotations travel in interpreter state through
children, CPU jobs, and environment projection. Instrumentation failures enter
an explicitly bounded/unbounded Diagnostics service and preserve application
outcomes. `effect-tracing` receives an explicit tracing dispatcher, capacity,
and diagnostics at construction. Provider accessors should be infallible;
accessor unwinds are isolated, but an inaccessible logger/tracer cannot supply
its own diagnostic service.

HTTP/Fetch and WebSocket workflows obtain transports from R. Native providers
retain their reactor/client, return owned operations, make aborted operations
settle, and await disposal. No global HTTP client, socket implementation, or
transport SDK is selected by core. [RUNTIMES.md](RUNTIMES.md) describes provider obligations and costs.

`Pool` owns resource slots, validates borrowed idle values, applies explicit
capacity/idle TTL, populates through the construction environment's task provider,
and joins population and leases at shutdown. Lease inspection borrows an
invalidatable resource; release/discard return status while the owning scope
retains canonical cleanup errors.

`Cache` shares in-flight producers and retains outcomes according to explicit
TTL, capacity, equality, and expected-error policy. `SharedValue<A/E>` handles
share canonical data without cloning it. Waiter cancellation leaves the producer
owned by the cache; invalidation/shutdown request and join that producer.
Observed cleanup failures are removed from diagnostic retention.

`WeakCache` uses weak key and weak association references. A consumer value handle
keeps its association alive; dropping the last such handle may cause a miss even
while the key lives. This is an explicit Rust adaptation, not tracing-GC ephemeron
semantics. In-flight ownership and unobserved cleanup can retain their graphs
until settlement/drain.

The [effect-app consumer](../effect_app/src/lib.rs) combines actual
async file I/O, validation, bounded retry, scoped cache/children/staging file,
Rayon CPU work, Tokio blocking work, tracing, cancellation, and complete Exit
reporting. All deployment counts and timing policy are required configuration.

## Migration

JSON Schema export uses a safe AST approximation. In-memory workflow engines
remain volatile.

The Effect crates and their native consumers require Rust 1.99.0.
`Effect<A, E, R>` retains its runtime-independent capability boundary. An
application can supply asynchronous, CPU and blocking providers in one `R`.
No executor is selected by schema or procedural-macro expansion.

### Native and effectful schemas

A native record declaration supplies both a representation and an unknown-field
policy. Each field codec has the same encoded representation and independently
selected decode and encode environments. Values move into constructors and
encoders; cloning a codec shares its immutable description.

```rust
use effect::{schema, json_i64, Codec, JsonRepresentation, JsonValue,
    ParsePolicy, SchemaIssue, UnknownFields};
struct Count { count: i64 }
fn count_schema(policy: ParsePolicy) -> Result<Codec<Count, JsonValue, (), ()>, SchemaIssue> {
    schema! {
        struct Count {
            #[schema(default_with = || effect::Effect::succeed(1i64))]
            count: json_i64().into_effect::<(), ()>()
        }
        policy: policy,
        representation: JsonRepresentation,
        unknown_fields: UnknownFields::Reject
    }
}
```

Defaults run only for absent fields; present null is passed to the field codec.
A default needs an independent borrowed type-validation witness. Missing
witnesses fail with `MissingValidator`. The former
`with_effectful_field_default` JSON adapter has been removed: it encoded a
supplied default and decoded it again. Use `NativeRecordBuilder::default_with`
or the declaration above to retain the supplied typed value.

`DomainCodec` preserves an application error type and both environments through
optional/null, sequence and map composition. Representation-boundary failures
have separate caller-supplied decode and encode mappings. Optional native
values use explicit absence/omission factories rather than treating null as an
absent field.

`configured(SchemaPolicy)` supplies independent schema traversal, JSON depth,
Unicode string and integer digit budgets. Checked patterns obey their explicit
`PatternPolicy` bounds and share the configured `pattern_steps` budget. Invocation state is fresh
and owned by that run; immutable AST preparation is shared. Exact decimal JSON
number constraints use `JsonExactNumber`, whose comparisons never expand a
large exponent or pass through floating point. Checked regex patterns require
explicit compilation, syntax and execution budgets and reject unsupported
syntax. JSON Schema export labels checked/opaque constraints as approximation
rather than claiming constraints it cannot represent.

### Encoding

A direct writer is an explicit witness. `write_into` writes one value into a
caller-owned checked container, and `write`/`write_once` finish a document under
`JsonWriterPolicy`. Effectful writing owns the output across suspension.
Writing failure never retries through the ordinary encoder. Codecs without a
writer fail before invoking callbacks; callers can explicitly choose the
ordinary JSON-tree encoder. UTF-8 parsing is strict and duplicate JSON fields
follow the declared policy.

The existing SchemaBinary identity and format version are unchanged.
`SchemaBinary::effect_codec` adapts typed effectful directions to that format.
Its existing numeric representation rejects exact JSON numbers that cannot be
represented losslessly; it does not coerce newly supported arbitrary decimal
numbers into a floating-point value or silently change durable bytes.

### Caching, callbacks and cleanup

`CacheOptions` adds explicit active-load admission and fixed or outcome-based
TTL. `get_observed` distinguishes retained results, shared in-flight work and
new loads at selection time. `DependencyCache` reuses that engine, with separate
retention, active and dependency bounds. Invalidation fences old entries before
replacement becomes visible, then cancels and awaits retirement. Equality and
TTL callbacks run outside the cache gate. A cancelled waiter does not cancel a
still-owned shared producer; producer interruption is abandonment.

`CallbackPubSub` excludes backpressure and uses immediate publication with an
explicit contention policy. Its subscriptions and streams remain scoped.
`Stream::from_async_iterable_effect` obtains its source through an effect and
awaits disposal through the existing resource scope. Finalizer protection is
established before invoking release factories. `EffectConfig` preserves its
decode environment and uses defaults only for genuinely missing input.

### Lifecycle migrations

- `WorkflowEngine::transition` and `WorkflowCommand::Transition` now require a
  `WorkflowCheckpoint`, obtained from the current snapshot's `checkpoint()`.
  Snapshots include `execution`, initialized to zero on a new run and advanced
  on each new running body. Recovery retains the acquired checkpoint, so an
  old body cannot reset a replacement execution. Snapshot providers/restoration
  code must retain this counter; do not fabricate an old checkpoint from status
  alone. This volatile engine does not claim physical persistence compatibility.
- `EntityResource::open` requires activation-store access through `R`, an error
  scope for `EntityResourceError`, and an explicit effect factory obtaining the
  current instant. Ownership is checked after construction before live state
  is published; the scope retains disposal ownership on failure.
- `EventReplicator::import`/`import_and_ack` require a `ReplicaImportPermit`.
  Obtain it with the application's typed authorization decision through
  `ReplicaImportPermit::authorize`. It is bound to a source and journal stream.
  Denial or a mismatched token precedes cursor/journal access. The unencrypted
  `EventLogRpc::handler` requires the same checked authorization callback.
  Authenticated protected transfers retain their existing protection contract.
- RPC stream readers share an exactly-once close gate with their scope. Failed
  or interrupted admitted reads/acknowledgements retire the owned session.
  Reactive cleanup removes a node only when it still owns that registration,
  and dependency replacement checks the current evaluation generation.

Transaction cancellation remains interruption. Each retry owns a fresh journal
and rebuilds its iterable factory. Transaction descriptions retain the stronger
compile-time boundary that excludes arbitrary effects and futures.

### Exhaustive matches and composition migrations

Update exhaustive `IssueSegment` matches for `PropertyName` and `Alternative`.
Update `SchemaIssueKind` matches for `MissingValidator`, `NoAlternative`,
`AmbiguousAlternatives`, `Constraint`, `ResourceDemand`, `PatternBudget`, and
`MergeConflict`; update `SchemaResource` for `JsonDepth`, `IntegerDigits`,
`StringLength`, and `PatternSteps`. `PubSubError` includes `Contended`.

`AnyOfValue` retains private construction invariants; inspect it with `input()`
and `matches()`. Type-side unique arrays need a genuine semantic equality
witness (`with_equivalence`) as well as an independent element validator.
Custom native field implementations now receive the active `ParsePolicy` and
shared `IssueBudget` for decode, encode, and validation.

`WorkflowRunError::Recovery` retains `run` and the exact failed reset
`checkpoint`. A reusable run effect retains that fence and its encoded input
for typed `Effect::retry` with an application schedule. Cluster callers can
send `WorkflowCommand::Resume { run, checkpoint: Some(checkpoint) }` to retry
that fenced reset. Ordinary suspended resumes use `checkpoint: None`.
Completed runs acknowledge their recorded outcome; a stale reset fence cannot
reset a replacement execution. The in-memory engine remains volatile.

Reactive error matches also handle `AtomError::RetainedFailure`: a publication
may already have shared a non-Clone evaluator error before reporting a defect.
Inspect the retained error with `SharedValue::with`; no error payload is copied.
`AtomError` and `AtomCacheError` make their existing execution `Send + 'static`
requirements explicit. Native custom defaults can use `default_with_context`
to retain the invocation policy; `schema!` uses this contextual witness.

`AtomRegistryError::StaleOwner(AtomId)` rejects old source writes, dependency reads,
and derived refreshes after an identity is removed and registered again. Update
exhaustive matches; registration identity is checked under the graph gate.
