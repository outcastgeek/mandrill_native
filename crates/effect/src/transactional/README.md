# Transactional memory

`Transaction<A, E>` describes reads, writes, retry, alternatives, and collection operations over a single journal. `atomically` obtains the `TransactionDomain` from the Effect environment, evaluates the description, and publishes all writes together after validating the read set. A typed failure abandons the journal. Conflict retry is bounded by the caller's `TransactionPolicy`; an explicit `retry` waits for an observed cell to change.

The domain is process-local synchronization, not persistent storage. Values must implement `TxValue`, whose contract forbids mutation through aliases outside a transaction. Reads share immutable values through `Arc`; writes replace whole values. `Transaction::for_each` rebuilds its iterator on optimistic retry and uses the same journal for every step. The `transaction!` macro offers Rust block syntax for transaction operations while rejecting ordinary `Effect` and `Future` steps at the type boundary.

A cell occupies domain capacity while a `TxRef`, transaction description, or subscription owns its reference handle. Dropping the last handle removes the cell and releases that capacity. An `Arc` value already returned by a read remains valid as an immutable snapshot after the cell is released; a newly allocated cell receives a different identity.

`TxSubscriptionRef` exposes committed values through the common reactive source contract. Subscription captures its initial version and value together, then registers a waiter before checking for the next version. Rolled-back writes are not published.

Run the [coffee basket story](../../examples/live_coffee_basket.rs) to reserve all selected drinks together:

```sh
cargo +1.99.0 run --locked -p effect --example live_coffee_basket
```

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`transaction_cpu`, for a `transaction!` program with the transaction domain
supplied through `R`.
