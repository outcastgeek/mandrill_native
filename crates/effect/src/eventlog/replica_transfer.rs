//! Replica import, deduplication, and gap handling.

use super::{
    EventJournal, EventReplicator, HasEventJournal, HasReplicaCursorStore, HasReplicaProtection,
    JournalCursor, JournalStream, ProtectedReplicaPayload, ReplicaBatch, ReplicaCursorStore,
    ReplicaExport, ReplicaExportError, ReplicaId, ReplicaProtection, ReplicaSessionId, ReplicaSync,
    ReplicaSyncError,
};
use crate::{Effect, JsonValue, ParsePolicy, PureCodec, SchemaIssue, SchemaIssueKind};
use crate::{
    effect, effect_map, effect_map_error, effect_provide_some, effect_sync, effect_try_sync,
    service,
};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// Type alias for journal error in eventlog replica transfer.
///
/// # Type parameters
/// - `R`: Environment type.
type JournalError<R> = <<R as HasEventJournal>::Journal as EventJournal>::Error;
/// Type alias for protection error in eventlog replica transfer.
///
/// # Type parameters
/// - `R`: Environment type.
type ProtectionError<R> = <<R as HasReplicaProtection>::Protection as ReplicaProtection>::Error;
/// Type alias for cursor error in eventlog replica transfer.
///
/// # Type parameters
/// - `R`: Environment type.
type CursorError<R> = <<R as HasReplicaCursorStore>::Store as ReplicaCursorStore>::Error;
/// Type alias for protected export effect in eventlog replica transfer.
///
/// # Type parameters
/// - `R`: Environment type.
pub type ProtectedExportEffect<R> = Effect<
    ProtectedReplicaExport,
    ProtectedReplicaExportError<JournalError<R>, ProtectionError<R>>,
    R,
>;
/// Type alias for protected import effect in eventlog replica transfer.
///
/// # Type parameters
/// - `R`: Environment type.
pub type ProtectedImportEffect<R> = Effect<
    ReplicaSync,
    ProtectedReplicaImportError<ProtectionError<R>, JournalError<R>, CursorError<R>>,
    R,
>;

#[derive(Debug)]
/// A framing, protection, or replication failure during transfer.
///
/// # Type parameters
/// - `ProtectionError`: Error returned by the replica-protection provider.
pub enum ReplicaTransferError<ProtectionError> {
    /// The value failed schema validation. Retains the `SchemaIssue` detail.
    Schema(SchemaIssue),
    /// The replica protection provider rejected the payload. Retains the `ProtectionError` detail.
    Protection(ProtectionError),
    /// The session differs from the required or recorded value.
    SessionMismatch,
    /// An owned input was already consumed.
    InputConsumed,
}

/// An exported batch wrapped with the selected replica-protection metadata.
pub enum ProtectedReplicaExport {
    /// The message carries application payload bytes.
    Payload {
        /// The payload carried by this request or record. Type: `ProtectedReplicaPayload`.
        payload: ProtectedReplicaPayload,
        /// Whether more entries remain after this batch. Type: `bool`.
        has_more: bool,
    },
    /// Replay requires history older than the retained boundary.
    HistoryTruncated {
        /// Earliest position still retained for replay. Type: `JournalCursor`.
        first_available: JournalCursor,
    },
    /// The supplied cursor is ahead of the recorded head.
    CursorAhead {
        /// Current head position of the ordered stream. Type: `JournalCursor`.
        head: JournalCursor,
    },
}

#[derive(Debug)]
/// A journal, encoding, or protection failure during protected export.
///
/// # Type parameters
/// - `JournalError`: Error returned by the event journal.
/// - `ProtectionError`: Error returned by the replica-protection provider.
pub enum ProtectedReplicaExportError<JournalError, ProtectionError> {
    /// The authoritative event journal rejected or failed the operation. Retains the `ReplicaExportError<JournalError>` detail.
    Journal(ReplicaExportError<JournalError>),
    /// Sending or receiving protected replica data failed. Retains the `ReplicaTransferError<ProtectionError>` detail.
    Transfer(ReplicaTransferError<ProtectionError>),
}

#[derive(Debug)]
/// A protection, decoding, or journal failure during protected import.
///
/// # Type parameters
/// - `ProtectionError`: Error returned by the replica-protection provider.
/// - `JournalError`: Error returned by the event journal.
/// - `CursorError`: Error returned by the replica cursor store.
pub enum ProtectedReplicaImportError<ProtectionError, JournalError, CursorError> {
    /// Sending or receiving protected replica data failed. Retains the `ReplicaTransferError<ProtectionError>` detail.
    Transfer(ReplicaTransferError<ProtectionError>),
    /// Synchronizing replica cursors or batches failed. Retains the `ReplicaSyncError<JournalError, CursorError>` detail.
    Sync(ReplicaSyncError<JournalError, CursorError>),
}

/// Codec and protection are independent capabilities. Encoded batches remain
/// bounded by the supplied parse policy before encryption and after opening.
pub struct ProtectedReplicaTransfer {
    codec: PureCodec<ReplicaBatch, JsonValue>,
    policy: ParsePolicy,
}

impl Clone for ProtectedReplicaTransfer {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            codec: self.codec.clone(),
            policy: self.policy,
        }
    }
}

impl ProtectedReplicaTransfer {
    #[must_use]
    /// Construct a `ProtectedReplicaTransfer` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `codec` (`PureCodec<ReplicaBatch, JsonValue>`): The codec used to convert and validate values.
    /// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(codec: PureCodec<ReplicaBatch, JsonValue>, policy: ParsePolicy) -> Self {
        Self { codec, policy }
    }

    /// Export through the journal authority and protect the bounded page.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasEventJournal + HasReplicaProtection + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `session` (`ReplicaSessionId`): Scoped connection or stream session retained until settlement.
    /// - `source` (`ReplicaId`): Underlying provider or input from which values are read.
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    /// - `after` (`JournalCursor`): Cursor or instant after which results are selected.
    /// - `limit` (`NonZeroUsize`): The caller-supplied limit.
    ///
    /// # Returns
    /// Type: `ProtectedExportEffect<R>`.
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
        {
            let __dsl_program = effect_map_error! { program: EventReplicator::export::<R>(source, stream, after, limit), map: ProtectedReplicaExportError::Journal, };
            effect! {
            let __dsl_program = __dsl_program.clone(); let session = session.clone(); let transfer = transfer.clone(); let page = __dsl_program?; match page {
                ReplicaExport::Batch { batch, has_more } => effect_map! {
                    program: effect_map_error! {
                    program: transfer
                    .seal::<R>(session.clone(), batch), map: ProtectedReplicaExportError::Transfer, }, map: move |payload| ProtectedReplicaExport::Payload { payload, has_more }, },
                ReplicaExport::HistoryTruncated { first_available } => effect_sync! {
                    body: move || {
                    ProtectedReplicaExport::HistoryTruncated { first_available }
                }, },
                ReplicaExport::CursorAhead { head } => {
                    effect_sync! { body: move || ProtectedReplicaExport::CursorAhead { head }, }
                }
            }? }
        }
    }

    /// Lazily open and authenticate a protected replica batch, then import and acknowledge it.
    ///
    /// Protection and parse policy are applied before creating the source/stream-bound
    /// import permit. Authentication failure performs no journal or cursor access;
    /// ordinary authorized import retains its deduplication and cursor-fencing behavior.
    pub fn import_and_ack<R>(
        &self,
        expected_session: ReplicaSessionId,
        payload: ProtectedReplicaPayload,
    ) -> ProtectedImportEffect<R>
    where
        R: HasReplicaProtection + HasEventJournal + HasReplicaCursorStore + Send + Sync + 'static,
    {
        {
            let __dsl_program = effect_map_error! { program: self.open::<R>(expected_session, payload), map: ProtectedReplicaImportError::Transfer, };
            effect! {
            let batch = __dsl_program.clone()?; {
                effect_map_error! {
                    program: {
                    let permit = super::ReplicaImportPermit::authenticated(&batch);
                    EventReplicator::import_and_ack::<R>(batch, permit)
                }, map: ProtectedReplicaImportError::Sync, }
            }? }
        }
    }

    /// Protect encoded replica bytes with the selected session metadata.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasReplicaProtection + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `session` (`ReplicaSessionId`): Scoped connection or stream session retained until settlement.
    /// - `batch` (`ReplicaBatch`): Ordered group of values processed together.
    ///
    /// # Returns
    /// Type: `Effect<ProtectedReplicaPayload, ReplicaTransferError<<R::Protection as ReplicaProtection>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
        {
            let __dsl_program = effect_try_sync! {
            body: move || {
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
            }, on_error: std::convert::identity, };
            effect! {
                    let session = session.clone(); let bytes = __dsl_program.clone()?; {
                {
                    let __dsl_program = service! { env: |env: &R| env.replica_protection(), }; let session = session.clone(); effect! {
                    let __dsl_program = __dsl_program.clone(); let bytes = bytes.clone(); let session = session.clone(); let protection = __dsl_program?; {
                        let expected = session.clone();
                        {
                            let __dsl_program = effect_provide_some! {
                            program: effect_map_error! {
                            program: protection
                            .seal(session.clone(), Arc::clone(&bytes)), map: ReplicaTransferError::Protection, }, project: |_: &R| (), }; effect! {
                                let expected = expected.clone(); let payload = __dsl_program.clone()?; {
                                effect_try_sync! {
                                    once; body: {
                                    let __dsl_result = if payload.session == expected {
                                    Ok(payload)
                                } else {
                                    Err(ReplicaTransferError::SessionMismatch)
                                }; move || __dsl_result }, on_error: ::core::convert::identity, }
                            }? } }
                    }? } }
            }? }
        }
    }

    /// Open the resource within its declared scope and ownership boundary.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasReplicaProtection + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `expected` (`ReplicaSessionId`): Value or revision required for the proposed state transition.
    /// - `payload` (`ProtectedReplicaPayload`): Application data moved into the message or stored record.
    ///
    /// # Returns
    /// Type: `Effect<ReplicaBatch, ReplicaTransferError<<R::Protection as ReplicaProtection>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn open<R>(
        &self,
        expected: ReplicaSessionId,
        payload: ProtectedReplicaPayload,
    ) -> Effect<ReplicaBatch, ReplicaTransferError<<R::Protection as ReplicaProtection>::Error>, R>
    where
        R: HasReplicaProtection + Send + Sync + 'static,
    {
        if payload.session != expected {
            return effect_try_sync! { body: { let __dsl_error = || ReplicaTransferError::SessionMismatch; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
        }
        let payload = Mutex::new(Some(payload));
        let codec = self.codec.clone();
        let policy = self.policy;
        {
            let __dsl_program = effect! {
            bind;
            program: service! { env: |env: &R| env.replica_protection(), },
            body: move |protection| {
                           let payload = payload
                               .lock()
                               .unwrap_or_else(std::sync::PoisonError::into_inner)
                               .take();
                           match payload {
                               Some(payload) => effect_provide_some! {
                                   program: effect_map_error! {
                                   program: protection
                                   .open(payload), map: ReplicaTransferError::Protection, }, project: |_: &R| (), },
                               None => effect_try_sync! { once; body: { let __dsl_result = Err(ReplicaTransferError::InputConsumed); move || __dsl_result }, on_error: ::core::convert::identity, },
                           }
                       },
            };
            effect! {
            let codec = codec.clone(); let bytes = __dsl_program.clone()?; {
                       let codec = codec.clone();
                       effect_try_sync! {
                           body: move || {
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
                           }, on_error: std::convert::identity, }
                   }? }
        }
    }
}
