# Build and run the Identity example

Run these commands from the workspace root (the directory containing the root `Cargo.toml`) with Rust 1.99.0 installed. `cargo run` also builds the selected example.

## `customer_account`

[Source](customer_account.rs). Package `effect-server-identity`; required feature: `sqlx-turso`. No positional arguments are accepted by the recipe. `CAFE_DATABASE` selects a local database filename; the parent directory must exist. The native driver creates the database if needed.

```sh
cargo +1.99.0 build --locked -p effect-server-identity --features sqlx-turso --example customer_account
mkdir -p target/example-data
CAFE_DATABASE=target/example-data/receipt-accounts.db cargo +1.99.0 run --locked -p effect-server-identity --features sqlx-turso --example customer_account
```

The example registers a uniquely named Maya account, verifies its supplied demo password, and saves the private email-receipt preference. Expected output:

```text
Maya registered, logged in, and saved email_receipts=true.
```

It closes the pool and exits; the selected database remains on disk. Repeated runs create fresh account aliases. This account workflow needs no HTTP listener, cookie key ring or external identity provider.

See the [Web host guide](../../effect_server_web/examples/README.md) for native cookie configuration and [all example guides](../../effect/examples/README.md#support-modules-and-extension-examples).

The example now selects the native factory with an inferred `native_factory` block.
After native sign-in, `authorize_once` mints a collection scope and `accounts_page`
uses an extracted decision recipe plus an inline per-account visibility rule. Each
emitted account revalidates the actor and policy; the example permits only Maya's
own account. Preferences still use the selected versioned consent store.

The [Identity facade](../src/lib.rs) documents the finite callback families: query
`decide`/`allow`, administrative `decide`/`operation`, snapshot `transform`, durable
`transport`/`dispatch`/`accept`, whole native/capability/HTTP operations and scoped
authorization. Ordinary Effect-returning credential, factor, passkey, external,
session and record operations bind directly in workflow blocks. The [browser pickup
example](../../effect_server_web/examples/README.md) demonstrates typed route input,
workflow, renderer and exact request/resource guards under the same request owner.
