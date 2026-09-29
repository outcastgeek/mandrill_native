//! Journal append, read, and change-stream contracts.

use super::{EventEnvelope, EventId, JournalCursor, JournalEntry, JournalStream};
use crate::Effect;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
/// The journal’s atomic accept, duplicate, or conflict decision for an append.
pub enum AppendDecision {
    /// The state change was committed and is now authoritative. Contains `JournalEntry`.
    Committed(JournalEntry),
    /// The  already exists or occurred twice. Carries `JournalEntry`.
    Duplicate(JournalEntry),
    /// The event identity is already bound to different content.
    ConflictingEventId,
    /// The event identity refers to compacted history.
    CompactedEventId,
    /// The append cursor no longer names the current journal head.
    StaleCursor {
        /// Current authoritative journal cursor. Type: `JournalCursor`.
        actual: JournalCursor,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The recorded identity and content currently associated with an event.
pub enum EventInspection {
    /// The selected value exists. Contains `JournalEntry`.
    Present(JournalEntry),
    /// The requested history has been removed by compaction.
    Compacted,
    /// No matching value exists.
    Missing,
}

/// An asynchronous provider may durably accept before final commit. Callers
/// must inspect by stable event ID after lost acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppendReceipt {
    /// The journal accepted this event for its stream.
    Accepted {
        /// Identity of the accepted event. Type: `EventId`.
        event: EventId,
    },
    /// A terminal decision has been recorded. Contains `AppendDecision`.
    Decided(AppendDecision),
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The cursor and limit supplied when reading a journal stream.
pub struct JournalRead {
    /// Journal entries in stream order. Type: `Vec<JournalEntry>`.
    pub entries: Vec<JournalEntry>,
    /// Next position after the current record. Type: `JournalCursor`.
    pub next: JournalCursor,
    /// Whether more entries remain after this batch. Type: `bool`.
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// The entries and continuation position returned by a bounded journal read.
pub enum JournalReadOutcome {
    /// The operation has a recorded terminal result. Contains `JournalRead`.
    Complete(JournalRead),
    /// Replay requires history older than the retained boundary.
    HistoryTruncated {
        /// Earliest position still retained for replay. Type: `JournalCursor`.
        first_available: JournalCursor,
    },
    /// The caller's checkpoint is newer than this journal's head. Treat it
    /// as a source-history conflict, never as an empty caught-up page.
    CursorAhead {
        /// Current head position of the ordered stream. Type: `JournalCursor`.
        head: JournalCursor,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A gap or retention boundary that prevents a requested journal read.
pub enum JournalReadViolation {
    /// The requested page exceeds the configured bound.
    PageLimitExceeded {
        /// The caller-supplied bound. Type: `NonZeroUsize`.
        limit: NonZeroUsize,
        /// The value observed when the requested condition failed. Type: `usize`.
        actual: usize,
    },
    /// The supplied stream does not match recorded state.
    WrongStream {
        /// The ordered sequence position. Type: `NonZeroU64`.
        sequence: NonZeroU64,
    },
    /// No further sequence can be allocated within its type range.
    SequenceExhausted,
    /// The supplied cursor does not immediately follow the current journal cursor.
    NonContiguous {
        /// The value required by the requested condition. Type: `JournalCursor`.
        expected: JournalCursor,
        /// The value observed when the requested condition failed. Type: `JournalCursor`.
        actual: JournalCursor,
    },
    /// The cursor differs from the recorded or required value.
    CursorMismatch {
        /// The value required by the requested condition. Type: `JournalCursor`.
        expected: JournalCursor,
        /// The value observed when the requested condition failed. Type: `JournalCursor`.
        actual: JournalCursor,
    },
    /// No entries were returned although a continuation exists.
    EmptyPageWithMore,
}

/// # Parameters
/// - `read` (`&JournalRead`): The read supplied to this operation.
/// - `stream` (`&JournalStream`): Scoped source of ordered chunks or events.
/// - `after` (`JournalCursor`): Cursor or instant after which results are selected.
/// - `limit` (`NonZeroUsize`): The caller-supplied limit.
///
/// # Returns
/// Type: `Result<(), JournalReadViolation>`.
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
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;

    /// # Parameters
    /// - `event` (`EventEnvelope`): Typed event identity and payload submitted to the journal.
    /// - `expected` (`JournalCursor`): Value or revision required for the proposed state transition.
    ///
    /// # Returns
    /// Type: `Effect<AppendReceipt, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn append(
        &self,
        event: EventEnvelope,
        expected: JournalCursor,
    ) -> Effect<AppendReceipt, Self::Error>;
    /// # Parameters
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    /// - `event` (`EventId`): Typed event identity and payload submitted to the journal.
    ///
    /// # Returns
    /// Type: `Effect<EventInspection, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn inspect(
        &self,
        stream: JournalStream,
        event: EventId,
    ) -> Effect<EventInspection, Self::Error>;
    /// # Parameters
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    /// - `after` (`JournalCursor`): Cursor or instant after which results are selected.
    /// - `limit` (`NonZeroUsize`): The caller-supplied limit.
    ///
    /// # Returns
    /// Type: `Effect<JournalReadOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn read(
        &self,
        stream: JournalStream,
        after: JournalCursor,
        limit: NonZeroUsize,
    ) -> Effect<JournalReadOutcome, Self::Error>;
    /// # Parameters
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    ///
    /// # Returns
    /// Type: `Effect<JournalCursor, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn cursor(&self, stream: JournalStream) -> Effect<JournalCursor, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The retained-history boundary resulting from an authorized compaction.
pub enum CompactionOutcome {
    /// No change is available.
    NoChange,
    /// The requested history has been removed by compaction.
    Compacted {
        /// Number of journal entries removed by compaction. Type: `usize`.
        removed: usize,
        /// Earliest position still retained for replay. Type: `JournalCursor`.
        first_available: JournalCursor,
    },
}

/// A provider supplies its own retention-proof capability. Compaction must
/// verify that permit against all registered replay/replica consumers at the
/// compaction linearization point.
pub trait JournalCompaction: EventJournal {
    /// Proof that compaction is authorized by the retention policy.
    type Permit: Send + 'static;
    /// # Parameters
    /// - `permit` (`Self::Permit`): The permit supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<CompactionOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn compact(&self, permit: Self::Permit) -> Effect<CompactionOutcome, Self::Error>;
}

/// Selects the event journal in an application environment.
pub trait HasEventJournal {
    /// Event journal selected by this environment.
    type Journal: EventJournal;
    /// # Returns
    /// Type: `Arc<Self::Journal>`.
    fn event_journal(&self) -> Arc<Self::Journal>;
}
