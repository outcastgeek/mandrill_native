# Workflow

A workflow definition and its stable run ID, version and compatibility signature determine the recorded execution history. Coordination is expressed as `Durable` operations; ordinary `Effect` work belongs in named activities. The `workflow!` procedural macro composes durable operations using Rust block syntax. The workflow engine, wall clock and any worker runtime are capabilities of the Effect environment `R`, so the core does not choose Tokio, Rayon or a mandatory Cluster.

`MemoryWorkflowEngine` records registrations, activities, signals, deadlines, children and terminal outcomes in process memory. It supports reconstruction from supplied snapshots, but does not claim physical durability. A production engine must make its acknowledged records recoverable under its stated failure model. Replays must use stable activity and signal identities; changing a definition's compatibility signature under one ID and version is a conflict.

`workflow! { prepare_engine; memory: &memory, region: region, capacity: limits }`
selects regional definition, run and per-kind history tables in the same
engine. The prepared policy checks identity lengths and retained encoded
payload lengths before publishing records. Existing `Arc` payload backing
retains its ordinary ownership. Regional replay pages keep their own slot and
the records' byte owners until dropped. A cursor rejects a changed history
instead of combining records from different revisions. Workflow execution and
recovery read a compact header without copying the whole history.

History append refusal leaves the recorded state unchanged. An external
activity can already have finished when its outcome append fails. A replay
can repeat that unrecorded activity under the existing activity contract.

`DurableQueue` provides the result-before-ack replay protocol with either volatile or durable providers. When process-loss recovery is required, construct `DurableQueue::new(queue).require_physical_durability::<ProductionEnv>()` and use the returned `DurableRequiredQueue<Payload, Domain, ProductionEnv>`. `ProductionEnv` must expose the same queue and workflow capabilities used by those operations; its queue store must implement `DurablePersistedQueueStore` and its engine `DurableWorkflowEngine`. Those markers are explicit provider warranties about acknowledged state surviving each provider's documented restart boundary. Neither memory provider implements them, so a memory-only environment fails to compile at this constructor.

`DurableDeferred<A>` binds a typed signal to one run and definition version. Its result can be inspected after recording, including when a queue acknowledgement is lost. The queue and workflow engine remain separate authorities; providers must uphold their respective acknowledgement and recovery contracts.

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`workflow_cpu`, for `activity!` and `workflow!` composition with providers supplied
through `R`.
