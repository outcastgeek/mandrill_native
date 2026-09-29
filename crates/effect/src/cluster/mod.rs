//! Logical entity execution and recoverable message delivery.
//!
//! An addressed request is accepted by a message store before a runner claims
//! and executes it. Ownership generations and claim leases fence stale workers;
//! replies are recorded before delivery. The store is the authority for those
//! decisions. In-memory providers preserve the logical rules only while the
//! process lives. `ClusterWorkflowEngine` connects addressed delivery to an
//! independent workflow engine, without becoming a second workflow journal.

mod activation;
mod client;
mod cluster_workflow_engine;
mod entity;
mod entity_proxy;
mod entity_resource;
mod memory_store;
mod message_store;
mod runner;
mod schedule;
mod singleton;
mod types;

pub use activation::{
    ActivationLease, ActivationOutcome, ActivationPhase, ActivationStatus, ClusterActivationStore,
    HasClusterActivationStore, PassivationOutcome,
};
pub use client::{ClusterClient, ClusterClientError, ClusterPoll};
pub use cluster_workflow_engine::{ClusterWorkflowEngine, ClusterWorkflowError};
pub use entity::{
    ClusterExecutor, EntityDispatchError, EntityDispatcher, EntityRegistrationError,
    HasClusterExecutor,
};
pub use entity_proxy::EntityProxy;
pub use entity_resource::EntityResource;
pub use memory_store::MemoryClusterMessageStore;
pub use message_store::{
    AcceptancePolicy, AcceptedRequest, ClaimOptions, ClaimToken, ClusterMessageStore,
    ClusterStoreError, ClusterStoreOperation, CompletedReply, DurableClusterMessageStore,
    HasClusterMessageStore, MessageAcceptance, MessageClaim, MessageDisposition, MessageInspection,
    MessageScan, MessageStoreCapacity, ObserverCursor, PayloadConflictPolicy, ReplyKind,
    SavedChunk, SavedMessage, SavedMessageState, SavedReply, StoreCompletion, StoreMutation,
    StoreObservation,
};
pub use runner::{ClusterRunOutcome, ClusterRunner, ClusterRunnerError};
pub use schedule::{
    ClusterOccurrenceId, ClusterSchedule, ClusterScheduleError, ClusterScheduleId,
    ConcurrentOccurrencePolicy, PlannedOccurrence, ScheduleDisposition,
};
pub use singleton::{ClusterSingleton, SingletonError, SingletonOutcome};
pub use types::{
    ActivationId, ClusterAddress, ClusterEntityId, ClusterEntityType, ClusterIdentityError,
    ClusterObserverId, ClusterOwner, ClusterRequestId, ClusterShardId, OwnershipGeneration,
};
