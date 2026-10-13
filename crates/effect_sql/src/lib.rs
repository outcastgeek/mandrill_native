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

extern crate self as effect_sql;

/// Typed callback blocks using this facade's existing Effect dependency.
pub use effect::{
    codec, effect_all, effect_all_concurrent, effect_blocking, effect_cpu, effect_die,
    effect_finalizer, effect_for_each, effect_for_each_concurrent, effect_from_option,
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
mod error;
#[cfg(feature = "turso")]
pub mod kernel;
#[cfg(feature = "turso")]
mod migration;
#[cfg(feature = "turso")]
mod row;
#[cfg(feature = "turso")]
mod statement;
#[cfg(feature = "turso")]
mod telemetry;

#[cfg(feature = "turso")]
pub use client::{
    BatchResult, HasSqlClient, Sql, SqlClient, SqlSession, SqlTransaction, StatementSource,
    sql_with,
};
#[cfg(feature = "turso")]
pub use error::{MigrationPhase, RowError, SqlError};
#[cfg(feature = "turso")]
pub use kernel::{NativeCursor, NativeFuture, NativeOperation, NativeSession, SqlKernel};
#[cfg(feature = "turso")]
pub use migration::{
    DIRTY_METADATA, DirtyMigration, DownPlan, MaintenanceGuard, MaintenanceProvider,
    MigrationCatalog, MigrationDirection, MigrationStep, RecoveryDecision, Rollback, SQLX_JOURNAL,
    SqlMigrator,
};
#[cfg(feature = "turso")]
pub use row::{
    CellCodec, ColumnKey, RowContext, RowDecoder, RowRead, RowStep, choice_text, column,
    native_column, optional_column, refine, validate,
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
pub use effect_macros::row;
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
    /// Existing interpreter and macro lowering; never a separate SQL runtime.
    pub use effect;
}

/// Inferred statement ownership markers; no runtime values or authority are carried.
#[doc(hidden)]
#[cfg(feature = "turso")]
pub use client::{FreshStatement, OwnedStatement};
