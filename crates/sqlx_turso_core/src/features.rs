#[cfg(test)]
mod tests {
    use sqlx_core::{
        connection::ConnectOptions, executor::Executor, query::query, row::Row,
        sql_str::AssertSqlSafe,
    };

    #[cfg(feature = "sync")]
    use crate::TursoSyncOptions;
    use crate::{Turso, TursoConnectOptions, TursoEncryptionOptions, TursoExperimentalFeature};

    #[tokio::test]
    async fn supports_generated_columns_when_enabled() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new()
            .experimental_feature(TursoExperimentalFeature::GeneratedColumns, true)
            .connect()
            .await?;

        (&mut connection)
            .execute(
                "CREATE TABLE test (\
                 value INTEGER, \
                 doubled INTEGER GENERATED ALWAYS AS (value * 2) VIRTUAL\
                 )",
            )
            .await?;
        (&mut connection)
            .execute("INSERT INTO test (value) VALUES (21)")
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT doubled FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("doubled")?, 42);

        Ok(())
    }

    #[tokio::test]
    async fn supports_materialized_views_when_enabled() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new()
            .experimental_feature(TursoExperimentalFeature::MaterializedViews, true)
            .connect()
            .await?;

        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)")
            .await?;
        (&mut connection)
            .execute("INSERT INTO test (id, value) VALUES (1, 'alpha')")
            .await?;
        (&mut connection)
            .execute("CREATE MATERIALIZED VIEW test_view AS SELECT id, value FROM test")
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT value FROM test_view WHERE id = 1")
            .await?;
        assert_eq!(row.try_get::<String, _>("value")?, "alpha");

        Ok(())
    }

    #[tokio::test]
    async fn supports_attach_when_enabled() -> sqlx_core::Result<()> {
        let path =
            std::env::temp_dir().join(format!("sqlx-turso-attached-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut connection = TursoConnectOptions::new()
            .experimental_feature(TursoExperimentalFeature::Attach, true)
            .connect()
            .await?;

        let attach_sql = format!("ATTACH DATABASE '{}' AS attached", path.display());
        query::<Turso>(AssertSqlSafe(attach_sql))
            .execute(&mut connection)
            .await?;
        (&mut connection)
            .execute("CREATE TABLE attached.test (id INTEGER PRIMARY KEY)")
            .await?;
        (&mut connection)
            .execute("INSERT INTO attached.test (id) VALUES (1)")
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM attached.test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        let _ = (&mut connection).execute("DETACH DATABASE attached").await;
        drop(connection);
        let _ = std::fs::remove_file(&path);

        Ok(())
    }

    #[tokio::test]
    async fn supports_custom_types_when_enabled() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new()
            .experimental_feature(TursoExperimentalFeature::CustomTypes, true)
            .connect()
            .await?;

        (&mut connection)
            .execute("CREATE TYPE user_id BASE integer")
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT name FROM sqlite_turso_types WHERE name = 'user_id'")
            .await?;
        assert_eq!(row.try_get::<String, _>("name")?, "user_id");

        Ok(())
    }

    #[tokio::test]
    async fn supports_domains_when_custom_types_enabled() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new()
            .experimental_feature(TursoExperimentalFeature::CustomTypes, true)
            .connect()
            .await?;

        (&mut connection)
            .execute("CREATE DOMAIN positive_int AS integer CHECK (VALUE > 0)")
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT name FROM sqlite_turso_types WHERE name = 'positive_int'")
            .await?;
        assert_eq!(row.try_get::<String, _>("name")?, "positive_int");

        Ok(())
    }

    #[tokio::test]
    async fn supports_without_rowid_when_enabled() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new()
            .experimental_feature(TursoExperimentalFeature::WithoutRowid, true)
            .connect()
            .await?;

        (&mut connection)
            .execute("CREATE TABLE test (code TEXT PRIMARY KEY, value TEXT) WITHOUT ROWID")
            .await?;
        (&mut connection)
            .execute("INSERT INTO test (code, value) VALUES ('a', 'alpha')")
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT value FROM test WHERE code = 'a'")
            .await?;
        assert_eq!(row.try_get::<String, _>("value")?, "alpha");

        Ok(())
    }

    #[tokio::test]
    async fn supports_vacuum_when_enabled() -> sqlx_core::Result<()> {
        let path =
            std::env::temp_dir().join(format!("sqlx-turso-vacuum-{}.db", std::process::id()));
        remove_database_sidecars(&path);

        let mut connection = TursoConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .experimental_feature(TursoExperimentalFeature::Vacuum, true)
            .connect()
            .await?;

        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;
        (&mut connection).execute("VACUUM").await?;

        drop(connection);
        remove_database_sidecars(&path);

        Ok(())
    }

    #[tokio::test]
    async fn rejects_autovacuum_without_native_builder_support() -> sqlx_core::Result<()> {
        let path =
            std::env::temp_dir().join(format!("sqlx-turso-autovacuum-{}.db", std::process::id()));
        remove_database_sidecars(&path);

        let error = match TursoConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .pragma("auto_vacuum", Some("FULL".to_owned()))
            .experimental_feature(TursoExperimentalFeature::Vacuum, true)
            .connect()
            .await
        {
            Err(error) => error,
            Ok(_) => {
                return Err(sqlx_core::Error::Protocol(
                    "autovacuum unexpectedly succeeded".to_owned(),
                ));
            }
        };
        assert!(error.to_string().contains("Autovacuum is not enabled"));

        remove_database_sidecars(&path);
        Ok(())
    }

    #[tokio::test]
    async fn supports_named_vfs_when_configured() -> sqlx_core::Result<()> {
        let path = std::env::temp_dir().join(format!("sqlx-turso-vfs-{}.db", std::process::id()));
        remove_database_sidecars(&path);

        let mut connection = TursoConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .vfs("memory")
            .connect()
            .await?;

        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;
        (&mut connection)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        drop(connection);
        remove_database_sidecars(&path);

        Ok(())
    }

    #[tokio::test]
    async fn supports_custom_io_when_configured() -> sqlx_core::Result<()> {
        let path =
            std::env::temp_dir().join(format!("sqlx-turso-custom-io-{}.db", std::process::id()));
        remove_database_sidecars(&path);

        let mut connection = TursoConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .custom_io(std::sync::Arc::new(turso::core::MemoryIO::new()))
            .connect()
            .await?;

        (&mut connection)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;
        (&mut connection)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        drop(connection);
        remove_database_sidecars(&path);

        Ok(())
    }

    #[tokio::test]
    async fn encrypted_sqlx_database_reopens_natively_and_rejects_wrong_keys()
    -> sqlx_core::Result<()> {
        const KEY: &str = "b1bbfda4f589dc9daaf004fe21111e00dc00c98237102f5c7002a5669fc76327";
        const WRONG_KEY: &str = "c2ccfda4f589dc9daaf004fe21111e00dc00c98237102f5c7002a5669fc76327";

        let path = std::env::temp_dir().join(format!(
            "sqlx-turso-native-encryption-{}.db",
            std::process::id()
        ));
        remove_database_sidecars(&path);

        let mut connection = TursoConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .encryption_options(TursoEncryptionOptions::new("aegis256", KEY)?)
            .connect()
            .await?;
        (&mut connection)
            .execute("CREATE TABLE encrypted_data (value TEXT NOT NULL)")
            .await?;
        (&mut connection)
            .execute("INSERT INTO encrypted_data (value) VALUES ('from-sqlx')")
            .await?;
        drop(connection);

        let native_options = turso::EncryptionOpts {
            cipher: "aegis256".to_owned(),
            hexkey: KEY.to_owned(),
        };
        let native_database = turso::Builder::new_local(&path.to_string_lossy())
            .experimental_encryption(true)
            .with_encryption(native_options)
            .build()
            .await
            .map_err(crate::executor::map_turso_error)?;
        let native = native_database
            .connect()
            .map_err(crate::executor::map_turso_error)?;
        let row = native
            .query("SELECT value FROM encrypted_data", ())
            .await
            .map_err(crate::executor::map_turso_error)?
            .next()
            .await
            .map_err(crate::executor::map_turso_error)?
            .ok_or(sqlx_core::Error::RowNotFound)?;
        assert_eq!(
            row.get::<String>(0)
                .map_err(crate::executor::map_turso_error)?,
            "from-sqlx"
        );
        drop(native);
        drop(native_database);

        let wrong_database = turso::Builder::new_local(&path.to_string_lossy())
            .experimental_encryption(true)
            .with_encryption(turso::EncryptionOpts {
                cipher: "aegis256".to_owned(),
                hexkey: WRONG_KEY.to_owned(),
            })
            .build()
            .await;
        let wrong_key_failed = match wrong_database {
            Err(_) => true,
            Ok(database) => match database.connect() {
                Err(_) => true,
                Ok(connection) => connection
                    .query("SELECT * FROM encrypted_data", ())
                    .await
                    .is_err(),
            },
        };
        assert!(wrong_key_failed);

        let plain_database = turso::Builder::new_local(&path.to_string_lossy())
            .build()
            .await;
        let missing_key_failed = match plain_database {
            Err(_) => true,
            Ok(database) => match database.connect() {
                Err(_) => true,
                Ok(connection) => connection
                    .query("SELECT * FROM encrypted_data", ())
                    .await
                    .is_err(),
            },
        };
        assert!(missing_key_failed);

        remove_database_sidecars(&path);
        Ok(())
    }

    #[tokio::test]
    async fn supports_multiprocess_wal_when_enabled() -> sqlx_core::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "sqlx-turso-multiprocess-wal-{}.db",
            std::process::id()
        ));
        remove_database_sidecars(&path);

        let options = TursoConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .experimental_feature(TursoExperimentalFeature::MultiprocessWal, true);
        let mut first = options.clone().connect().await?;
        let mut second = options.connect().await?;

        (&mut first)
            .execute("CREATE TABLE test (id INTEGER PRIMARY KEY)")
            .await?;
        (&mut first)
            .execute("INSERT INTO test (id) VALUES (1)")
            .await?;

        let row = (&mut second)
            .fetch_one("SELECT COUNT(*) AS count FROM test")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);

        drop(first);
        drop(second);
        remove_database_sidecars(&path);

        Ok(())
    }

    #[cfg(feature = "sync")]
    #[tokio::test]
    async fn rejects_sync_custom_io_without_native_hook() -> sqlx_core::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "sqlx-turso-sync-custom-io-{}.db",
            std::process::id()
        ));
        remove_database_sidecars(&path);

        let error = match TursoConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .custom_io(std::sync::Arc::new(turso::core::MemoryIO::new()))
            .with_sync_options(
                TursoSyncOptions::new("http://127.0.0.1:9").with_bootstrap_if_empty(false),
            )
            .connect()
            .await
        {
            Err(error) => error,
            Ok(_) => {
                return Err(sqlx_core::Error::Protocol(
                    "sync custom IO unexpectedly succeeded".to_owned(),
                ));
            }
        };
        assert!(
            error
                .to_string()
                .contains("custom IO Turso sync connections")
        );

        remove_database_sidecars(&path);
        Ok(())
    }

    #[tokio::test]
    async fn supports_vector_functions_through_blob_values() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let row = (&mut connection)
            .fetch_one("SELECT vector_extract(vector32('[1.0, 2.0]')) AS vector")
            .await?;
        let value = row.try_get::<String, _>("vector")?;
        assert!(value.contains("1"));
        assert!(value.contains("2"));

        Ok(())
    }

    #[tokio::test]
    async fn triggers_and_strict_tables_work_through_sqlx() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;

        (&mut connection)
            .execute(
                "CREATE TABLE source (id INTEGER PRIMARY KEY, value INTEGER) STRICT; \
                 CREATE TABLE audit (source_id INTEGER, value INTEGER) STRICT; \
                 CREATE TRIGGER source_audit AFTER INSERT ON source BEGIN \
                   INSERT INTO audit (source_id, value) VALUES (NEW.id, NEW.value); \
                   UPDATE audit SET value = value + 1 WHERE source_id = NEW.id; \
                 END; \
                 INSERT INTO source (id, value) VALUES (1, 41)",
            )
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT value FROM audit WHERE source_id = 1")
            .await?;
        assert_eq!(row.try_get::<i64, _>("value")?, 42);

        let strict_error = (&mut connection)
            .execute("INSERT INTO source (id, value) VALUES (2, 'not-an-integer')")
            .await;
        assert!(strict_error.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn native_batches_share_sqlx_state_and_preserve_failure_details() -> sqlx_core::Result<()>
    {
        let mut connection = TursoConnectOptions::new().connect().await?;

        let results = connection
            .native_batch([
                "CREATE TABLE batch_data (id INTEGER PRIMARY KEY, value TEXT)",
                "INSERT INTO batch_data (id, value) VALUES (1, 'native')",
                "SELECT id, value FROM batch_data",
            ])
            .await?;
        assert_eq!(results.len(), 3);
        assert_eq!(
            results.get(1).map(turso::BatchResult::rows_affected),
            Some(1)
        );
        assert_eq!(
            results
                .get(1)
                .and_then(turso::BatchResult::last_insert_rowid),
            Some(1)
        );
        assert_eq!(results.get(1).and_then(turso::BatchResult::rows_read), None);
        assert_eq!(
            results.get(1).and_then(turso::BatchResult::rows_written),
            None
        );
        assert_eq!(
            results
                .get(1)
                .and_then(turso::BatchResult::query_duration_ms),
            None
        );
        assert_eq!(results.get(2).map(|result| result.rows().len()), Some(1));
        assert_eq!(results.get(2).map(|result| result.columns().len()), Some(2));
        let selected = results
            .get(2)
            .and_then(|result| result.rows().first())
            .ok_or(sqlx_core::Error::RowNotFound)?;
        assert_eq!(
            selected
                .get::<String>(1)
                .map_err(crate::executor::map_turso_error)?,
            "native"
        );

        let row = (&mut connection)
            .fetch_one("SELECT value FROM batch_data WHERE id = 1")
            .await?;
        assert_eq!(row.try_get::<String, _>("value")?, "native");

        let error = connection
            .native_transactional_batch(
                [
                    "INSERT INTO batch_data (id, value) VALUES (2, 'rolled-back')",
                    "INSERT INTO missing_table (id) VALUES (1)",
                    "INSERT INTO batch_data (id, value) VALUES (3, 'not-run')",
                ],
                turso::transaction::TransactionBehavior::Immediate,
            )
            .await;
        match error {
            Err(sqlx_core::Error::Database(database_error)) => {
                let native_error = database_error
                    .downcast_ref::<crate::TursoDatabaseError>()
                    .native_error();
                let turso::Error::BatchStatementFailed { index, results, .. } = native_error else {
                    return Err(sqlx_core::Error::Protocol(format!(
                        "unexpected native batch database error: {database_error}"
                    )));
                };
                assert_eq!(*index, 1);
                assert_eq!(results.len(), 3);
                assert!(results.first().is_some_and(Option::is_some));
                assert!(results.get(1).is_some_and(Option::is_none));
                assert!(results.get(2).is_some_and(Option::is_none));
            }
            Err(error) => {
                return Err(sqlx_core::Error::Protocol(format!(
                    "unexpected native batch error: {error}"
                )));
            }
            Ok(_) => {
                return Err(sqlx_core::Error::Protocol(
                    "failing transactional batch succeeded".to_owned(),
                ));
            }
        }

        let row = (&mut connection)
            .fetch_one("SELECT COUNT(*) AS count FROM batch_data")
            .await?;
        assert_eq!(row.try_get::<i64, _>("count")?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn native_statements_pragmas_and_cache_share_sqlx_state() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new().connect().await?;
        (&mut connection)
            .execute("CREATE TABLE native_api (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
            .await?;

        let native = connection.into_native_connection().await?;
        native
            .busy_timeout(std::time::Duration::from_millis(25))
            .map_err(crate::executor::map_turso_error)?;
        let mut insert = native
            .prepare_cached("INSERT INTO native_api (id, value) VALUES (?1, ?2)")
            .await
            .map_err(crate::executor::map_turso_error)?;
        assert_eq!(
            insert
                .execute((1_i64, "native-one"))
                .await
                .map_err(crate::executor::map_turso_error)?,
            1
        );
        assert_eq!(insert.n_change(), 1);
        insert.reset().map_err(crate::executor::map_turso_error)?;
        assert_eq!(
            insert
                .execute((2_i64, "native-two"))
                .await
                .map_err(crate::executor::map_turso_error)?,
            1
        );
        assert_eq!(native.last_insert_rowid(), 2);
        native
            .cacheflush()
            .map_err(crate::executor::map_turso_error)?;
        assert!(
            native
                .is_autocommit()
                .map_err(crate::executor::map_turso_error)?
        );

        let mut select = native
            .prepare("SELECT id, value FROM native_api WHERE id = ?1")
            .await
            .map_err(crate::executor::map_turso_error)?;
        assert_eq!(select.column_count(), 2);
        assert_eq!(
            select
                .column_name(1)
                .map_err(crate::executor::map_turso_error)?,
            "value"
        );
        assert_eq!(select.column_names(), ["id", "value"]);
        assert_eq!(
            select
                .column_index("VALUE")
                .map_err(crate::executor::map_turso_error)?,
            1
        );
        let columns = select.columns();
        assert_eq!(columns.first().map(turso::Column::name), Some("id"));

        let mut rows = select
            .query((1_i64,))
            .await
            .map_err(crate::executor::map_turso_error)?;
        assert_eq!(rows.column_count(), 2);
        assert_eq!(rows.column_names(), ["id", "value"]);
        assert_eq!(
            rows.column_index("value")
                .map_err(crate::executor::map_turso_error)?,
            1
        );
        let row = rows
            .next()
            .await
            .map_err(crate::executor::map_turso_error)?
            .ok_or_else(|| sqlx_core::Error::RowNotFound)?;
        assert_eq!(row.column_count(), 2);
        assert_eq!(
            row.get::<i64>(0)
                .map_err(crate::executor::map_turso_error)?,
            1
        );
        assert_eq!(
            row.get_value(1).map_err(crate::executor::map_turso_error)?,
            turso::Value::Text("native-one".to_owned())
        );

        native
            .pragma_update("user_version", 7)
            .await
            .map_err(crate::executor::map_turso_error)?;
        let mut user_version = None;
        native
            .pragma_query("user_version", |row| {
                user_version = row.get::<i64>(0).ok();
                Ok(())
            })
            .await
            .map_err(crate::executor::map_turso_error)?;
        assert_eq!(user_version, Some(7));

        drop(rows);
        drop(select);
        drop(insert);
        let mut connection = crate::TursoConnection::from_native_connection(native)?;

        (&mut connection)
            .execute("INSERT INTO native_api (id, value) VALUES (3, 'sqlx-three')")
            .await?;
        let native = connection.into_native_connection().await?;
        assert_eq!(native.last_insert_rowid(), 3);
        let row = native
            .query("SELECT value FROM native_api WHERE id = ?1", (3_i64,))
            .await
            .map_err(crate::executor::map_turso_error)?
            .next()
            .await
            .map_err(crate::executor::map_turso_error)?
            .ok_or(sqlx_core::Error::RowNotFound)?;
        assert_eq!(
            row.get::<String>(0)
                .map_err(crate::executor::map_turso_error)?,
            "sqlx-three"
        );

        Ok(())
    }

    #[cfg(feature = "fts")]
    #[tokio::test]
    async fn supports_fts_index_methods_when_feature_enabled() -> sqlx_core::Result<()> {
        let mut connection = TursoConnectOptions::new()
            .experimental_feature(TursoExperimentalFeature::IndexMethod, true)
            .connect()
            .await?;

        (&mut connection)
            .execute("CREATE TABLE users (email TEXT, age INTEGER)")
            .await?;
        (&mut connection)
            .execute("CREATE INDEX fts_users_email ON users USING fts (email)")
            .await?;
        (&mut connection)
            .execute("INSERT INTO users (email, age) VALUES ('foo@example.com', 21)")
            .await?;

        let row = (&mut connection)
            .fetch_one("SELECT age FROM users WHERE email MATCH 'foo@example.com'")
            .await?;
        assert_eq!(row.try_get::<i64, _>("age")?, 21);

        Ok(())
    }

    fn remove_database_sidecars(path: &std::path::Path) {
        let path = path.to_string_lossy();
        for suffix in ["", "-wal", "-shm", "-wal-tshm"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
    }
}
