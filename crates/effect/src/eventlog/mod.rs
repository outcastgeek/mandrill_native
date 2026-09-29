//! Typed event histories, projections, and replication.
//!
//! The journal is the authority for appended events. Stable event identities
//! reject duplicate deliveries, and cursor compare-and-set rejects stale
//! writers. A projection folds journal entries into a derived state and moves
//! its checkpoint atomically with that state; it can be rebuilt from retained
//! history. Reactive invalidation is a downstream notification, not evidence
//! of an append. In-memory journals do not promise recovery after process loss.

mod event;
mod event_builder;
mod event_group;
mod event_journal;
mod event_log;
mod event_log_message;
mod event_log_remote;
mod event_observation;
mod memory_event_journal;
mod memory_projection_store;
mod projection;
mod replica_cursor;
mod replica_protection;
mod replica_transfer;
mod retention;

pub use event::{
    AggregateId, EncodedEventOutcome, EventEnvelope, EventGroupId, EventId, EventIdentityError,
    EventJournalId, EventOutcomeKind, EventTag, EventVersion, JournalCursor, JournalEntry,
    JournalStream,
};
pub use event_builder::{
    EventBinaryFormats, EventCodecError, EventCodecs, EventDefinition, EventShape,
};
pub use event_group::{EventGroup, EventGroupError};
pub use event_journal::{
    AppendDecision, AppendReceipt, CompactionOutcome, EventInspection, EventJournal,
    HasEventJournal, JournalCompaction, JournalRead, JournalReadOutcome, JournalReadViolation,
};
pub use event_log::{EventCommitPolicy, EventInvocation, EventLog, EventRunError, EventRunOutcome};
pub use event_log_message::{EventLogRpc, ReplicaReadRequest};
pub use event_log_remote::{
    EventReplicator, HasReplicaConflictHandler, ReplicaBatch, ReplicaBatchError, ReplicaConflict,
    ReplicaConflictHandler, ReplicaExport, ReplicaExportError, ReplicaId, ReplicaImport,
    ReplicaImportError, ReplicaSync, ReplicaSyncError,
};
pub use event_observation::{EventObservation, EventObservationError, EventObservationReport};
pub use memory_event_journal::{MemoryEventJournal, MemoryJournalCapacity, MemoryJournalError};
pub use memory_projection_store::{MemoryProjectionError, MemoryProjectionStore};
pub use projection::{
    AtomicProjectionStore, HasAtomicProjectionStore, Projection, ProjectionAdvance,
    ProjectionAdvanceError, ProjectionId, ProjectionKey, ProjectionSnapshot, ProjectionVersion,
    ProjectionWrite,
};
pub use replica_cursor::{
    HasReplicaCursorStore, MemoryReplicaCursorError, MemoryReplicaCursorStore,
    ReplicaCursorAdvance, ReplicaCursorKey, ReplicaCursorStore,
};
pub use replica_protection::{
    HasReplicaProtection, ProtectedReplicaPayload, ProtectionMetadata, ReplicaProtection,
    ReplicaSessionId,
};
pub use replica_transfer::{
    ProtectedReplicaExport, ProtectedReplicaExportError, ProtectedReplicaImportError,
    ProtectedReplicaTransfer, ReplicaTransferError,
};
pub use retention::{
    CompactionPermit, MemoryRetentionAuthority, ReplayRetention, RetentionConsumerId,
    RetentionError,
};
