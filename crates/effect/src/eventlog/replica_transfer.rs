use super::{
    EventJournal, EventReplicator, HasEventJournal, HasReplicaCursorStore, HasReplicaProtection,
    JournalCursor, JournalStream, ProtectedReplicaPayload, ReplicaBatch, ReplicaCursorStore,
    ReplicaExport, ReplicaExportError, ReplicaId, ReplicaProtection, ReplicaSessionId, ReplicaSync,
    ReplicaSyncError,
};
use crate::{Effect, JsonValue, ParsePolicy, PureCodec, SchemaIssue, SchemaIssueKind};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

type JournalError<R> = <<R as HasEventJournal>::Journal as EventJournal>::Error;
type ProtectionError<R> = <<R as HasReplicaProtection>::Protection as ReplicaProtection>::Error;
type CursorError<R> = <<R as HasReplicaCursorStore>::Store as ReplicaCursorStore>::Error;
pub type ProtectedExportEffect<R> = Effect<
    ProtectedReplicaExport,
    ProtectedReplicaExportError<JournalError<R>, ProtectionError<R>>,
    R,
>;
pub type ProtectedImportEffect<R> = Effect<
    ReplicaSync,
    ProtectedReplicaImportError<ProtectionError<R>, JournalError<R>, CursorError<R>>,
    R,
>;

#[derive(Debug)]
pub enum ReplicaTransferError<ProtectionError> {
    Schema(SchemaIssue),
    Protection(ProtectionError),
    SessionMismatch,
    InputConsumed,
}

pub enum ProtectedReplicaExport {
    Payload {
        payload: ProtectedReplicaPayload,
        has_more: bool,
    },
    HistoryTruncated {
        first_available: JournalCursor,
    },
    CursorAhead {
        head: JournalCursor,
    },
}

#[derive(Debug)]
pub enum ProtectedReplicaExportError<JournalError, ProtectionError> {
    Journal(ReplicaExportError<JournalError>),
    Transfer(ReplicaTransferError<ProtectionError>),
}

#[derive(Debug)]
pub enum ProtectedReplicaImportError<ProtectionError, JournalError, CursorError> {
    Transfer(ReplicaTransferError<ProtectionError>),
    Sync(ReplicaSyncError<JournalError, CursorError>),
}

/// Codec and protection are independent capabilities. Encoded batches remain
/// bounded by the supplied parse policy before encryption and after opening.
pub struct ProtectedReplicaTransfer {
    codec: PureCodec<ReplicaBatch, JsonValue>,
    policy: ParsePolicy,
}

impl Clone for ProtectedReplicaTransfer {
    fn clone(&self) -> Self {
        Self {
            codec: self.codec.clone(),
            policy: self.policy,
        }
    }
}

impl ProtectedReplicaTransfer {
    #[must_use]
    pub fn new(codec: PureCodec<ReplicaBatch, JsonValue>, policy: ParsePolicy) -> Self {
        Self { codec, policy }
    }

    /// Export through the journal authority and protect the bounded page.
    pub fn export<R>(
        &self,
        session: ReplicaSessionId,
        source: ReplicaId,
        stream: JournalStream,
        after: JournalCursor,
        limit: NonZeroUsize,
    ) -> ProtectedExportEffect<R>
    where
        R: HasEventJournal + HasReplicaProtection + Send + Sync + 'static,
    {
        let transfer = self.clone();
        EventReplicator::export::<R>(source, stream, after, limit)
            .map_error(ProtectedReplicaExportError::Journal)
            .flat_map(move |page| match page {
                ReplicaExport::Batch { batch, has_more } => transfer
                    .seal::<R>(session.clone(), batch)
                    .map_error(ProtectedReplicaExportError::Transfer)
                    .map(move |payload| ProtectedReplicaExport::Payload { payload, has_more }),
                ReplicaExport::HistoryTruncated { first_available } => Effect::sync(move || {
                    ProtectedReplicaExport::HistoryTruncated { first_available }
                }),
                ReplicaExport::CursorAhead { head } => {
                    Effect::sync(move || ProtectedReplicaExport::CursorAhead { head })
                }
            })
    }

    /// Open, validate and import before advancing the source cursor. A lost
    /// cursor acknowledgement is safe to retry against event-ID deduplication.
    pub fn import_and_ack<R>(
        &self,
        expected_session: ReplicaSessionId,
        payload: ProtectedReplicaPayload,
    ) -> ProtectedImportEffect<R>
    where
        R: HasReplicaProtection + HasEventJournal + HasReplicaCursorStore + Send + Sync + 'static,
    {
        self.open::<R>(expected_session, payload)
            .map_error(ProtectedReplicaImportError::Transfer)
            .flat_map(|batch| {
                EventReplicator::import_and_ack::<R>(batch)
                    .map_error(ProtectedReplicaImportError::Sync)
            })
    }

    pub fn seal<R>(
        &self,
        session: ReplicaSessionId,
        batch: ReplicaBatch,
    ) -> Effect<
        ProtectedReplicaPayload,
        ReplicaTransferError<<R::Protection as ReplicaProtection>::Error>,
        R,
    >
    where
        R: HasReplicaProtection + Send + Sync + 'static,
    {
        let codec = self.codec.clone();
        let policy = self.policy;
        let batch = Mutex::new(Some(batch));
        Effect::try_sync(
            move || {
                let batch = batch
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(ReplicaTransferError::InputConsumed)?;
                let encoded = codec
                    .encode_ref(&batch)
                    .map_err(ReplicaTransferError::Schema)?;
                let bytes =
                    crate::schema::internal::parser::encode_json_bytes(&encoded, &policy.limits)
                        .map_err(ReplicaTransferError::Schema)?;
                Ok(Arc::<[u8]>::from(bytes))
            },
            std::convert::identity,
        )
        .flat_map(move |bytes| {
            Effect::service(|env: &R| env.replica_protection()).flat_map({
                let session = session.clone();
                move |protection| {
                    let expected = session.clone();
                    protection
                        .seal(session.clone(), Arc::clone(&bytes))
                        .map_error(ReplicaTransferError::Protection)
                        .provide_some(|_: &R| ())
                        .flat_map(move |payload| {
                            Effect::from_result_once(if payload.session == expected {
                                Ok(payload)
                            } else {
                                Err(ReplicaTransferError::SessionMismatch)
                            })
                        })
                }
            })
        })
    }

    pub fn open<R>(
        &self,
        expected: ReplicaSessionId,
        payload: ProtectedReplicaPayload,
    ) -> Effect<ReplicaBatch, ReplicaTransferError<<R::Protection as ReplicaProtection>::Error>, R>
    where
        R: HasReplicaProtection + Send + Sync + 'static,
    {
        if payload.session != expected {
            return Effect::fail_with(|| ReplicaTransferError::SessionMismatch);
        }
        let payload = Mutex::new(Some(payload));
        let codec = self.codec.clone();
        let policy = self.policy;
        Effect::service(|env: &R| env.replica_protection())
            .flat_map(move |protection| {
                let payload = payload
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                match payload {
                    Some(payload) => protection
                        .open(payload)
                        .map_error(ReplicaTransferError::Protection)
                        .provide_some(|_: &R| ()),
                    None => Effect::from_result_once(Err(ReplicaTransferError::InputConsumed)),
                }
            })
            .flat_map(move |bytes| {
                let codec = codec.clone();
                Effect::try_sync(
                    move || {
                        let text = std::str::from_utf8(&bytes).map_err(|error| {
                            ReplicaTransferError::Schema(SchemaIssue::new(
                                SchemaIssueKind::JsonSyntax {
                                    detail: Arc::from(error.to_string()),
                                },
                            ))
                        })?;
                        let encoded = crate::schema::internal::parser::parse_json(text, policy)
                            .map_err(ReplicaTransferError::Schema)?;
                        codec
                            .decode_ref(&encoded)
                            .map_err(ReplicaTransferError::Schema)
                    },
                    std::convert::identity,
                )
            })
    }
}
