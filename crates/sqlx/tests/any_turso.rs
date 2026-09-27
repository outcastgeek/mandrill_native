use std::{path::Path, process, str::FromStr};

use sqlx::{ConnectOptions, Connection, Executor, Row, any::AnyConnectOptions};

#[tokio::test]
async fn ordinary_sqlx_any_dispatch_includes_turso() -> sqlx::Result<()> {
    let path = std::env::temp_dir().join(format!("sqlx-turso-facade-any-{}.db", process::id()));
    remove_database_files(&path);
    sqlx::any::install_default_drivers();

    let url = format!("turso://{}?mode=rwc", path.display());
    let mut connection = AnyConnectOptions::from_str(&url)?.connect().await?;
    connection
        .execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
        .await?;
    connection
        .execute("INSERT INTO test (id, value) VALUES (1, 'turso')")
        .await?;

    let row = connection
        .fetch_one("SELECT value FROM test WHERE id = 1")
        .await?;
    assert_eq!(row.try_get::<String, _>("value")?, "turso");

    connection.close().await?;
    remove_database_files(&path);
    Ok(())
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ordinary_sqlx_any_dispatch_keeps_sqlite() -> sqlx::Result<()> {
    sqlx::any::install_default_drivers();

    let mut connection = AnyConnectOptions::from_str("sqlite::memory:")?
        .connect()
        .await?;
    let row = connection.fetch_one("SELECT 42 AS value").await?;
    assert_eq!(row.try_get::<i64, _>("value")?, 42);
    connection.close().await
}

fn remove_database_files(path: &Path) {
    let base = path.as_os_str().to_string_lossy();
    for suffix in ["", "-wal", "-shm", "-wal-tshm"] {
        let _ = std::fs::remove_file(format!("{base}{suffix}"));
    }
}
