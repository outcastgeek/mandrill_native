# sqlx-turso

`sqlx-turso` is the SQLx driver for the Rust Turso database engine. It exposes a distinct
`Turso` database type while re-exporting the exact native `turso` crate used by the adapter.
SQLx and native calls can operate on the same database owner and connection.

## Crates

- `sqlx_turso`: public facade and application-facing imports
- `sqlx-turso-core`: SQLx runtime implementation
- `sqlx-turso-macros`: checked query macro integration
- `sqlx-turso-cli`: Turso-aware offline metadata preparation

## Basic use

Use the Turso-enabled facade under the ordinary dependency name `sqlx`. The direct `turso`
dependency must use the same pinned native release as the adapter.

```toml
[dependencies]
sqlx = { package = "sqlx-turso-sqlx", version = "0.1.0", features = ["macros", "migrate", "runtime-tokio"] }
sqlx_turso = { version = "0.1.0" }
turso = { version = "=0.8.0-pre.11", default-features = false }
```

```rust
use sqlx::{Connection, Executor, Row};
use sqlx_turso::TursoConnection;

async fn example() -> sqlx::Result<()> {
    let mut connection = TursoConnection::connect("turso::memory:").await?;
    connection
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await?;
    connection
        .execute("INSERT INTO users (id, name) VALUES (1, 'alice')")
        .await?;

    let row = connection
        .fetch_one("SELECT name FROM users WHERE id = 1")
        .await?;
    assert_eq!(row.try_get::<String, _>("name")?, "alice");
    Ok(())
}
```

Checked queries use the same connection directly:

```rust
# use sqlx::Connection;
# async fn example() -> sqlx::Result<()> {
let mut connection = sqlx_turso::TursoConnection::connect("turso::memory:").await?;
let row = sqlx::query!("SELECT CAST(42 AS INTEGER) AS `value!: i64`")
    .fetch_one(&mut connection)
    .await?;
assert_eq!(row.value, 42);
# Ok(())
# }
```

Use `TursoConnectOptions::from_native_database` or
`TursoConnectOptions::from_native_sync_database` to retain a preconfigured native owner. Use
`TursoConnection::from_native_connection` or `from_native_transaction` for an existing native
session. The coordinated `native_execute`, `native_execute_batch`, `native_batch`, and
`native_transactional_batch` methods keep native handles inside a checked-out SQLx connection.
Use `into_native_connection` for unrestricted native access and explicit ownership transfer;
detach a pooled connection before transferring it. `native_sync_database` provides access to the
retained sync database owner. For sync connections, `into_native_sync_parts` and
`from_native_sync_parts` move the connection and sync owner together.

For example, a native-configured database and SQLx pool retain one database owner:

```rust
# use sqlx::{Executor, Row};
# async fn example() -> sqlx::Result<()> {
let database = turso::Builder::new_local(":memory:")
    .experimental_generated_columns(true)
    .build()
    .await
    .map_err(sqlx::Error::config)?;
let native = database.connect().map_err(sqlx::Error::config)?;
native
    .execute("CREATE TABLE data(value INTEGER)", ())
    .await
    .map_err(sqlx::Error::config)?;

let pool = sqlx_turso::TursoPoolOptions::new()
    .connect_with(sqlx_turso::TursoConnectOptions::from_native_database(database))
    .await?;
pool.execute("INSERT INTO data(value) VALUES (42)").await?;
let row = pool.fetch_one("SELECT value FROM data").await?;
assert_eq!(row.try_get::<i64, _>("value")?, 42);
# Ok(())
# }
```

## Features

The default feature set enables SQLx macros, migrations, `Any`, offline metadata, Tokio, sync,
FTS, mimalloc, and all implemented SQLx value integrations.

- SQLx integrations: `any`, `macros`, `migrate`, `offline`, and `runtime-tokio`
- Values: `bigdecimal`, `bit-vec`, `bstr`, `chrono`, `ipnet`, `ipnetwork`, `json`,
  `mac_address`, `rust_decimal`, `time`, and `uuid`
- Turso: `fts`, `io-memory-yield`, `mimalloc`, `pure-rust-crypto`, `stacker`, `sync`,
  `test-helper`, and `turso-default`

Decimal values use a tagged text representation so Turso does not coerce arbitrary-precision
values through a binary float. Network values, MAC addresses, and bit vectors use reversible
text representations.

## Checked queries and offline preparation

Depend on `sqlx-turso-sqlx` under the dependency name `sqlx` to use ordinary `sqlx::query!`,
`sqlx::query_as!`, `sqlx::query_scalar!`, their file variants, and their unchecked variants with
Turso database support. The `sqlx_turso` crate re-exports the same facade as `sqlx_turso::sqlx`.

Generate real Turso metadata with:

```sh
cargo run -p sqlx-turso-cli -- prepare \
  -D turso::memory: \
  --offline-dir crates/sqlx_turso/.sqlx \
  -- -p your-crate
```

Then verify a database-free build with `SQLX_OFFLINE=true`.

Validate that checked-in metadata matches the current queries and schema with:

```sh
cargo run -p sqlx-turso-cli -- prepare --check \
  -D turso::memory: \
  --offline-dir crates/sqlx_turso/.sqlx \
  -- -p your-crate
```

## Local replica integration test

`tests/local_replica.rs` starts the Turso Database CLI as a local sync server and performs a
bidirectional SQLx/native replica round trip, including a server restart. Test-owned databases
are uniquely named under `/tmp/sqlx-turso/replicas` and removed by the test guard. Set
`TURSO_DATABASE_CLI` to an installed CLI executable; otherwise the test uses `tursodb` when
available and falls back to the pinned npm CLI.

```sh
cargo test -p sqlx_turso --all-features --test local_replica -- --nocapture
```
