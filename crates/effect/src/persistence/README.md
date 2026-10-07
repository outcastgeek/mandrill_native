# Persistence

Persistence APIs use typed stable keys and explicit schemas, versions, expiry, retry and capacity policies. `KeyValueStore`, `ConditionalKeyValueStore`, `PersistedQueueStore`, and the durable provider marker express different guarantees; an ordinary key/value store does not imply atomic queue claims or crash durability. The in-memory stores implement the logical contracts for testing and process-local use, but do not survive process loss.

`PersistedQueue<P, D>` encodes `P` with a caller-selected schema identity and resource limits. Offering is idempotent by stable message ID and content. Claim tokens include owner and generation fences. A handler failure leaves its claim unsettled; the caller chooses retry or dead-letter behavior. Queue records distinguish ready, claimed, acknowledged and dead-letter states.

`DurableQueue` composes a persisted queue with a workflow deferred result. The worker records the result with the workflow engine before acknowledging the queue claim. If acknowledgement is uncertain, redelivery can call `acknowledge_recorded` to inspect the existing result and settle the valid claim without rerunning the handler.

The replay protocol requires recoverable provider state to survive process loss. Consumers requiring physical recovery use the workflow module's `require_physical_durability::<R>()` constructor; it requires durable markers on both the queue store and workflow engine supplied by `R`.

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`persistence_cpu`, for a typed `persistable!` definition with the store supplied
through `R`.
