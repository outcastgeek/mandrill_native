use sqlx_turso::{
    Turso, TursoConnectOptions, TursoPool, TursoPoolOptions,
    sqlx::{self, Executor, Row, pool::PoolConnection},
};

#[sqlx::test(migrations = false, fixtures("basic"))]
async fn stock_sqlx_test_attribute_builds_turso_pool(pool: TursoPool) -> sqlx::Result<()> {
    let row = sqlx::query("SELECT value FROM fixture_data")
        .fetch_one(&pool)
        .await?;
    assert_eq!(row.try_get::<String, _>("value")?, "fixture-applied");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn stock_sqlx_test_attribute_supplies_pool_connection(
    mut connection: PoolConnection<Turso>,
) -> sqlx::Result<()> {
    let value: (i64,) = sqlx::query_as("SELECT 42")
        .fetch_one(&mut *connection)
        .await?;
    assert_eq!(value.0, 42);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn stock_sqlx_test_attribute_supplies_options(
    pool_options: TursoPoolOptions,
    connect_options: TursoConnectOptions,
) -> sqlx::Result<()> {
    let pool = pool_options.connect_with(connect_options).await?;
    pool.execute("CREATE TABLE options_signature (id INTEGER PRIMARY KEY)")
        .await?;
    let value: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM options_signature")
        .fetch_one(&pool)
        .await?;
    assert_eq!(value.0, 0);
    pool.close().await;
    Ok(())
}
