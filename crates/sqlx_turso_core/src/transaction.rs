use sqlx_core::sql_str::SqlSafeStr;
use sqlx_core::{
    error::Error,
    sql_str::SqlStr,
    transaction::{
        TransactionManager, begin_ansi_transaction_sql, commit_ansi_transaction_sql,
        rollback_ansi_transaction_sql,
    },
};

use crate::{Turso, connection::TursoConnection, executor::map_turso_error};

/// SQLx-compatible transaction handle for Turso connections
pub type TursoTransaction<'c> = sqlx_core::transaction::Transaction<'c, Turso>;

/// SQLx transaction manager for Turso connections
#[derive(Debug)]
pub struct TursoTransactionManager;

impl TursoTransactionManager {
    /// Begin an operation-owned transaction through the existing SQLx manager.
    /// Callers must retain exclusive connection ownership until explicit settlement.
    pub async fn begin_owned(connection: &mut TursoConnection, immediate: bool) -> Result<(), Error> {
        <Self as TransactionManager>::begin(connection, immediate.then(|| sqlx_core::sql_str::AssertSqlSafe("BEGIN IMMEDIATE").into_sql_str())).await
    }

    /// Await commit and publish the existing driver's depth update together.
    /// A caller losing acknowledgment must quarantine its connection rather than retry.
    pub async fn commit_owned(connection: &mut TursoConnection) -> Result<(), Error> {
        <Self as TransactionManager>::commit(connection).await
    }

    /// Await rollback using SQLx's existing frame accounting, rather than drop fallback.
    pub async fn rollback_owned(connection: &mut TursoConnection) -> Result<(), Error> {
        <Self as TransactionManager>::rollback(connection).await
    }
}

impl TransactionManager for TursoTransactionManager {
    type Database = Turso;

    async fn begin(conn: &mut TursoConnection, statement: Option<SqlStr>) -> Result<(), Error> {
        conn.clear_pending_rollback().await?;

        if statement.is_some() && conn.transaction_depth() > 0 {
            return Err(Error::InvalidSavePointStatement);
        }

        let sql = statement.unwrap_or_else(|| begin_ansi_transaction_sql(conn.transaction_depth()));
        conn.raw()
            .execute(sql.as_str(), ())
            .await
            .map_err(map_turso_error)?;
        conn.increment_transaction_depth();
        Ok(())
    }

    async fn commit(conn: &mut TursoConnection) -> Result<(), Error> {
        conn.clear_pending_rollback().await?;

        let depth = conn.transaction_depth();
        if depth <= conn.native_outer_transaction_depth() {
            return Ok(());
        }

        let sql = commit_ansi_transaction_sql(depth);
        conn.raw()
            .execute(sql.as_str(), ())
            .await
            .map_err(map_turso_error)?;
        conn.decrement_transaction_depth();
        Ok(())
    }

    async fn rollback(conn: &mut TursoConnection) -> Result<(), Error> {
        conn.clear_pending_rollback().await?;

        let depth = conn.transaction_depth();
        if depth <= conn.native_outer_transaction_depth() {
            return Ok(());
        }

        let sql = rollback_ansi_transaction_sql(depth);
        conn.raw()
            .execute(sql.as_str(), ())
            .await
            .map_err(map_turso_error)?;
        conn.decrement_transaction_depth();
        Ok(())
    }

    fn start_rollback(conn: &mut TursoConnection) {
        conn.mark_rollback_needed();
    }

    fn get_transaction_depth(conn: &TursoConnection) -> usize {
        conn.transaction_depth()
    }
}

#[cfg(test)]
mod tests {
    use sqlx_core::{
        connection::{ConnectOptions, Connection},
        executor::Executor,
        row::Row,
    };
    use turso::transaction::{DropBehavior, TransactionBehavior};

    use crate::{TursoConnectOptions, TursoConnection};

    #[tokio::test]
    async fn commits_top_level_transaction() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        let mut transaction = connection.begin().await?;
        (&mut *transaction)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;
        transaction.commit().await?;

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        Ok(())
    }

    #[tokio::test]
    async fn rolls_back_top_level_transaction() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        let mut transaction = connection.begin().await?;
        (&mut *transaction)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;
        transaction.rollback().await?;

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 0);

        Ok(())
    }

    #[tokio::test]
    async fn supports_nested_savepoint_rollback() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        let mut transaction = connection.begin().await?;
        (&mut *transaction)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        let mut nested = transaction.begin().await?;
        (&mut *nested)
            .execute("INSERT INTO test (id) VALUES (2)")
            .await?;
        nested.rollback().await?;

        transaction.commit().await?;

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        Ok(())
    }

    #[tokio::test]
    async fn supports_nested_savepoint_commit() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        let mut transaction = connection.begin().await?;
        (&mut *transaction)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        let mut nested = transaction.begin().await?;
        (&mut *nested)
            .execute("INSERT INTO test (id) VALUES (2)")
            .await?;
        nested.commit().await?;

        transaction.commit().await?;

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 2);

        Ok(())
    }

    #[tokio::test]
    async fn supports_custom_top_level_begin() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;
        (&mut *transaction)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;
        transaction.commit().await?;

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        Ok(())
    }

    #[tokio::test]
    async fn rejects_custom_begin_inside_nested_transaction() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        let mut transaction = connection.begin().await?;

        let error = match transaction.begin_with("BEGIN IMMEDIATE").await {
            Err(error) => error,
            Ok(_) => {
                return Err(sqlx_core::Error::Protocol(
                    "custom nested begin succeeded".to_owned(),
                ));
            }
        };
        assert!(matches!(
            error,
            sqlx_core::error::Error::InvalidSavePointStatement
        ));

        transaction.rollback().await?;
        Ok(())
    }

    #[tokio::test]
    async fn dropped_transaction_rolls_back_on_next_use() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        {
            let mut transaction = connection.begin().await?;
            (&mut *transaction)
                .execute("INSERT INTO test (id) VALUES (1)")
                .await?;
        }

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 0);

        Ok(())
    }

    #[tokio::test]
    async fn dropped_nested_transaction_rolls_back_to_savepoint() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        let mut transaction = connection.begin().await?;
        (&mut *transaction)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        {
            let mut nested = transaction.begin().await?;
            (&mut *nested)
                .execute("INSERT INTO test (id) VALUES (2)")
                .await?;
        }

        (&mut *transaction)
            .execute("INSERT INTO test (id) VALUES (3)")
            .await?;
        transaction.commit().await?;

        let rows = (&mut connection)
            .fetch_all("SELECT id FROM test ORDER BY id")
            .await?;
        let ids = rows
            .iter()
            .map(|row| row.try_get::<i64, _>("id"))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(ids, [1, 3]);

        Ok(())
    }

    #[tokio::test]
    async fn dropped_nested_and_outer_transactions_roll_back_all() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        {
            let mut transaction = connection.begin().await?;
            (&mut *transaction)
                .execute("INSERT INTO test (id) VALUES (1)")
                .await?;

            {
                let mut nested = transaction.begin().await?;
                (&mut *nested)
                    .execute("INSERT INTO test (id) VALUES (2)")
                    .await?;
            }
        }

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 0);

        Ok(())
    }

    #[tokio::test]
    async fn native_owned_transaction_runs_sqlx_and_nested_savepoints() -> sqlx_core::Result<()> {
        let database = turso::Builder::new_local(":memory:")
            .build()
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut native_connection = database
            .connect()
            .map_err(crate::executor::map_turso_error)?;
        native_connection
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", ())
            .await
            .map_err(crate::executor::map_turso_error)?;

        let native_transaction = native_connection
            .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
            .await
            .map_err(crate::executor::map_turso_error)?;
        {
            let mut sqlx_executor = TursoConnection::from_native_transaction(&native_transaction);
            (&mut *sqlx_executor)
                .execute("INSERT INTO test (id) VALUES (1)")
                .await?;
            let mut savepoint = sqlx_executor.begin().await?;
            (&mut *savepoint)
                .execute("INSERT INTO test (id) VALUES (2)")
                .await?;
            savepoint.rollback().await?;
        }
        native_transaction
            .commit()
            .await
            .map_err(crate::executor::map_turso_error)?;

        let mut statement = native_connection
            .prepare("SELECT COUNT(*) FROM test")
            .await
            .map_err(crate::executor::map_turso_error)?;
        let row = statement
            .query_row(())
            .await
            .map_err(crate::executor::map_turso_error)?;
        assert_eq!(
            row.get::<i64>(0)
                .map_err(crate::executor::map_turso_error)?,
            1
        );

        Ok(())
    }

    #[tokio::test]
    async fn native_operations_participate_in_sqlx_owned_transaction() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        {
            let mut transaction = connection.begin().await?;
            transaction
                .native_execute("INSERT INTO test (id) VALUES (1)", ())
                .await?;
            (&mut *transaction)
                .execute("INSERT INTO test (id) VALUES (2)")
                .await?;
            transaction.rollback().await?;
        }

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 0);

        Ok(())
    }

    #[tokio::test]
    async fn native_transaction_modes_retain_outer_ownership_during_sqlx_use()
    -> sqlx_core::Result<()> {
        let database = turso::Builder::new_local(":memory:")
            .build()
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut native = database
            .connect()
            .map_err(crate::executor::map_turso_error)?;
        native
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", ())
            .await
            .map_err(crate::executor::map_turso_error)?;

        for (id, behavior) in [
            (1_i64, TransactionBehavior::Deferred),
            (2_i64, TransactionBehavior::Immediate),
            (3_i64, TransactionBehavior::Exclusive),
        ] {
            let transaction = native
                .transaction_with_behavior(behavior)
                .await
                .map_err(crate::executor::map_turso_error)?;
            let mut sqlx = TursoConnection::from_native_transaction(&transaction);
            sqlx_core::query::query::<crate::Turso>("INSERT INTO test (id) VALUES (?)")
                .bind(id)
                .execute(&mut *sqlx)
                .await?;
            drop(sqlx);
            transaction
                .commit()
                .await
                .map_err(crate::executor::map_turso_error)?;
        }

        let mut statement = native
            .prepare("SELECT COUNT(*) FROM test")
            .await
            .map_err(crate::executor::map_turso_error)?;
        let count = statement
            .query_row(())
            .await
            .and_then(|row| row.get::<i64>(0))
            .map_err(crate::executor::map_turso_error)?;
        assert_eq!(count, 3);

        Ok(())
    }

    #[tokio::test]
    async fn native_drop_behaviors_preserve_sqlx_changes_exactly() -> sqlx_core::Result<()> {
        let database = turso::Builder::new_local(":memory:")
            .build()
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut native = database
            .connect()
            .map_err(crate::executor::map_turso_error)?;
        native
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", ())
            .await
            .map_err(crate::executor::map_turso_error)?;

        for (id, drop_behavior) in [
            (1_i64, DropBehavior::Rollback),
            (2_i64, DropBehavior::Commit),
        ] {
            let mut transaction = native
                .transaction()
                .await
                .map_err(crate::executor::map_turso_error)?;
            let mut sqlx = TursoConnection::from_native_transaction(&transaction);
            sqlx_core::query::query::<crate::Turso>("INSERT INTO test (id) VALUES (?)")
                .bind(id)
                .execute(&mut *sqlx)
                .await?;
            drop(sqlx);
            transaction.set_drop_behavior(drop_behavior);
            drop(transaction);
            native
                .execute(
                    "CREATE TABLE IF NOT EXISTS transaction_drop_probe (id INTEGER)",
                    (),
                )
                .await
                .map_err(crate::executor::map_turso_error)?;
        }

        let mut ignored = native
            .transaction()
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut sqlx = TursoConnection::from_native_transaction(&ignored);
        sqlx_core::query::query::<crate::Turso>("INSERT INTO test (id) VALUES (3)")
            .execute(&mut *sqlx)
            .await?;
        drop(sqlx);
        ignored.set_drop_behavior(DropBehavior::Ignore);
        drop(ignored);
        assert!(
            !native
                .is_autocommit()
                .map_err(crate::executor::map_turso_error)?
        );
        native
            .execute("ROLLBACK", ())
            .await
            .map_err(crate::executor::map_turso_error)?;

        let mut finished = native
            .transaction()
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut sqlx = TursoConnection::from_native_transaction(&finished);
        sqlx_core::query::query::<crate::Turso>("INSERT INTO test (id) VALUES (4)")
            .execute(&mut *sqlx)
            .await?;
        drop(sqlx);
        finished.set_drop_behavior(DropBehavior::Commit);
        finished
            .finish()
            .await
            .map_err(crate::executor::map_turso_error)?;

        let mut statement = native
            .prepare("SELECT id FROM test ORDER BY id")
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut rows = statement
            .query(())
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut ids = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(crate::executor::map_turso_error)?
        {
            ids.push(
                row.get::<i64>(0)
                    .map_err(crate::executor::map_turso_error)?,
            );
        }
        assert_eq!(ids, [2, 4]);

        Ok(())
    }

    #[tokio::test]
    #[should_panic(expected = "Transaction dropped unexpectedly")]
    async fn native_panic_drop_behavior_is_preserved_across_sqlx_use() {
        let result: sqlx_core::Result<()> = async {
            let database = turso::Builder::new_local(":memory:")
                .build()
                .await
                .map_err(crate::executor::map_turso_error)?;
            let mut native = database
                .connect()
                .map_err(crate::executor::map_turso_error)?;
            let mut transaction = native
                .transaction()
                .await
                .map_err(crate::executor::map_turso_error)?;
            let mut sqlx = TursoConnection::from_native_transaction(&transaction);
            (&mut *sqlx).execute("SELECT 1").await?;
            drop(sqlx);
            transaction.set_drop_behavior(DropBehavior::Panic);
            drop(transaction);
            native
                .execute(
                    "CREATE TABLE IF NOT EXISTS transaction_drop_probe (id INTEGER)",
                    (),
                )
                .await
                .map_err(crate::executor::map_turso_error)?;
            Ok(())
        }
        .await;
        assert!(
            result.is_ok(),
            "native transaction setup failed: {result:?}"
        );
    }

    #[tokio::test]
    async fn detects_native_control_that_ends_sqlx_transaction() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        let mut transaction = connection.begin().await?;
        transaction.native_execute("COMMIT", ()).await?;

        let error = match (&mut *transaction).execute("SELECT 1").await {
            Err(error) => error,
            Ok(_) => {
                return Err(sqlx_core::Error::Protocol(
                    "SQLx reused a transaction ended by native code".to_owned(),
                ));
            }
        };
        assert!(
            error
                .to_string()
                .contains("native transaction control ended an active SQLx transaction")
        );

        Ok(())
    }

    #[tokio::test]
    async fn supports_begin_concurrent_transaction() -> sqlx_core::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "sqlx-turso-begin-concurrent-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let mut connection = TursoConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .mvcc(true)
            .connect()
            .await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        let mut transaction = connection.begin_with("BEGIN CONCURRENT").await?;
        (&mut *transaction)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;
        transaction.commit().await?;

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        let mut native = connection.into_native_connection().await?;
        let transaction = native
            .transaction_with_behavior(TransactionBehavior::Concurrent)
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut sqlx = TursoConnection::from_native_transaction(&transaction);
        (&mut sqlx)
            .execute("INSERT INTO test (id) VALUES (2)")
            .await?;
        drop(sqlx);
        transaction
            .commit()
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut statement = native
            .prepare("SELECT COUNT(*) FROM test")
            .await
            .map_err(crate::executor::map_turso_error)?;
        let count = statement
            .query_row(())
            .await
            .and_then(|row| row.get::<i64>(0))
            .map_err(crate::executor::map_turso_error)?;
        assert_eq!(count, 2);

        drop(native);
        let _ = std::fs::remove_file(&path);

        Ok(())
    }

    #[tokio::test]
    async fn begin_concurrent_conflict_rolls_back_and_keeps_connection_usable()
    -> sqlx_core::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "sqlx-turso-begin-concurrent-conflict-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let options = TursoConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .mvcc(true);
        let mut first = options.clone().connect().await?;
        let mut second = options.connect().await?;

        (&mut first)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        let mut first_transaction = first.begin_with("BEGIN CONCURRENT").await?;
        (&mut *first_transaction)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        let mut second_transaction = second.begin_with("BEGIN CONCURRENT").await?;
        (&mut *second_transaction)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        first_transaction.commit().await?;
        if second_transaction.commit().await.is_ok() {
            return Err(sqlx_core::Error::Protocol(
                "conflicting concurrent transaction committed".to_owned(),
            ));
        }

        let row = (&mut second)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        drop(first);
        drop(second);
        let _ = std::fs::remove_file(&path);

        Ok(())
    }

    #[tokio::test]
    async fn rollback_cleanup_failure_marks_connection_unusable() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;

        {
            let mut transaction = connection.begin().await?;
            let mut nested = transaction.begin().await?;
            (&mut *nested)
                .execute("INSERT INTO test (id) VALUES (1)")
                .await?;
        }

        connection
            .raw()
            .execute("ROLLBACK", ())
            .await
            .map_err(sqlx_core::error::Error::config)?;

        let error = match (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await
        {
            Err(error) => error,
            Ok(_) => {
                return Err(sqlx_core::Error::Protocol(
                    "invalid scheduled rollback succeeded".to_owned(),
                ));
            }
        };
        assert!(matches!(error, sqlx_core::error::Error::Database(_)));

        let error = match (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await
        {
            Err(error) => error,
            Ok(_) => {
                return Err(sqlx_core::Error::Protocol(
                    "connection remained reusable after cleanup failure".to_owned(),
                ));
            }
        };
        assert!(matches!(error, sqlx_core::error::Error::WorkerCrashed));

        Ok(())
    }
}
