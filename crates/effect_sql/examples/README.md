# Build and run the SQL example

Run these commands from the workspace root (the directory containing the root `Cargo.toml`) with Rust 1.100.0 installed. `cargo run` also builds the selected example.

## `cafe_orders`

[Source](cafe_orders.rs). Package `effect-sql`; required features: `turso,macros`. There are no positional arguments. `DATABASE_URL=turso::memory:` is required when compiling the checked SQL macros, including when `cargo run` compiles the example:

```sh
DATABASE_URL=turso::memory: cargo +1.100.0 build --locked -p effect-sql --features turso,macros --example cafe_orders
DATABASE_URL=turso::memory: cargo +1.100.0 run --locked -p effect-sql --features turso,macros --example cafe_orders
```

The runtime creates a fresh temporary file-backed café database, applies the migration, saves a 400-cent flat white, and corrects its price within a transaction. It selects a pending checkout admission on the SQL client and a materialization byte admission on the prepared statement pool used to reload the order. It also reads the saved drink as text with `atomic_query` and as a blob with `atomic_batch`, using prepared byte buffers. The example prints the following line.

```text
Order 1: flat white, 425 cents, ready=false; 1 saved order.
```

It closes the pool, removes its temporary database directory, and exits. `DATABASE_URL` selects compile-time checking; it does not select this example’s runtime file.

The demo pool selects a zero-capacity SQLx-facing statement cache. Its prepared
statements still use live native storage during execution. Startup passes
`None` to `SqlClient::open` to select ordinary native SQL allocation alongside
the prepared Effect-owned slots. The same startup function rejects either
strict native SQL policy before running its directory and pool factory.
The atomic drink projections use private prepared native row lanes selected on
the SQL client.

`support/cafe_database.rs` is a support module, not a runnable target. See the [SQL crate guide](../README.md) and [other example guides](../../effect/examples/README.md#support-modules-and-extension-examples).
