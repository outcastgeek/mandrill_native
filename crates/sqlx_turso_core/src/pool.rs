use sqlx_core::{executor::Executor, pool::Pool};

/// SQLx-compatible pool options for Turso connections
pub type TursoPoolOptions = sqlx_core::pool::PoolOptions<crate::Turso>;

/// SQLx-compatible pool handle for Turso connections
pub type TursoPool = Pool<crate::Turso>;

/// Executor facade for Turso SQLx operations
pub trait TursoExecutor<'c>: Executor<'c, Database = crate::Turso> {}
impl<'c, T> TursoExecutor<'c> for T where T: Executor<'c, Database = crate::Turso> {}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use sqlx_core::{connection::Connection, error::Error, executor::Executor, row::Row};

    use crate::{TursoConnectOptions, TursoEncryptionOptions, TursoPoolOptions};

    #[tokio::test]
    async fn pool_acquires_and_releases_file_connections() -> sqlx_core::Result<()> {
        let path = temp_database_path("pool-file");
        remove_database_sidecars(&path);

        let pool = TursoPoolOptions::new()
            .max_connections(2)
            .connect_with(
                TursoConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .busy_timeout(std::time::Duration::from_millis(50)),
            )
            .await?;

        let mut connection = pool.acquire().await?;
        (&mut *connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;
        drop(connection);

        let mut connection = pool.acquire().await?;
        (&mut *connection)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;
        let row = (&mut *connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);
        drop(connection);

        pool.close().await;
        remove_database_sidecars(&path);

        Ok(())
    }

    #[tokio::test]
    async fn pool_shares_in_memory_state() -> sqlx_core::Result<()> {
        let pool = TursoPoolOptions::new()
            .max_connections(2)
            .connect_with(TursoConnectOptions::new())
            .await?;

        let mut first = pool.acquire().await?;
        let mut second = pool.acquire().await?;

        (&mut *first)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;
        (&mut *first)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        let row = (&mut *second)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);
        drop(first);
        drop(second);

        pool.close().await;

        Ok(())
    }

    #[tokio::test]
    async fn dropped_transaction_rolls_back_before_pool_reuse() -> sqlx_core::Result<()> {
        let pool = TursoPoolOptions::new()
            .max_connections(1)
            .connect_with(TursoConnectOptions::new())
            .await?;

        {
            let mut connection = pool.acquire().await?;
            (&mut *connection)
                .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
                .await?;
        }

        {
            let mut transaction = pool.begin().await?;
            (&mut *transaction)
                .execute("INSERT INTO test (id) VALUES (1)")
                .await?;
        }

        let mut connection = pool.acquire().await?;
        let row = (&mut *connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 0);
        drop(connection);

        pool.close().await;

        Ok(())
    }

    #[tokio::test]
    async fn pool_supports_concurrent_readers() -> sqlx_core::Result<()> {
        let path = temp_database_path("pool-readers");
        remove_database_sidecars(&path);

        let pool = TursoPoolOptions::new()
            .max_connections(4)
            .connect_with(
                TursoConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await?;

        let mut connection = pool.acquire().await?;
        (&mut *connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;
        (&mut *connection)
            .execute("INSERT INTO test (id) VALUES (1), (2), (3)")
            .await?;
        drop(connection);

        let first_pool = pool.clone();
        let second_pool = pool.clone();
        let first = tokio::spawn(async move { count_rows(first_pool).await });
        let second = tokio::spawn(async move { count_rows(second_pool).await });

        assert_eq!(
            first
                .await
                .map_err(|error| Error::Protocol(format!("reader task failed: {error}")))??,
            3
        );
        assert_eq!(
            second
                .await
                .map_err(|error| Error::Protocol(format!("reader task failed: {error}")))??,
            3
        );

        pool.close().await;
        remove_database_sidecars(&path);

        Ok(())
    }

    #[tokio::test]
    async fn eager_and_lazy_pools_run_all_lifecycle_hooks() -> sqlx_core::Result<()> {
        let after_connects = Arc::new(AtomicUsize::new(0));
        let before_acquires = Arc::new(AtomicUsize::new(0));
        let after_releases = Arc::new(AtomicUsize::new(0));

        let pool = TursoPoolOptions::new()
            .min_connections(2)
            .max_connections(2)
            .after_connect({
                let calls = after_connects.clone();
                move |_connection, _metadata| {
                    let calls = calls.clone();
                    Box::pin(async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                }
            })
            .before_acquire({
                let calls = before_acquires.clone();
                move |_connection, _metadata| {
                    let calls = calls.clone();
                    Box::pin(async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(true)
                    })
                }
            })
            .after_release({
                let calls = after_releases.clone();
                move |_connection, _metadata| {
                    let calls = calls.clone();
                    Box::pin(async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(true)
                    })
                }
            })
            .connect_with(TursoConnectOptions::new())
            .await?;

        assert_eq!(pool.size(), 2);
        assert_eq!(after_connects.load(Ordering::SeqCst), 2);

        let mut connection = pool.acquire().await?;
        connection.return_to_pool().await;
        assert_eq!(after_releases.load(Ordering::SeqCst), 1);

        let mut connection = pool.acquire().await?;
        connection.return_to_pool().await;
        assert!(before_acquires.load(Ordering::SeqCst) >= 2);
        assert_eq!(after_releases.load(Ordering::SeqCst), 2);
        pool.close().await;

        let lazy_connects = Arc::new(AtomicUsize::new(0));
        let lazy_pool = TursoPoolOptions::new()
            .max_connections(1)
            .after_connect({
                let calls = lazy_connects.clone();
                move |_connection, _metadata| {
                    let calls = calls.clone();
                    Box::pin(async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                }
            })
            .connect_lazy_with(TursoConnectOptions::new());

        assert_eq!(lazy_pool.size(), 0);
        assert_eq!(lazy_connects.load(Ordering::SeqCst), 0);
        let connection = lazy_pool.acquire().await?;
        assert_eq!(lazy_connects.load(Ordering::SeqCst), 1);
        drop(connection);
        lazy_pool.close().await;

        Ok(())
    }

    #[tokio::test]
    async fn closing_pool_wakes_waiting_acquires() -> sqlx_core::Result<()> {
        let pool = TursoPoolOptions::new()
            .max_connections(1)
            .connect_with(TursoConnectOptions::new())
            .await?;
        let held = pool.acquire().await?;
        let waiting_pool = pool.clone();
        let waiter = tokio::spawn(async move { waiting_pool.acquire().await });

        tokio::task::yield_now().await;
        let closing_pool = pool.clone();
        let close = tokio::spawn(async move { closing_pool.close().await });

        let error = waiter
            .await
            .map_err(|error| Error::Protocol(format!("waiter task failed: {error}")))?
            .err()
            .ok_or_else(|| Error::Protocol("waiting acquire succeeded during close".to_owned()))?;
        assert!(matches!(error, Error::PoolClosed));

        drop(held);
        close
            .await
            .map_err(|error| Error::Protocol(format!("close task failed: {error}")))?;
        Ok(())
    }

    #[tokio::test]
    async fn detached_and_leaked_connections_follow_sqlx_pool_capacity_rules()
    -> sqlx_core::Result<()> {
        let pool = TursoPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(25))
            .connect_with(TursoConnectOptions::new())
            .await?;

        let mut detached = pool.acquire().await?.detach();
        assert_eq!(pool.size(), 0);
        (&mut detached).execute("SELECT 1").await?;

        let replacement = pool.acquire().await?;
        assert_eq!(pool.size(), 1);
        drop(replacement);
        detached.close().await?;
        pool.close().await;

        let leaked_pool = TursoPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(25))
            .connect_with(TursoConnectOptions::new())
            .await?;
        let leaked = leaked_pool.acquire().await?.leak();
        let error = leaked_pool.acquire().await.err().ok_or_else(|| {
            Error::Protocol("acquire succeeded after leaking all pool capacity".to_owned())
        })?;
        assert!(matches!(error, Error::PoolTimedOut));
        leaked.close().await?;

        Ok(())
    }

    #[tokio::test]
    async fn detached_connection_transfers_to_native_without_pool_reuse() -> sqlx_core::Result<()> {
        let pool = TursoPoolOptions::new()
            .max_connections(1)
            .connect_with(TursoConnectOptions::new())
            .await?;
        (&pool)
            .execute("CREATE TABLE handoff (value INTEGER NOT NULL)")
            .await?;

        let detached = pool.acquire().await?.detach();
        let native = detached.into_native_connection().await?;
        native
            .execute("INSERT INTO handoff (value) VALUES (41)", ())
            .await
            .map_err(crate::executor::map_turso_error)?;

        let mut replacement = pool.acquire().await?;
        let row = (&mut *replacement)
            .fetch_one("SELECT value FROM handoff")
            .await?;
        assert_eq!(row.try_get::<i64, _>(0)?, 41);
        drop(replacement);

        let mut readopted = crate::TursoConnection::from_native_connection(native)?;
        (&mut readopted)
            .execute("UPDATE handoff SET value = 42")
            .await?;
        let row = (&pool).fetch_one("SELECT value FROM handoff").await?;
        assert_eq!(row.try_get::<i64, _>(0)?, 42);

        readopted.close().await?;
        pool.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn pool_surfaces_native_write_contention() -> sqlx_core::Result<()> {
        let path = temp_database_path("pool-contention");
        remove_database_sidecars(&path);

        let pool = TursoPoolOptions::new()
            .max_connections(2)
            .connect_with(
                TursoConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await?;

        let mut first = pool.acquire().await?;
        let mut second = pool.acquire().await?;

        (&mut *first)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;
        (&mut *first).execute("BEGIN IMMEDIATE").await?;
        (&mut *first)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        let error = match (&mut *second)
            .execute("INSERT INTO test (id) VALUES (2)")
            .await
        {
            Err(error) => error,
            Ok(_) => return Err(Error::Protocol("contending write succeeded".to_owned())),
        };
        assert!(
            error.to_string().contains("database is locked")
                || error.to_string().contains("busy")
                || error.to_string().contains("locked")
        );

        (&mut *first).execute("ROLLBACK").await?;
        drop(first);
        drop(second);

        pool.close().await;
        remove_database_sidecars(&path);

        Ok(())
    }

    #[tokio::test]
    async fn pool_supports_encrypted_files() -> sqlx_core::Result<()> {
        let path = temp_database_path("pool-encrypted");
        remove_database_sidecars(&path);

        let pool = TursoPoolOptions::new()
            .max_connections(2)
            .connect_with(
                TursoConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .encryption_options(TursoEncryptionOptions::new(
                        "aegis256",
                        "b1bbfda4f589dc9daaf004fe21111e00dc00c98237102f5c7002a5669fc76327",
                    )?),
            )
            .await?;

        let mut connection = pool.acquire().await?;
        (&mut *connection)
            .execute("CREATE TABLE test (value TEXT)")
            .await?;
        (&mut *connection)
            .execute("INSERT INTO test (value) VALUES ('secret')")
            .await?;
        let row = (&mut *connection)
            .fetch_one("SELECT value FROM test")
            .await?;
        assert_eq!(row.try_get::<String, _>("value")?, "secret");
        drop(connection);

        pool.close().await;
        remove_database_sidecars(&path);

        Ok(())
    }

    #[tokio::test]
    async fn pool_supports_mvcc_connections() -> sqlx_core::Result<()> {
        let pool = TursoPoolOptions::new()
            .max_connections(2)
            .connect_with(TursoConnectOptions::new().mvcc(true))
            .await?;

        let mut connection = pool.acquire().await?;
        let row = (&mut *connection).fetch_one("PRAGMA journal_mode").await?;
        assert_eq!(row.try_get::<String, _>(0)?.to_ascii_lowercase(), "mvcc");
        drop(connection);

        pool.close().await;

        Ok(())
    }

    #[tokio::test]
    async fn pool_uses_preconfigured_native_database() -> sqlx_core::Result<()> {
        let database = turso::Builder::new_local(":memory:")
            .experimental_generated_columns(true)
            .build()
            .await
            .map_err(crate::executor::map_turso_error)?;
        let native = database
            .connect()
            .map_err(crate::executor::map_turso_error)?;
        native
            .execute(
                "CREATE TABLE test (value INTEGER, doubled INTEGER GENERATED ALWAYS AS (value * 2) VIRTUAL)",
                (),
            )
            .await
            .map_err(crate::executor::map_turso_error)?;

        let pool = TursoPoolOptions::new()
            .max_connections(2)
            .connect_with(TursoConnectOptions::from_native_database(database))
            .await?;

        (&pool)
            .execute("INSERT INTO test (value) VALUES (21)")
            .await?;
        let row = (&pool).fetch_one("SELECT doubled FROM test").await?;
        assert_eq!(row.try_get::<i64, _>(0)?, 42);

        let mut connection = pool.acquire().await?;
        connection
            .native_execute("INSERT INTO test (value) VALUES (7)", ())
            .await?;
        let row = (&mut *connection)
            .fetch_one("SELECT SUM(doubled) FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>(0)?, 56);
        drop(connection);

        pool.close().await;
        Ok(())
    }

    async fn count_rows(pool: crate::TursoPool) -> sqlx_core::Result<i64> {
        let mut connection = pool.acquire().await?;
        let row = (&mut *connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        row.try_get::<i64, _>("count")
    }

    fn temp_database_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{name}-{}.db", std::process::id()))
    }

    fn remove_database_sidecars(path: &std::path::Path) {
        let path = path.to_string_lossy();
        for suffix in ["", "-wal", "-shm", "-wal-tshm"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
    }
}
