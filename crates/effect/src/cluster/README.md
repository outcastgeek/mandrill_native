# Cluster

Cluster separates accepted messages, ownership/activation, execution and recorded replies. Stable addresses and request IDs determine identity; store claims and ownership generations fence stale workers. The message store owns accepted requests and completion records. `MemoryClusterMessageStore` is process-local and cannot satisfy a physical-durability promise; an external provider must document its restart and multi-process guarantees.

`ClusterWorkflowEngine` is an adapter from addressed Cluster delivery to the independent `WorkflowEngine`. It dispatches typed `WorkflowCommand` values to that engine and returns `WorkflowReply` values; it does not create another workflow journal. `ClusterWorkflowEngine::register` couples an entity type with an RPC operation and verifies that the addressed entity ID matches the workflow run ID.

`ClusterRunner` obtains its message store, scheduler and wall clock through `R`. It claims accepted messages, dispatches through the adapter and records replies. Expired claim leases permit reclaim; ownership generations fence stale settlement. Encoded requests pass through `EntityDispatcher` for frame and envelope decoding, address checks and typed reply encoding.

`cluster! { memory: &memory, region: region, store_capacity: limits }` selects
the prepared volatile store. It reserves the record and activation tables,
payload slots, per-message history and observer records, and leased read pages.
Inactive addresses retain their owner generations, so the activation bound
counts remembered addresses. Stable identities retain their caller-owned bytes
under the configured identity limits. Reply and history readers retain actual
payload slots after record removal.

`PreparedClusterClient` writes a typed request directly into a reserved store
slot and decodes a leased observation. Acknowledgement remains separate.
`ClusterRunner::run_next_prepared` acquires a regional pending slot, applies the
per-entity bound, and reserves reply backing, a diagnostic token frame and
native executor capacity before claiming. The executor receives the claim and
backing together and returns them after work settles. A failure retains its
original token frame until dropped. The completion timestamp and fenced store transition run
without suspension once the reply returns. `run_next_prepared_in` can use the
enclosing prepared context's cancellation registration.

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`entity_macro`, for an `entity!` handler and addressed framed calls.
