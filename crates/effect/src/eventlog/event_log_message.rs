//! Typed messages for replica transfer.

use super::{
    EventJournal, EventReplicator, HasEventJournal, HasReplicaCursorStore, JournalCursor,
    JournalStream, ReplicaBatch, ReplicaCursorStore, ReplicaExport, ReplicaExportError, ReplicaId,
    ReplicaSync, ReplicaSyncError,
};
use crate::rpc::{RpcHandler, RpcOperation};
use std::num::NonZeroUsize;

/// Type alias for replica message error in eventlog event log message.
///
/// # Type parameters
/// - `R`: Environment type.
type ReplicaMessageError<R> = ReplicaSyncError<
    <<R as HasEventJournal>::Journal as EventJournal>::Error,
    <<R as HasReplicaCursorStore>::Store as ReplicaCursorStore>::Error,
>;
/// Type alias for replica rpc operation in eventlog event log message.
///
/// # Type parameters
/// - `R`: Environment type.
pub type ReplicaRpcOperation<R> = RpcOperation<ReplicaBatch, ReplicaSync, ReplicaMessageError<R>>;
/// Type alias for replica rpc handler in eventlog event log message.
///
/// # Type parameters
/// - `R`: Environment type.
pub type ReplicaRpcHandler<R> = RpcHandler<ReplicaBatch, ReplicaSync, ReplicaMessageError<R>, R>;

/// A stream and cursor request for a bounded replica read.
pub struct ReplicaReadRequest {
    /// The underlying failure or provider cause. Type: `ReplicaId`.
    pub source: ReplicaId,
    /// Stable identity of the ordered stream. Type: `JournalStream`.
    pub stream: JournalStream,
    /// Cursor immediately before the first requested journal entry. Type: `JournalCursor`.
    pub after: JournalCursor,
    /// The caller-supplied bound. Type: `NonZeroUsize`.
    pub limit: NonZeroUsize,
}

/// Builds typed RPC operations for event-log replica transfer using supplied
/// message codecs, RPC runtime, and transport. The journal remains the append
/// authority and the replica cursor is advanced only after import.
pub struct EventLogRpc;

impl EventLogRpc {
    #[must_use]
    /// Build the typed RPC handler that exports bounded journal entries.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasEventJournal + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `operation` (`RpcOperation<ReplicaReadRequest, ReplicaExport, ReplicaExportError<<R::Journal as EventJournal>::Error>>`): Typed operation definition or callback selected for execution.
    ///
    /// # Returns
    /// Type: `RpcHandler<ReplicaReadRequest, ReplicaExport, ReplicaExportError<<R::Journal as EventJournal>::Error>, R>`.
    /// Returns the value described by this method's return type.
    pub fn export_handler<R>(
        operation: RpcOperation<
            ReplicaReadRequest,
            ReplicaExport,
            ReplicaExportError<<R::Journal as EventJournal>::Error>,
        >,
    ) -> RpcHandler<
        ReplicaReadRequest,
        ReplicaExport,
        ReplicaExportError<<R::Journal as EventJournal>::Error>,
        R,
    >
    where
        R: HasEventJournal + Send + Sync + 'static,
    {
        RpcHandler::new(operation, |request: ReplicaReadRequest| {
            EventReplicator::export::<R>(
                request.source,
                request.stream,
                request.after,
                request.limit,
            )
        })
    }

    /// Return the typed handler registered for this definition.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasEventJournal + HasReplicaCursorStore + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `operation` (`ReplicaRpcOperation<R>`): Typed operation definition or callback selected for execution.
    ///
    /// # Returns
    /// Type: `ReplicaRpcHandler<R>`.
    pub fn handler<R>(operation: ReplicaRpcOperation<R>) -> ReplicaRpcHandler<R>
    where
        R: HasEventJournal + HasReplicaCursorStore + Send + Sync + 'static,
    {
        RpcHandler::new(operation, EventReplicator::import_and_ack::<R>)
    }
}
