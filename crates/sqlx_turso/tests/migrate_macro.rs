use sqlx::migrate;
use sqlx::{Executor, Row};
use sqlx_turso::{TursoConnectOptions, TursoPoolOptions};

static MIGRATOR: sqlx::migrate::Migrator = migrate!("tests/migrations");

#[tokio::test]
async fn ordinary_sqlx_migrate_macro_runs_on_turso() -> sqlx::Result<()> {
    let pool = TursoPoolOptions::new()
        .max_connections(1)
        .connect_with(TursoConnectOptions::new())
        .await?;

    MIGRATOR.run(&pool).await?;

    let row = pool
        .fetch_one("SELECT value FROM macro_migration_data WHERE id = 1")
        .await?;
    assert_eq!(row.try_get::<String, _>("value")?, "migrated");

    pool.close().await;
    Ok(())
}
