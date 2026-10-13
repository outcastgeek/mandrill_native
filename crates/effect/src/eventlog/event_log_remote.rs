//! Transport-neutral replica exchange.

use super::event_journal::validate_journal_read;
use super::{
    AppendDecision, AppendReceipt, EventId, EventInspection, EventJournal, HasEventJournal,
    HasReplicaCursorStore, JournalCursor, JournalEntry, JournalReadOutcome, JournalReadViolation,
    JournalStream, ReplicaCursorAdvance, ReplicaCursorKey, ReplicaCursorStore,
};
use crate::{Effect, KeyDomain, StableKey};
use crate::{
    effect, effect_map, effect_map_error, effect_provide_some, effect_sync, effect_try_sync,
    service,
};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// An application authorization decision bound to one replica source and journal stream.
///
/// Construct it before import. Import verifies this identity before journal or
/// cursor access; possessing an unprotected batch alone does not authorize it.
/// Clones retain the same decision and do not re-run the application's check.
#[derive(Clone)]
pub struct ReplicaImportPermit {
    source: ReplicaId,
    stream: JournalStream,
}
impl ReplicaImportPermit {
    /// Apply the supplied authorization check eagerly and retain its accepted source/stream.
    ///
    /// A rejection is returned unchanged as E before any journal or cursor access.
    /// The application defines the authority; this method adds no authentication
    /// mechanism and the resulting permit is valid only for matching batch identities.
    pub fn authorize<E>(
        source: ReplicaId,
        stream: JournalStream,
        check: impl FnOnce(&ReplicaId, &JournalStream) -> Result<(), E>,
    ) -> Result<Self, E> {
        check(&source, &stream)?;
        Ok(Self { source, stream })
    }
    pub(crate) fn authenticated(batch: &ReplicaBatch) -> Self {
        Self {
            source: batch.source.clone(),
            stream: batch.stream.clone(),
        }
    }
    fn accepts(&self, batch: &ReplicaBatch) -> bool {
        self.source == batch.source && self.stream == batch.stream
    }
}

/// Type alias for journal error in eventlog event log remote.
///
/// # Type parameters
/// - `R`: Environment type.
type JournalError<R> = <<R as HasEventJournal>::Journal as EventJournal>::Error;
/// Type alias for cursor error in eventlog event log remote.
///
/// # Type parameters
/// - `R`: Environment type.
type CursorError<R> = <<R as HasReplicaCursorStore>::Store as ReplicaCursorStore>::Error;
/// Type alias for replica sync effect in eventlog event log remote.
///
/// # Type parameters
/// - `R`: Environment type.
pub type ReplicaSyncEffect<R> =
    Effect<ReplicaSync, ReplicaSyncError<JournalError<R>, CursorError<R>>, R>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by eventlog event log remote operations.
pub struct ReplicaId(Arc<[u8]>);

impl ReplicaId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone)]
/// An ordered group of journal entries transferred between replicas.
pub struct ReplicaBatch {
    source: ReplicaId,
    stream: JournalStream,
    after: JournalCursor,
    entries: Arc<[JournalEntry]>,
    maximum_entries: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A stream, sequence, or size violation in a replica batch.
pub enum ReplicaBatchError {
    /// The replica batch exceeds the caller’s entry bound.
    TooManyEntries,
    /// The supplied stream does not match the required or recorded value.
    WrongStream,
    /// Replica entries contain a gap in their expected sequence.
    NonContiguous,
    /// No further sequence can be allocated without exceeding its range.
    SequenceExhausted,
}

impl ReplicaBatch {
    /// Construct a `ReplicaBatch` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `source` (`ReplicaId`): Underlying provider or input from which values are read.
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    /// - `after` (`JournalCursor`): Cursor or instant after which results are selected.
    /// - `entries` (`Arc<[JournalEntry]>`): The entries supplied to this operation.
    /// - `maximum_entries` (`NonZeroUsize`): The maximum entries supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, ReplicaBatchError>`.
    pub fn new(
        source: ReplicaId,
        stream: JournalStream,
        after: JournalCursor,
        entries: Arc<[JournalEntry]>,
        maximum_entries: NonZeroUsize,
    ) -> Result<Self, ReplicaBatchError> {
        if entries.len() > maximum_entries.get() {
            return Err(ReplicaBatchError::TooManyEntries);
        }
        let mut previous = after.value();
        for entry in entries.iter() {
            if entry.event.stream() != stream {
                return Err(ReplicaBatchError::WrongStream);
            }
            let next = previous
                .checked_add(1)
                .ok_or(ReplicaBatchError::SequenceExhausted)?;
            if entry.sequence.get() != next {
                return Err(ReplicaBatchError::NonContiguous);
            }
            previous = next;
        }
        Ok(Self {
            source,
            stream,
            after,
            entries,
            maximum_entries,
        })
    }

    #[must_use]
    /// Borrow the journal or replica source used by this transfer.
    ///
    /// # Returns
    /// Type: `&ReplicaId`.
    pub fn source(&self) -> &ReplicaId {
        &self.source
    }
    #[must_use]
    /// Return the stream associated with this value.
    ///
    /// # Returns
    /// Type: `&JournalStream`.
    pub fn stream(&self) -> &JournalStream {
        &self.stream
    }
    #[must_use]
    /// # Returns
    /// Type: `JournalCursor`.
    pub const fn after(&self) -> JournalCursor {
        self.after
    }
    #[must_use]
    /// Borrow the ordered entries carried by this replica batch.
    ///
    /// # Returns
    /// Type: `&[JournalEntry]`.
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }
    #[must_use]
    /// # Returns
    /// Type: `NonZeroUsize`.
    pub const fn maximum_entries(&self) -> NonZeroUsize {
        self.maximum_entries
    }
}

/// The accepted entries and cursor produced by importing a replica batch.
pub enum ReplicaImport {
    /// The proposed state transition was committed.
    Applied {
        /// The position from which reading or replay resumes. Type: `JournalCursor`.
        cursor: JournalCursor,
        /// The number of recorded items or operations. Type: `usize`.
        count: usize,
    },
    /// The request was accepted but has not completed.
    AcceptedPending {
        /// Event associated with this record. Type: `EventId`.
        event: EventId,
        /// The position from which reading or replay resumes. Type: `JournalCursor`.
        cursor: JournalCursor,
    },
    /// The local time does not exist because the clock moved forward.
    Gap {
        /// The value required by the requested condition. Type: `JournalCursor`.
        expected: JournalCursor,
        /// The value observed when the requested condition failed. Type: `JournalCursor`.
        actual: JournalCursor,
    },
    /// Replay produced a result incompatible with the recorded history.
    Diverged {
        /// The value required by the requested condition. Type: `JournalCursor`.
        expected: JournalCursor,
        /// The value observed when the requested condition failed. Type: `JournalCursor`.
        actual: JournalCursor,
    },
    /// An event with the same identity has incompatible content.
    ConflictingEvent {
        /// Event associated with this record. Type: `EventId`.
        event: EventId,
    },
    /// The requested event was removed by authorized compaction.
    CompactedEvent {
        /// Event associated with this record. Type: `EventId`.
        event: EventId,
    },
    /// The batch failed validation.
    InvalidBatch,
}

/// A retained-history conflict requiring an application decision. Handling a
/// conflict does not itself append events or acknowledge a replica cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplicaConflict {
    /// The cursor conflicts with recorded state.
    CursorConflict {
        /// The value observed when the requested condition failed. Type: `JournalCursor`.
        actual: JournalCursor,
    },
    /// The local time does not exist because the clock moved forward.
    Gap {
        /// The value required by the requested condition. Type: `JournalCursor`.
        expected: JournalCursor,
        /// The value observed when the requested condition failed. Type: `JournalCursor`.
        actual: JournalCursor,
    },
    /// Replay produced a result incompatible with the recorded history.
    Diverged {
        /// The value required by the requested condition. Type: `JournalCursor`.
        expected: JournalCursor,
        /// The value observed when the requested condition failed. Type: `JournalCursor`.
        actual: JournalCursor,
    },
    /// An event with the same identity has incompatible content.
    ConflictingEvent {
        /// Event associated with this record. Type: `EventId`.
        event: EventId,
    },
    /// The requested event was removed by authorized compaction.
    CompactedEvent {
        /// Event associated with this record. Type: `EventId`.
        event: EventId,
    },
}

impl ReplicaImport {
    #[must_use]
    /// Report a conflicting identity or state transition as a typed result.
    ///
    /// # Returns
    /// Type: `Option<ReplicaConflict>`.
    /// Returns the value when present, or `None`.
    pub fn conflict(&self) -> Option<ReplicaConflict> {
        match self {
            Self::Gap { expected, actual } => Some(ReplicaConflict::Gap {
                expected: *expected,
                actual: *actual,
            }),
            Self::Diverged { expected, actual } => Some(ReplicaConflict::Diverged {
                expected: *expected,
                actual: *actual,
            }),
            Self::ConflictingEvent { event } => Some(ReplicaConflict::ConflictingEvent {
                event: event.clone(),
            }),
            Self::CompactedEvent { event } => Some(ReplicaConflict::CompactedEvent {
                event: event.clone(),
            }),
            Self::Applied { .. } | Self::AcceptedPending { .. } | Self::InvalidBatch => None,
        }
    }
}

/// Application-owned response to a replica conflict. The offered batch keeps
/// its source, stream, cursor and encoded entries available for inspection.
/// The handler may inspect external authority or initiate reconciliation, but a
/// successful return is only an acknowledgement that it handled the signal.
/// The journal and replica cursor remain unchanged by this capability.
pub trait ReplicaConflictHandler: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;

    /// # Parameters
    /// - `batch` (`ReplicaBatch`): Ordered group of values processed together.
    /// - `conflict` (`ReplicaConflict`): The conflict supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn handle(&self, batch: ReplicaBatch, conflict: ReplicaConflict) -> Effect<(), Self::Error>;
}

/// Selects the application policy for conflicting replica histories.
pub trait HasReplicaConflictHandler {
    /// Conflict handler selected by this environment.
    type Handler: ReplicaConflictHandler;

    /// # Returns
    /// Type: `Arc<Self::Handler>`.
    fn replica_conflict_handler(&self) -> Arc<Self::Handler>;
}

/// A bounded batch and continuation cursor prepared for another replica.
pub enum ReplicaExport {
    /// A bounded page of replica entries is available.
    Batch {
        /// Entries and cursor metadata in this page. Type: `ReplicaBatch`.
        batch: ReplicaBatch,
        /// Whether another page follows. Type: `bool`.
        has_more: bool,
    },
    /// Required journal history has already been compacted.
    HistoryTruncated {
        /// Earliest cursor still available for transfer. Type: `JournalCursor`.
        first_available: JournalCursor,
    },
    /// The requested replica cursor is beyond the journal head.
    CursorAhead {
        /// Current authoritative journal head. Type: `JournalCursor`.
        head: JournalCursor,
    },
}

#[derive(Debug)]
/// A journal or encoding failure while producing a replica batch.
///
/// # Type parameters
/// - `JournalError`: Error returned by the event journal.
pub enum ReplicaExportError<JournalError> {
    /// The authoritative event journal rejected or failed the operation. Retains the `JournalError` detail.
    Journal(JournalError),
    /// The journal read failed validation. Carries `JournalReadViolation`.
    InvalidJournalRead(JournalReadViolation),
    /// The journal batch failed validation. Carries `ReplicaBatchError`.
    InvalidJournalBatch(ReplicaBatchError),
}

#[derive(Debug)]
/// A sequence, identity, or journal failure while applying a replica batch.
///
/// # Type parameters
/// - `JournalError`: Error returned by the event journal.
pub enum ReplicaImportError<JournalError> {
    /// The permit does not authorize this batch's replica source and stream; no journal or cursor access is allowed.
    Unauthorized,
    /// The append may have committed; the journal outcome could not be confirmed.
    UncertainAppend {
        /// Event associated with this record. Type: `EventId`.
        event: EventId,
        /// The underlying failure or provider cause. Type: `JournalError`.
        source: JournalError,
    },
    /// Inspection could not establish whether the operation committed.
    UncertainInspection {
        /// Event associated with this record. Type: `EventId`.
        event: EventId,
        /// The underlying failure or provider cause. Type: `JournalError`.
        source: JournalError,
    },
}

/// `count` is the number of entries processed by this call. An already
/// acknowledged page returns zero without replaying its entries.
pub enum ReplicaSync {
    /// Replica entries were accepted and the cursor advanced.
    Applied {
        /// Cursor after accepted import. Type: `JournalCursor`.
        cursor: JournalCursor,
        /// Number of entries accepted. Type: `usize`.
        count: usize,
    },
    /// The operation has not completed. Carries `ReplicaImport`.
    Pending(ReplicaImport),
    /// Replica cursor state changed before this update committed.
    CursorConflict {
        /// Cursor currently recorded by the destination. Type: `JournalCursor`.
        actual: JournalCursor,
    },
}

impl ReplicaSync {
    #[must_use]
    /// Report a conflicting identity or state transition as a typed result.
    ///
    /// # Returns
    /// Type: `Option<ReplicaConflict>`.
    /// Returns the value when present, or `None`.
    pub fn conflict(&self) -> Option<ReplicaConflict> {
        match self {
            Self::Pending(result) => result.conflict(),
            Self::CursorConflict { actual } => {
                Some(ReplicaConflict::CursorConflict { actual: *actual })
            }
            Self::Applied { .. } => None,
        }
    }
}

#[derive(Debug)]
/// A transport, protection, or journal failure during replica synchronization.
///
/// # Type parameters
/// - `JournalError`: Error returned by the event journal.
/// - `CursorError`: Error returned by the replica cursor store.
pub enum ReplicaSyncError<JournalError, CursorError> {
    /// Importing the replica batch failed. Retains the `ReplicaImportError<JournalError>` detail.
    Import(ReplicaImportError<JournalError>),
    /// The requested cursor cannot be read from retained history.
    CursorRead {
        /// Key used to address the stored value. Type: `ReplicaCursorKey`.
        key: ReplicaCursorKey,
        /// The underlying failure or provider cause. Type: `CursorError`.
        source: CursorError,
    },
    /// Events were imported, and the cursor update may have committed before
    /// its acknowledgement was lost. Reload this key before retrying.
    UncertainCursorAdvance {
        /// Key used to address the stored value. Type: `ReplicaCursorKey`.
        key: ReplicaCursorKey,
        /// The value required by the requested condition. Type: `JournalCursor`.
        expected: JournalCursor,
        /// Next position after the current record. Type: `JournalCursor`.
        next: JournalCursor,
        /// The underlying failure or provider cause. Type: `CursorError`.
        source: CursorError,
    },
}

/// Coordinates bounded export and import between event journal replicas.
pub struct EventReplicator;

impl EventReplicator {
    /// Dispatch an already observed conflict to the handler supplied through
    /// `R`. No cursor advancement or implicit merge follows the callback.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasReplicaConflictHandler + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `batch` (`ReplicaBatch`): Ordered group of values processed together.
    /// - `conflict` (`ReplicaConflict`): The conflict supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), <<R as HasReplicaConflictHandler>::Handler as ReplicaConflictHandler>::Error, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn handle_conflict<R>(
        batch: ReplicaBatch,
        conflict: ReplicaConflict,
    ) -> Effect<(), <<R as HasReplicaConflictHandler>::Handler as ReplicaConflictHandler>::Error, R>
    where
        R: HasReplicaConflictHandler + Send + Sync + 'static,
    {
        effect! {
        bind;
        program: service! { env: |env: &R| env.replica_conflict_handler(), },
        body: move |handler| {
                   effect_provide_some! {
                       program: handler
                       .handle(batch.clone(), conflict.clone()), project: |_: &R| (), }
               },
        }
    }

    /// Read one bounded page from a source journal. The receiver imports it
    /// using the same stable sequence and event identities.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasEventJournal + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `source` (`ReplicaId`): Underlying provider or input from which values are read.
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    /// - `after` (`JournalCursor`): Cursor or instant after which results are selected.
    /// - `limit` (`NonZeroUsize`): The caller-supplied limit.
    ///
    /// # Returns
    /// Type: `Effect<ReplicaExport, ReplicaExportError<<R::Journal as EventJournal>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn export<R>(
        source: ReplicaId,
        stream: JournalStream,
        after: JournalCursor,
        limit: NonZeroUsize,
    ) -> Effect<ReplicaExport, ReplicaExportError<<R::Journal as EventJournal>::Error>, R>
    where
        R: HasEventJournal + Send + Sync + 'static,
    {
        effect! {
            bind;
            program: service! { env: |env: &R| env.event_journal(), },
            body: move |journal| {
            let source = source.clone();
            let stream = stream.clone();
            effect! {
                bind;
                program: effect_provide_some! {
                    program: effect_map_error! {
                    program: journal
                    .read(stream.clone(), after, limit), map: ReplicaExportError::Journal, }, project: |_: &R| (), },
                body: move |read| match read {
                    JournalReadOutcome::HistoryTruncated { first_available } => {
                        effect_sync! { body: move || ReplicaExport::HistoryTruncated { first_available }, }
                    }
                    JournalReadOutcome::CursorAhead { head } => {
                        effect_sync! { body: move || ReplicaExport::CursorAhead { head }, }
                    }
                    JournalReadOutcome::Complete(read) => {
                        if let Err(issue) = validate_journal_read(&read, &stream, after, limit) {
                            return effect_try_sync! {
                                once; body: {
                                let __dsl_result = Err(
                                ReplicaExportError::InvalidJournalRead(issue),
                            ); move || __dsl_result }, on_error: ::core::convert::identity, };
                        }
                        let batch = ReplicaBatch::new(
                            source.clone(),
                            stream.clone(),
                            after,
                            Arc::from(read.entries),
                            limit,
                        )
                        .map_err(ReplicaExportError::InvalidJournalBatch);
                        effect_try_sync! {
                            once; body: {
                            let __dsl_result = batch.map(|batch| ReplicaExport::Batch {
                            batch,
                            has_more: read.has_more,
                        }); move || __dsl_result }, on_error: ::core::convert::identity, }
                    }
                },
            }
            },
        }
    }

    /// Lazily import an authorized bounded batch and conditionally advance its source cursor.
    ///
    /// The permit must match source and stream before any journal or cursor access.
    /// An already acknowledged page returns zero imported entries without replaying
    /// it. Cursor/history conflicts retain authoritative state; journal event IDs
    /// deduplicate retries, and cursor acknowledgement follows successful import.
    pub fn import_and_ack<R>(
        batch: ReplicaBatch,
        permit: ReplicaImportPermit,
    ) -> ReplicaSyncEffect<R>
    where
        R: HasEventJournal + HasReplicaCursorStore + Send + Sync + 'static,
    {
        if !permit.accepts(&batch) {
            return effect_try_sync! {
                once; body: {
                let __dsl_result = Err(ReplicaSyncError::Import(
                ReplicaImportError::Unauthorized,
            )); move || __dsl_result }, on_error: ::core::convert::identity, };
        }
        let key = ReplicaCursorKey {
            source: batch.source.clone(),
            stream: batch.stream.clone(),
        };
        let expected = batch.after;
        let last = batch
            .entries
            .last()
            .map(|entry| JournalCursor::new(entry.sequence.get()))
            .unwrap_or(expected);
        effect! {
        bind;
        program: service! { env: |env: &R| env.replica_cursor_store(), },
        body: move |store| {
                   let key_for_read = key.clone();
                   let key_for_import = key.clone();
                   let batch = batch.clone();
                   let permit = permit.clone();
                   effect! {
        bind;
        program: effect_provide_some! {
            program: effect_map_error! {
            program: store
                       .load(key.clone()), map: move |source| ReplicaSyncError::CursorRead {
                           key: key_for_read.clone(),
                           source,
                       }, }, project: |_: &R| (), },
        body: move |current| {
                           if current == last {
                               return effect_sync! {
                                   body: move || ReplicaSync::Applied {
                                   cursor: current,
                                   count: usize::default(),
                               }, };
                           }
                           if current != expected {
                               return effect_sync! {
                                   body: move || ReplicaSync::CursorConflict {
                                   actual: current,
                               }, };
                           }
                           { let __dsl_program = effect_map_error! { program: Self::import::<R>(batch.clone(), permit.clone()), map: ReplicaSyncError::Import, }; let key = key_for_import.clone(); effect! { let __dsl_program = __dsl_program.clone(); let key = key.clone(); let imported = __dsl_program?; acknowledge_import::<R>(key.clone(), expected, imported)? } }
                       },
        }
               },
        }
    }

    /// Lazily import an authorized ordered batch into the existing journal authority.
    ///
    /// Rejects a mismatched permit before store access. Event IDs deduplicate retries;
    /// a gap or conflicting history prevents admitting later entries. This operation
    /// does not advance the replica cursor; acknowledge only after inspecting its result.
    pub fn import<R>(
        batch: ReplicaBatch,
        permit: ReplicaImportPermit,
    ) -> Effect<ReplicaImport, ReplicaImportError<<R::Journal as EventJournal>::Error>, R>
    where
        R: HasEventJournal + Send + Sync + 'static,
    {
        if !permit.accepts(&batch) {
            return effect_try_sync! { once; body: { let __dsl_result = Err(ReplicaImportError::Unauthorized); move || __dsl_result }, on_error: ::core::convert::identity, };
        }
        if batch.entries.len() > batch.maximum_entries.get()
            || batch
                .entries
                .iter()
                .any(|entry| entry.event.stream() != batch.stream)
        {
            return effect_sync! { body: || ReplicaImport::InvalidBatch, };
        }
        let mut expected = batch.after.value();
        for entry in batch.entries.iter() {
            let Some(next) = expected.checked_add(1) else {
                return effect_sync! { body: || ReplicaImport::InvalidBatch, };
            };
            if entry.sequence.get() != next {
                return effect_sync! { body: || ReplicaImport::InvalidBatch, };
            }
            expected = next;
        }
        effect! {
        bind;
        program: service! { env: |env: &R| env.event_journal(), },
        body: move |journal| {
                   import_step(journal, Arc::clone(&batch.entries), 0, batch.after, 0)
               },
        }
    }
}

///
/// # Type parameters
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasReplicaCursorStore + HasEventJournal + Send + Sync + 'static`.
///
/// # Parameters
/// - `key` (`ReplicaCursorKey`): Stable key used to locate or identify the selected value.
/// - `expected` (`JournalCursor`): Value or revision required for the proposed state transition.
/// - `imported` (`ReplicaImport`): The imported supplied to this operation.
///
/// # Returns
/// Type: `ReplicaSyncEffect<R>`.
/// Returns `ReplicaSyncEffect<R>`.
fn acknowledge_import<R>(
    key: ReplicaCursorKey,
    expected: JournalCursor,
    imported: ReplicaImport,
) -> ReplicaSyncEffect<R>
where
    R: HasReplicaCursorStore + HasEventJournal + Send + Sync + 'static,
{
    if let ReplicaImport::Applied { cursor, count } = imported {
        {
            let __dsl_program = service! { env: |env: &R| env.replica_cursor_store(), };
            effect! {
                let __dsl_program = __dsl_program.clone(); let key = key.clone(); let store = __dsl_program?; {
                effect_map! {
                    program: effect_provide_some! {
                    program: effect_map_error! {
                    program: store
                    .advance(key.clone(), expected, cursor), map: {
                        let error_key = key.clone();
                        move |source| ReplicaSyncError::UncertainCursorAdvance {
                            key: error_key.clone(),
                            expected,
                            next: cursor,
                            source,
                        }
                    }, }, project: |_: &R| (), }, map: move |result| match result {
                        ReplicaCursorAdvance::Advanced(cursor)
                        | ReplicaCursorAdvance::AlreadyAdvanced(cursor) => {
                            ReplicaSync::Applied { cursor, count }
                        }
                        ReplicaCursorAdvance::Conflict(actual) => {
                            ReplicaSync::CursorConflict { actual }
                        }
                    }, }
            }? }
        }
    } else {
        effect_sync! { once; body: move || ReplicaSync::Pending(imported), }
    }
}

///
/// # Type parameters
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasEventJournal + Send + Sync + 'static`.
///
/// # Parameters
/// - `journal` (`Arc<R::Journal>`): The journal supplied to this operation.
/// - `entries` (`Arc<[JournalEntry]>`): The entries supplied to this operation.
/// - `index` (`usize`): Position of the selected item in its ordered collection.
/// - `cursor` (`JournalCursor`): The cursor supplied to this operation.
/// - `count` (`usize`): Caller-selected number of iterations, values, or permits.
///
/// # Returns
/// Type: `Effect<ReplicaImport, ReplicaImportError<<R::Journal as EventJournal>::Error>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn import_step<R>(
    journal: Arc<R::Journal>,
    entries: Arc<[JournalEntry]>,
    index: usize,
    cursor: JournalCursor,
    count: usize,
) -> Effect<ReplicaImport, ReplicaImportError<<R::Journal as EventJournal>::Error>, R>
where
    R: HasEventJournal + Send + Sync + 'static,
{
    let Some(entry) = entries.get(index).cloned() else {
        return effect_sync! { body: move || ReplicaImport::Applied { cursor, count }, };
    };
    let predecessor = JournalCursor::new(entry.sequence.get() - 1);
    let event_id = entry.event.id.clone();
    effect! {
    bind;
    program: effect_provide_some! {
        program: effect_map_error! {
        program: journal
           .append(entry.event.clone(), predecessor), map: move |source| ReplicaImportError::UncertainAppend {
               event: event_id.clone(),
               source,
           }, }, project: |_: &R| (), },
    body: move |receipt| match receipt {
               AppendReceipt::Accepted { event } => {
                   if event != entry.event.id {
                       let event = entry.event.id.clone();
                       effect_sync! { once; body: move || ReplicaImport::ConflictingEvent { event }, }
                   } else {
                       let inspect_event = event.clone();
                       let journal_for_inspection = Arc::clone(&journal);
                       let entries_for_inspection = Arc::clone(&entries);
                       let entry_for_inspection = entry.clone();
                       effect! {
    bind;
    program: effect_provide_some! {
        program: effect_map_error! {
        program: journal
                           .inspect(entry.event.stream(), event.clone()), map: move |source| ReplicaImportError::UncertainInspection {
                               event: inspect_event.clone(),
                               source,
                           }, }, project: |_: &R| (), },
    body: move |inspection| match inspection {
                               EventInspection::Present(actual) => continue_import::<R>(
                                   Arc::clone(&journal_for_inspection),
                                   Arc::clone(&entries_for_inspection),
                                   index,
                                   count,
                                   entry_for_inspection.clone(),
                                   actual,
                               ),
                               EventInspection::Compacted => {
                                   let event = event.clone();
                                   effect_sync! { once; body: move || ReplicaImport::CompactedEvent { event }, }
                               }
                               EventInspection::Missing => {
                                   let event = event.clone();
                                   effect_sync! {
                                       once; body: move || ReplicaImport::AcceptedPending {
                                       event,
                                       cursor,
                                   }, }
                               }
                           },
    }
                   }
               }
               AppendReceipt::Decided(AppendDecision::Committed(actual))
               | AppendReceipt::Decided(AppendDecision::Duplicate(actual)) => continue_import::<R>(
                   Arc::clone(&journal),
                   Arc::clone(&entries),
                   index,
                   count,
                   entry.clone(),
                   actual,
               ),
               AppendReceipt::Decided(AppendDecision::ConflictingEventId) => {
                   let event = entry.event.id.clone();
                   effect_sync! { once; body: move || ReplicaImport::ConflictingEvent { event }, }
               }
               AppendReceipt::Decided(AppendDecision::CompactedEventId) => {
                   let event = entry.event.id.clone();
                   effect_sync! { once; body: move || ReplicaImport::CompactedEvent { event }, }
               }
               AppendReceipt::Decided(AppendDecision::StaleCursor { actual }) => {
                   if actual.value() < predecessor.value() {
                       effect_sync! {
                           body: move || ReplicaImport::Gap {
                           expected: predecessor,
                           actual,
                       }, }
                   } else {
                       effect_sync! {
                           body: move || ReplicaImport::Diverged {
                           expected: predecessor,
                           actual,
                       }, }
                   }
               }
           },
    }
}

///
/// # Type parameters
/// - `R`: Environment type supplying required capabilities. Additional bound: `HasEventJournal + Send + Sync + 'static`.
///
/// # Parameters
/// - `journal` (`Arc<R::Journal>`): The journal supplied to this operation.
/// - `entries` (`Arc<[JournalEntry]>`): The entries supplied to this operation.
/// - `index` (`usize`): Position of the selected item in its ordered collection.
/// - `count` (`usize`): Caller-selected number of iterations, values, or permits.
/// - `expected` (`JournalEntry`): Value or revision required for the proposed state transition.
/// - `actual` (`JournalEntry`): The actual supplied to this operation.
///
/// # Returns
/// Type: `Effect<ReplicaImport, ReplicaImportError<<R::Journal as EventJournal>::Error>, R>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn continue_import<R>(
    journal: Arc<R::Journal>,
    entries: Arc<[JournalEntry]>,
    index: usize,
    count: usize,
    expected: JournalEntry,
    actual: JournalEntry,
) -> Effect<ReplicaImport, ReplicaImportError<<R::Journal as EventJournal>::Error>, R>
where
    R: HasEventJournal + Send + Sync + 'static,
{
    if actual.event != expected.event {
        let event = expected.event.id.clone();
        return effect_sync! { once; body: move || ReplicaImport::ConflictingEvent { event }, };
    }
    if actual.sequence != expected.sequence {
        let predecessor = JournalCursor::new(expected.sequence.get() - 1);
        return effect_sync! {
            body: move || ReplicaImport::Diverged {
            expected: predecessor,
            actual: JournalCursor::new(actual.sequence.get()),
        }, };
    }
    let (Some(next_index), Some(next_count)) = (index.checked_add(1), count.checked_add(1)) else {
        return effect_sync! { body: || ReplicaImport::InvalidBatch, };
    };
    import_step::<R>(
        journal,
        entries,
        next_index,
        JournalCursor::new(actual.sequence.get()),
        next_count,
    )
}
