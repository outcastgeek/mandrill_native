# Effectful macro expansion reference

Import macros from the package that owns the returned type, such as
`effect::effect`, `effect_sql::sql`, or `effect_server::endpoint`. The
[public macro declarations](src/lib.rs) document each entry point. This
reference shows the typed operations that the expanders select. Private helper
names, inferred generic arguments, and generated hygiene are implementation
details.

## Evaluation and ownership

- In the shared named-field lowering, a configuration expression is evaluated
  once in its written position. The expander binds expressions before passing
  them to the selected constructor.
  Roles that require type inference become arguments to a typed helper in the
  same order. Type and const selectors supply types, while callbacks retain
  their delayed execution boundary. Duplicate, unknown, and mutually exclusive
  fields are checked by the owning parser.
- A role can be an extracted Rust expression, such as `body: helper`, where
  that form is supported. Inline roles use a braced body. A native role follows
  Rust's value, `Result`, or `Future` rules. An Effect role constructs its
  program with the enclosing typed binder. A factory with borrowed parameters
  performs its borrowing step before returning owned delayed work.
- Reusable callbacks produce a fresh computation on each invocation. A
  `once;` form explicitly moves owned captures into one run. Writing
  `.clone()` in a reusable factory controls which handles are copied. The
  expanders do not copy captured payloads.
- The core and extension crate resolvers honor Cargo dependency aliases.
  Prepared lowering recognizes the DSL forms owned by the resolved crate.
  Unrelated qualified macros keep their own meaning.

The shared field and role lowering lives in [domain_expand.rs](src/domain_expand.rs).
The typed Effect entry point resolves its path in [expand.rs](src/expand.rs).

## Ordinary Effect

`effect! { ... }` builds a lazy computation. Within this block, `step()?`
binds an Effect. A simple sequence lowers to the reusable kernel and its
typed success/bind operations. Loops and control flow that need checkpoints
lower to the native async callback with a typed execution context.
Loop back-edges and exits use that context to cooperate and close the relevant
scope suffix. Nested closures, async blocks, and items retain their native Rust
meaning. The final value becomes the success output.

`effect! { bind; program: source, body: next, }` selects
`source.flat_map(next)`. The generated typed helper checks that `next`
returns the same error and environment types. `next` runs only after the
source succeeds. `effect! { while_loop; guard: guard, body: body, }` selects
`Effect::while_loop(guard, body)`, evaluating the guard before each body
factory. The prepared form adds its own `on_capacity` field for rearm.
`effect! { defer; body: factory, }` invokes a reusable Effect-returning factory
when the program runs.

Native `effect_sync!` selects `Effect::sync`. Its `once;` mode selects
`Effect::sync_once`. `effect_try_sync!` selects the corresponding
fallible constructor, optionally mapping its native error. Native future,
CPU, and blocking forms preserve the callback's native output and whatever
environment/cancellation arguments the selected mode supplies. A native
callback's Rust `?` propagates its `Result` failure. An
`exit_with_context;` future supplies the full `Exit`. Infallible native
adapters still compose with the enclosing Effect's inferred error type.
In the ordinary interpreter, `effect_future! { exit_with_context; body: factory,
on_capacity: mapper }` selects both pending and completion frames from the
run's `RunFrames` before invoking the factory. The mapper retains the caller's
typed memory refusal without switching to an ordinary boxed future.


Use `return lookup(id)?;` to delegate to the typed computation and
propagate its outcome. The return exits the enclosing Effect body, including
when written inside a branch or loop. A plain `return value;` succeeds with that value.
Delegation preserves the current environment and resource scope.

Use `effect_fail! { error: DomainError::Rejected }` as a bindable expected
failure. The error expression runs for each execution and does not need
`Clone`. For an existing Rust result, `effect_from_result! { result: value }`
lifts it as a reusable Effect by cloning the stored outcome on each run.
`effect_from_result! { once; result: value, on_error: DomainError::Input }`
moves the stored outcome into one run and optionally maps its expected error
at execution. The result expression runs at leaf construction. Use
`effect_try_sync!` when the native operation itself should run on each
execution. Defects still use the explicit defect form.

See the [ordinary expansion](src/expand.rs), [async checkpoints](src/async_expand.rs),
and [typed callback variants](src/callback_expand.rs).

At the interpreter boundary, `effect! { run; program: program, environment: r, }`
borrows the description and returns its native future with the complete
`Exit`. Choose `run_cancellable` for the supplied R cancellation capability,
and the corresponding `_with_frames` form to supply explicit `frames:`.
`run_in_context` forwards the program through an existing native context.
`effect_provide_some! { shared; ... }` borrows an existing selected Arc
environment while retaining the same cancellation and cleanup context.
`effect_scope!` has `fork` and `join` forms for the existing live child scope.
`effect! { either; program: source, }` exposes expected success or failure as
a `Result` value while defects, interruption and cleanup failure retain their
existing channels. Its prepared counterpart lowers through the prepared
graph's matching operation. Use `effect!` with `sleep`, `log` or
`console_write` to select the existing clock or output capability.

`effect_prepare! { run; invocation: invocation, }` transfers the admitted
invocation into its native future. `transaction! { atomically; program: tx,
policy: policy, }` selects the caller's journal policy at the boundary of a
transaction. `workflow! { run; workflow: definition, run: run, input: value, }`
passes its input through the engine's existing replay and codec boundary.

## Memory forms

```rust,ignore
let configuration = effect::memory! {
    total: 192 KiB,
    mode: prepared,
    metadata: metadata,
    partition metadata: 32 KiB,
    partition receipts: 128 KiB,
}?;
let prepared = configuration.prepare()?;
let receipts = prepared.texts(effect::memory::RegionId::named("receipts"), 8, 1024.into())?;
let memory = prepared.seal()?;
```

The configuration expression yields `Result<MemoryConfiguration<N>, MemoryError>`.
Its typed equivalent starts with `MemoryConfiguration::new(total, partitions)`,
then applies the selected policy and optional alignment and metadata region.
The `prepared` and `bounded` modes select `PreparedReuse` and
`BoundedAllocation`. Sizes accept typed byte expressions or literal
`B`, `kB` (also `KB`), `MB`, `KiB`, `MiB`, and `GiB` units.
`slots region: { count: n, each: bytes }` computes a checked partition
capacity. Provision actual typed slots with the prepared owner, including
their metadata and alignment demand. `prepare_in` and `prepare_borrowed`
select caller-owned backing policies on the same inert configuration.

| Public form | Typed operation and lifetime |
|---|---|
| `memory! { capability App => self.memory; }` | Implements `HasMemory` by borrowing the named member. The first declared lifetime selects the capability lifetime, with `'static` as the default. |
| `memory_lease! { pool: key, on_error: map }` | Uses `Effect::try_future_with_env` to select `R: HasMemoryPool<P>` and call that pool's `checkout`. An optional `environment: R` specifies the type. The returned `SlotLease` owns the backing until transfer or drop. Use the selected host adapter when admission must precede request dispatch. |
| `memory_vec! { allocator: a, capacity: n }` | Calls `RegionAllocator::vector` for a reserved empty vector. Adding an `init` role calls `initialize_vector` in index order. Memory and initializer failures stay distinct. Partially initialized values drop on refusal or unwind. |
| `memory_write! { into: output, format: format_args!(...) }` | Borrows the destination and calls `fmt::Write::write_fmt` with `fmt::Arguments`. The expansion creates no intermediate `String`. The destination and formatter determine how much is written before failure. Use the typed buffer/text append API when the application needs its specific `MemoryError`. |
| `effect_drop_guard! { value: owner, cleanup: rollback }` | Constructs `core::mem::DropGuard<T, F>`, with synchronous `FnOnce(T)` cleanup. Bind the guard before work. `DropGuard::dismiss` transfers the value once the receiving owner can accept it. Await asynchronous release through the scope/resource machinery. |

The initializer can be supplied as `init |index| { ... }` or a typed
`init: initializer` callback.

The literal declaration remains inert. The same parser also supports these
explicit helper modes.

| `memory!` mode | Fields and typed operation |
|---|---|
| `schema` | `memory`, `region`, `capacity`, `frames` select `PreparedSchemaMemory::prepare`. Adding `issues` selects `prepare_with_issues`. |
| `schema_array` | `memory`, `region`, `capacity`, `frames` select `PreparedJsonArrayPool::prepare`. |
| `schema_array_operations`, `schema_map_operations`, `schema_record_operations`, `schema_sequence_operations`, `schema_object_operations` | `memory`, `region`, `capacity` prepare the corresponding typed operation pool. |
| `issues` | `memory`, `region`, `capacity`, `path_frames`, `child_frames`, `text_frames` prepare `PreparedIssueMemory`. |
| `numeric`, `binary_unions` | `memory`, `region`, `capacity` prepare `PreparedNumericScratchPool` or `PreparedBinaryUnionPool`. |
| `pattern` | `pattern`, `memory`, `region`, `caches` select `pattern.with_prepared_caches(...)`. |
| `config_store` | `memory`, `region`, `capacity` select `PreparedMemoryConfigSource::prepare`. |
| `config_source` | `store` and `on_error` select `PreparedConfigStoreSource::new`. |
| `config`, `config_effect` | `path` and `codec` build the typed config. The effectful mode also takes `contexts`, `policy`, and optional `on_error`. Both allow an explicit `default` with their respective typed contract. |

See [memory_expand.rs](src/memory_expand.rs), the
[memory ownership reference](../effect/src/memory/README.md), and the
[bounded receipt source](../effect/examples/bounded_receipt.rs).

## Prepared execution

The prepared forms make a run description, prepare its physical storage, and
admit an invocation as separate operations. Select the descriptors and
resources before creating an opaque ordinary Effect. Preparation derives
frame demand from concrete types. The plan's `demand` and
`demand_with_layout` describe the selected graph's storage requirements.

| `effect_prepare!` mode | Fields and typed operation |
|---|---|
| `plan` (native default) | `body`, `reserve`, and `contract` construct `PreparedRunPlan`. Optional `panic_diagnostics` attaches a prepared diagnostic pool. Optional `trace` and `poll_instrumentation` select the trace and hook for admitted runs. The reserve callback receives the admitted environment and context. The body receives both and the owned reserve result. |
| `graph` | `contract` and one of `program` or `input` construct `PreparedGraphPlan::new` or `new_with_input`. Optional `cleanup`, `panic_diagnostics`, `trace`, and `poll_instrumentation` attach typed owners. `construction` requires a matching two-argument `reserve` role. `plan` with a `program` or `input` selects this same graph path. |
| `description` | `program` lowers to a typed descriptor, before graph preparation or admission. `capture_errors` and `classify_error` are optional as a pair. |
| `prepare` | `plan`, `memory`, `region`, `capacity`, `frames` call `plan.prepare(&memory, region, capacity, frames)`. |
| `admit` | `run`, `environment` call `run.admit(environment)`. Optional `trace` calls `run.admit_with_prepared_trace(environment, &trace)` for a native or external-environment graph run. Admission can refuse in `MemoryError` before the body, including when the business error is `!`. A late owned-input graph uses its `admit_owned_input` and `publish_input` methods. |
| `native`, `reserve_native`, `native_reserved` | `context` and `body` call `context.native(body)`. The split forms call `context.reserve_native()` and `reservation.native(body)`. A refusal on an owned context returns that context with its error. |
| `scheduled` | `provider_kind`, optional `construction_key`, and native `body` select a concrete scheduled descriptor. |
| `fork`, `race` | `permit` and `program` select the scoped fork. `left` and `right` select the race. Optional provider kinds and construction keys select task authority for the corresponding branch. The default provider kind is `TaskProvider`. |
| `import` | `key` selects the admitted typed target the descriptor consumes at execution. Optional `environment` chooses the projected-environment import. |
| `provider_result` | `result` transfers the already settled unique provider result and its working-set owner into the corresponding graph description. |

Admission retains the real frame, output, cancellation, and configured cleanup
owners. Prepared native callbacks receive their typed companion context and
environment views. Their pending futures remain pinned. The admitted
invocation keeps physical ownership through completion and destruction.
Escapable output or child handles keep their backing after a parent returns.
An accepted task's own settlement and cleanup govern when its provider
capacity returns. Application callbacks and provider internals retain their
own allocation contracts.

Prepare a `PreparedTraceContext` from the actual byte backing of a
`PreparedTracePool`. The plan retains its default trace, or an admit field
retains the selected snapshot for that invocation. A prepared callback can
borrow the span and annotation text with `context.prepared_trace()`.
`PreparedPollInstrumentation::prepare` stores a concrete `PreparedPollHook`
in a caller-prepared frame. Its concrete guard scopes a poll without a library
allocation for the guard. The hook and view retain their backing independently
of the caller's input text. The caller owns any allocations inside hook
captures, guards or native provider code.

Inside a selected graph, familiar effect, native, scope, resource, provider,
and layer syntax lowers through the same typed interpreter. An explicitly
selected prepared scope permit binds a real owner. A mismatched target
returns the original owned inputs with the checked error. Cleanup has its own
admission configuration and preserves the primary outcome and the configured
secondary failure policy. Reusable while loops reserve finite lanes, rearm
after previous settlement, and map rearm refusal with `on_capacity`.

An explicitly selected construction callback lowers descriptors in its own
body. Nested closures, async and const blocks, and item definitions keep
their native Rust scope. Prepared lowering recognizes unqualified DSL forms,
the resolved Effect dependency name, and the owning facade's actual macro
re-exports. Other qualified invocations are passed through for Rust to expand.

Prepared `effect_using!` accepts an explicit `release` in its binding and
body forms, through the same prepared resource and scope finalizer as
`effect_resource!`. A one-argument inline release is an Effect body. A
two-argument release receives the owned resource and a borrowed
`PrimaryOutcomeRef` while constructing the cleanup descriptor. A supplied
native factory uses the same two-argument signature. Omit `release` to
drop the owned resource during cleanup.

See [prepared_expand.rs](src/prepared_expand.rs), the
[native root contract](../effect/src/internal/runtime/PREPARED.md), the
[generated graph contract](../effect/src/internal/runtime/prepared/graph/README.md),
and the [native receipt source](../effect/examples/prepared_native_receipt.rs).

## Prepared cache, pool, and resolver forms

| Public form | Selected typed operation |
|---|---|
| `cache!` with `scope`, `config`, `options`, `memory`, and a loader | `memory: PreparedCacheStorage<K, A, E>` and `on_capacity` select `Cache::make_prepared`. Supplying `refusals` selects `make_prepared_with_refusals`. With `refusals`, the optional `independent` expression selects `make_prepared_independent` when true. |
| `dependency_cache!` with `scope`, `policy`, `memory`, `dependency_pool`, and `load` | Selects `PreparedDependencyCache::make_prepared`, or `make_prepared_with_refusals` with `refusals`. The loader receives `CacheKey<K>` and the admitted `PreparedDependencyWriter<D>`. Its Effect returns the value together with that writer so the cache can retain the dependency backing. |
| `weak_cache!` with `scope` and `memory` | Selects `WeakCache::make_with_prepared_storage` using `on_capacity`, or `make_with_prepared_refusals` using `refusals`. Later lookups use the separate `cache`, `key`, and `load` form, optionally supplying `partition`. |
| `pool!` with `scope`, `memory`, `idle_ttl`, `create`, `validate`, and `dispose` | Selects `Pool::make_with_prepared_storage` with `PreparedPoolStorage<E>`. The validation factory borrows the resource to construct its Effect. In `exists_idle;` mode, `pool`, `memory`, `predicate`, and `on_capacity` select `exists_idle_prepared` with a prepared snapshot pool. |
| `request_resolver!` with `scope`, `memory`, `on_capacity`, and `run_prepared` | Selects `RequestResolver::make_with_prepared_batches`. The loader receives the group and an owned `PreparedRequestBatch<Q, A, E>`. It consumes requests and writes outcomes into the batch before returning it. With `run` in its place, `make_with_prepared_index` retains the callback taking `Vec<Q>` and returning `Vec<Exit<A, E>>` through its Effect. |

The prepared resolver's `resolve_many_prepared` returns results in input
order and retains the result slot until the caller drops it. The [pickup
resolver](../effect/examples/resolve_pickup_tickets.rs), [menu price
cache](../effect/examples/update_menu_prices.rs), and [receipt
pool](../effect/examples/write_receipt_book.rs) show the actual composed
calls. See [domain lowering](src/domain_expand.rs) and [ownership
lowering](src/ownership_expand.rs).

## Prepared SQL and HTTP composition

- `sql! { client; pool: pool, ... }` starts with `SqlClient::new`.
  `row_lanes` selects prepared native row production and
  `pending_checkout_admission` applies the shared pending-acquisition limit.
  `prepared_statement_pool;` prepares the statement owner with optional
  `materialization_admission`. These are distinct storage and admission
  owners.
- `sql! { atomic_query; client: client, statement: source, decoder: decoder,
  rows: pool }` selects `query_into_rows_atomic`. For a retained text/blob
  projection, choose `column`, `kind`, and `buffers` in place of `decoder`.
  The `atomic_batch;` forms use `batch` in place of `statement` and select
  the corresponding batch operation. `reserve_rows;` and `reserve_session;`
  transfer the actual checked provider reservation through the client's
  kernel. These forms select Effectful row data and session ownership. Native
  database allocations follow the provider contract.
- `server!` selects `PreparedServer::prepare_with_request_controls` when
  supplied `memory`, `region`, `capacity`, `result_frames`, `cleanup`,
  and `request_controls`. An optional `admission` attaches the selected
  request-control quota. `overload` configures the static capacity response.
  After construction, `PreparedServer::with_request_queue(gate, wait)`
  selects a prepared semaphore and positive wait for parsed heads before
  request leases are acquired or incoming bodies are polled. Omit that
  method for immediate admission or refusal. The prepared server also
  exposes typed setters for parser, connection deadline, and response limits.
- A prepared `endpoint!` supplies `prepared` and `contract`. An inline
  prepared body may have one owned input pattern. Optional `cleanup`,
  `panic_diagnostics`, `on_capacity`, and `byte_pools` attach their typed
  owners or policy to the plan. `capacity(&memory, region, count, frames)`
  explicitly prepares the plan. Configuration expressions evaluate once in
  written order, including the four `capacity` arguments from left to right
  at that operation's position. The prepared body stays in the typed lazy
  recipe. The selected route reserves its body, decode and output buffers
  before application work.
- `pipeline!` uses `prepared_stage` for the prepared continuation cursor.
  `reply! { lease(view, media); }` calls `Reply::lease` with a
  `BufferView<'static>`. `reply! { frame(frame, media); }` calls
  `Reply::frame` with a `PreparedNativeFrame`. For a stream, use the
  single operation `server_stream! { leases: source }` or
  `server_stream! { frames: source }`. These select `BodyProgram::leases`
  and `BodyProgram::frames`, respectively, and transfer each retained owner
  into the response. A caller using `PreparedNativeBytesPool::try_copy_frame`
  copies the fragment into an admitted native buffer at acquisition.
- `web!` requires `memory` when selecting `admission`, `byte_pools`,
  or `overload`. Its lowering attaches those values to the existing
  Web builder. Prepared read-through cache forms pair selected `memory`
  with `refusals` to share the typed capacity failure with callers.

See the [SQL ownership guide](../effect_sql/README.md), the
[Server examples](../effect_server/examples/README.md), and the
[Web examples](../effect_server_web/examples/README.md). The
[native transport reference](../effect_server/src/NATIVE_TRANSPORT.md)
describes ingress, transport retention, and drain ownership.

## Prepared stream and sink forms

| Form | Fields and ownership |
|---|---|
| `stream! { iterable_prepared; … }` | `iterable` is an owned iterator, `memory` is a `PreparedChunkPool`, and `on_capacity` maps a `MemoryError` to the stream error type. The iterator is advanced only after a batch slot is checked out. |
| `stream! { pull_prepared; … }` | `pull` is an owned `PreparedStreamPull` implementation. Its native future returns `Exit<PreparedChunk<_>, _>`. |
| `stream! { map_prepared; … }` | `stream` and the destination `memory` pool are owned. `map` returns a native future producing `Exit`. The stage reserves output before pulling input and retains that input through the mapped futures. `on_capacity` maps a reservation or batch-size failure. |
| `stream! { rechunk; … }` | `stream`, destination `memory`, and `on_capacity` select demand-driven splitting. An unread remainder retains the upstream slot and is consumed before the next upstream pull. |
| `sink! { collect_prepared; … }` | `stream: &mut stream`, `memory: &output_pool`, and `on_capacity` construct the collection future. It reserves output before pulling and refuses a batch that would exceed the output capacity. The returned batch retains the output slot. |
| `sink! { fold_prepared; … }` | `stream: &mut stream`, `memory: &fold_pool`, `initial`, `step`, and `on_capacity` construct the fold future. The `PreparedFoldPool` reserves the accumulator before calling `initial`. `step` takes `&mut` accumulator and an owned item and returns `Result<(), E>`. The completed fold retains the accumulator slot. |

Runtime field expressions evaluate once in written order. Polling the resulting
future starts the selected work. Size pools for simultaneous owners when stages
share a pool. A retained chunk keeps its backing unavailable to later pulls
until it is dropped. Native callback payloads follow the caller's own storage
contract. Use `PreparedStream::with_context(context)` to use the enclosing prepared
future's cancellation registration. The selected forms lower in
[ownership_expand.rs](src/ownership_expand.rs).

## Prepared transaction domains

`transaction! { prepare_domain; memory: &memory, region: region, capacity: limits }`
evaluates its three fields once in written order and provisions a
`TransactionDomain` from `PreparedTransactionCapacity`. Supply this domain via
the existing `HasTransactionDomain` capability. `atomically` uses its physical
read and write tables, including distinct nested branch drafts. When both
alternatives retry, their read union is checked before merging. A waiting retry
holds an observation table and a change registration while its running journal
slots return to the pool. Scoped transactional locks and permits retain a
journal for protected release before acquiring the underlying resource.

Choose transaction recipes at setup to keep recipe construction outside
retrying bodies. Prepared journal limits cover their recorded state and
registrations. Regular `TxRef` values keep their `Arc` ownership and a
`PreparedTxRef` can retain a value prepared in its own regional pool.
The prepared domain form lowers in [restricted_expand.rs](src/restricted_expand.rs).

## Prepared RPC operation

`rpc_operation! { id: id, version: version, limits: limits, prepared_codec: codec }`
constructs a unary `PreparedRpcOperation` and returns its typed startup result.
The fields evaluate once in written order. The prepared codec reads a borrowed
`PreparedJsonValue` and writes one value through an admitted `JsonWriter`.
The wire path keeps separate operation-schema and envelope limits and validates
the complete frame before correlation. The selected client and channel both
admit their codec and exchange owners before invoking a request writer.
An encode reservation holds two buffered writers, two parse-counting writers,
and two document slots. A decode reservation holds two counting writers, one
document slot, and one payload-validation slot. A shared pool must cover the
simultaneous client and in-process channel reservations. Issue storage and the
request and response frame buffers have their own capacity.

## Prepared cluster store

`cluster! { memory: &memory, region: region, store_capacity: limits }`
provisions the volatile `MemoryClusterMessageStore` from
`PreparedClusterStoreCapacity`. Each field evaluates once in written order.
Message records and activation fences share the same ownership gate. Each
remembered address takes one activation slot, including addresses whose
activation has been passivated or abandoned. Keeping that slot lets the store
increase the generation on reactivation and continue rejecting stale owners.
Size `addresses` for the distinct addresses the store must remember over its
lifetime. The table checks retained identity bytes separately per address.

## Prepared workflow engine

`workflow! { prepare_engine; memory: &memory, region: region, capacity: limits }`
selects the prepared in-memory engine at startup. The fields evaluate once in
written order. Normal execution and recovery use the engine's head operations,
which return the authoritative run identity, status, waits, and checkpoint
without copying its replay histories. Existing providers can use the default
projection from their ordinary snapshots. This startup form uses
[restricted_expand.rs](src/restricted_expand.rs).

## Public expansion index

Each row links the implementation of the listed macro families. Source-level
forms and examples live in the package that re-exports the macro.

| Public macros | Expansion owner and result |
|---|---|
| `memory!`, `memory_lease!`, `memory_vec!`, `memory_write!`, `effect_drop_guard!` | [memory_expand.rs](src/memory_expand.rs) selects the typed configuration, lease, collection, writer, or guard described above. |
| `effect!` | [expand.rs](src/expand.rs), [async_expand.rs](src/async_expand.rs), and [callback_expand.rs](src/callback_expand.rs) select the existing binder, native control-flow runner, or typed continuation. |
| `effect_sync!`, `effect_try_sync!`, `effect_future!`, `effect_try_future!`, `effect_cpu!`, `effect_try_cpu!`, `effect_blocking!`, `effect_try_blocking!`, `effect_die!` | [callback_expand.rs](src/callback_expand.rs) chooses typed native Effect constructors. [prepared_future_expand.rs](src/prepared_future_expand.rs) selects typed native graph leaves and prepared queue, publication, and subscription operations. |
| `effect_map!`, `effect_map_error!`, `effect_provide_some!`, `effect_sandbox!`, `effect_from_option!`, `effect_timeout!`, `effect_recover!`, `effect_for_each!`, `effect_for_each_concurrent!`, `effect_provided!`, `effect_race!`, `effect_uninterruptible!`, `effect_on_interrupt!`, `effect_all!`, `effect_all_concurrent!`, `effect_retry!`, `effect_repeat!` | [callback_expand.rs](src/callback_expand.rs) constructs existing typed Effect combinators. Chosen providers, ordering, resource scopes, and error mapping come from the typed operation and its explicit arguments. |
| `effect_prepare!` | [prepared_expand.rs](src/prepared_expand.rs) selects the native or graph preparation/admission contracts. [prepared_program_expand.rs](src/prepared_program_expand.rs) lowers the selected computation before ordinary Effect erasure. |
| `effect_resource!`, `effect_using!`, `effect_scope!`, `effect_scope_acquire!`, `effect_finalizer!`, `layer!`, `effect_with_layer!`, `effect_permit!` | [ownership_expand.rs](src/ownership_expand.rs), [prepared_scope_expand.rs](src/prepared_scope_expand.rs), [prepared_resource_expand.rs](src/prepared_resource_expand.rs), and [prepared_layer_expand.rs](src/prepared_layer_expand.rs) use existing scope, resource, permit, and layer contracts. Cleanup modes preserve the distinction between native Release factories and already constructed Effects. |
| `schedule!`, `effect_ref!`, `pool!`, `weak_cache!`, `request_resolver!` | [ownership_expand.rs](src/ownership_expand.rs) selects typed scheduling, references, resource pooling, cache lifetime, and request resolution. [prepared_program_expand.rs](src/prepared_program_expand.rs) specializes the selected prepared reference forms. |
| `stream!`, `sink!`, `effect_stream_for_each!`, `effect_socket!` | [stream_expand.rs](src/stream_expand.rs) and [ownership_expand.rs](src/ownership_expand.rs) construct typed demand-driven producers/consumers and selected transport scopes. Release remains with the stream/transport owner. |
| `schema!`, `schema_enum!`, `persistable!`, `codec!`, `effect_schema_parse!` | [schema_expand.rs](src/schema_expand.rs), [schema_enum_expand.rs](src/schema_enum_expand.rs), [persistable_expand.rs](src/persistable_expand.rs), and [codec_expand.rs](src/codec_expand.rs) select typed records, enums, persistence metadata, codec directions, and schema parser policies. |
| `transaction!`, `event!`, `event_group!`, `handled_event!`, `projection!`, `workflow!`, `activity!`, `workflow_definition!`, `activity_definition!` | [transaction_expand.rs](src/transaction_expand.rs), [descriptor_expand.rs](src/descriptor_expand.rs), [durable_expand.rs](src/durable_expand.rs), [restricted_expand.rs](src/restricted_expand.rs), and [domain_expand.rs](src/domain_expand.rs) lower typed transaction bodies, event definitions, durable workflow/activity definitions, and domain callbacks. |
| `rpc_operation!`, `rpc_stream_operation!`, `rpc_protocol!`, `rpc_stream_protocol!`, `rpc_middleware!`, `rpc_stream_middleware!` | [descriptor_expand.rs](src/descriptor_expand.rs) and [domain_expand.rs](src/domain_expand.rs) build the respective typed operation, protocol, and middleware definitions. |
| `entity!`, `entity_resource!`, `singleton!`, `cluster_schedule!`, `persisted_queue!`, `durable_queue!`, `cluster!` | [descriptor_expand.rs](src/descriptor_expand.rs), [restricted_expand.rs](src/restricted_expand.rs), and [domain_expand.rs](src/domain_expand.rs) select explicit cluster, entity, and persistence authorities. |
| `reactive_source!`, `reactive_derived!`, `reactive_query!`, `reactive_bridge!`, `atom!`, `hydration!`, `span!`, `service!`, `cache!`, `dependency_cache!` | [descriptor_expand.rs](src/descriptor_expand.rs), [restricted_expand.rs](src/restricted_expand.rs), and [domain_expand.rs](src/domain_expand.rs) bind the selected environment, reactive graph, trace, and cache constructors. Cache capacity and error callbacks are part of the chosen domain contract. |
| `sql!`, `sql_insert!`, `sql_update!`, `row!`, `sql_with!` | [extension_sql.rs](src/extension_sql.rs) assembles native SQL and bound values, guarded mutations, row projections, typed sessions, and selected prepared resources. See [Effect SQL](../effect_sql/README.md). |
| `server!`, `endpoint!`, `registered!`, `reply!`, `representations!`, `route!`, `router!`, `routes!`, `pipeline!`, `server_stream!` | [extension_server.rs](src/extension_server.rs) lowers typed server construction, route registration, middleware and body/stream ownership. See the [Server examples](../effect_server/examples/README.md). |
| `identity_store!`, `identity!`, `identity_endpoints!` | [extension_identity.rs](src/extension_identity.rs) selects explicit Identity capabilities and endpoint definitions. See the [Identity examples](../effect_server_identity/examples/README.md). |
| `web!`, `security!`, `csp!`, `read_through!`, `dependent_read_through!`, `availability!`, `message_arguments!`, `localization!` | [extension_web.rs](src/extension_web.rs) builds typed Web policies and capabilities. See the [Web examples](../effect_server_web/examples/README.md). |
| `otel!` | [extension_otel.rs](src/extension_otel.rs) composes the selected tracing boundary with the typed program. See the [Effect OTel exports](../effect_otel/src/lib.rs). |

### Web application composition and error labels

Use `web! { prepare; source: application, scope: scope, }` to compose
startup in the existing host scope. `with_host` invokes the caller's typed
body and `run` selects the ordinary or prepared definition's existing terminal
profile. `server! { request; }` binds the current owned request handle through R.
`dependency_cache!` accepts `load_shared:` for an extracted loader that takes
the existing shared key, and `coalescing:` for an explicit no-retention policy.

`#[derive(EffectErrorLabel)]` implements Display from the explicit literal
`#[effect_error_label("...")]` on each enum variant. It never reads payloads,
changes error sources, or adds Debug formatting.

### Identity operations

Use `identity! { identity: definition, register(input), }` and the equivalent
named workflow or authentication stage forms with their original typed
arguments. The selected definition supplies authority, storage and error policy.
`invoke_prepared_once(admission, input)` consumes the actual predecode
invocation reservation, while `invoke_once(input)` uses the ordinary path.
Both forms take the same typed native callback. The macro does not mint an
authentication grant or acquire a second slot.

`identity_store!` names the selected SQL, prepared invocation, account and
record owners. A native initializer retains its existing Rust `.await` and
`Result`. The caller supplies capacities and codecs, including the exact
record category and schema policy.

### Server operations and capability derives

Use `server! { request_binding; body: resolve, }` to install an extracted
request environment program. `server! { policy; ... }` describes policy without
acquiring a listener. Review outer selection, final-body observers and prepared
capacity in their existing scopes. `router! { configure; source: router, ... }`
applies infallible routing configuration. `server! { with_host; source: host,
body: use_host, }` runs the existing scoped host operation. The same form
retains the prepared owner's typed admission result. `prepared_configure`
applies one selected prepared provider with its existing error contract.

Use `#[derive(EffectEnvironment)]` with `#[effect_environment(request = request)]`
to borrow a field. A projection such as `tokio = self.runtime.handle()` is
an explicit borrow. For an associated provider use
`task_spawner(provider = Tasks, get = self.tasks.as_ref())`.
`requires(R: HasTaskSpawner)` attaches bounds to that capability alone,
leaving the environment's other capabilities available under their own bounds.
`server_error(error = AppError, map = AppError::Http)` names the actual error
type and mapper.
For a capability whose contract returns an owned `Arc`, use an explicit
projection such as `transaction_domain = Arc::clone(&self.transactions)` or
`event_journal(provider = Journal, get = Arc::clone(&self.events))`.
`atomic_projection_store` also takes `state = State` in the environment
attribute. Independent capability requirements remain visible
in each projection's `requires(...)` clause.

### Streaming and telemetry

Core `stream!` uses `open_in; stream:, scope:` for an explicitly owned reader
scope and `collect; stream:, limit:` for the existing bounded collection
operation. Server's `stream!` selects `sse; writer:, record:, policy:` or
`flush; writer:` on the existing body writer. These forms retain the stream's
completion and cleanup path.

`otel!` retains typed provider scope through `acquire; scope:, factories:,
shutdown:, on_error:`. Use `tag`, `event` or `log` with named fields in a
program. `event` accepts `name:` and optional `attributes:`. All four operations
delegate to the same typed OTel provider and instrumentation context.

### Web application handoff

The prepared café passes its existing arena preparation owner to `web!`'s
`memory:` role using an inferred callback. The application supplies its
prepared output cache in `web! { prepared_output_cache; ... }` with explicit
storage, refusal, gate, authentication, key selection and error policy.
`web! { admit; ... }` selects the admitted body or the caller's refusal Effect
using the same application environment and transport ownership.
[Prepared café application](../effect_server_web/examples/cafe_application.rs)

### CMS composition

`cms! { definition_services; options: options, }` builds the existing service
graph from selected resource definitions. Use `definition_list_reply` with
`renderer`, `services` and `kind` to obtain the typed CMS reply Effect. Pass it
to `endpoint! { effect: ..., }` when registering a reusable Web handler.
`register_routes` retains the caller's scope, router and mounter through startup.
`EffectEnvironment` supports an explicitly selected `cms_memory` for a local
environment. CMS generates the forwarding adapter on Server's request wrapper
from the local capability trait without changing either crate's dependencies.
[CMS Web catalogue](../effect_cms/examples/cms_application.rs)

### SQL operations

With an explicit `client:`, `sql!` can execute or query through a selected
client, session, or transaction. `execute` and `scalar` take `statement:`.
`query`, `first`, and `first_prepared` additionally take `decoder:`.
Omitting `client:` in `execute`, `query`, `first`, or `scalar` selects
the existing `HasSqlClient` capability. `statement:` can transfer an
owned statement or supply a reusable statement factory. `first_prepared`
retains the existing capacity and borrowed-decoder requirements.

`session`, `transaction`, and `savepoint` select a scoped `body:` Effect
factory or an inline `body |session| { ... }` computation. The
`transaction_once` and `savepoint_once` variants transfer owned captures
into one execution. `native` accepts a checked native callback whose
future borrows the SQL connection. It retains ordinary Rust `Result`
propagation inside that callback. `fold` and `fold_effect` take the
statement, decoder, initial accumulator factory and typed step, using the
existing cursor owner and cleanup path.

Native forms such as `kernel_transaction`, `connection_operation`,
`transaction_operation`, `reserve_transaction_operation`, and
`run_transaction_operation` retain their caller's ordinary Rust
`.await` and `Result` boundary. The receiver is written before its body or
operation so its original temporary lifetime is retained. The `open` and
migration forms compose the original native startup policy and error types.
`client; session_pool:` transfers the selected session owner after other
client configuration succeeds.

`#[derive(EffectEnvironment)]` supplies explicitly selected R capabilities.
`#[effect_environment(sql_client = sql)]` borrows the named field.
`#[effect_environment(sql_client = self.sql.as_ref())]` uses the supplied
borrow expression. The derive does not change the trait's environment bounds.

`#[derive(EffectSqlRow)]` uses the named Rust record fields as the row shape.
Select `R` in the record's `#[effect_sql_row(...)]` attribute and put each
field's existing `row!` expression in its field attribute. The generated
`row_decoder()` keeps column names, native types, refinements, and conversions
under the same typed row scope. [Coffee order decoder](../effect_sql/examples/cafe_orders.rs)

`#[derive(EffectJsonSchema)]` generates the selected factory name from the
owned record or enum. Supply `policy = ...` and `error_context = ...` in
`#[effect_json_schema(...)]`, and select each field's codec and each enum
variant's wire tag. The runtime policy, nested codec errors and explicit tag
and content fields stay under the author's control. [Order tracking schemas](../effect/examples/order_tracking/mod.rs)


## DSL lint and verification

The [workspace DSL lint](../../tools/effect-dsl-lints/README.md) checks
production construction paths and owns the
[reviewed expansion fingerprints](../../tools/effect-dsl-lints/reviewed.rs).
The typed equivalent in this reference explains generated behavior. Use the
DSL at a call site where the workspace lint requires it. Review the exact
changed lowering before updating its fingerprint. Source review and the
applicable package checks establish expansion, typing, and ownership
contracts. Runtime cancellation, allocation, and capacity qualification use
their separate validation phase.
