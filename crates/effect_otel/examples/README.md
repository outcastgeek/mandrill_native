# Build and run the OTel examples

Run these commands from the workspace root (the directory containing the root `Cargo.toml`) with Rust 1.99.0 installed. `cargo run` also builds the selected example.

Both targets use package `effect-otel` with `--features server`, including `await_pickup`: its executable workflow is enabled by that feature. No external telemetry collector or environment variables are required.

## `await_pickup`

[Source](await_pickup.rs). Four arguments are required, in order: north status file, south status file, positive wait-window milliseconds, positive poll-interval milliseconds. Each UTF-8 file contains `104,pending` or `104,ready` with no header. Create one pending outlet and one ready outlet:

```sh
cargo +1.99.0 build --locked -p effect-otel --features server --example await_pickup
pickup_dir=$(mktemp -d "${TMPDIR:-/tmp}/effect-pickup.XXXXXX")
printf '104,pending\n' > "$pickup_dir/north.status"
printf '104,ready\n' > "$pickup_dir/south.status"
cargo +1.99.0 run --locked -p effect-otel --features server --example await_pickup -- "$pickup_dir/north.status" "$pickup_dir/south.status" 2000 20
```

The example prints both availability snapshots, reports `Ticket 104 is ready at south.`, releases its native providers, and exits. With both files pending, staff can update either file while it polls. If neither becomes ready within the supplied window, the run returns a wait-window error. Missing or malformed files fail; only transient native I/O is retried.

## `watch_order_progress`

[Source](watch_order_progress.rs). This is a persistent HTTP/SSE host. Create a UTF-8 history CSV without a header, then run it:

```sh
cargo +1.99.0 build --locked -p effect-otel --features server --example watch_order_progress
timeline_dir=$(mktemp -d "${TMPDIR:-/tmp}/effect-timeline.XXXXXX")
printf '0,accepted\n400,preparing\n800,ready\n' > "$timeline_dir/history.csv"
cargo +1.99.0 run --locked -p effect-otel --features server --example watch_order_progress -- 127.0.0.1:8080 "$timeline_dir/history.csv" 256 32 32 16 8 5000 inline
```

All nine positional arguments are required:

| Position | Supplied value | Meaning |
| --- | --- | --- |
| 1 | `127.0.0.1:8080` | Listener IP address and port; select a free port. |
| 2 | `"$timeline_dir/history.csv"` | History file path. |
| 3 | `256` | Positive maximum SSE frame bytes. |
| 4 | `32` | Positive maximum cursor bytes. |
| 5 | `32` | Positive active-span capacity. |
| 6 | `16` | Positive metric-descriptor capacity. |
| 7 | `8` | Positive series capacity per descriptor. |
| 8 | `5000` | Positive host/provider shutdown budget in milliseconds. |
| 9 | `inline` | Writer mode: `inline`, `extracted` or `body`. |

Rows have two fields: `elapsed_ms,status`. Elapsed time is a nonnegative integer number of milliseconds since acceptance, in nondecreasing order. Valid statuses are exactly `accepted`, `preparing` and `ready`; at least one record is required.

Keep the host running. After Cargo starts the executable, open another terminal and request its stream:

```sh
curl -N -H 'Accept: text/event-stream' http://127.0.0.1:8080/orders/104/events
```

The response emits `order-status` SSE events with data `accepted`, `preparing` and `ready`, at the supplied elapsed times. The stream finishes after the recorded history; the host remains running. Client disconnection releases the request’s subscription.

Press Ctrl-C in the host terminal to drain requests and providers. Successful cleanup prints `Order timeline host closed; all subscriptions released.` To use the other supported writer forms, stop the previous host, then run either command in the original terminal using the same history file:

```sh
cargo +1.99.0 run --locked -p effect-otel --features server --example watch_order_progress -- 127.0.0.1:8080 "$timeline_dir/history.csv" 256 32 32 16 8 5000 extracted
cargo +1.99.0 run --locked -p effect-otel --features server --example watch_order_progress -- 127.0.0.1:8080 "$timeline_dir/history.csv" 256 32 32 16 8 5000 body
```

Run these individually, stopping one before starting the next. All three modes mount the same route and replay the same supplied history.

`cafe_telemetry/` contains shared provider wiring, not a runnable Cargo target. See [all example guides](../../effect/examples/README.md#support-modules-and-extension-examples).
