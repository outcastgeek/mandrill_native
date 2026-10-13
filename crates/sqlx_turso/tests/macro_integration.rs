//! Compile-time and runtime coverage for ordinary `sqlx::query*` macros on Turso.

use sqlx::{
    query, query_as, query_as_unchecked, query_file, query_file_as, query_file_as_unchecked,
    query_file_scalar, query_file_scalar_unchecked, query_file_unchecked, query_scalar,
    query_scalar_unchecked, query_unchecked,
};
use std::str::FromStr;

use sqlx::ConnectOptions;
use sqlx::{Executor, Row};
use sqlx_turso::Turso;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, sqlx::FromRow)]
struct User {
    id: i64,
    name: String,
}

#[derive(Debug)]
struct MacroValue {
    value: i64,
}

#[tokio::test]
async fn test_query_as_runtime() -> TestResult {
    let mut conn = sqlx_turso::TursoConnectOptions::from_str("turso::memory:")?
        .connect()
        .await?;

    conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await?;

    conn.execute("INSERT INTO users (name) VALUES ('Alice'), ('Bob')")
        .await?;

    let users: Vec<User> = sqlx::query_as::<Turso, User>("SELECT id, name FROM users ORDER BY id")
        .fetch_all(&mut conn)
        .await?;

    assert_eq!(users.len(), 2);
    assert_eq!(users.first().map(|user| user.id), Some(1));
    assert_eq!(users.first().map(|user| user.name.as_str()), Some("Alice"));
    assert_eq!(users.get(1).map(|user| user.id), Some(2));
    assert_eq!(users.get(1).map(|user| user.name.as_str()), Some("Bob"));
    Ok(())
}

#[tokio::test]
async fn facade_query_macros_select_turso() -> TestResult {
    let mut conn = sqlx_turso::TursoConnectOptions::from_str("turso::memory:")?
        .connect()
        .await?;

    let row = query!("SELECT CAST(41 AS INTEGER) AS `value!: i64`")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(row.value, 41);

    let row = query_as!(MacroValue, "SELECT CAST(42 AS INTEGER) AS `value!: i64`")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(row.value, 42);

    let value = query_scalar!("SELECT CAST(43 AS INTEGER) AS `value!: i64`")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(value, 43);

    let row = query_file!("tests/stock_literal.sql")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(row.value, 43);

    let row = query_file_as!(MacroValue, "tests/stock_literal.sql")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(row.value, 43);

    let value = query_file_scalar!("tests/stock_literal.sql")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(value, 43);

    let row = query_unchecked!("SELECT CAST(44 AS INTEGER) AS `value!: i64`")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(row.value, 44);

    let row = query_as_unchecked!(MacroValue, "SELECT CAST(45 AS INTEGER) AS `value!: i64`")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(row.value, 45);

    let value = query_scalar_unchecked!("SELECT CAST(46 AS INTEGER) AS `value!: i64`")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(value, 46);

    let row = query_file_unchecked!("tests/stock_literal.sql")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(row.value, 43);

    let row = query_file_as_unchecked!(MacroValue, "tests/stock_literal.sql")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(row.value, 43);

    let value = query_file_scalar_unchecked!("tests/stock_literal.sql")
        .fetch_one(&mut conn)
        .await?;
    assert_eq!(value, 43);

    Ok(())
}

#[tokio::test]
async fn test_query_scalar_runtime() -> TestResult {
    let mut conn = sqlx_turso::TursoConnectOptions::from_str("turso::memory:")?
        .connect()
        .await?;

    let count: i64 = sqlx::query_scalar::<Turso, i64>("SELECT 42")
        .fetch_one(&mut conn)
        .await?;

    assert_eq!(count, 42);
    Ok(())
}

#[tokio::test]
async fn test_query_bind_runtime() -> TestResult {
    let mut conn = sqlx_turso::TursoConnectOptions::from_str("turso::memory:")?
        .connect()
        .await?;

    conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        .await?;
    conn.execute("INSERT INTO t (id) VALUES (1), (2), (3)")
        .await?;

    let row = sqlx::query::<Turso>("SELECT id FROM t WHERE id = ?")
        .bind(2i32)
        .fetch_one(&mut conn)
        .await?;

    let id: i64 = row.try_get("id")?;
    assert_eq!(id, 2);
    Ok(())
}
