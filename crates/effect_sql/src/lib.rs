//! Composable SQL Effects over this workspace's native SQLx/Turso provider.
//!
//! `SqlClient` is inert shared configuration. Each lazy operation obtains a fresh
//! pool lease; transactions/savepoints retain one exclusive owner through ordered
//! cleanup. Native Identity stores call `SqlKernel` directly with operation-owned
//! transactions. Tokio is the local provider's reactor, selected by the application;
//! this crate creates no runtime and core Effect remains provider-neutral.
//!
//! Construct moved statements inside an Effect body, or supply a fresh statement
//! factory to reusable client operations. `Sql::execute/query/first` and explicit
//! `*_once` APIs and owned `StatementSource` inputs transfer a payload once; a second execution reports consumption
//! rather than cloning business data. SQLx's checked macros/offline metadata remain
//! available through the re-exported native facade. Dynamic DSL statements retain
//! typed binding/identifier validation and native SQL execution diagnostics.
//!
//! SQL text, parameters, credentials and connection strings are excluded from
//! completion observations. Rich error diagnostics are for trusted callers and
//! should be mapped to application-safe errors at HTTP/log boundaries.
//!
//! `PreparedSqlRowsPool<A>` provides finite inline result slots, independent
//! of the native driver's row/value allocations and any pointees owned by `A`.
//! Given an existing prepared memory and region, configure it explicitly:
//!
//! ```ignore
//! let capacity = effect_sql::PreparedSqlRowsCapacity { outputs: 4, rows_per_output: 32 };
//! let bytes = capacity.demand::<MyRow>()?;
//! let rows: effect_sql::PreparedSqlRowsPool<MyRow> = effect_sql::sql! {
//!     prepared_rows; memory: memory, region: region, capacity: capacity,
//! }?;
//! let mut output = rows.reserve()?;
//! output.try_push(my_row).map_err(|row| /* retain or handle row */ row)?;
//! let owned = output.finish();
//! for row in owned.iter() { /* use row */ }
//! ```
//! Demand excludes allocator rounding/overhead and any caller-owned outer Arc.
//! This storage alone does not prepare query, decoder or native driver backing.
//! A `RowDecoder::pure` (including `row! { project: ... }`) also exposes its
//! original synchronous borrowed projection through `decode_ref`; effectful
//! row programs do not acquire that witness. The caller owns panic/context
//! handling, and both native input-row backing and returned-value pointees
//! remain separate from prepared output slots.
//!
//! `query_into_rows_atomic` explicitly selects a transaction for `SqlClient`,
//! or a savepoint for `SqlTransaction`. It admits the output before invoking
//! the statement factory, requires a genuine borrowed decoder, and checks one
//! further native row after the output fills to distinguish EOF from overflow.
//! The cursor is dropped before transaction settlement. For example, given an
//! `Arc<PreparedSqlRowsPool<MyRow>>` and a pure `RowDecoder`:
//!
//! ```ignore
//! let query = effect_sql::sql! {
//!     atomic_query; client: client, statement: make_statement,
//!     decoder: decoder, rows: rows,
//! };
//! // Run `query` in the application's existing Effect environment.
//! ```
//! Projection callbacks run in row order and may have external side effects;
//! those effects and transaction-control SQL are governed by the existing
//! transaction contract. Native VM, parser, row, argument and returned-value
//! pointee allocations are not bounded by the prepared output vector.
//!
//! A genuine environment-aware projection can retain a fixed buffer from the
//! selected `R`. Given a startup-prepared pool of buffer slots:
//!
//! ```ignore
//! use effect_sql::sqlx::Row;
//! struct Services {
//!     names: effect::memory::SlotPool<'static, effect::memory::Buffer<'static>>,
//! }
//! let decoder = effect_sql::row! {
//!     project_with_env: |row: &effect_sql::sqlx::TursoRow, env: &Services| {
//!         let name: &str = row.try_get(0).map_err(|source| effect_sql::RowError::Native {
//!             column: "[0]".into(), source,
//!         })?;
//!         let mut bytes = env.names.checkout().map_err(effect_sql::RowError::Memory)?;
//!         bytes.clear();
//!         bytes.extend_from_slice(name.as_bytes()).map_err(effect_sql::RowError::Memory)?;
//!         Ok(bytes)
//!     },
//! };
//! ```
//! The result owns that region-backed slot. The native input row, startup
//! service/control metadata and any caller-owned Arc remain separate.

extern crate self as effect_sql;

/// Typed callback blocks using this facade's existing Effect dependency.
pub use effect::{
    EffectEnvironment,
    codec, effect_all, effect_all_concurrent, effect_blocking, effect_cpu, effect_die, effect_fail,
    effect_finalizer, effect_for_each, effect_for_each_concurrent, effect_from_option, effect_from_result,
    effect_future, effect_map, effect_map_error, effect_on_interrupt, effect_permit,
    effect_provide_some, effect_provided, effect_race, effect_recover, effect_ref, effect_repeat,
    effect_resource, effect_retry, effect_sandbox, effect_schema_parse, effect_scope,
    effect_scope_acquire, effect_socket, effect_stream_for_each, effect_sync, effect_timeout,
    effect_try_blocking, effect_try_cpu, effect_try_future, effect_try_sync,
    effect_uninterruptible, effect_using, effect_with_layer, layer, pool, request_resolver,
    schedule, sink, weak_cache,
};

#[cfg(feature = "turso")]
mod client;
#[cfg(feature = "turso")]
fn try_prepared_lock<T>(mutex: &std::sync::Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, effect::memory::MemoryError> {
    match mutex.try_lock() {
        Ok(guard) => Ok(guard),
        Err(std::sync::TryLockError::Poisoned(error)) => Ok(error.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => Err(effect::memory::MemoryError::Busy),
    }
}
#[cfg(feature = "turso")]
mod error;
#[cfg(feature = "turso")]
pub mod kernel;
#[cfg(feature = "turso")]
mod migration;
#[cfg(feature = "turso")]
mod prepared_batch;
#[cfg(feature = "turso")]
mod prepared_rows;
#[cfg(feature = "turso")]
mod prepared_statement;
#[cfg(feature = "turso")]
mod row;
#[cfg(feature = "turso")]
mod statement;
#[cfg(feature = "turso")]
mod telemetry;

#[cfg(feature = "turso")]
pub use client::{
    BatchResult, HasSqlClient, PreparedSqlBatchEffect, PreparedSqlBytesKind, PreparedSqlFrameDemand, Sql, SqlClient,
    SqlSession, SqlTransaction, StatementSource, sql_with,
};
#[cfg(feature = "turso")]
pub use error::{MigrationPhase, RowError, SqlError, SqlStartupError};
#[cfg(feature = "turso")]
pub use kernel::{
    NativeCursor, NativeFuture, NativeOperation, NativeSession, PreparedSqlSessionCapacity,
    PreparedSqlSessionDemand, PreparedSqlSessionPool, PreparedSqlSessionReservation, SqlKernel,
};
#[cfg(feature = "turso")]
pub use migration::{
    DIRTY_METADATA, DirtyMigration, DownPlan, MaintenanceGuard, MaintenanceProvider,
    MigrationCatalog, MigrationDirection, MigrationStep, RecoveryDecision, Rollback, SQLX_JOURNAL,
    SqlMigrator,
};
#[cfg(feature = "turso")]
pub use prepared_batch::{
    PreparedSqlBatch, PreparedSqlBatchBuilder, PreparedSqlBatchCapacity, PreparedSqlBatchItem,
    PreparedSqlBatchPool,
};
#[cfg(feature = "turso")]
pub use prepared_rows::{
    PreparedNativeRowsError, PreparedSqlRowLanes, PreparedSqlRowLanesDemand, PreparedSqlRows,
    PreparedSqlRowsBuilder, PreparedSqlRowsCapacity, PreparedSqlRowsPool, prepare_native_rows,
};
#[cfg(feature = "turso")]
pub use prepared_statement::{
    PreparedSqlStatement, PreparedSqlStatementBuilder, PreparedSqlStatementCapacity,
    PreparedSqlStatementError, PreparedSqlStatementPool,
};
#[cfg(feature = "turso")]
pub use row::{
    CellCodec, ColumnKey, RowContext, RowDecoder, RowRead, RowStep, choice_text, column,
    native_column, native_column_ref, optional_column, refine, validate,
};
#[cfg(feature = "turso")]
pub use statement::{
    Identifier, MutationBuilder, SqlPredicate, SqlStatement, StatementBuilder, TrustedSql,
};
#[cfg(feature = "turso")]
pub use telemetry::{SqlCompletion, SqlObservation, SqlOperation, observe};

/// Native SQLx facade retaining checked macros, Type/Decode and direct executor interoperation.
#[cfg(feature = "turso")]
pub use sqlx;

/// Construct a reusable native row program with scoped column reads and Effect validation.
/// `row! { project: callback }` adapts an extracted borrowed native row projection.
#[cfg(feature = "turso")]
pub use effect_macros::{EffectSqlRow, row};
/// Build an inert statement with text/identifier/fragment/where/param operations.
/// Returns Result; literals are trusted syntax and values always use native bindings.
#[cfg(feature = "turso")]
pub use effect_macros::sql;
/// Build a checked nonempty INSERT with repeated value(column, native_value) entries.
#[cfg(feature = "turso")]
pub use effect_macros::sql_insert;
/// Build a checked nonempty UPDATE with set entries and exactly one typed where predicate.
#[cfg(feature = "turso")]
pub use effect_macros::sql_update;
/// Project a SQL Effect with select/handle/project and explicit optional error mapping.
#[cfg(feature = "turso")]
pub use effect_macros::sql_with;

/// Narrow expansion surface resolved through renamed effect-sql dependencies.
#[doc(hidden)]
pub mod __private {
    #[cfg(feature = "turso")]
    /// Row operations are scoped to the row-program carrier constructed by D05.
    pub use crate::{choice_text, column, native_column, optional_column, refine, validate};
    #[cfg(feature = "turso")]
    pub use sqlx::TursoConnection;
    /// Existing interpreter and macro lowering; never a separate SQL runtime.
    pub use effect;
}

/// Inferred statement ownership markers; no runtime values or authority are carried.
#[doc(hidden)]
#[cfg(feature = "turso")]
pub use client::{FreshStatement, OwnedStatement};
