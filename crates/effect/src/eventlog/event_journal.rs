use super::{EventEnvelope, EventId, JournalCursor, JournalEntry, JournalStream};
use crate::Effect;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppendDecision {
    Committed(JournalEntry),
    Duplicate(JournalEntry),
    ConflictingEventId,
    CompactedEventId,
    StaleCursor { actual: JournalCursor },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventInspection {
    Present(JournalEntry),
    Compacted,
    Missing,
}

/// An asynchronous provider may durably accept before final commit. Callers
/// must inspect by stable event ID after lost acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppendReceipt {
    Accepted { event: EventId },
    Decided(AppendDecision),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalRead {
    pub entries: Vec<JournalEntry>,
    pub next: JournalCursor,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalReadOutcome {
    Complete(JournalRead),
    HistoryTruncated {
        first_available: JournalCursor,
    },
    /// The caller's checkpoint is newer than this journal's head. Treat it
    /// as a source-history conflict, never as an empty caught-up page.
    CursorAhead {
        head: JournalCursor,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalReadViolation {
    PageLimitExceeded {
        limit: NonZeroUsize,
        actual: usize,
    },
    WrongStream {
        sequence: NonZeroU64,
    },
    SequenceExhausted,
    NonContiguous {
        expected: JournalCursor,
        actual: JournalCursor,
    },
    CursorMismatch {
        expected: JournalCursor,
        actual: JournalCursor,
    },
    EmptyPageWithMore,
}

pub(crate) fn validate_journal_read(
    read: &JournalRead,
    stream: &JournalStream,
    after: JournalCursor,
    limit: NonZeroUsize,
) -> Result<(), JournalReadViolation> {
    if read.entries.len() > limit.get() {
        return Err(JournalReadViolation::PageLimitExceeded {
            limit,
            actual: read.entries.len(),
        });
    }
    if read.entries.is_empty() && read.has_more {
        return Err(JournalReadViolation::EmptyPageWithMore);
    }
    let mut expected = after;
    for entry in &read.entries {
        if entry.event.stream() != *stream {
            return Err(JournalReadViolation::WrongStream {
                sequence: entry.sequence,
            });
        }
        expected = JournalCursor::new(
            expected
                .value()
                .checked_add(1)
                .ok_or(JournalReadViolation::SequenceExhausted)?,
        );
        let actual = JournalCursor::new(entry.sequence.get());
        if actual != expected {
            return Err(JournalReadViolation::NonContiguous { expected, actual });
        }
    }
    if read.next != expected {
        return Err(JournalReadViolation::CursorMismatch {
            expected,
            actual: read.next,
        });
    }
    Ok(())
}

/// Append uses stream-local compare-and-set over the expected cursor. A
/// provider must keep event-ID deduplication and sequence assignment atomic.
/// Reads with `after` beyond the stream head return `CursorAhead`, including
/// when the stream has never existed.
pub trait EventJournal: Send + Sync + 'static {
    type Error: Send + 'static;

    fn append(
        &self,
        event: EventEnvelope,
        expected: JournalCursor,
    ) -> Effect<AppendReceipt, Self::Error>;
    fn inspect(
        &self,
        stream: JournalStream,
        event: EventId,
    ) -> Effect<EventInspection, Self::Error>;
    fn read(
        &self,
        stream: JournalStream,
        after: JournalCursor,
        limit: NonZeroUsize,
    ) -> Effect<JournalReadOutcome, Self::Error>;
    fn cursor(&self, stream: JournalStream) -> Effect<JournalCursor, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionOutcome {
    NoChange,
    Compacted {
        removed: usize,
        first_available: JournalCursor,
    },
}

/// A provider supplies its own retention-proof capability. Compaction must
/// verify that permit against all registered replay/replica consumers at the
/// compaction linearization point.
pub trait JournalCompaction: EventJournal {
    type Permit: Send + 'static;
    fn compact(&self, permit: Self::Permit) -> Effect<CompactionOutcome, Self::Error>;
}

pub trait HasEventJournal {
    type Journal: EventJournal;
    fn event_journal(&self) -> Arc<Self::Journal>;
}
