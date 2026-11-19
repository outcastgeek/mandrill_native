# Effect SQL

`effect-sql` composes with `Effect<A, E, R>` and uses this workspace's SQLx/Turso
provider. The caller supplies the Tokio reactor for native I/O. The crate creates
no runtime; CPU and blocking capabilities elsewhere in the same `R` remain
independent. The `turso` feature supplies native execution. The optional `macros`
feature enables SQLx's checked query macros and offline metadata interoperation.

The opt-in `fts` feature forwards the existing Turso full-text search capability
through the SQLx provider. Enable it with `features = ["fts"]` on the
`effect-sql` dependency. Turso's index method must also be enabled explicitly
in the connection options. With an FTS-capable provider build, use the existing
statement and client APIs for Turso FTS SQL:

```ignore
use effect::Effect;
use effect_sql::SqlError;
use effect_sql::sqlx::{TursoConnectOptions, TursoExperimentalFeature};

let options = TursoConnectOptions::new()
    .experimental_feature(TursoExperimentalFeature::IndexMethod, true);
// Use `options` to construct the SQLx pool and `client`.
// The `docs` table with a `body` column already exists.
let statement = effect_sql::sql! {
    text: "CREATE INDEX docs_fts ON docs USING fts(body)",
}?;
let create_index: Effect<u64, SqlError, ()> = client.execute(statement);

let search = effect_sql::sql! {
    text("SELECT id FROM docs WHERE fts_match(body, ");
    param(query);
    text(")");
}?;
// Here the decoder reads an i64 document ID and uses the unit environment.
let search_effect: Effect<Vec<i64>, SqlError, ()> = client.query(search, decoder);
```

The same DDL can be included in a `MigrationStep::transactional` step. Scoped
sessions and transactions accept these statements through their existing
execute and query methods. The search expression is a bound parameter, and
results use the usual row decoder. The Effectful `sql!` macro assembles SQL
for runtime execution. SQLx's checked-query macros are a separate path.

The workspace pins Turso 0.9.0-pre.5. The `native_full_text_search_matches_bound_query`
contract test verifies index creation and bound `fts_match` queries through
`SqlClient` on Rust 1.100.

`SqlClient` retains immutable service configuration. Reusable client operations
accept a `StatementSource`: a fresh factory is evaluated once per execution and acquire a fresh pool
lease. Build statements inside the Effect body when inserting a moved payload.
`Sql::execute`, `query`, `first` and `scalar`, and the explicit `execute_once`
methods transfer an execution-local statement once. Reusing that transferred
payload reports consumption instead of copying it. Cloning a client, decoder or
transaction handle shares configuration or a checked owner; it does not clone
application rows, bound blobs or errors.

The five SQL macros resolve renamed dependencies and return typed descriptions:

| Macro | Grammar and result |
| --- | --- |
| `sql!` | Optional pure local bindings; `text: expression,` and ordered `text(syntax); identifier(name); fragment(statement); where(predicate); param(native_value);`. Returns `Result<SqlStatement, SqlError>`. |
| `sql_insert!` | Required `table: expression,` before one or more `value(column, native_value);` entries. Duplicate literal fields/columns fail expansion; dynamic names are checked by the builder. Returns a typed statement Result. |
| `sql_update!` | Required `table: expression,`, nonempty `set(column, native_value);` entries, then exactly one `where(typed_predicate);`. No implicit full-table update. Returns a typed statement Result. |
| `row!` | Ordinary Rust bindings/control flow with scoped `column`, `optional_column`, `native_column`, `refine`, `validate` and `choice_text` operations. `?` binds typed row steps, compatible Effects or core ResourceBindings. Returns `RowDecoder<A, R>`. |
| `sql_with!` | Exactly one `select: selector,`, `project: selector,` or `handle: client,`; exactly one `run \|client\| { body }`, `run: extracted_callback,` or `effect: typed_program,`; optional `on_error: mapper,` (`errors:` is an alias). All three forms explicitly project a `SqlClient` body into the caller’s outer `R`, preserving complete failure channels. Inline captures transfer once through core lowering. |

Trusted SQL syntax and application values are different types/operations.
Identifiers use Turso/SQLite double-quote escaping per path component. Bound
values use native SQLx `Encode`/`Type`, retaining insertion order. Use
`SqlPredicate::equal` for NULL-aware equality: encoded NULL becomes `IS NULL`.
Fragments move binding buffers and use anonymous placeholders. SQLx's checked
query route remains available on the real native pool or through scoped native
callbacks; dynamic statements do not claim compile-time database verification.

Row programs retain one owned native SQLx row, not a borrowed cursor converted
into a static payload. Native `Type`/`Decode` preserves NULL, missing columns,
ordinals/names and representation failures. `CellCodec` adapts existing native
schema codecs and retains `SchemaIssue` paths. Its strict boolean, finite-real and
invariant decimal-text codecs provide the reference's domain policies.
For domain values without a native SQLx encoder, bind `CellCodec::encode_once`
explicitly before moving its native result into `param`/`value`/`set`. This preserves
exact decimal text rather than coercing it into a floating-point binding. Native
string/blob decoding performs the native conversion into the requested Rust
owned field; it does not clone the whole row or serialize it through JSON.
Effectful row validation selects services in `R`; each row's core resource scope
closes before the cursor advances. Row-program bodies and extracted decoders are
reusable, while their local values move once per row.

Client, session and transaction operations include affected-row execution,
scalar, buffered query, optional first row, ordered batches and incremental pure
or effectful folds. `None` means no row; decode/provider failures stay errors.
Batches preserve native row/result ordering and do not become transactional
unless explicitly enclosed in a transaction. Turso's parameter-free multiple SQL
result sets are supported; native provider restrictions on binding one argument
buffer to multiple statements stay typed SQLx errors.

Transactions own one exclusive lease. `SqlClient::transaction` runs an Effect
body; `SqlTransaction::savepoint` uses that same owner and the existing SQLx
transaction manager. Private frame identities fence completed handles even when
another transaction/savepoint later reaches the same depth. Success releases or
commits; `transaction_once`/`savepoint_once` move owned callback captures into the
same settlement path. Other exits await rollback and ordered cleanup. A live fold keeps its
cursor connection private; callbacks may acquire independent sessions through
`R`, and reentrant operations on the same live cursor frame fail with `InactiveSession`
instead of waiting for their own retained guard.

Begin/commit/rollback acknowledgment and state publication are awaited together.
Before disposing or returning an owned lease, the kernel drains pending SQLx rollback
and awaits resolution of remaining native transactions; borrowed owners retain their
caller’s terminal authority.
Cancellation is cooperative at operation/row/batch boundaries. The current Turso
native API exposes no command cancellation acknowledgment, so an in-flight native
call is awaited before its owner is disposed; the adapter never treats a dropped
future as rollback acknowledgment. Hosts must drive Effect/native owners through
settlement. Operation-owned native callback leases are retired on abandonment;
transaction leases are also closed after confirmed settlement because SQLx's
close-on-drop quarantine flag cannot be cleared. Unknown commit acknowledgment is
`CommitOutcomeUnknown`, never a retry or definitive rollback. Cleanup failures
retain their original error, defect or interruption.

`SqlKernel` is the shared native async boundary for Identity traits. Its scoped
`NativeOperation` consumes borrowed credentials/slices under `Self: 'a`, so native
stores need no payload copies or nested Effect execution. Native manager methods
own independent `BEGIN IMMEDIATE` transactions rather than silently joining an
ambient application transaction. `SqlError::NativeInvocation` retains a richer
invocation-local outcome when a native interface narrows its error channel;
`into_exit` restores it at the outer Effect boundary. `with_borrowed` and
`borrowed_operation` retain the caller's native owner and Rust borrow lifetime.
Transferred SQLx/native connections use the driver's real adoption/transfer APIs,
including retained sync database ownership. Replicas may read, but cannot claim
authoritative schema/security writes.

`MigrationCatalog` requires unique ordered SQLx versions, native checksums and
explicit `Rollback::Reversible` or `Irreversible` declarations. The migrator reuses
SQLx's selected journal (normally `_sqlx_migrations`) and adds only versioned dirty
direction metadata. Transactional steps revalidate history and mutate schema and
history under database `BEGIN IMMEDIATE` authority. A preflighted `DownPlan` binds
its catalog, database configuration and exact history; changed history is rejected
before schema work. The migrator streams journal rows, compares checksum bytes
in place, and retains the validated catalog prefix length for down plans. The
catalog assembles its journal SQL once and shares the text across operations.
Native row materialization and statement execution remain provider-owned. SQLx
`success=false` entries and extension dirty markers need
explicit recovery. Recovery assessments inspect actual schema under the same
write authority and reconcile the journal deliberately. Normal migration entries
record native elapsed nanoseconds; recovered entries record assessment/publication
duration rather than claiming an unknown original execution time.

A step marked `no_tx` requires a selected `MaintenanceProvider` whose retained
guard excludes all schema writers for that deployment. Dirty metadata commits
before schema I/O, and remains after unacknowledged failure. A process mutex or
filesystem lock does not establish replica-wide authority. Synced replica
configurations are rejected for migrations. No second independent applied history
or guessed recovery result is created.

Ordinary decoded `query` calls finish native fetch before the first decoder
runs. Native rows then retire in blocks during decoding as the result vector
grows fallibly. These compatibility methods still allow the full result to
grow with the query. Select a prepared output or incremental fold when the
caller needs to control retained row storage.

The driver row pool selected by `with_rows_pool` is used by
`NativeSession::incremental_cursor` and the prepared atomic query and batch
paths. The ordinary `first`, `query`, `batch`, and fold APIs retain their
existing fetch behavior. In particular, an unbound multi-command batch in an
ordinary fold executes all commands before delivering the first row, and
ordinary partial reads use the provider's row lookahead. The incremental
cursor claims its first row backing before native preparation and advances
commands when polled. Fold accumulators and decoder outputs use the caller's
storage contract.

`observe` selects safe completion observation through `R` and its monotonic clock.
It carries operation/category, duration, selected result count and cleanup/trace
classification. SQL, bindings, credentials and connection strings are excluded.
Observer failures are best effort and do not replace the business exit; exporters
are optional. Rich native error diagnostics remain available to trusted callers
and should be mapped before public HTTP responses or logs.

## Prepared session controls

The existing SQLx pool can be paired with explicitly prepared session controls:

```rust,ignore
use std::sync::Arc;

let session_capacity = effect_sql::PreparedSqlSessionCapacity {
    sessions: 4,
    frames_per_session: 3, // root, transaction, one nested savepoint
};
let demand = session_capacity.demand()?;
let sessions = Arc::new(effect_sql::sql! {
    prepared_session_pool; memory: memory, region: region,
    capacity: session_capacity,
}?);
let client = effect_sql::SqlClient::new(sqlx_pool).with_session_pool(sessions);
```

Before wrapping the session pool in `Arc`, `with_cleanup(Arc<PreparedCleanupConfig<SqlError>>)`
can select the existing prepared cleanup buffer and frame pools. The four root
atomic query and batch operations reserve one of those owners before statement
factories, batch consumption or transaction begin, and use its configured overflow
policy for ordered rollback and close failures. Their transaction savepoint
counterparts reserve a separate owner before beginning the savepoint and use it
for rollback failures, so concurrent root and nested operations need separate
buffer and frame capacity. Without this selection, existing cleanup behavior applies;
provider allocations remain separate.
The same selection also covers `transaction`, `transaction_once`, `savepoint`
and `savepoint_once`; a cleanup admission refusal leaves a one-use callback
unconsumed. When a prepared session pool is selected, client `transaction` and
`transaction_once` acquire SQLx with its configured hooks before claiming the
session and cleanup owners. Hooks can run before a later local refusal.

`reserve_prepared_session` claims its session control before SQLx pool acquisition.
For the four prepared client atomic query/batch paths, when a session pool is
selected, hook-preserving SQLx acquisition happens first, followed by local
owner admission before statement factories, batch consumption and BEGIN.
Effectful's output, session and cleanup acquisitions refuse on contention.
Shared driver row pools use the provider's registry mutex. Private row lanes
admit one cursor per registry and use nonblocking lane admission.
Prepared transaction atomic savepoint admission also returns `Busy` if the
prepared session state is contended; ordinary savepoints retain their wait path.
Owning root transactions claim the callback's selected session loan with the
session reservation. Selected savepoints transfer an existing run-time loan to
the child frame. Their once form checks for a consumed callback before waiting
for the current session and takes it after frame and any cleanup admission. The
internal one-use query and batch Effects move that owned session into the cursor
runner with the accepted input.
SQLx `before_acquire` or `after_connect` hooks can therefore run before a later
local capacity refusal. Native pool waiting and SQLx/VM memory are separate.
Each root or
nested transaction consumes a configured frame authority; exhaustion is a
typed memory refusal before `BEGIN` or savepoint. Retained session/cursor
handles keep their actual control occupied; the final prepared loan fences the
state and drops any native connection owner outside its lock, after cursor
statement/guard retirement. Region demand and startup state
and busy Arc payload/counts are reported separately; Arc headers, allocator
rounding, the caller's outer Arc, SQLx connection and native VM allocations
are separate. Existing native connection and transaction APIs not using
`SqlKernel::session` retain their ordinary behavior.

For explicit composition, claim that same physical control without database
work and open it later:

```rust,ignore
let reservation = effect_sql::sql! { reserve_session; client: client, }?;
// Other pre-work admission may occur here; dropping the reservation releases it.
let session = reservation.open().await?;
```

The reservation retains the selected SQLx pool, but acquires no native
connection until `open`; SQLx and VM memory remain outside this bound. Atomic
client query and batch paths claim this control and their result vector before
statement materialization or batch consumption.

## Prepared statement descriptions

An application can prepare a statement pool at startup and select it from its
existing environment. SQL text, binding descriptors and copied text/blob
bytes use the selected fixed region storage:

```rust,ignore
use effect::memory::{Admission, CapacityUnit, ResourceCapacity};
use effect_sql::PreparedSqlStatementCapacity;
use std::sync::Arc;

let materialization = Arc::new(Admission::prepare(&memory, region, [ResourceCapacity {
    unit: CapacityUnit::Bytes, work: max_materialized_bytes, control: 0,
}])?);
let statements = Arc::new(effect_sql::sql! {
    prepared_statement_pool; memory: memory, region: region,
    capacity: PreparedSqlStatementCapacity {
        statements: 4, sql_bytes: 512, bindings: 8, binding_bytes: 1024,
    },
    materialization_admission: materialization,
}?);
// `env.statements` is the application's selected Arc<PreparedSqlStatementPool>;
// `client`, `decoder` and `rows` are its existing query selections.
let statement = effect_sql::sql! {
    prepared_statement; pool: env.statements,
    text("SELECT "); identifier("orders.name");
    text(" FROM orders WHERE id = "); integer(order_id);
}?;
let query = effect_sql::sql! {
    atomic_query; client: client, statement: statement,
    decoder: decoder, rows: rows,
};
```

The owned prepared statement is a one-use `StatementSource`: atomic query
reserves its row output before invoking the factory, then materializes the
existing native SQL/SQLx argument backing. `materialize()` exposes the same
native conversion explicitly. Its SQL, argument-vector and text/blob native
copies use fallible reservations and report `CollectionReservationFailed`;
the owned copies then move into SQLx. This is not allocation-free and does not
bound the driver, SDK or VM. Pool demand excludes allocator rounding, outer Arc controls and callback
pointees. A second execution of the transferred statement returns `Consumed`.
The prepared statement source lease follows a one-use `StatementSource` through
the operation or cursor. Direct `materialize()` leaves that source lease with
its original statement and returns an independent copy. The optional
`materialization_admission` field takes the caller's shared `Arc<Admission<'static, 1>>`
with the `Bytes` unit. The macro prepares the pool and applies
`PreparedSqlStatementPool::with_materialization_admission` to charge each live
copy before it allocates. The charge covers
the requested SQL text, binding payloads, and SQLx argument descriptors.
Fragments and guarded updates carry their materialized binding charges through
Effect-sql execution. Composed SQL buffers, combined argument arrays,
composition bookkeeping, allocator rounding, driver conversion, caches, and
VM memory have separate storage. `SqlStatement::into_parts`
releases the Effect-sql charges and gives the raw buffers to the caller.

Reusable Effects reserve conversion bytes before taking a one-use statement
or batch input. Admission refusal leaves that input available for retry. Once
the input has transferred, native-copy failures follow the existing one-use
rules. A prepared graph that has already accepted its one-shot input owns it
through refusal and settlement.

The incremental driver cursor also fallibly reserves its native argument array
and copies only prepared text/blob spans; already-native buffers move into the
result. Reservation failure is reported as a SQLx out-of-memory I/O error.

## Prepared row output and environment-owned bytes

A prepared output pool may also couple each actual checked-out vector slot to
one unit of an explicitly selected admission coordinator. Given the existing
`memory`, `region`, and output `capacity`:

```ignore
use effect::memory::{Admission, CapacityUnit, ResourceCapacity};
let admission = Admission::prepare(&memory, region, [ResourceCapacity {
    unit: CapacityUnit::Slots, work: capacity.outputs, control: 0,
}])?;
let rows = PreparedSqlRowsPool::<MyRow>::prepare(&memory, region, capacity)?
    .with_admission(admission)?;
```

Each builder and its finished rows retain both the real slot and quota until
drop; an absent admission selection preserves standalone pool behavior. This
slot accounting does not include memory owned by row values or their pointees.
`PreparedSqlStatementPool::with_admission` applies the same one-work-slot
coupling to each statement builder and retained statement.
`PreparedSqlBatchPool::with_admission` couples the batch vector slot; statements
inside the batch may select their own separate slot admission. These selections
account for those slots only, rather than native SQLx or VM allocations.
`PreparedSqlSessionPool::with_admission` similarly charges one work slot after
checking out a session control. The checkout or its final session/cursor loan
retains that charge until the control retires; the native connection and its
allocations remain separate. An unconfigured pool keeps standalone behavior.

Select a dedicated `Slots` admission to limit the number of Effect-sql calls
awaiting native pool acquisition.

```rust,ignore
let pending_checkouts = Arc::new(Admission::prepare(&memory, region, [ResourceCapacity {
    unit: CapacityUnit::Slots, work: max_pending_checkouts, control: 0,
}])?);
let client = effect_sql::sql! {
    client; pool: pool, pending_checkout_admission: pending_checkouts,
}?;
```

The `client` form constructs a `SqlClient` from the supplied native pool and
applies `SqlClient::with_pending_checkout_admission` when the optional field is
present. An existing kernel can use `SqlKernel::with_pending_checkout_admission`.

The selected admission refuses immediately when capacity is unavailable. Cloned
clients, kernel calls, and explicit prepared-session opens share the same
admission. Each admitted attempt holds its slot until SQLx acquisition and its
hooks finish or the acquisition future is dropped. Acquired connections and
their later work use their own owners. Direct use of `SqlKernel::pool()` goes
through SQLx without the Effect-sql admission. The slot limit covers pending
Effect-sql acquisitions. SQLx hook and VM allocation sizes remain provider-owned.

An atomic query can retain each projected value in a fixed output vector. A
projection may select a separately prepared buffer pool from the original
environment and return the actual buffer lease as its row value:

```rust,ignore
use effect::memory::{Buffer, Bytes, SlotPool};
use effect_sql::{RowError, PreparedSqlRowsCapacity, PreparedSqlRowsPool};
use effect_sql::sqlx::{Row, TursoRow};
use std::sync::Arc;

struct Services {
    names: SlotPool<'static, Buffer<'static>>,
}

// `memory` and `region` are the application's existing prepared region.
let names = memory.slots(region, 32, |_| memory.buffer(region, Bytes::new(256)))?;
let services = Arc::new(Services { names });
let capacity = PreparedSqlRowsCapacity { outputs: 4, rows_per_output: 32 };
let rows: Arc<PreparedSqlRowsPool<_>> = Arc::new(effect_sql::sql! {
    prepared_rows; memory: memory, region: region, capacity: capacity,
}?);
let decoder = effect_sql::row! {
    project_with_env: |row: &TursoRow, env: &Services| {
        let name: &str = row.try_get(0).map_err(|source| RowError::Native {
            column: "[0]".into(), source,
        })?;
        let mut bytes = env.names.checkout().map_err(RowError::Memory)?;
        bytes.clear();
        bytes.extend_from_slice(name.as_bytes()).map_err(RowError::Memory)?;
        Ok(bytes)
    },
};
let query = effect_sql::sql! {
    atomic_query; client: client, statement: make_statement,
    decoder: decoder, rows: rows,
};
// Run `query` in the existing Effect environment using `services`.
```

The returned row owns its buffer slot until dropped. The native input row and
VM, startup Arc/control metadata, and any other value pointees are separate
from the prepared region demand. The explicitly atomic operation uses the
existing transaction or savepoint path; projection side effects outside that
transaction are not rolled back.

For a single text or blob column, `atomic_query` and `atomic_batch` accept
`column`, `kind`, `rows` and `buffers`. The byte fields and the `decoder`
field are mutually exclusive. The byte form calls
`query_bytes_into_rows_atomic` or `batch_bytes_into_rows_atomic` on the
selected client or transaction. These methods claim
the output vector and one actual buffer per output position before the statement
factory or transaction begin. The selected `ColumnKey` and `PreparedSqlBytesKind`
read a borrowed SQLx cell into each buffer. Batch metadata occupies an output
position and releases its unused buffer; unused tail buffers are returned.

```rust,ignore
use effect::{Effect, memory::{BufferLease, Bytes}};
use effect_sql::{ColumnKey, PreparedSqlBatchItem, PreparedSqlBytesKind, PreparedSqlRowsCapacity, PreparedSqlRowsPool, SqlError};
use std::sync::Arc;

let buffers = Arc::new(memory.buffers(region, 32, Bytes::new(256))?);
let rows: Arc<PreparedSqlRowsPool<BufferLease<'static>>> = Arc::new(effect_sql::sql! {
    prepared_rows; memory: memory, region: region,
    capacity: PreparedSqlRowsCapacity { outputs: 1, rows_per_output: 32 },
}?);
let batch_rows: Arc<PreparedSqlRowsPool<PreparedSqlBatchItem<BufferLease<'static>>>> =
    Arc::new(effect_sql::sql! {
        prepared_rows; memory: memory, region: region,
        capacity: PreparedSqlRowsCapacity { outputs: 1, rows_per_output: 32 },
    }?);
let query: Effect<_, SqlError, Services> = effect_sql::sql! {
    atomic_query; client: client, statement: make_statement,
    column: ColumnKey::Ordinal(0), kind: PreparedSqlBytesKind::Text,
    rows: rows, buffers: buffers.clone(),
};
let blob_batch: Effect<_, SqlError, Services> = effect_sql::sql! {
    atomic_batch; client: client, batch: batch,
    column: ColumnKey::Ordinal(0), kind: PreparedSqlBytesKind::Blob,
    rows: batch_rows, buffers: buffers,
};
```

The example shares enough buffers to run the two operations sequentially.
Provision separate or additional buffers for concurrently held outputs.

Oversize values return the existing typed memory error and excess rows return
`RowLimit`; the transaction or savepoint follows its existing rollback path.
SQLx input, SDK and VM allocations remain separate from these destinations.

The additive `_with_frames` query methods return their exact frame layouts.
The root and nested typed and text/blob variants report outer and cursor
pending/completion layouts plus nested outcome, terminal-store, typed entry and
four instrumentation layouts; all eleven frames can be live at once. Provision the
caller-selected work pool for
the reported layouts and concurrent frames, keep a distinct cleanup pool, then
run the returned Effect with `RunFrames`. Missing or insufficient outer frames
return `SqlError::Memory` before the query factory runs; cursor frames are
claimed before the statement factory and transaction or savepoint begin.
The surrounding interpreter and native SQLx allocations remain separate.
The corresponding root and nested `batch_into_rows_atomic_with_frames` and
`batch_bytes_into_rows_atomic_with_frames` methods return the same frame
demand and reserve the cursor frames before consuming their one-use batch.

```rust,ignore
let (query, demand) = client.query_bytes_into_rows_atomic_with_frames::<_, Services>(
    make_statement, ColumnKey::Ordinal(0), PreparedSqlBytesKind::Text, rows, buffers,
);
// Include other concurrently required work/cleanup frames in the caller's
// budget; `cleanup` is a separately provisioned, distinct frame pool.
let work = memory.frames(region, demand.work_frames(), demand.maximum_layout()?)?;
let frames = effect::memory::RunFrames::new(work, cleanup)?;
let outcome = query.run_with_frames(environment, frames).await;
```

For a prepared graph root, `SqlClient::query_bytes_into_rows_atomic_prepared`
returns a one-use Description. For example, with an explicitly selected graph
`contract` and a reusable `make_statement` function:

```rust,ignore
let plan = effect::effect_prepare! {
    graph;
    program: client.query_bytes_into_rows_atomic_prepared::<_, Services>(
        make_statement, ColumnKey::Ordinal(0), PreparedSqlBytesKind::Text,
        rows.clone(), buffers.clone(),
    ),
    contract: contract,
};
let demand = plan.demand(1)?;
let frames = memory.frames(region, demand.frames_per_run, demand.frame_layout)?;
let graph = plan.prepare(&memory, region, 1, frames)?;
let output = graph.admit(environment)?.run().await;
```

The typed `query_into_rows_atomic_prepared` companion uses the same graph path
and invokes the genuine borrowed decoder with the prepared environment only
during each synchronous projection; it does not hold that borrow over SQL I/O.

The graph preadmits its own control, future and output backing before the
description runs. Configure the statement, row, session and cleanup owners
separately as needed; SQLx connection acquisition and native SDK/VM memory
remain outside the graph's frame demand. Runtime qualification is still pending.

Root prepared queries and batches await SQLx connection acquisition before
claiming SQL output, session, first-row, cleanup or diagnostic backing. Root
`_with_frames` variants also acquire the connection before reserving their
nested cursor frames. Both orders apply without a prepared session pool. SQLx
hooks can run before a later local capacity refusal. When the driver row pool
is selected, queries and nonempty batches claim its first row after checkout
and before the other SQL claims. Private lanes add exclusive registry
ownership to that claim. Statement factories and batch materialization follow
the local claims.

The root graph bodies borrow the active transaction session through cursor
retirement. The runner keeps that session for commit, rollback, and connection
cleanup. A caught body panic reaches SQL rollback before publication.

For these prepared graph queries and batches, callers can select an existing
`Arc<PreparedPanicPool>` with `client.with_prepared_panic_diagnostics(pool)`.
Each operation reserves six actual SQL diagnostics after connection checkout and
before statement construction or batch materialization: one for the cursor,
one shared by begin or body-future construction, one shared by commit or
rollback, and three for the close sequence. The six slots are claimed together
through one nonblocking registry gate; contention returns `MemoryError::Busy`.
Retained defects continue to occupy their slots. This pool is separate
from graph panic diagnostics selected with `panic_diagnostics: graph_panic_pool`
in `effect_prepare!`. Without the SQL selection, these catches use the existing
ordinary diagnostic conversion. SQLx and native memory remain separate.

The root `batch_into_rows_atomic_prepared` and
`batch_bytes_into_rows_atomic_prepared` descriptions accept an owned batch.
Use the graph's input form to admit the graph before transferring that batch:

```rust,ignore
let plan = effect::effect_prepare! {
    graph;
    input: move |batch| client.batch_bytes_into_rows_atomic_prepared::<Services>(
        batch, ColumnKey::Ordinal(0), PreparedSqlBytesKind::Text,
        rows.clone(), buffers.clone(),
    ),
    contract: contract,
};
let demand = plan.demand(1)?;
let frames = memory.frames(region, demand.frames_per_run, demand.frame_layout)?;
let graph = plan.prepare(&memory, region, 1, frames)?;
let admitted = graph.admit_owned_input(environment)?;
let output = admitted.publish_input(batch).await;
```

The same four `*_atomic_prepared` methods on `SqlTransaction` run a savepoint
on an existing active transaction. They take the transaction handle by value,
so the description can receive it through late graph input without cloning it
during construction. For example, a prepared graph for a text/blob batch can
receive the active handle and the batch together:

```rust,ignore
let plan = effect::effect_prepare! {
    graph;
    input: move |(transaction, batch): (effect_sql::SqlTransaction, effect_sql::PreparedSqlBatch)|
        transaction.batch_bytes_into_rows_atomic_prepared::<Services>(
            batch, ColumnKey::Ordinal(0), PreparedSqlBytesKind::Text,
            rows.clone(), buffers.clone(),
        ),
    contract: contract,
};
let demand = plan.demand(1)?;
let frames = memory.frames(region, demand.frames_per_run, demand.frame_layout)?;
let graph = plan.prepare(&memory, region, 1, frames)?;
let admitted = graph.admit_owned_input(environment)?;
let output = admitted.publish_input((transaction, batch)).await;
```

The enclosing transaction must remain active until this run settles. The
savepoint joins its cursor and commit or rollback before publishing its
outcome. The graph owns the concrete future and its cancellation context.
Select session, row and cleanup backing on the existing transaction; SQLx
and VM memory are separate.

Nested prepared descriptions and nested `_with_frames` methods claim selected
first-row backing before their additional output, cleanup and savepoint claims.
Empty batches skip the first-row claim. The existing transaction stays live
throughout admission.

A transaction acquired through a client configured with
`with_prepared_panic_diagnostics` carries that pool into the nested prepared
graph descriptions. Each graph savepoint claims three slots together before
statement-factory invocation or batch materialization. The slots cover begin or body failure,
cursor polling, and commit or rollback with its final release. Refusal uses
the existing typed memory error. Without the selection, native panic conversion
follows the ordinary path. Graph panic diagnostics cover their own boundaries.

If graph admission fails, `batch` has not been transferred. After publication,
the graph owns it and reports SQL/output admission and settlement failures
through its existing result channels.

## Atomic prepared batches

Given the statement pool and environment-aware decoder above, a batch pool
holds actual statement owners; the separate rows pool holds both decoded rows
and native result metadata:

```rust,ignore
let batch_pool = effect_sql::sql! {
    prepared_batch_pool; memory: memory, region: region,
    capacity: effect_sql::PreparedSqlBatchCapacity {
        batches: 2, statements_per_batch: 4,
    },
}?;
let batch = effect_sql::sql! {
    prepared_batch; pool: batch_pool,
    statement(effect_sql::sql! {
        prepared_statement; pool: env.statements,
        text("SELECT name FROM orders WHERE id = "); integer(first_id);
    }?);
    statement(effect_sql::sql! {
        prepared_statement; pool: env.statements,
        text("SELECT name FROM orders WHERE id = "); integer(second_id);
    }?);
}?;
let output = Arc::new(effect_sql::sql! {
    prepared_rows; memory: memory, region: region,
    capacity: effect_sql::PreparedSqlRowsCapacity { outputs: 2, rows_per_output: 16 },
}?);
let batch_query = effect_sql::sql! {
    atomic_batch; client: client, batch: batch, decoder: decoder, rows: output,
};
```

Each returned item identifies its input statement and contains either a decoded
row or result metadata. Both consume item capacity. The operation checks the
minimum remaining metadata space before starting each statement and uses the
existing explicit transaction or savepoint path. Native SQL conversion,
parser, VM, row and application pointee allocations remain separate.
After output, session and first-row preadmission, every batch statement is
materialized in the existing batch vector before beginning a transaction or
savepoint. A materialization failure retires the consumed batch without BEGIN or
statement execution; selected SQLx acquisition hooks may already have run.
An earlier capacity refusal leaves the one-use batch input unconsumed.
Each prepared statement lease remains alongside its native copy until its
cursor retires; unused entries drop the copy before the lease. Provider caches
and VM allocations are separate from this ownership.

## Café consumer and fixed SQL fixtures

See the [example build/run guide](examples/README.md) for explicit build and run
commands, compile-time database configuration, and expected output.

`examples/cafe_orders.rs` covers D02–D06 together: it moves a non-Clone order
through a transaction and nested savepoint, decodes/refines native rows, extracts
an update helper, projects the outer application environment and maps SQL errors
explicitly. Its native checked query uses the local SQLx facade. The owning host
selects Tokio once; `examples/support/cafe_database.rs` documents the temporary
file, pool admission, lock-wait and migration policy. Required features are
`turso,macros`; the literal checked query can be prepared with
`DATABASE_URL=turso::memory:` for compilation. Run:

```sh
DATABASE_URL=turso::memory: cargo +1.100.0 run -p effect-sql --features turso,macros --example cafe_orders
```

The story saves a 400-cent flat white, corrects its price to 425 cents inside the
checked transaction, then reloads the same order with `ready=false`.

D02 builds the read statement in `load_order_program`; D03 inserts the moved
payload in `save_order`; D04 updates it in the extracted `mark_ready` savepoint
helper; D05 decodes and refines it in `order_row`. D06 uses inline `run` with
`select` and an explicit `handle` for the transaction, `run: ready_count` for an
extracted client callback, and `effect: load_order_program(id)` with `project` for
an extracted typed Effect. Both outer forms map `SqlError` through
`CafeError::Sql` while projecting `CafeEnv` into the inner `SqlClient` environment.

`tests/contracts.rs` groups the finite V03 contracts: native bindings/identifiers
and row outcomes; scalar/optional/fold/batch modes; transaction/savepoint and
stale-frame behavior; deterministic cancellation/cleanup/uncertainty; and migration
history, dirty recovery and independent database write authority. The shared test
support owns isolated temporary directories and two native leases, with immediate
lock contention used as a semantic boundary rather than a stress target.


Incremental cursors can retain SQLx row values in a selected prepared region.
Given prepared `memory`, `region`, `session_pool`, `native_pool`, a decoder,
and an output pool, configure both controls explicitly:

```ignore
let capacity = effect_sql::sqlx::PreparedTursoRowsCapacity {
    rows: 8, max_columns: 16, bytes_per_row: 4096,
};
let native_rows = Arc::new(effect_sql::sql! {
    prepared_native_rows; memory: memory, region: region, capacity: capacity,
}?);
let client = effect_sql::SqlClient::new(native_pool)
    .with_session_pool(session_pool)
    .with_rows_pool(native_rows);
let query = effect_sql::sql! {
    atomic_query; client: client, statement: statement, decoder: decoder, rows: output,
};
let batch_query = effect_sql::sql! {
    atomic_batch; client: client, batch: batch, decoder: batch_decoder, rows: batch_output,
};
```

Atomic query and nonempty batch paths preclaim the first input row before the
statement factory or native materialization. For explicit composition with the
shared pool, `effect_sql::sql! { reserve_rows; client: client, }` returns that
actual permit.
Each later batch statement reserves its input row before cursor construction.
The selected cursor checks the native statement's column count against the
row pool before constructing SQLx column metadata; native statement preparation
itself remains separate.

Each retained row or cloned text/blob value can hold a whole byte slot. A next
fetch, including the fetch that discovers EOF, needs an available row slot;
size the pool for actual retained outputs and scratch or handle the typed
refusal. Scalar and copy-into-region-buffer projections can release the input
slot for reuse. Native SDK `get_value` temporary copies, VM, parser, cursor
future and column metadata remain separate, as do decoded output pointees.

For exclusive registry access, prepare private lanes and select them on the
client. Each lane has its own driver pool and first-row backing. The lane
admission checks the configured lanes without waiting. It moves the lease from
first-row admission into the cursor and returns it after the provider stream
and reservations retire. Published rows and cloned text/blob values retain
their own row or byte owners. A retained value can exhaust one lane's row
backing, so admission can try other eligible lanes.

```rust,ignore
let row_lanes = Arc::new(effect_sql::sql! {
    prepared_native_rows; memory: memory, region: region,
    lanes: concurrent_cursors, capacity: capacity_per_lane,
}?);
let client = effect_sql::sql! { client; pool: native_pool, row_lanes: row_lanes, }?;
```

`PreparedSqlRowLanes::demand` reports requested region storage and the startup
driver controls per lane. The raw `reserve_rows` form is available on the
shared selection. Exclusive selection keeps all registry owners inside
Effect SQL and returns `UnsupportedRunContract` through that raw form. The
exclusive registry policy covers explicit incremental cursors and selected
atomic queries and batches. Native SDK values, metadata creation, cursors and
VM allocations still use their own storage.

## Current prepared SQL memory contract

The following describes the selected APIs and actual owner lifetimes. It is
source and scoped-check evidence, not a measured native-memory or RSS bound.

Use `SqlClient::open(native_memory, acquire_pool)` when assembling SQL during
host startup. It checks the requested **native SQL** memory policy before it
calls the pool factory. Supplying `MemoryPolicy::PreparedReuse` or
`MemoryPolicy::BoundedAllocation` as `Some(policy)` currently returns
`SqlStartupError::MissingNativeMemoryEnvelope` for SQLx/Turso. Propagate that
startup error before starting a listener. A host that validates its adapters
separately can call `SqlClient::validate_native_memory` during configuration.

`None` explicitly selects ordinary SQLx/Turso allocation for the native SQL
domain. Prepared Effect-owned statements, rows and session controls can be
used with that selection. A host choosing ordinary native SQL accounts for
it outside those prepared capacities. `SqlClient::new` adopts an existing pool.

| Owner and API | Claim, reuse and refusal | Separate native or caller storage |
| --- | --- | --- |
| `PreparedSqlStatementPool` and `sql! { prepared_statement; ... }` | A builder claims one fixed SQL-text, binding-byte and descriptor slot. `finish` transfers that lease; a one-use `StatementSource` retains it through native materialization. Optional materialization admission also charges each live copy for the requested SQL text, used text/blob binding bytes and SQLx argument descriptor storage. | Allocator overhead, provider-side conversions, parser, statement cache and VM storage are separate. |
| `PreparedSqlRowsPool<A>` and atomic query/batch | The output vector is claimed before the statement factory or batch input is consumed. The returned `PreparedSqlRows` retains it until drop; exhausted output reports `RowLimit`. A batch item includes either a decoded row or result metadata. | Each `A` and its pointees have the caller's own storage contract; the inline vector does not bound them. |
| `PreparedSqlSessionPool`, `reserve_session`, `SqlKernel::with_session_pool` | An explicit reservation claims a control before pool acquisition. Prepared client atomic paths acquire SQLx first and then claim the session control without waiting. Live session and cursor loans retain it until retirement. Frame/authority scarcity is checked before BEGIN/savepoint. | SQLx pool connections, native transaction/rollback work and VM allocations are separate. |
| `PreparedSqlBatchPool` and `atomic_batch` | The builder holds a finite ordered vector of actual prepared statements; the finished batch retains that vector until consumed or dropped. An excess statement is refused without replacement or truncation. Atomic execution uses the existing transaction/savepoint settlement. | Native materialization for each statement, transaction machinery and decoded pointees remain separate. |
| `PreparedTursoRowsPool`, `reserve_rows`, `SqlKernel::with_rows_pool` | Atomic query and nonempty batch preclaim the first row owner before the factory or input take; later fetches claim actual row/byte backing before SDK `Rows::next`. Published rows and cloned text/blob values retain the relevant backing, and scarcity or a copied-byte/column limit returns a typed error. | Native SDK row/value copies, parser, VM, column metadata and cursor futures are separate. The preclaimed row also must be available for the fetch that discovers EOF. |

For a concrete **source-only** counterexample, a short statement such as
`SELECT zeroblob(8192)` can run with a prepared row byte capacity of 4096.
Pinned `turso_core` 0.8.3 `vdbe/value.rs` (`exec_zeroblob` and
`exec_randomblob`) creates a native vector of the requested length before our
row-copy capacity check; its built-in blob ceiling is 1,000,000,000 bytes,
not the selected row-pool capacity. Pinned `turso` 0.8.3 `src/lib.rs`
(`Statement::step`) constructs owned SDK row values; `turso_sdk_kit` 0.8.3
`src/rsapi.rs` (`row_value`) obtains an owned value, and `turso` 0.8.3
`src/rows.rs` (`Row::get_value`) clones text/blob values before
`PreparedTursoRowReservation::copy_native` can reject an oversized copy.
This is a source trace; it has not been executed as a measurement.

The inspected public `turso` 0.8.3 `Builder`, `Connection`, `Statement` and
`Rows` APIs expose database setup, preparation, querying and row access, but
no general per-database or per-statement allocator or hard native VM-byte
limit.

The café example selects `TursoConnectOptions::statement_cache_capacity(0)`
when it creates its pool. Each connection skips insertion into the driver's
SQLx statement cache, so the cache retains no SQL keys, column metadata or
native statement handles across operations and pool checkouts. Preparing and
executing SQL still uses native parser, statement, cursor, row, page and VM
storage. Pool and connection metadata also have their own lifetimes.
`PRAGMA cache_size` selects a soft page-cache target. Turso can admit pages
above that target when it cannot spill or evict pages.

`turso_core` 0.8.3 `database.rs` exposes `OpenOptions::allocators` for the
`mv_store` and `fts` fields; that is not an all-VM allocator exposed by the
public `turso` builder. Its `alloc/backend.rs` setter is process-wide,
unsafe and once-only; this integration does not select it. In the pinned
stable configuration `alloc/collections/vec/mod.rs` aliases `DynVec` to
`std::vec::Vec` and the allocator-taking extension methods ignore their
allocator argument. Meeting a native byte bound requires an actual upstream
capability covering the relevant native allocation domains, or separately
proven bounds for those domains; the prepared retained-owner capacities alone
do not establish one.
