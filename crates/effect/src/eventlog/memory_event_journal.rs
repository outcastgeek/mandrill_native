//! Volatile journal storage and ordered replay.

use super::{
    AppendDecision, AppendReceipt, CompactionOutcome, CompactionPermit, EventEnvelope, EventId,
    EventInspection, EventJournal, JournalCompaction, JournalCursor, JournalEntry, JournalRead,
    JournalReadOutcome, JournalStream, MemoryRetentionAuthority, RetentionError,
};
use crate::Effect;
use crate::{effect_sync, effect_try_sync};
use std::collections::BTreeMap;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for eventlog memory event journal.
pub struct MemoryJournalCapacity {
    /// Maximum distinct journal streams retained in memory. Type: `NonZeroUsize`.
    pub streams: NonZeroUsize,
    /// Maximum journal entries retained for each stream. Type: `NonZeroUsize`.
    pub entries_per_stream: NonZeroUsize,
    /// Maximum deduplication tombstones retained for each stream. Type: `NonZeroUsize`.
    pub tombstones_per_stream: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A capacity, cursor, or retention failure in the process-local journal.
pub enum MemoryJournalError {
    /// The journal reached its configured stream capacity.
    StreamCapacity,
    /// The journal reached its configured entry capacity.
    EntryCapacity,
    /// No further sequence can be allocated without exceeding its range.
    SequenceExhausted,
    /// An owned input was already consumed.
    InputConsumed,
    /// The requested compaction violates a registered retention policy. Retains the `RetentionError` detail.
    Retention(RetentionError),
    /// The compaction permit came from a different retention authority.
    WrongRetentionAuthority,
    /// The journal reached its configured deduplication tombstone capacity.
    TombstoneCapacity,
    /// The compaction boundary lies beyond the journal head.
    CompactionBeyondHead,
    /// No later compaction boundary can be represented.
    CompactionBoundaryExhausted,
    /// The stored index is inconsistent with its records.
    CorruptIndex,
}

#[derive(Default)]
/// Typed stream state used by eventlog memory event journal.
struct StreamState {
    entries: Vec<JournalEntry>,
    by_id: BTreeMap<EventId, EntryLocation>,
    tombstones: usize,
    cursor: u64,
    first_available: u64,
}

/// Variants of entry location used by eventlog memory event journal.
enum EntryLocation {
    Retained(usize),
    Compacted,
}

#[derive(Default)]
/// Typed state used by eventlog memory event journal.
struct State {
    streams: BTreeMap<JournalStream, StreamState>,
}

/// Atomic process-local journal with explicit capacities. The mutex is its
/// stream append linearization point; it is never held while running a handler.
pub struct MemoryEventJournal {
    state: Arc<Mutex<State>>,
    retention: MemoryRetentionAuthority,
    capacity: MemoryJournalCapacity,
}

impl MemoryEventJournal {
    #[must_use]
    /// Construct a `MemoryEventJournal` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `capacity` (`MemoryJournalCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(capacity: MemoryJournalCapacity) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            retention: MemoryRetentionAuthority::new(),
            capacity,
        }
    }

    /// Return the authority whose permits this journal accepts for compaction.
    #[must_use]
    /// # Returns
    /// Type: `MemoryRetentionAuthority`.
    pub fn retention_authority(&self) -> MemoryRetentionAuthority {
        self.retention.clone()
    }
}

impl EventJournal for MemoryEventJournal {
    /// Type alias for error in eventlog memory event journal.
    type Error = MemoryJournalError;

    ///
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
    ) -> Effect<AppendReceipt, Self::Error> {
        let state = Arc::clone(&self.state);
        let capacity = self.capacity;
        let event = Mutex::new(Some(event));
        effect_try_sync! { body: move || {
            let event = event
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .ok_or(MemoryJournalError::InputConsumed)?;
            let key = event.stream();
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !state.streams.contains_key(&key) {
                if expected.value() != 0 {
                    return Ok(AppendReceipt::Decided(AppendDecision::StaleCursor {
                        actual: JournalCursor::new(0),
                    }));
                }
                if state.streams.len() >= capacity.streams.get() {
                    return Err(MemoryJournalError::StreamCapacity);
                }
            }
            let stream = state.streams.entry(key).or_insert_with(|| StreamState {
                first_available: 1,
                ..StreamState::default()
            });
            if let Some(location) = stream.by_id.get(&event.id) {
                return Ok(AppendReceipt::Decided(match location {
                    EntryLocation::Compacted => AppendDecision::CompactedEventId,
                    EntryLocation::Retained(index) => {
                        let existing = stream
                            .entries
                            .get(*index)
                            .ok_or(MemoryJournalError::CorruptIndex)?
                            .clone();
                        if existing.event == event {
                            AppendDecision::Duplicate(existing)
                        } else {
                            AppendDecision::ConflictingEventId
                        }
                    }
                }));
            }
            if expected.value() != stream.cursor {
                return Ok(AppendReceipt::Decided(AppendDecision::StaleCursor {
                    actual: JournalCursor::new(stream.cursor),
                }));
            }
            if stream.entries.len() >= capacity.entries_per_stream.get() {
                return Err(MemoryJournalError::EntryCapacity);
            }
            let sequence = stream
                .cursor
                .checked_add(1)
                .and_then(NonZeroU64::new)
                .ok_or(MemoryJournalError::SequenceExhausted)?;
            let entry = JournalEntry { sequence, event };
            stream.by_id.insert(
                entry.event.id.clone(),
                EntryLocation::Retained(stream.entries.len()),
            );
            stream.entries.push(entry.clone());
            stream.cursor = sequence.get();
            Ok(AppendReceipt::Decided(AppendDecision::Committed(entry)))
        }, on_error: std::convert::identity, }
    }

    ///
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
    ) -> Effect<EventInspection, Self::Error> {
        let state = Arc::clone(&self.state);
        effect_try_sync! { body: move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(stream) = state.streams.get(&stream) else {
                return Ok(EventInspection::Missing);
            };
            match stream.by_id.get(&event) {
                None => Ok(EventInspection::Missing),
                Some(EntryLocation::Compacted) => Ok(EventInspection::Compacted),
                Some(EntryLocation::Retained(index)) => stream
                    .entries
                    .get(*index)
                    .cloned()
                    .map(EventInspection::Present)
                    .ok_or(MemoryJournalError::CorruptIndex),
            }
        }, on_error: std::convert::identity, }
    }

    ///
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
    ) -> Effect<JournalReadOutcome, Self::Error> {
        let state = Arc::clone(&self.state);
        effect_sync! { body: move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(stream) = state.streams.get(&stream) else {
                if after.value() != 0 {
                    return JournalReadOutcome::CursorAhead {
                        head: JournalCursor::new(0),
                    };
                }
                return JournalReadOutcome::Complete(JournalRead {
                    entries: Vec::new(),
                    next: after,
                    has_more: false,
                });
            };
            if after.value() > stream.cursor {
                return JournalReadOutcome::CursorAhead {
                    head: JournalCursor::new(stream.cursor),
                };
            }
            if after.value().saturating_add(1) < stream.first_available {
                return JournalReadOutcome::HistoryTruncated {
                    first_available: JournalCursor::new(stream.first_available),
                };
            }
            let start = stream
                .entries
                .partition_point(|entry| entry.sequence.get() <= after.value());
            let remaining = stream.entries.get(start..).unwrap_or(&[]);
            let has_more = remaining.len() > limit.get();
            let entries: Vec<_> = remaining.iter().take(limit.get()).cloned().collect();
            let next = entries
                .last()
                .map_or(after, |entry| JournalCursor::new(entry.sequence.get()));
            JournalReadOutcome::Complete(JournalRead {
                entries,
                next,
                has_more,
            })
        }, }
    }

    ///
    /// # Parameters
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    ///
    /// # Returns
    /// Type: `Effect<JournalCursor, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn cursor(&self, stream: JournalStream) -> Effect<JournalCursor, Self::Error> {
        let state = Arc::clone(&self.state);
        effect_sync! { body: move || {
            let state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            JournalCursor::new(state.streams.get(&stream).map_or(0, |stream| stream.cursor))
        }, }
    }
}

impl JournalCompaction for MemoryEventJournal {
    /// Type alias for permit in eventlog memory event journal.
    type Permit = CompactionPermit;

    ///
    /// # Parameters
    /// - `permit` (`CompactionPermit`): The permit supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<CompactionOutcome, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn compact(&self, permit: CompactionPermit) -> Effect<CompactionOutcome, Self::Error> {
        let state = Arc::clone(&self.state);
        let retention_state = Arc::clone(&self.retention.state);
        let capacity = self.capacity;
        let permit = Mutex::new(Some(permit));
        effect_try_sync! { body: move || {
            let permit = permit
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .ok_or(MemoryJournalError::InputConsumed)?;
            if !Arc::ptr_eq(&permit.state, &retention_state) {
                return Err(MemoryJournalError::WrongRetentionAuthority);
            }
            let retention = permit
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            retention
                .validate(&permit.stream, permit.through)
                .map_err(MemoryJournalError::Retention)?;
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(stream) = state.streams.get_mut(&permit.stream) else {
                return Ok(CompactionOutcome::NoChange);
            };
            if permit.through.value() > stream.cursor {
                return Err(MemoryJournalError::CompactionBeyondHead);
            }
            let boundary = permit
                .through
                .value()
                .checked_add(1)
                .ok_or(MemoryJournalError::CompactionBoundaryExhausted)?;
            if boundary <= stream.first_available {
                return Ok(CompactionOutcome::NoChange);
            }
            let removed = stream
                .entries
                .iter()
                .take_while(|entry| entry.sequence.get() <= permit.through.value())
                .count();
            let next_tombstones = stream
                .tombstones
                .checked_add(removed)
                .ok_or(MemoryJournalError::TombstoneCapacity)?;
            if next_tombstones > capacity.tombstones_per_stream.get() {
                return Err(MemoryJournalError::TombstoneCapacity);
            }
            let discarded: Vec<_> = stream.entries.drain(..removed).collect();
            for entry in discarded {
                stream
                    .by_id
                    .insert(entry.event.id, EntryLocation::Compacted);
            }
            for (index, entry) in stream.entries.iter().enumerate() {
                stream
                    .by_id
                    .insert(entry.event.id.clone(), EntryLocation::Retained(index));
            }
            stream.tombstones = next_tombstones;
            stream.first_available = boundary;
            Ok(CompactionOutcome::Compacted {
                removed,
                first_available: JournalCursor::new(boundary),
            })
        }, on_error: std::convert::identity, }
    }
}
