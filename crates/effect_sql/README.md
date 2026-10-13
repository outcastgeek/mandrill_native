# Effect SQL

`effect-sql` composes with `Effect<A, E, R>` and uses this workspace's SQLx/Turso
provider. The caller supplies the Tokio reactor for native I/O. The crate creates
no runtime; CPU and blocking capabilities elsewhere in the same `R` remain
independent. The `turso` feature supplies native execution. The optional `macros`
feature enables SQLx's checked query macros and offline metadata interoperation.

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
before schema work. SQLx `success=false` entries and extension dirty markers need
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

`observe` selects safe completion observation through `R` and its monotonic clock.
It carries operation/category, duration, selected result count and cleanup/trace
classification. SQL, bindings, credentials and connection strings are excluded.
Observer failures are best effort and do not replace the business exit; exporters
are optional. Rich native error diagnostics remain available to trusted callers
and should be mapped before public HTTP responses or logs.

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
DATABASE_URL=turso::memory: cargo +1.99.0 run -p effect-sql --features turso,macros --example cafe_orders
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
