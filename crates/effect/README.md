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

## Composable domain blocks

The crate exposes typed computation and domain macros plus named callback blocks. Each domain retains a typed execution
boundary and uses the same `Effect<A, E, R>` kernel and environment capabilities.
`yield_from!` is a local marker consumed inside `stream!`.

| Domain | Macro surface | Execution boundary |
| --- | --- | --- |
| General composition | `effect!`, `span!`, `service!` | Ordinary Effect; tracing uses R, selected services retain R, `project:` explicitly supplies another environment. |
| Streams and caches | `stream!`, `cache!`, `dependency_cache!` | Pull/consume and scoped cache creation/lookup Effects; policies are explicit. |
| Schema and persistence | `schema!`, `schema_enum!`, `persistable!`, `persisted_queue!`, `durable_queue!` | Existing encode/decode, record and queue execution APIs. |
| RPC | `rpc_operation!`, `rpc_stream_operation!`, `rpc_protocol!`, `rpc_stream_protocol!`, `rpc_middleware!`, `rpc_stream_middleware!` | Typed handler calls, protocol handling and full session serving. |
| Reactivity | `atom!`, `reactive_source!`, `reactive_derived!`, `reactive_query!`, `reactive_bridge!`, `hydration!` | Tracked `AtomProgram::evaluate`, atom refresh and hydration export/import. |
| Cluster | `entity_resource!`, `entity!`, `cluster!`, `singleton!`, `cluster_schedule!` | Resource acquisition/activation under an explicit owner and scope, existing fenced dispatch and singleton run. |
| Events | `event!`, `handled_event!`, `event_group!`, `projection!` | EventLog execution and atomic projection advance. |
| Restricted coordination | `transaction!`, `workflow!`, `activity!`, `workflow_definition!`, `activity_definition!` | Atomic journal execution or recorded durable workflow/activity execution. |

Inline callback fields omit the colon (`handler |request| { ... }`) to request
Effect lowering. Colon fields (`handler: handle_request`) accept already typed
callbacks and extracted helpers. Native closure parameters may carry ordinary Rust type annotations when field
access needs a receiver type before constructor inference. The new domain parsers
evaluate runtime fields and list entries once in written order. Construction validates descriptions but does
not execute effectful bodies. Native match guards remain intact.

```rust
use effect::{Effect, Stream, service, stream};
use std::sync::Arc;

fn increment<R: Send + Sync + 'static>(value: i64) -> Effect<i64, (), R> {
    Effect::sync_once(move || value + 1)
}
let selected: Effect<i64, (), ()> = service! {
    select: |_: &()| 4i64,
    run |service| { increment(*service)? },
};
assert_eq!(futures_executor::block_on(selected.run(Arc::new(()))), effect::Exit::Success(5));
let values: Stream<i64, (), ()> = stream! {
    for value in 0..2 { yield increment(value)?; }
    yield_from!(Stream::from_iterable(|| [3].into_iter()));
};
assert_eq!(futures_executor::block_on(values.run_collect(Some(3)).run(Arc::new(()))), effect::Exit::Success(vec![1, 2, 3]));
```

Stream bodies are pull driven. Resource binds survive yields and release at
lexical completion, loop exits or consumer cleanup. No producer task is started.
Inline cache loaders receive `Arc<K>` so an owned key handle can enter a deferred
Effect; colon-form loaders retain the existing borrowed-key constructor contract.
Consumers explicitly clone shared handles when a reusable body moves a handle.

Extract tracked helpers as `AtomProgram<A, E, R>`, then use `program:` in
`reactive_derived!`; tracked reads use `AtomProgram::read`/`read_derived`. Extract
activation helpers as `EntityResourcePlan<A, E, R>` and pass them to the
resource-aware `entity!` form. `cluster!` returns a checked catalog; activation
requires the caller's existing owner and scope, and each dispatch rechecks its
fence. `rpc_middleware! { acquire { ... } }` returns service-producing middleware;
its typed `apply` or `serve_stream` operation owns acquisition through handler
completion and cleanup, including session closure.

`handled_event!` joins explicit aggregate/commit metadata to a typed Effect
handler. `projection!` typed `cases:` derive registered identities and decoders
from those same definitions; failures use `ProjectionCaseError<F>`. Ordinary
`accepted:`/`fold:` projection and descriptor-only RPC/event forms remain valid.
Queue workers retain the existing recorded-result-before-acknowledgement order.
Transactions admit only Transactions in every bind position; workflows admit
only Durable descriptions. Put external Effects inside recorded activities,
and execute transactions through `atomically`. Retry/replay locals are rebuilt
lazily. Durable coordination supports deterministic `for` iteration; unbounded
`while`/`loop` remains rejected at that boundary.

## Runnable café stories

Start with a customer or staff task below. The [example build/run guide](examples/README.md)
provides explicit commands for all 15 targets, including file setup and supported modes.
Each file introduces its own café models and reads as one recipe with direct `?`
binds and one execution boundary in `main`. Prices use integral cents and are
printed as dollars. `CoffeeOrder`, `Basket`, `Receipt`, and the other café models
are application functionality; the explicitly imported Effect types and macros
provide composition, resource lifetimes, recording, and reactive updates.
Inside `effect!`, `service!`, and inline domain callbacks, ordinary statements
and return values already belong to the deferred recipe. Ticket construction,
request cleanup, status values, and receipt calculations use those bodies
directly. Explicit constructors remain at required resource, typed Result,
CPU/blocking, codec, and provider boundaries.

Macro layout is enforced by a source test: if a braced group spans multiple
source lines, its first code token must start after the opening brace's line.
This includes nested groups inside macro inputs. One-line groups are allowed;
normal comments are omitted, doc-comment attributes are skipped, and literal
interiors are not traversed. Run the focused check with
`cargo +1.99.0 test --locked -p effect-macros --test macro_layout -j 2`.
The same check runs with `cargo test -p effect-macros`; stable rustfmt does not
currently enforce this custom macro-input rule.

| Task | Concrete input and useful output | Main DSL role |
| --- | --- | --- |
| [Browse the menu](examples/browse_cafe_menu.rs) | Three drinks priced in cents; Maya identifies dairy and confirms her oat-latte price. | Stream choices, select the board, retain the visit menu in a scoped cache. |
| [Publish a price change](examples/update_menu_prices.rs) | Oat latte changes from 475 to 500 cents; the same live visit owner still returns Maya's accepted 475-cent quote. | Dependency invalidation and weak-key quote retention. |
| [Save a corrected order](examples/save_coffee_order.rs) | Quantity 0 is corrected to two oat lattes; the saved note and quantity reload intact. | Configured directional codecs, validation and persistence. |
| [Check out a basket](examples/live_coffee_basket.rs) | Totals change from $12.75 to $8.00; stock reservation is atomic, excessive demand leaves stock unchanged, and the saved basket restores. | Tracked totals, stock transactions, query snapshots and hydration. |
| [Place an order](examples/place_coffee_order.rs) | A padded pickup name and two cups become accepted ticket 104 and a typed RPC response. | Request middleware, a selected desk and unary protocol dispatch. |
| [Follow recorded order history](examples/track_coffee_order.rs) | Ticket 104's supplied stages become accepted/preparing/ready messages with its recorded cup count. | Streaming protocol, middleware and scoped transport close. |
| [Read phone messages](examples/phone_order_status.rs) | A supplied finite native status feed is consumed through peer close. Add `-- --accepted` for an already accepted socket. | Checked socket ownership and pull-driven consumption. |
| [Issue recorded pickup labels](examples/prepare_coffee_orders.rs) | Maya's two-cup request issues labels once; the same recorded pickup is returned twice. | Workflow/activity recording and replay. |
| [Deliver one queued ticket](examples/queue_coffee_orders.rs) | Nora's ticket 106 produces pickup instructions and acknowledgement; the next claim is empty. | Persisted queue, durable delivery and recorded result. |
| [Resolve pickup tickets](examples/resolve_pickup_tickets.rs) | Stored tickets 104/105 return their customer/cup data; 999 returns UnknownTicket. | Batched resolver requests with ordered per-request outcomes. |
| [Return a cancelled reservation](examples/reserve_cancelled_order.rs) | Stock 8 becomes 6 after reserving two cups, then 8 after compensation; replay leaves it at 8. | Atomic stock modification and recorded compensation. |
| [File the closing checklist](examples/close_coffee_shop.rs) | Ticket 104's recorded two cups appear in the inspection and scheduled 17:00 UTC checklist. | Resource-aware addressed entities, calendar planning and a closing lease. |
| [Issue receipts and total sales](examples/coffee_shop_receipts.rs) | Maya/Lee receive 950/325-cent receipts; the default view reports three cups/$12.75. `-- --book` selects the detailed receipt book. | Journal commits and raw/typed projection views. |
| [Export customer totals](examples/export_receipts.rs) | Caller-supplied counter/mobile CSV batches become sorted customer totals in a new output file. | Bounded borrowed/shared traversal, blocking file I/O and CPU aggregation. |
| [Append the receipt book](examples/write_receipt_book.rs) | Two receipt rows go into a caller-named new CSV, then the file is flushed and closed. | Pooled file ownership and per-outlet admission. |

Examples share [native provider wiring](examples/cafe_support/mod.rs),
[transport mechanics](examples/cafe_support/transport.rs), and small domain
modules for menu data and ticket codecs. Their business recipes remain in the
named entry files. Core examples use in-memory journals/stores and do not persist
across restarts. Export and receipt-book examples use the actual files selected
by the caller; neither collects a payment or sends a notification.

For file examples, supply the declared paths after `--`. Receipt export accepts
`COUNTER.csv MOBILE.csv OUTPUT.csv`; input rows are
`ticket,customer,cups,unit_cents` without a header or quoted commas. For
`104,Maya,2,475` and `105,Lee,1,325`, output is `Lee,325` then `Maya,950`.
Output files must not already exist.

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

Directional `Codec` descriptions keep decode and encode environments separate.
A configured codec prepares metadata once and supplies fresh context/budgets for
parsing. The following recipe reuses one explicit text envelope for its resource
bounds; schema errors remain typed throughout the Effect.

```rust
use effect::*;
use std::num::NonZeroUsize;
use std::sync::Arc;
let bytes = NonZeroUsize::new(4096).unwrap();
let depth = NonZeroUsize::new(16).unwrap();
let parse = ParsePolicy {
    limits: ParseLimits { bytes, depth, nodes: NonZeroUsize::new(128).unwrap(), collection: NonZeroUsize::new(128).unwrap(), issues: NonZeroUsize::new(8).unwrap() },
    errors: SchemaErrorMode::First, unknown_fields: UnknownFields::Reject, duplicate_fields: DuplicateFields::Reject,
};
// JSON nesting follows the schema envelope; scalar and digit counts follow
// input bytes. Pattern search uses an explicit work budget from that envelope.
let codec = Arc::new(json_i64().into_effect::<(), ()>().configured(SchemaPolicy {
    parse, json_depth: depth, string_length: bytes, integer_digits: bytes, pattern_steps: bytes,
}).unwrap());
let program: Effect<i64, SchemaIssue> = effect! {
    let codec = codec.clone();
    let mut total = 0;
    for text in ["20", "22"] {
        total += effect_schema_parse! { codec: codec.as_ref(), text: text }?;
    }
    total
};
assert!(matches!(futures_executor::block_on(program.run(Arc::new(()))), Exit::Success(42)));
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

### Native extension café consumers

These stories keep application identities and caller policies visible. SQL,
Server and Identity finish their demonstrated workflow and exit. Web and OTel
serve until Ctrl-C; their outer owners then join host/resource and provider
shutdown. Run from the workspace root on Rust 1.99:

| Task | Input, outcome and DSL role |
| --- | --- |
| [Correct a saved price](../effect_sql/examples/cafe_orders.rs) | Enter a flat white at 400 cents, correct it to 425, then reload `ready=false`. SQL statement/row blocks preserve bound values, strict decoding and the transaction owner. |
| [Retrieve a saved order](../effect_server/examples/serve_cafe_orders.rs) | Customer 7 requests saved order 42; the finite native HTTP exchange prints authorized HTML and JSON. Endpoint, representation, route and pipeline blocks retain request/body ownership. |
| [Save a receipt preference](../effect_server_identity/examples/customer_account.rs) | Register a fresh Maya alias, verify its password with native Identity, then save/reload `email_receipts=true`. Store/component blocks select the real SQLx store and native security policies. |
| [Sign in and collect an order](../effect_server_web/examples/README.md) | Open the local café page, sign in through native Identity, view your own ready order and confirm collection. Owner-bound SQL records collection once and retains Collected on reload; the public menu and operational probes remain available. |
| [Replay a recorded timeline](../effect_otel/examples/watch_order_progress.rs) | Caller-provided `elapsed_ms,status` records become SSE updates with native spans. Choose one inline, extracted or body writer; Ctrl-C joins host/provider cleanup. |
| [Wait for either outlet](../effect_otel/examples/await_pickup.rs) | Two actual ticket-status files show current availability, then identify the first ready outlet or return a wait-window error. Native I/O, concurrent snapshots, polling/retry schedules, race and timeout use the existing Tokio providers. |

```sh
DATABASE_URL=turso::memory: cargo +1.99.0 run -p effect-sql --features turso,macros --example cafe_orders
CAFE_CHUNK_BYTES=4096 CAFE_JSON_BYTES=1024 CAFE_DRAIN_MS=3000 CAFE_TELEMETRY_CAPACITY=32 cargo +1.99.0 run -p effect-server --example serve_cafe_orders
CAFE_DATABASE=target/receipt-accounts.db cargo +1.99.0 run -p effect-server-identity --features sqlx-turso --example customer_account
cargo +1.99.0 run -p effect-otel --features server --example watch_order_progress -- 127.0.0.1:8080 HISTORY.csv 256 32 32 16 8 5000 inline
cargo +1.99.0 run -p effect-otel --features server --example await_pickup -- NORTH.status SOUTH.status 2000 20
```

Timeline CSV contains no header: for example, `0,accepted`, `400,preparing`,
`800,ready`. Read `/orders/104/events` with `curl -N`. The writer argument may
also be `extracted` or `body`; each run mounts the same single route. Timeline
replay reports the supplied history and does not operate a coffee machine.
Outlet status files contain `104,pending` or `104,ready`; staff may update either
while the caller waits. The final two outlet arguments are positive wait-window
and poll milliseconds. Retry covers only transient native I/O failures, without
manufacturing a failure. Race and timeout retain loser cancellation and settlement.

Complete build/run instructions and input setup are available in the
[SQL](../effect_sql/examples/README.md),
[Server](../effect_server/examples/README.md),
[Identity](../effect_server_identity/examples/README.md),
[Web](../effect_server_web/examples/README.md), and
[OTel](../effect_otel/examples/README.md) example guides.
Web's native configuration is implemented in
[cafe_host](../effect_server_web/examples/cafe_host/mod.rs). It serves `/menu`,
cookie-authorized `/pickup`, and availability on the selected `CAFE_LISTEN`.
Supply the selected native key/catalogue files before running; account-only
registration/login/preference work needs just its selected database, without
cookie-key or external-provider setup.

## Named typed callback blocks

Callback roles use `role |pattern| { statements; tail }`, zero-input `role { ... }`,
or `role: typed_helper`. Metadata uses `name: expression,`. Fields evaluate once
in written order; executable bodies remain delayed by their owning operation.
Native Result/Future bodies keep Rust `?` and explicit `async move`. Effect roles
use Effect binds. A reusable outer callback may return a fresh one-shot inner
Effect for each owned invocation input. `once;` consumes a native capture without
copying it. R continues to select the environment and async/CPU/blocking providers.

Each runnable task above has an explicit input and observable business result.
Codecs describe the order's wire shape; stock transactions preserve atomic
reservation; workflow/queue recording retains pickup results; scoped providers
retain native resource ownership through cleanup. Optional extension facades
re-export the same callback macros through their existing Effect dependency.

A small intake recipe can validate confirmed pickup names and build the labels
staff will actually read. The input below is a finite native stream of two names;
its Future transfers the owned intake batch. Lee supplied a confirmed correction
for the blank entry, so recovery uses actual customer input. The result is two pickup labels.

```rust
use effect::{Effect, Exit, effect_all, effect_future, effect_map, effect_map_error, effect_recover, effect_try_sync};
use futures_util::StreamExt;
use std::sync::Arc;
use futures_executor::block_on;

/// Validate a customer's supplied pickup name without copying its owned payload.
fn pickup_name(name: String) -> Effect<String, &'static str> {
    effect_try_sync! {
        once;
        body {
            if name.trim().is_empty() { Err("provide a pickup name") }
            else { Ok(name.trim().to_owned()) }
        },
    }
}
let input = futures_util::stream::iter([(" Maya ".to_owned(), None), (String::new(), Some("Lee".to_owned()))]);
let intake: Effect<Vec<(String, Option<String>)>, &'static str> = effect_future! {
    once;
    body |_cancel| {
        async move { input.collect::<Vec<_>>().await }
    },
};
let program = intake.flat_map(|names| {
    effect_all! {
        items {
            for (entered, confirmed_correction) in names {
                let validated = effect_map_error! {
                    program: pickup_name(entered), map |_error| { "pickup name needs correction" },
                };
                let corrected = effect_recover! {
                    program: validated,
                    recover |reason| {
                        let correction = confirmed_correction.clone();
                        match correction {
                            Some(name) => pickup_name(name)?,
                            None => Effect::fail_with(move || reason)?,
                        }
                    },
                };
                item!(effect_map! {
                    program: corrected, map |name| { format!("Pickup for {name}") },
                });
            }
        },
    }
});
assert_eq!(block_on(program.run(Arc::new(()))),
    Exit::Success(vec!["Pickup for Maya".to_owned(), "Pickup for Lee".to_owned()]));
```

A receipt export can narrow the caller's configuration to a file writer, commit
and flush one receipt, then read it back through a Drop-owned file. The native
file adapters retain I/O errors. The short commit is masked across write/flush;
an interrupted attempt removes its unpublished file. A finalizer removes this
example's owned temporary receipt after reading, including on failure.

```rust
use effect::*;
use std::{fs::File, io::{Read, Write}, path::PathBuf, sync::Arc};
use futures_executor::block_on;

/// Caller selects the receipt destination before constructing the recipe.
struct ReceiptHost { /// Explicit caller-owned export path.
    path: PathBuf }
/// Narrow writer configuration; it has no runtime or global lookup.
struct ReceiptTarget { /// Destination selected by the caller's layer.
    path: PathBuf }
/// Native file failures remain distinct from an unexpected adapter defect.
#[derive(Debug)]
enum ReceiptError {
    /// Original native file error.
    Io(std::io::Error),
    /// Original unexpected native adapter defect.
    Native(Defect),
}
/// Remove only this recipe's output; absence is already the desired cleanup state.
fn remove_receipt(path: PathBuf) -> Effect<(), ReceiptError, ReceiptHost> {
    effect_try_sync! {
        body {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(ReceiptError::Io(error)),
            }
        },
    }
}
/// Acquisition is delayed; resource release owns the actual file flush and close.
fn write_receipt() -> Effect<(), ReceiptError, ReceiptTarget> {
    effect! {
        let path = service! { env |target: &ReceiptTarget| { target.path.clone() }, }?;
        effect_resource! {
            acquire: effect_try_sync! {
                once; body { File::options().write(true).create_new(true).open(path).map_err(ReceiptError::Io) },
            },
            body |file: ResourceValue<File>| {
                let result = file.access(|file| {
                    let mut file = file;
                    file.write_all(b"104,Maya,950\n")
                })?;
                Effect::from_result_once(result.map_err(ReceiptError::Io))?;
            },
            release |file: File, _primary| {
                effect_try_sync! { once; body { file.sync_all().map_err(ReceiptError::Io) }, }
            },
        }?;
    }
}
/// Native File Drop closes the read descriptor after the owned CSV is returned.
fn read_receipt() -> Effect<String, ReceiptError, PathBuf> {
    effect! {
        let path = service! { env |path: &PathBuf| { path.clone() }, }?;
        effect_using! {
            acquire: effect_try_sync! { once; body { File::open(path).map_err(ReceiptError::Io) }, },
            body |file: ResourceValue<File>| {
                let text = file.access(|file| {
                    let mut file = file; let mut text = String::new();
                    file.read_to_string(&mut text).map(|_| text)
                })?;
                Effect::from_result_once(text.map_err(ReceiptError::Io))?
            },
        }?
    }
}
let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
let path = std::env::temp_dir().join(format!("effect-receipt-{}-{nonce}.csv", std::process::id()));
let host = Arc::new(ReceiptHost { path: path.clone() });
let writer: Layer<ReceiptHost, ReceiptError, ReceiptTarget> = layer! {
    build |_scope| {
        service! { env |host: &ReceiptHost| { ReceiptTarget { path: host.path.clone() } }, }?
    },
};
let commit = effect_with_layer! { layer: writer, program: write_receipt() };
let commit = effect_uninterruptible! { program: commit };
let interrupted_path = path.clone();
let commit = effect_on_interrupt! {
    program: commit, finalizer { let path = interrupted_path.clone(); remove_receipt(path)?; },
};
let reader = effect_provide_some! {
    program: read_receipt(), project |host: &ReceiptHost| { host.path.clone() },
};
let cleanup_check = path.clone();
let export: Effect<String, ReceiptError, ReceiptHost> = effect_scope! {
    body |scope: Scope<ReceiptError>| {
        let commit = commit.clone(); let reader = reader.clone(); let path = path.clone();
        effect_finalizer! { scope: scope, body { let path = path.clone(); remove_receipt(path)?; }, }?;
        effect_sandbox! { program: commit, defect |defect| { ReceiptError::Native(defect) }, }?;
        reader?
    },
};
let provided = effect_provided! { environment: host, program: export };
assert!(matches!(block_on(provided.run(Arc::new(()))), Exit::Success(csv) if csv == "104,Maya,950\n"));
assert!(!cleanup_check.exists());
```

The runnable [receipt-book task](examples/write_receipt_book.rs) demonstrates
checked file ownership, native write/flush errors, scoped disposal and per-outlet
admission. [Closing](examples/close_coffee_shop.rs) retains its addressed pickup
record until the activation scope closes. The owning resource operation selects
whether release receives a checked value, an owned payload, or a borrowed primary
outcome; that primary borrow cannot escape into a deferred cleanup task.

The [basket](examples/live_coffee_basket.rs) computes a tracked customer total,
reserves real stock together, commits restock and restores actual saved selections.
The [cancelled-reservation task](examples/reserve_cancelled_order.rs) returns
reserved stock once under a recorded compensation identity.

Provider-dependent callbacks use the application's selected R and policies:

| Executing task | Callback role and observable result |
| --- | --- |
| [Export receipts](examples/export_receipts.rs) | Two real file reads use bounded borrowed traversal; shared snapshots are validated on CPU workers and aggregated before creating sorted output. |
| [Wait for pickup](../effect_otel/examples/await_pickup.rs) | Concurrent native outlet reads produce actual snapshots; repeat polls pending records, retry handles transient reads, race selects the first ready outlet and timeout bounds the wait. |
| [Write the receipt book](examples/write_receipt_book.rs) | One actual pooled file is reused for both appends; a partitioned permit protects that outlet, and final scope cleanup flushes the file. |
| [Resolve stored tickets](examples/resolve_pickup_tickets.rs) | The resolver returns supplied customer/cup records and a typed missing-ticket outcome. |
| [Retain a visit quote](examples/update_menu_prices.rs) | The customer retains the visit key and quote owner across board publication; the invalidated public cache reloads 500 cents while the quote remains 475 cents. |
| [Consume phone status](examples/phone_order_status.rs) | Connected and already-accepted modes consume one selected native feed until peer close, then complete close/disposal. |

A receipt total is a useful stream fold: two customers buy two and three cups
at 475 cents each. The iterator is dropped normally when consumed; the fold
returns 2375 cents ($23.75).

```rust
use effect::{Exit, Sink, Stream, sink, stream};
use futures_executor::block_on;
use std::sync::Arc;
let purchases: Stream<usize, &'static str, ()> = stream! {
    iterable { [2usize, 3].into_iter() },
};
let total: Sink<usize, usize, &'static str, ()> = sink! {
    fold; initial { 0usize },
    step |total, cups| { total + cups * 475 },
};
assert_eq!(block_on(total.run(purchases).run(Arc::new(()))), Exit::Success(2375));
```

The [saved-order recipe](examples/save_coffee_order.rs) uses pure codecs for the
persisted draft and directional effectful codecs for the online form. Absent size
selects the café's actual default; quantity zero produces business feedback before
the corrected two-cup order is stored. Configured parsing retains its declared
JSON, schema, pattern and issue budgets. Contextual callbacks forward ParseContext;
AST metadata alone grants no validator or writer witness. Recursive and lazy codec
constructors retain their existing ownership/budget contracts.

[Recorded pickup labels](examples/prepare_coffee_orders.rs) retrieve the same
recorded result on replay. [Cancelled reservation](examples/reserve_cancelled_order.rs)
returns two actual reserved cups; calling its compensation again leaves the
restored stock unchanged. Stable IDs, codecs and parse limits identify those
recorded operations, without starting an inner Effect run.

[Basket checkout](examples/live_coffee_basket.rs) maps tracked-read failures,
reserves stock atomically and commits a restock. SoldOut rejection leaves the
journal's stock unchanged; tracked reads and transaction transitions remain in
their respective interpreters.
