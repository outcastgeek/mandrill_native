use super::{
    EventJournal, EventReplicator, HasEventJournal, HasReplicaCursorStore, JournalCursor,
    JournalStream, ReplicaBatch, ReplicaCursorStore, ReplicaExport, ReplicaExportError, ReplicaId,
    ReplicaSync, ReplicaSyncError,
};
use crate::rpc::{RpcHandler, RpcOperation};
use std::num::NonZeroUsize;

type ReplicaMessageError<R> = ReplicaSyncError<
    <<R as HasEventJournal>::Journal as EventJournal>::Error,
    <<R as HasReplicaCursorStore>::Store as ReplicaCursorStore>::Error,
>;
pub type ReplicaRpcOperation<R> = RpcOperation<ReplicaBatch, ReplicaSync, ReplicaMessageError<R>>;
pub type ReplicaRpcHandler<R> = RpcHandler<ReplicaBatch, ReplicaSync, ReplicaMessageError<R>, R>;

pub struct ReplicaReadRequest {
    pub source: ReplicaId,
    pub stream: JournalStream,
    pub after: JournalCursor,
    pub limit: NonZeroUsize,
}

/// Typed RPC composition for replica transfer. The application supplies the
/// message codecs, RPC runtime and transport; the journal remains the append
/// authority and the replica cursor is advanced only after import.
pub struct EventLogRpc;

impl EventLogRpc {
    #[must_use]
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

    pub fn handler<R>(operation: ReplicaRpcOperation<R>) -> ReplicaRpcHandler<R>
    where
        R: HasEventJournal + HasReplicaCursorStore + Send + Sync + 'static,
    {
        RpcHandler::new(operation, EventReplicator::import_and_ack::<R>)
    }
}
