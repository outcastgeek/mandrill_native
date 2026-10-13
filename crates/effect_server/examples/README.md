# Build and run the native Server example

Run these commands from the workspace root (the directory containing the root `Cargo.toml`) with Rust 1.99.0 installed. `cargo run` also builds the selected example.

## `serve_cafe_orders`

[Source](serve_cafe_orders.rs). Package `effect-server`; its default features provide the native HTTP transport. No additional features or positional arguments are required.

```sh
cargo +1.99.0 build --locked -p effect-server --example serve_cafe_orders
CAFE_CHUNK_BYTES=4096 CAFE_JSON_BYTES=1024 CAFE_TELEMETRY_CAPACITY=32 CAFE_DRAIN_MS=3000 cargo +1.99.0 run --locked -p effect-server --example serve_cafe_orders
```

All four environment variables are required:

| Variable | Meaning |
| --- | --- |
| `CAFE_CHUNK_BYTES` | Positive maximum bytes per body chunk. |
| `CAFE_JSON_BYTES` | Positive maximum bytes for the order JSON policy. |
| `CAFE_TELEMETRY_CAPACITY` | Positive counts for active spans, metric descriptors and series per descriptor. |
| `CAFE_DRAIN_MS` | Request-drain budget in milliseconds. |

The host binds an OS-selected loopback port and sends its own two `GET /orders/42` requests with `Authorization: Bearer cafe-demo-customer-7`. One requests `Accept: text/html`, the other `Accept: application/json`. It prints both HTTP responses and a `completed café spans:` count, drains the host, shuts down its providers and exits automatically. No second terminal or manual curl request is needed for this finite example.

`cafe_runtime/` is support code, not a runnable target. For a host that remains running and accepts curl requests, see the [Web](../../effect_server_web/examples/README.md) or [OTel timeline](../../effect_otel/examples/README.md#watch_order_progress) example. See [all example guides](../../effect/examples/README.md#support-modules-and-extension-examples).
