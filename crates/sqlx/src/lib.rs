//! SQLx facade with the Turso driver included in ordinary `sqlx::` APIs.

pub use sqlx_upstream::{
    Acquire, Arguments, AssertSqlSafe, Column, ColumnIndex, ColumnOrigin, ConnectOptions,
    Connection, Database, Decode, Describe, Either, Encode, Error, Execute, Executor, FromRow,
    IntoArguments, Pool, QueryBuilder, RawSql, Result, Row, SqlSafeStr, SqlStr, Statement,
    Transaction, Type, TypeInfo, Value, ValueRef, database, decode, encode, error, pool, prelude,
    query_builder, raw_sql, spec_error, types,
};

pub use sqlx_core::query::{query, query_with};
pub use sqlx_core::query_as::{query_as, query_as_with};
pub use sqlx_core::query_scalar::{query_scalar, query_scalar_with};

/// SQLx query types for every enabled backend, including Turso.
pub mod query {
    pub use sqlx_upstream::query::*;
}

#[allow(deprecated)]
pub use sqlx_upstream::{
    warn_on_ambiguous_inferred_date_time_crate, warn_on_ambiguous_inferred_numeric_crate,
};

#[doc(hidden)]
pub use sqlx_upstream::{
    __query_scalar_with_result, __query_with_result, __rt, _unstable, sqlx_macros, test_block_on,
};

/// SQLx checked-query type matching, available with the native macro feature.
#[doc(hidden)]
#[cfg(feature = "macros")]
pub use sqlx_upstream::ty_match;

#[cfg(feature = "_unstable-docs")]
pub use sqlx_upstream::_config;

#[cfg(feature = "migrate")]
pub use sqlx_upstream::{migrate, testing};

#[cfg(feature = "macros")]
pub use sqlx_upstream::test;

#[cfg(feature = "mysql")]
pub use sqlx_upstream::{
    MySql, MySqlConnection, MySqlExecutor, MySqlPool, MySqlTransaction, mysql,
};

#[cfg(feature = "postgres")]
pub use sqlx_upstream::{PgConnection, PgExecutor, PgPool, PgTransaction, Postgres, postgres};

#[cfg(any(feature = "sqlite", feature = "sqlite-unbundled"))]
pub use sqlx_upstream::{
    Sqlite, SqliteConnection, SqliteExecutor, SqlitePool, SqliteTransaction, sqlite,
};

pub use sqlx_turso_core::{
    Turso, TursoAdapterError, TursoArguments, TursoColumn, TursoConnectOptions, TursoConnection,
    TursoDatabaseError, TursoDatabaseTarget, TursoDescribeExt, TursoEncryptionOptions,
    TursoExecutor, TursoExperimentalFeature, TursoExperimentalFeatures, TursoIo,
    TursoNativeTransaction, TursoPool, TursoPoolOptions, TursoQueryResult, TursoRow,
    TursoStatement, TursoSyncOptions, TursoTransaction, TursoTransactionManager, TursoTypeChecking,
    TursoTypeInfo, TursoValue, TursoValueRef,
};

#[cfg(feature = "any")]
pub use any::{Any, AnyConnection, AnyExecutor, AnyPool};

#[cfg(feature = "any")]
pub mod any {
    //! Runtime-generic SQLx support including Turso.

    use std::sync::Once;

    #[allow(deprecated)]
    pub use sqlx_upstream::any::AnyKind;
    pub use sqlx_upstream::any::{
        Any, AnyArguments, AnyConnectOptions, AnyExecutor, AnyPoolOptions, AnyQueryResult, AnyRow,
        AnyStatement, AnyTransactionManager, AnyTypeInfo, AnyTypeInfoKind, AnyValue, AnyValueRef,
        install_drivers,
    };
    pub use sqlx_upstream::{AnyConnection, AnyPool};

    /// Installs Turso and every enabled stock SQLx driver for `Any` dispatch.
    pub fn install_default_drivers() {
        static ONCE: Once = Once::new();

        ONCE.call_once(|| {
            let result = install_drivers(&[
                sqlx_turso_core::TURSO_ANY_DRIVER,
                #[cfg(feature = "mysql")]
                sqlx_upstream::mysql::any::DRIVER,
                #[cfg(feature = "postgres")]
                sqlx_upstream::postgres::any::DRIVER,
                #[cfg(any(feature = "sqlite", feature = "sqlite-unbundled"))]
                sqlx_upstream::sqlite::any::DRIVER,
            ]);
            assert!(
                result.is_ok(),
                "non-default SQLx Any drivers were already installed"
            );
        });
    }
}

#[cfg(feature = "macros")]
pub use sqlx_turso_macros::{
    query, query_as, query_as_unchecked, query_file, query_file_as, query_file_as_unchecked,
    query_file_scalar, query_file_scalar_unchecked, query_file_unchecked, query_scalar,
    query_scalar_unchecked, query_unchecked,
};
