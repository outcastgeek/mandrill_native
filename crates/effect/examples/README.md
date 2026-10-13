# Build and run the Effect examples

Run these commands from the workspace root (the directory containing the root `Cargo.toml`) with Rust 1.99.0 installed. `cargo run` also builds the selected example.

These 15 targets use package `effect` and need no additional features or environment variables. Each run is finite. All but the two file examples use supplied in-memory data. Prices are integral cents.

## Browse the menu: `browse_cafe_menu`

[Source](browse_cafe_menu.rs). Prints three drinks and their prices, skips dairy, and confirms Maya’s oat latte at 475 cents.

```sh
cargo +1.99.0 build --locked -p effect --example browse_cafe_menu
cargo +1.99.0 run --locked -p effect --example browse_cafe_menu
```

## Publish a price change: `update_menu_prices`

[Source](update_menu_prices.rs). New visits see the 500-cent oat latte; Maya’s retained accepted quote stays at 475 cents.

```sh
cargo +1.99.0 build --locked -p effect --example update_menu_prices
cargo +1.99.0 run --locked -p effect --example update_menu_prices
```

## Save a corrected order: `save_coffee_order`

[Source](save_coffee_order.rs). Corrects quantity zero to two oat lattes and reloads the saved quantity and note.

```sh
cargo +1.99.0 build --locked -p effect --example save_coffee_order
cargo +1.99.0 run --locked -p effect --example save_coffee_order
```

## Check out a basket: `live_coffee_basket`

[Source](live_coffee_basket.rs). Changes the basket total from $12.75 to $8.00, reserves stock atomically, rejects excess demand without changing stock, and restores the saved basket.

```sh
cargo +1.99.0 build --locked -p effect --example live_coffee_basket
cargo +1.99.0 run --locked -p effect --example live_coffee_basket
```

## Place an order: `place_coffee_order`

[Source](place_coffee_order.rs). Normalizes the pickup name and accepts two cups as ticket 104 through the typed RPC response.

```sh
cargo +1.99.0 build --locked -p effect --example place_coffee_order
cargo +1.99.0 run --locked -p effect --example place_coffee_order
```

## Follow recorded order history: `track_coffee_order`

[Source](track_coffee_order.rs). Reports ticket 104’s accepted, preparing and ready stages with the recorded cup count.

```sh
cargo +1.99.0 build --locked -p effect --example track_coffee_order
cargo +1.99.0 run --locked -p effect --example track_coffee_order
```

## Read phone messages: `phone_order_status`

[Source](phone_order_status.rs). Consumes the finite status feed through peer close, then releases it. The transport is supplied in-process; the café URI needs no running network server.

```sh
cargo +1.99.0 build --locked -p effect --example phone_order_status
cargo +1.99.0 run --locked -p effect --example phone_order_status
```

To consume an already accepted socket instead of opening it:

```sh
cargo +1.99.0 run --locked -p effect --example phone_order_status -- --accepted
```

## Issue recorded pickup labels: `prepare_coffee_orders`

[Source](prepare_coffee_orders.rs). Issues Maya’s two-cup labels once and returns the same recorded pickup on replay.

```sh
cargo +1.99.0 build --locked -p effect --example prepare_coffee_orders
cargo +1.99.0 run --locked -p effect --example prepare_coffee_orders
```

## Deliver a queued ticket: `queue_coffee_orders`

[Source](queue_coffee_orders.rs). Delivers and acknowledges Nora’s ticket 106; the next claim is empty.

```sh
cargo +1.99.0 build --locked -p effect --example queue_coffee_orders
cargo +1.99.0 run --locked -p effect --example queue_coffee_orders
```

## Resolve pickup tickets: `resolve_pickup_tickets`

[Source](resolve_pickup_tickets.rs). Resolves customer/cup data for tickets 104 and 105; ticket 999 returns `UnknownTicket`.

```sh
cargo +1.99.0 build --locked -p effect --example resolve_pickup_tickets
cargo +1.99.0 run --locked -p effect --example resolve_pickup_tickets
```

## Return a cancelled reservation: `reserve_cancelled_order`

[Source](reserve_cancelled_order.rs). Stock changes from 8 to 6 to 8 after compensation; replay leaves it at 8.

```sh
cargo +1.99.0 build --locked -p effect --example reserve_cancelled_order
cargo +1.99.0 run --locked -p effect --example reserve_cancelled_order
```

## File the closing checklist: `close_coffee_shop`

[Source](close_coffee_shop.rs). Inspects Maya’s two recorded cups and runs the 17:00 UTC closing checklist using the supplied clock. It does not wait for the real wall clock to reach 17:00.

```sh
cargo +1.99.0 build --locked -p effect --example close_coffee_shop
cargo +1.99.0 run --locked -p effect --example close_coffee_shop
```

## Issue receipts and total sales: `coffee_shop_receipts`

[Source](coffee_shop_receipts.rs). Issues Maya’s 950-cent and Lee’s 325-cent receipts; the default view totals three cups and $12.75.

```sh
cargo +1.99.0 build --locked -p effect --example coffee_shop_receipts
cargo +1.99.0 run --locked -p effect --example coffee_shop_receipts
```

To print the detailed receipt book to stdout:

```sh
cargo +1.99.0 run --locked -p effect --example coffee_shop_receipts -- --book
```

This mode does not create a file. Use `write_receipt_book` below for a CSV file.

## Export customer totals: `export_receipts`

[Source](export_receipts.rs). Arguments, in order: counter CSV, mobile CSV, new output CSV. Inputs are UTF-8, without headers or quoted commas, with exactly four fields per row: `ticket,customer,cups,unit_cents`. Ticket and customer must be nonempty, cups must be a positive `u16`, and unit price must fit `u32` cents. The output’s parent directory must exist and the output file must not exist.

```sh
cargo +1.99.0 build --locked -p effect --example export_receipts
receipt_dir=$(mktemp -d "${TMPDIR:-/tmp}/effect-receipts.XXXXXX")
printf '104,Maya,2,475\n' > "$receipt_dir/counter.csv"
printf '105,Lee,1,325\n' > "$receipt_dir/mobile.csv"
cargo +1.99.0 run --locked -p effect --example export_receipts -- "$receipt_dir/counter.csv" "$receipt_dir/mobile.csv" "$receipt_dir/customer-totals.csv"
cat "$receipt_dir/customer-totals.csv"
```

The program reports two exported customer totals. The output has no header and is sorted by customer, with totals in cents:

```text
Lee,325
Maya,950
```

Run the setup again to get a fresh directory for another export.

## Write a receipt book: `write_receipt_book`

[Source](write_receipt_book.rs). The single argument is a new output CSV path. Its parent must exist; the file must not exist. This example writes its two supplied receipts, flushes and closes the file, and exits.

```sh
cargo +1.99.0 build --locked -p effect --example write_receipt_book
book_dir=$(mktemp -d "${TMPDIR:-/tmp}/effect-book.XXXXXX")
cargo +1.99.0 run --locked -p effect --example write_receipt_book -- "$book_dir/receipts.csv"
cat "$book_dir/receipts.csv"
```

The program reports that the book was flushed and closed. The file contains `ticket,customer,total_cents`, without a header:

```text
104,Maya,950
105,Lee,325
```

This three-field output has a different format from the four-field receipt-export inputs. Run the setup again to select a fresh output file.

## Support modules and extension examples

`cafe_support/`, `menu/` and `order_tracking/` contain support modules, not runnable Cargo example targets. See the [crate guide](../README.md) for the DSL roles of these stories, and the [SQL](../../effect_sql/examples/README.md), [Server](../../effect_server/examples/README.md), [Identity](../../effect_server_identity/examples/README.md), [Web](../../effect_server_web/examples/README.md) and [OTel](../../effect_otel/examples/README.md) guides for the other six executable examples.
