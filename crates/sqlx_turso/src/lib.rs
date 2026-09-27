//! SQLx adapter for the Rust Turso database engine
//!
//! `sqlx-turso` exposes a distinct SQLx [`Database`](sqlx::Database) implementation backed by
//! the Rust `turso` crate. The facade re-exports the runtime types from `sqlx-turso-core` and,
//! with the `macros` feature, first-party checked query macros such as [`query!`].
//!
//! # Example
//!
//! ```no_run
//! use sqlx_turso::{
//!     TursoConnection,
//!     sqlx::{Connection, Executor, Row},
//! };
//!
//! # async fn example() -> sqlx_turso::sqlx::Result<()> {
//! let mut conn = TursoConnection::connect("turso::memory:").await?;
//! conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").await?;
//! conn.execute("INSERT INTO users (id, name) VALUES (1, 'alice')").await?;
//!
//! let row = conn.fetch_one("SELECT name FROM users WHERE id = 1").await?;
//! assert_eq!(row.try_get::<String, _>("name")?, "alice");
//! # Ok(())
//! # }
//! ```
//!
//! # Features
//!
//! - `runtime-tokio` is enabled by default and is the supported production runtime
//! - `macros` enables `sqlx_turso::query!` and related checked query macros
//! - `migrate` enables SQLx migration traits and local database lifecycle helpers
//! - `any` enables SQLx `Any` driver registration for Turso
//! - `offline` enables serializable query metadata
//! - SQLx's optional value features are forwarded with Turso codecs
//! - all native Turso Cargo features have matching forwarding features
//! - `sync` enables local sync-backed connections and checkpoint/stat APIs

extern crate self as sqlx_turso;

pub use sqlx_turso_core::{
    Turso, TursoAdapterError, TursoArguments, TursoColumn, TursoConnectOptions, TursoConnection,
    TursoDatabaseError, TursoDatabaseTarget, TursoDescribeExt, TursoEncryptionOptions,
    TursoExecutor, TursoExperimentalFeature, TursoExperimentalFeatures, TursoIo,
    TursoNativeTransaction, TursoPool, TursoPoolOptions, TursoQueryResult, TursoRow,
    TursoStatement, TursoSyncOptions, TursoTransaction, TursoTransactionManager, TursoTypeChecking,
    TursoTypeInfo, TursoValue, TursoValueRef,
};

/// The exact native Turso crate used by this adapter.
///
/// Applications may also depend on `turso` directly; this re-export guarantees
/// an unambiguous path to the native types that interoperate with the adapter.
pub use turso;

#[cfg(feature = "migrate")]
pub use sqlx_turso_core::{Migrate, MigrateDatabase, Migration, MigrationType};

#[doc(hidden)]
pub use sqlx;

#[cfg(feature = "any")]
pub use sqlx_turso_core::{TURSO_ANY_DRIVER, install_turso_any_driver};

#[cfg(feature = "macros")]
pub use sqlx_turso_macros::{
    query, query_as, query_as_unchecked, query_file, query_file_as, query_file_as_unchecked,
    query_file_scalar, query_file_scalar_unchecked, query_file_unchecked, query_scalar,
    query_scalar_unchecked, query_unchecked,
};
