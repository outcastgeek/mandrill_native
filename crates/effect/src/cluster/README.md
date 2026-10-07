# Cluster

Cluster separates accepted messages, ownership/activation, execution and recorded replies. Stable addresses and request IDs determine identity; store claims and ownership generations fence stale workers. The message store owns accepted requests and completion records. `MemoryClusterMessageStore` is process-local and cannot satisfy a physical-durability promise; an external provider must document its restart and multi-process guarantees.

`ClusterWorkflowEngine` is an adapter from addressed Cluster delivery to the independent `WorkflowEngine`. It dispatches typed `WorkflowCommand` values to that engine and returns `WorkflowReply` values; it does not create another workflow journal. `ClusterWorkflowEngine::register` couples an entity type with an RPC operation and verifies that the addressed entity ID matches the workflow run ID.

`ClusterRunner` obtains its message store, scheduler and wall clock through `R`. It claims accepted messages, dispatches through the adapter and records replies. Expired claim leases permit reclaim; ownership generations fence stale settlement. Encoded requests pass through `EntityDispatcher` for frame and envelope decoding, address checks and typed reply encoding.

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`entity_macro`, for an `entity!` handler and addressed framed calls.
