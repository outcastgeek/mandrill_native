use super::event_journal::validate_journal_read;
use super::{
    AppendDecision, AppendReceipt, EventId, EventInspection, EventJournal, HasEventJournal,
    HasReplicaCursorStore, JournalCursor, JournalEntry, JournalReadOutcome, JournalReadViolation,
    JournalStream, ReplicaCursorAdvance, ReplicaCursorKey, ReplicaCursorStore,
};
use crate::{Effect, KeyDomain, StableKey};
use std::num::NonZeroUsize;
use std::sync::Arc;

type JournalError<R> = <<R as HasEventJournal>::Journal as EventJournal>::Error;
type CursorError<R> = <<R as HasReplicaCursorStore>::Store as ReplicaCursorStore>::Error;
pub type ReplicaSyncEffect<R> =
    Effect<ReplicaSync, ReplicaSyncError<JournalError<R>, CursorError<R>>, R>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicaId(Arc<[u8]>);

impl ReplicaId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone)]
pub struct ReplicaBatch {
    source: ReplicaId,
    stream: JournalStream,
    after: JournalCursor,
    entries: Arc<[JournalEntry]>,
    maximum_entries: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaBatchError {
    TooManyEntries,
    WrongStream,
    NonContiguous,
    SequenceExhausted,
}

impl ReplicaBatch {
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
    pub fn source(&self) -> &ReplicaId {
        &self.source
    }
    #[must_use]
    pub fn stream(&self) -> &JournalStream {
        &self.stream
    }
    #[must_use]
    pub const fn after(&self) -> JournalCursor {
        self.after
    }
    #[must_use]
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }
    #[must_use]
    pub const fn maximum_entries(&self) -> NonZeroUsize {
        self.maximum_entries
    }
}

pub enum ReplicaImport {
    Applied {
        cursor: JournalCursor,
        count: usize,
    },
    AcceptedPending {
        event: EventId,
        cursor: JournalCursor,
    },
    Gap {
        expected: JournalCursor,
        actual: JournalCursor,
    },
    Diverged {
        expected: JournalCursor,
        actual: JournalCursor,
    },
    ConflictingEvent {
        event: EventId,
    },
    CompactedEvent {
        event: EventId,
    },
    InvalidBatch,
}

/// A retained-history conflict requiring an application decision. Handling a
/// conflict does not itself append events or acknowledge a replica cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplicaConflict {
    CursorConflict {
        actual: JournalCursor,
    },
    Gap {
        expected: JournalCursor,
        actual: JournalCursor,
    },
    Diverged {
        expected: JournalCursor,
        actual: JournalCursor,
    },
    ConflictingEvent {
        event: EventId,
    },
    CompactedEvent {
        event: EventId,
    },
}

impl ReplicaImport {
    #[must_use]
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
    type Error: Send + 'static;

    fn handle(&self, batch: ReplicaBatch, conflict: ReplicaConflict) -> Effect<(), Self::Error>;
}

pub trait HasReplicaConflictHandler {
    type Handler: ReplicaConflictHandler;

    fn replica_conflict_handler(&self) -> Arc<Self::Handler>;
}

pub enum ReplicaExport {
    Batch { batch: ReplicaBatch, has_more: bool },
    HistoryTruncated { first_available: JournalCursor },
    CursorAhead { head: JournalCursor },
}

#[derive(Debug)]
pub enum ReplicaExportError<JournalError> {
    Journal(JournalError),
    InvalidJournalRead(JournalReadViolation),
    InvalidJournalBatch(ReplicaBatchError),
}

#[derive(Debug)]
pub enum ReplicaImportError<JournalError> {
    UncertainAppend {
        event: EventId,
        source: JournalError,
    },
    UncertainInspection {
        event: EventId,
        source: JournalError,
    },
}

/// `count` is the number of entries processed by this call. An already
/// acknowledged page returns zero without replaying its entries.
pub enum ReplicaSync {
    Applied { cursor: JournalCursor, count: usize },
    Pending(ReplicaImport),
    CursorConflict { actual: JournalCursor },
}

impl ReplicaSync {
    #[must_use]
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
pub enum ReplicaSyncError<JournalError, CursorError> {
    Import(ReplicaImportError<JournalError>),
    CursorRead {
        key: ReplicaCursorKey,
        source: CursorError,
    },
    /// Events were imported, and the cursor update may have committed before
    /// its acknowledgement was lost. Reload this key before retrying.
    UncertainCursorAdvance {
        key: ReplicaCursorKey,
        expected: JournalCursor,
        next: JournalCursor,
        source: CursorError,
    },
}

pub struct EventReplicator;

impl EventReplicator {
    /// Dispatch an already observed conflict to the handler supplied through
    /// `R`. No cursor advancement or implicit merge follows the callback.
    pub fn handle_conflict<R>(
        batch: ReplicaBatch,
        conflict: ReplicaConflict,
    ) -> Effect<(), <<R as HasReplicaConflictHandler>::Handler as ReplicaConflictHandler>::Error, R>
    where
        R: HasReplicaConflictHandler + Send + Sync + 'static,
    {
        Effect::service(|env: &R| env.replica_conflict_handler()).flat_map(move |handler| {
            handler
                .handle(batch.clone(), conflict.clone())
                .provide_some(|_: &R| ())
        })
    }

    /// Read one bounded page from a source journal. The receiver imports it
    /// using the same stable sequence and event identities.
    pub fn export<R>(
        source: ReplicaId,
        stream: JournalStream,
        after: JournalCursor,
        limit: NonZeroUsize,
    ) -> Effect<ReplicaExport, ReplicaExportError<<R::Journal as EventJournal>::Error>, R>
    where
        R: HasEventJournal + Send + Sync + 'static,
    {
        Effect::service(|env: &R| env.event_journal()).flat_map(move |journal| {
            let source = source.clone();
            let stream = stream.clone();
            journal
                .read(stream.clone(), after, limit)
                .map_error(ReplicaExportError::Journal)
                .provide_some(|_: &R| ())
                .flat_map(move |read| match read {
                    JournalReadOutcome::HistoryTruncated { first_available } => {
                        Effect::sync(move || ReplicaExport::HistoryTruncated { first_available })
                    }
                    JournalReadOutcome::CursorAhead { head } => {
                        Effect::sync(move || ReplicaExport::CursorAhead { head })
                    }
                    JournalReadOutcome::Complete(read) => {
                        if let Err(issue) = validate_journal_read(&read, &stream, after, limit) {
                            return Effect::from_result_once(Err(
                                ReplicaExportError::InvalidJournalRead(issue),
                            ));
                        }
                        let batch = ReplicaBatch::new(
                            source.clone(),
                            stream.clone(),
                            after,
                            Arc::from(read.entries),
                            limit,
                        )
                        .map_err(ReplicaExportError::InvalidJournalBatch);
                        Effect::from_result_once(batch.map(|batch| ReplicaExport::Batch {
                            batch,
                            has_more: read.has_more,
                        }))
                    }
                })
        })
    }

    /// Check the saved source cursor before import. A page already acknowledged
    /// returns without touching the journal, and an out-of-order page cannot
    /// append. After import, compare-and-advance still fences concurrent calls.
    /// If that update fails, event-ID deduplication makes a retry safe.
    pub fn import_and_ack<R>(batch: ReplicaBatch) -> ReplicaSyncEffect<R>
    where
        R: HasEventJournal + HasReplicaCursorStore + Send + Sync + 'static,
    {
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
        Effect::service(|env: &R| env.replica_cursor_store()).flat_map(move |store| {
            let key_for_read = key.clone();
            let key_for_import = key.clone();
            let batch = batch.clone();
            store
                .load(key.clone())
                .map_error(move |source| ReplicaSyncError::CursorRead {
                    key: key_for_read.clone(),
                    source,
                })
                .provide_some(|_: &R| ())
                .flat_map(move |current| {
                    if current == last {
                        return Effect::sync(move || ReplicaSync::Applied {
                            cursor: current,
                            count: usize::default(),
                        });
                    }
                    if current != expected {
                        return Effect::sync(move || ReplicaSync::CursorConflict {
                            actual: current,
                        });
                    }
                    Self::import::<R>(batch.clone())
                        .map_error(ReplicaSyncError::Import)
                        .flat_map({
                            let key = key_for_import.clone();
                            move |imported| acknowledge_import::<R>(key.clone(), expected, imported)
                        })
                })
        })
    }

    /// Import an ordered bounded batch into the ordinary journal authority.
    /// Event IDs deduplicate retries; a gap or conflicting history stops the
    /// batch before later entries are admitted. The source cursor is advanced
    /// by the caller only after this result has been acknowledged.
    pub fn import<R>(
        batch: ReplicaBatch,
    ) -> Effect<ReplicaImport, ReplicaImportError<<R::Journal as EventJournal>::Error>, R>
    where
        R: HasEventJournal + Send + Sync + 'static,
    {
        if batch.entries.len() > batch.maximum_entries.get()
            || batch
                .entries
                .iter()
                .any(|entry| entry.event.stream() != batch.stream)
        {
            return Effect::sync(|| ReplicaImport::InvalidBatch);
        }
        let mut expected = batch.after.value();
        for entry in batch.entries.iter() {
            let Some(next) = expected.checked_add(1) else {
                return Effect::sync(|| ReplicaImport::InvalidBatch);
            };
            if entry.sequence.get() != next {
                return Effect::sync(|| ReplicaImport::InvalidBatch);
            }
            expected = next;
        }
        Effect::service(|env: &R| env.event_journal()).flat_map(move |journal| {
            import_step(journal, Arc::clone(&batch.entries), 0, batch.after, 0)
        })
    }
}

fn acknowledge_import<R>(
    key: ReplicaCursorKey,
    expected: JournalCursor,
    imported: ReplicaImport,
) -> ReplicaSyncEffect<R>
where
    R: HasReplicaCursorStore + HasEventJournal + Send + Sync + 'static,
{
    if let ReplicaImport::Applied { cursor, count } = imported {
        Effect::service(|env: &R| env.replica_cursor_store()).flat_map(move |store| {
            store
                .advance(key.clone(), expected, cursor)
                .map_error({
                    let error_key = key.clone();
                    move |source| ReplicaSyncError::UncertainCursorAdvance {
                        key: error_key.clone(),
                        expected,
                        next: cursor,
                        source,
                    }
                })
                .provide_some(|_: &R| ())
                .map(move |result| match result {
                    ReplicaCursorAdvance::Advanced(cursor)
                    | ReplicaCursorAdvance::AlreadyAdvanced(cursor) => {
                        ReplicaSync::Applied { cursor, count }
                    }
                    ReplicaCursorAdvance::Conflict(actual) => {
                        ReplicaSync::CursorConflict { actual }
                    }
                })
        })
    } else {
        Effect::sync_once(move || ReplicaSync::Pending(imported))
    }
}

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
        return Effect::sync(move || ReplicaImport::Applied { cursor, count });
    };
    let predecessor = JournalCursor::new(entry.sequence.get() - 1);
    let event_id = entry.event.id.clone();
    journal
        .append(entry.event.clone(), predecessor)
        .map_error(move |source| ReplicaImportError::UncertainAppend {
            event: event_id.clone(),
            source,
        })
        .provide_some(|_: &R| ())
        .flat_map(move |receipt| match receipt {
            AppendReceipt::Accepted { event } => {
                if event != entry.event.id {
                    let event = entry.event.id.clone();
                    Effect::sync_once(move || ReplicaImport::ConflictingEvent { event })
                } else {
                    let inspect_event = event.clone();
                    let journal_for_inspection = Arc::clone(&journal);
                    let entries_for_inspection = Arc::clone(&entries);
                    let entry_for_inspection = entry.clone();
                    journal
                        .inspect(entry.event.stream(), event.clone())
                        .map_error(move |source| ReplicaImportError::UncertainInspection {
                            event: inspect_event.clone(),
                            source,
                        })
                        .provide_some(|_: &R| ())
                        .flat_map(move |inspection| match inspection {
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
                                Effect::sync_once(move || ReplicaImport::CompactedEvent { event })
                            }
                            EventInspection::Missing => {
                                let event = event.clone();
                                Effect::sync_once(move || ReplicaImport::AcceptedPending {
                                    event,
                                    cursor,
                                })
                            }
                        })
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
                Effect::sync_once(move || ReplicaImport::ConflictingEvent { event })
            }
            AppendReceipt::Decided(AppendDecision::CompactedEventId) => {
                let event = entry.event.id.clone();
                Effect::sync_once(move || ReplicaImport::CompactedEvent { event })
            }
            AppendReceipt::Decided(AppendDecision::StaleCursor { actual }) => {
                if actual.value() < predecessor.value() {
                    Effect::sync(move || ReplicaImport::Gap {
                        expected: predecessor,
                        actual,
                    })
                } else {
                    Effect::sync(move || ReplicaImport::Diverged {
                        expected: predecessor,
                        actual,
                    })
                }
            }
        })
}

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
        return Effect::sync_once(move || ReplicaImport::ConflictingEvent { event });
    }
    if actual.sequence != expected.sequence {
        let predecessor = JournalCursor::new(expected.sequence.get() - 1);
        return Effect::sync(move || ReplicaImport::Diverged {
            expected: predecessor,
            actual: JournalCursor::new(actual.sequence.get()),
        });
    }
    let (Some(next_index), Some(next_count)) = (index.checked_add(1), count.checked_add(1)) else {
        return Effect::sync(|| ReplicaImport::InvalidBatch);
    };
    import_step::<R>(
        journal,
        entries,
        next_index,
        JournalCursor::new(actual.sequence.get()),
        next_count,
    )
}
