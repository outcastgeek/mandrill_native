# Reactivity

Reactive state has a common `ReactiveStateSource<R>` contract: an Effect for a snapshot and a Stream of changes. `R` supplies any runtime or external capability required by a particular source. An atom, transactional subscription reference, persisted atom, or RPC-backed query can implement the same consumer-facing interface without selecting a library-wide executor.

`InvalidationRegistry` publishes stable-key invalidations to bounded subscribers. A revision can mark cached query dependencies stale, but a notification is not the authority for a durable write. Subscribers that lag must resnapshot. An initial state-and-subscription boundary belongs inside each source's `changes` implementation; separate `read` then `changes` calls do not promise an atomic handoff.

`TxSubscriptionRef` supplies a committed transactional source. Its subscription captures an initial version and value together, and a transaction rollback emits no committed change. EventLog observations can publish committed journal entries as invalidations, retaining a replay cursor on publication failure.

RPC mutation atoms invalidate only after a successful typed remote reply. For a client with middleware, that boundary occurs before its post-response transform: `CommittedWithMiddlewareFailure` preserves a typed transform failure while identifying the already acknowledged mutation. If publication fails, `CommittedWithoutInvalidation` likewise distinguishes the committed remote action from a safe-to-retry client failure.

Run the [coffee basket story](../../examples/live_coffee_basket.rs) for tracked basket totals, stock queries, and saved-state restoration.

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`reactivity_cpu`, for a `reactive_derived!` atom evaluated through the
environment's CPU capability.
