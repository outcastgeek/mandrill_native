//! Physical stream indexes, encoded values and retained replay pages.

use super::{MemoryJournalCapacity, MemoryJournalError};
use crate::UtcMillis;
use crate::eventlog::{
    AggregateId, CompactionOutcome, CompactionPermit, EventGroupId, EventId, EventJournalId,
    EventOutcomeKind, EventTag, EventVersion, JournalCursor, JournalStream,
    MemoryRetentionAuthority,
};
use crate::memory::{
    Buffer, BufferView, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease,
    SlotPool,
};
use crate::stream::{PreparedChunkPool, PreparedChunkValues};
use std::alloc::Layout;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex, Weak};

/// Physical storage and retained stable identity limits for the volatile journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedMemoryJournalCapacity {
    /// Stream, live entry and deduplication tombstone limits.
    pub journal: MemoryJournalCapacity,
    /// Simultaneous reserved and retained payload or outcome buffers, including pages.
    pub payloads: NonZeroUsize,
    /// Maximum physical bytes in one encoded payload or outcome.
    pub bytes_per_payload: Bytes,
    /// Maximum bytes in one caller-owned stable identity or tag.
    pub bytes_per_identity: Bytes,
    /// Simultaneous leased read pages.
    pub pages: NonZeroUsize,
    /// Maximum journal entries in one page.
    pub entries_per_page: NonZeroUsize,
}

type BytesLease = SlotLease<'static, Buffer<'static>>;
type StreamLease = SlotLease<'static, Stream>;
type StreamRecord = (JournalStream, StreamLease);
type IdentityRecord = (EventId, Location);

impl PreparedMemoryJournalCapacity {
    fn identities_per_stream(self) -> Result<usize, MemoryError> {
        self.journal
            .entries_per_stream
            .get()
            .checked_add(self.journal.tombstones_per_stream.get())
            .ok_or(MemoryError::ArithmeticOverflow)
    }

    /// Checked regional payload/control footprint before allocator headers and alignment.
    /// Native identity roots and retention authority state are specified separately.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let streams = self.journal.streams.get();
        let stream_entries = layout::<PreparedJournalEntry>(self.journal.entries_per_stream.get())?
            .checked_add(layout::<IdentityRecord>(self.identities_per_stream()?)?)?;
        layout::<StreamRecord>(streams)?
            .checked_add(SlotPool::<Stream>::storage_bytes(streams)?)?
            .checked_add(stream_entries.checked_mul(streams)?)?
            .checked_add(SlotPool::<Buffer<'static>>::storage_bytes(
                self.payloads.get(),
            )?)?
            .checked_add(self.bytes_per_payload.checked_mul(self.payloads.get())?)?
            .checked_add(PreparedChunkPool::<PreparedJournalEntry>::storage_bytes(
                self.pages,
                self.entries_per_page,
            )?)
    }
}

fn layout<T>(count: usize) -> Result<Bytes, MemoryError> {
    Layout::array::<T>(count)
        .map(|layout| Bytes::new(layout.size()))
        .map_err(|_| MemoryError::InvalidLayout)
}

/// The stable identity and metadata accompanying an admitted event payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedEventHeader {
    pub journal: EventJournalId,
    pub aggregate: AggregateId,
    pub id: EventId,
    pub group: EventGroupId,
    pub tag: EventTag,
    pub version: EventVersion,
    pub occurred_at: UtcMillis,
}

impl PreparedEventHeader {
    #[must_use]
    pub fn stream(&self) -> JournalStream {
        JournalStream {
            journal: self.journal.clone(),
            aggregate: self.aggregate.clone(),
        }
    }

    fn check_identities(&self, maximum: Bytes) -> Result<(), MemoryError> {
        for length in [
            self.journal.as_bytes().len(),
            self.aggregate.as_bytes().len(),
            self.id.as_bytes().len(),
            self.group.as_str().len(),
            self.tag.as_str().len(),
        ] {
            let requested = Bytes::new(length);
            if requested > maximum {
                return Err(MemoryError::ValueTooLarge {
                    requested,
                    capacity: maximum,
                });
            }
        }
        Ok(())
    }
}

/// A writer owning actual journal buffers before event encoding.
pub struct PreparedEventAppend {
    owner: Weak<Mutex<PreparedState>>,
    payload: BytesLease,
    outcome: Option<BytesLease>,
}

impl PreparedEventAppend {
    /// Write the encoded event directly into the admitted payload buffer.
    pub(crate) fn payload_mut(&mut self) -> &mut Buffer<'static> {
        &mut self.payload
    }

    /// Write an encoded outcome if its buffer was requested.
    pub(crate) fn outcome_mut(&mut self) -> Option<&mut Buffer<'static>> {
        self.outcome.as_deref_mut()
    }

    /// Append encoded payload bytes within the reserved capacity.
    pub fn extend_payload(&mut self, bytes: &[u8]) -> Result<(), MemoryError> {
        self.payload_mut().extend_from_slice(bytes)
    }

    /// Append encoded result bytes within the reserved outcome capacity.
    pub fn extend_outcome(&mut self, bytes: &[u8]) -> Result<(), MemoryError> {
        self.outcome_mut()
            .ok_or(MemoryError::UnsupportedRunContract)?
            .extend_from_slice(bytes)
    }

    /// Borrow the request bytes already written to this reservation.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        self.payload.as_slice()
    }
}

/// A recorded result retaining its actual encoded backing.
pub struct PreparedEncodedEventOutcome {
    pub kind: EventOutcomeKind,
    pub payload: BufferView<'static>,
}

impl std::fmt::Debug for PreparedEncodedEventOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedEncodedEventOutcome")
            .field("kind", &self.kind)
            .field("payload_bytes", &self.payload.as_ref().len())
            .finish()
    }
}

impl PreparedEncodedEventOutcome {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            kind: self.kind,
            payload: self.payload.try_clone()?,
        })
    }
}

/// One retained journal entry, including checked aliases of its encoded bytes.
pub struct PreparedJournalEntry {
    pub sequence: NonZeroU64,
    pub header: PreparedEventHeader,
    pub payload: BufferView<'static>,
    pub outcome: Option<PreparedEncodedEventOutcome>,
}

impl std::fmt::Debug for PreparedJournalEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedJournalEntry")
            .field("sequence", &self.sequence)
            .field("header", &self.header)
            .field("payload_bytes", &self.payload.as_ref().len())
            .field("outcome", &self.outcome)
            .finish()
    }
}

impl PreparedJournalEntry {
    /// Retain the same physical payload and outcome independently of the entry or page.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            sequence: self.sequence,
            header: self.header.clone(),
            payload: self.payload.try_clone()?,
            outcome: self
                .outcome
                .as_ref()
                .map(PreparedEncodedEventOutcome::try_clone)
                .transpose()?,
        })
    }

    fn matches(
        &self,
        header: &PreparedEventHeader,
        payload: &[u8],
        outcome: Option<(EventOutcomeKind, &[u8])>,
    ) -> bool {
        self.header == *header
            && self.payload.as_ref() == payload
            && self
                .outcome
                .as_ref()
                .map(|outcome| (outcome.kind, outcome.payload.as_ref()))
                == outcome
    }
}

/// A complete decision at the prepared local append linearization point.
#[derive(Debug)]
pub enum PreparedAppendDecision {
    Committed(PreparedJournalEntry),
    Duplicate(PreparedJournalEntry),
    ConflictingEventId,
    CompactedEventId,
    StaleCursor { actual: JournalCursor },
}

/// Stable event inspection using the same retained byte owner as replay.
#[derive(Debug)]
pub enum PreparedEventInspection {
    Present(PreparedJournalEntry),
    Compacted,
    Missing,
}

/// Local journal validation and prepared storage failures remain distinct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedJournalError {
    Journal(MemoryJournalError),
    Memory(MemoryError),
}

impl From<MemoryJournalError> for PreparedJournalError {
    fn from(error: MemoryJournalError) -> Self {
        Self::Journal(error)
    }
}

impl From<MemoryError> for PreparedJournalError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

/// A leased replay page whose payloads remain valid across compaction.
pub struct PreparedJournalRead {
    entries: PreparedChunkValues<PreparedJournalEntry>,
    pub next: JournalCursor,
    pub has_more: bool,
}

impl PreparedJournalRead {
    /// Borrow the retained entries in contiguous sequence order.
    #[must_use]
    pub fn entries(&self) -> &[PreparedJournalEntry] {
        self.entries.as_slice()
    }
}

/// Replay retains the same ahead-of-head and truncated-history distinctions.
pub enum PreparedJournalReadOutcome {
    Complete(PreparedJournalRead),
    HistoryTruncated { first_available: JournalCursor },
    CursorAhead { head: JournalCursor },
}

#[derive(Clone, Copy)]
enum Location {
    Retained(NonZeroU64),
    Compacted,
}

struct Stream {
    entries: FixedVector<'static, PreparedJournalEntry>,
    by_id: FixedVector<'static, IdentityRecord>,
    tombstones: usize,
    cursor: JournalCursor,
    first_available: u64,
}

impl Stream {
    fn position(&self, id: &EventId) -> Result<usize, usize> {
        self.by_id
            .as_slice()
            .binary_search_by(|(key, _)| key.cmp(id))
    }

    fn location(&self, id: &EventId) -> Option<Location> {
        let position = self.position(id).ok()?;
        self.by_id
            .as_slice()
            .get(position)
            .map(|(_, location)| *location)
    }

    fn entry(&self, sequence: NonZeroU64) -> Result<&PreparedJournalEntry, MemoryJournalError> {
        sequence
            .get()
            .checked_sub(self.first_available)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.entries.as_slice().get(index))
            .filter(|entry| entry.sequence == sequence)
            .ok_or(MemoryJournalError::CorruptIndex)
    }

    fn publish(&mut self, entry: PreparedJournalEntry, position: usize) {
        let sequence = entry.sequence;
        self.by_id
            .push_reserved((entry.header.id.clone(), Location::Retained(sequence)));
        if let Some(suffix) = self.by_id.as_mut_slice().get_mut(position..) {
            suffix.rotate_right(1);
        }
        self.entries.push_reserved(entry);
        self.cursor = JournalCursor::new(sequence.get());
    }
}

struct PreparedState {
    streams: FixedVector<'static, StreamRecord>,
}

impl PreparedState {
    fn position(&self, key: &JournalStream) -> Result<usize, usize> {
        self.streams
            .as_slice()
            .binary_search_by(|(stream, _)| stream.cmp(key))
    }

    fn get(&self, key: &JournalStream) -> Option<&Stream> {
        let position = self.position(key).ok()?;
        self.streams
            .as_slice()
            .get(position)
            .map(|(_, stream)| &**stream)
    }

    fn get_mut(&mut self, key: &JournalStream) -> Option<&mut Stream> {
        let position = self.position(key).ok()?;
        self.streams
            .as_mut_slice()
            .get_mut(position)
            .map(|(_, stream)| &mut **stream)
    }
}

/// Volatile event journal with region-owned indexes, actual encoded bytes and read pages.
pub struct PreparedMemoryEventJournal {
    state: Arc<Mutex<PreparedState>>,
    streams: SlotPool<'static, Stream>,
    payloads: SlotPool<'static, Buffer<'static>>,
    pages: PreparedChunkPool<PreparedJournalEntry>,
    retention: MemoryRetentionAuthority,
    capacity: PreparedMemoryJournalCapacity,
}

impl PreparedMemoryEventJournal {
    /// Provision every journal record and byte/page owner before starting work.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedMemoryJournalCapacity,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        let streams = memory.slots(region, capacity.journal.streams.get(), |_| {
            Ok(Stream {
                entries: FixedVector::prepare(
                    memory.allocator(region)?,
                    capacity.journal.entries_per_stream.get(),
                )?,
                by_id: FixedVector::prepare(
                    memory.allocator(region)?,
                    capacity.identities_per_stream()?,
                )?,
                tombstones: 0,
                cursor: JournalCursor::new(0),
                first_available: 1,
            })
        })?;
        let state = PreparedState {
            streams: FixedVector::prepare(
                memory.allocator(region)?,
                capacity.journal.streams.get(),
            )?,
        };
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            streams,
            payloads: memory.buffers(
                region,
                capacity.payloads.get(),
                capacity.bytes_per_payload,
            )?,
            pages: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.pages,
                capacity.entries_per_page,
            )?,
            retention: MemoryRetentionAuthority::new(),
            capacity,
        })
    }

    /// Return the authority whose permits the prepared journal accepts.
    #[must_use]
    pub fn retention_authority(&self) -> MemoryRetentionAuthority {
        self.retention.clone()
    }

    fn checkout_bytes(&self) -> Result<BytesLease, MemoryError> {
        let mut bytes = self.payloads.try_checkout()?;
        bytes.clear();
        Ok(bytes)
    }

    /// Admit the request and optional outcome storage before invoking an encoder.
    pub fn reserve_append(
        &self,
        with_outcome: bool,
    ) -> Result<PreparedEventAppend, PreparedJournalError> {
        let payload = self.checkout_bytes()?;
        let outcome = with_outcome.then(|| self.checkout_bytes()).transpose()?;
        Ok(PreparedEventAppend {
            owner: Arc::downgrade(&self.state),
            payload,
            outcome,
        })
    }

    /// Append only after ownership, cursor, identity, and all destination checks succeed.
    pub fn append_prepared(
        &self,
        header: PreparedEventHeader,
        expected: JournalCursor,
        bytes: PreparedEventAppend,
        kind: Option<EventOutcomeKind>,
    ) -> Result<PreparedAppendDecision, PreparedJournalError> {
        if !bytes.owner.ptr_eq(&Arc::downgrade(&self.state)) {
            return Err(MemoryError::UnsupportedRunContract.into());
        }
        let outcome = match (kind, bytes.outcome) {
            (Some(kind), Some(payload)) => Some(PreparedEncodedEventOutcome {
                kind,
                payload: BufferView::new(payload.share()),
            }),
            (None, _) => None,
            (Some(_), None) => return Err(MemoryError::UnsupportedRunContract.into()),
        };
        let payload = BufferView::new(bytes.payload.share());
        let key = header.stream();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match state.position(&key) {
            Ok(position) => {
                let (_, stream) = state
                    .streams
                    .as_mut_slice()
                    .get_mut(position)
                    .ok_or(MemoryJournalError::CorruptIndex)?;
                append_existing(
                    stream,
                    header,
                    expected,
                    payload,
                    outcome,
                    self.capacity.bytes_per_identity,
                )
            }
            Err(position) => {
                if expected.value() != 0 {
                    return Ok(PreparedAppendDecision::StaleCursor {
                        actual: JournalCursor::new(0),
                    });
                }
                if state.streams.len() == state.streams.capacity() {
                    return Err(MemoryJournalError::StreamCapacity.into());
                }
                let mut stream = self.streams.try_checkout()?;
                let decision = append_existing(
                    &mut stream,
                    header,
                    expected,
                    payload,
                    outcome,
                    self.capacity.bytes_per_identity,
                )?;
                state.streams.push_reserved((key, stream));
                if let Some(suffix) = state.streams.as_mut_slice().get_mut(position..) {
                    suffix.rotate_right(1);
                }
                Ok(decision)
            }
        }
    }

    /// Inspect by stable event identity before replaying or invoking a handler.
    pub fn inspect_prepared(
        &self,
        key: &JournalStream,
        event: &EventId,
    ) -> Result<PreparedEventInspection, PreparedJournalError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stream) = state.get(key) else {
            return Ok(PreparedEventInspection::Missing);
        };
        Ok(match stream.location(event) {
            None => PreparedEventInspection::Missing,
            Some(Location::Compacted) => PreparedEventInspection::Compacted,
            Some(Location::Retained(sequence)) => {
                PreparedEventInspection::Present(stream.entry(sequence)?.try_clone()?)
            }
        })
    }

    /// Read the current head without materializing a page.
    #[must_use]
    pub fn cursor_prepared(&self, key: &JournalStream) -> JournalCursor {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .get(key)
            .map_or(JournalCursor::new(0), |stream| stream.cursor)
    }

    /// Lease a contiguous page. The page and its encoded bytes may outlive the store.
    pub fn read_prepared(
        &self,
        key: &JournalStream,
        after: JournalCursor,
        limit: NonZeroUsize,
    ) -> Result<PreparedJournalReadOutcome, PreparedJournalError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stream = state.get(key);
        let head = stream.map_or(JournalCursor::new(0), |stream| stream.cursor);
        if after > head {
            return Ok(PreparedJournalReadOutcome::CursorAhead { head });
        }
        if let Some(stream) = stream
            && after.value().saturating_add(1) < stream.first_available
        {
            return Ok(PreparedJournalReadOutcome::HistoryTruncated {
                first_available: JournalCursor::new(stream.first_available),
            });
        }
        if limit > self.capacity.entries_per_page {
            return Err(MemoryError::CollectionReservationFailed.into());
        }
        let mut entries = self.pages.try_checkout()?;
        let remaining = stream.map_or(&[][..], |stream| {
            let records = stream.entries.as_slice();
            let start = records.partition_point(|entry| entry.sequence.get() <= after.value());
            records.split_at(start).1
        });
        for entry in remaining.iter().take(limit.get()) {
            entries.push_reserved(entry.try_clone()?);
        }
        let next = entries
            .as_slice()
            .last()
            .map_or(after, |entry| JournalCursor::new(entry.sequence.get()));
        Ok(PreparedJournalReadOutcome::Complete(PreparedJournalRead {
            entries,
            next,
            has_more: remaining.len() > limit.get(),
        }))
    }

    /// Compact under the same authority and lock order as the ordinary journal.
    pub fn compact_prepared(
        &self,
        permit: CompactionPermit,
    ) -> Result<CompactionOutcome, PreparedJournalError> {
        if !Arc::ptr_eq(&permit.state, &self.retention.state) {
            return Err(MemoryJournalError::WrongRetentionAuthority.into());
        }
        let retention = permit
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        retention
            .validate(&permit.stream, permit.through)
            .map_err(MemoryJournalError::Retention)?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stream) = state.get_mut(&permit.stream) else {
            return Ok(CompactionOutcome::NoChange);
        };
        compact_stream(
            stream,
            permit.through,
            self.capacity.journal.tombstones_per_stream,
        )
    }
}

fn append_existing(
    stream: &mut Stream,
    header: PreparedEventHeader,
    expected: JournalCursor,
    payload: BufferView<'static>,
    outcome: Option<PreparedEncodedEventOutcome>,
    identity_limit: Bytes,
) -> Result<PreparedAppendDecision, PreparedJournalError> {
    let position = match stream.position(&header.id) {
        Err(position) => position,
        Ok(index) => {
            let (_, location) = stream
                .by_id
                .as_slice()
                .get(index)
                .ok_or(MemoryJournalError::CorruptIndex)?;
            return Ok(match location {
                Location::Compacted => PreparedAppendDecision::CompactedEventId,
                Location::Retained(sequence) => {
                    let existing = stream.entry(*sequence)?;
                    if existing.matches(
                        &header,
                        payload.as_ref(),
                        outcome
                            .as_ref()
                            .map(|outcome| (outcome.kind, outcome.payload.as_ref())),
                    ) {
                        PreparedAppendDecision::Duplicate(existing.try_clone()?)
                    } else {
                        PreparedAppendDecision::ConflictingEventId
                    }
                }
            });
        }
    };
    if expected != stream.cursor {
        return Ok(PreparedAppendDecision::StaleCursor {
            actual: stream.cursor,
        });
    }
    if stream.entries.len() == stream.entries.capacity() {
        return Err(MemoryJournalError::EntryCapacity.into());
    }
    if stream.by_id.len() == stream.by_id.capacity() {
        return Err(MemoryJournalError::CorruptIndex.into());
    }
    let sequence = expected
        .value()
        .checked_add(1)
        .and_then(NonZeroU64::new)
        .ok_or(MemoryJournalError::SequenceExhausted)?;
    header.check_identities(identity_limit)?;
    let entry = PreparedJournalEntry {
        sequence,
        header,
        payload,
        outcome,
    };
    let receipt = entry.try_clone()?;
    stream.publish(entry, position);
    Ok(PreparedAppendDecision::Committed(receipt))
}

fn compact_stream(
    stream: &mut Stream,
    through: JournalCursor,
    tombstones: NonZeroUsize,
) -> Result<CompactionOutcome, PreparedJournalError> {
    if through > stream.cursor {
        return Err(MemoryJournalError::CompactionBeyondHead.into());
    }
    let boundary = through
        .value()
        .checked_add(1)
        .ok_or(MemoryJournalError::CompactionBoundaryExhausted)?;
    if boundary <= stream.first_available {
        return Ok(CompactionOutcome::NoChange);
    }
    let removed = stream
        .entries
        .as_slice()
        .partition_point(|entry| entry.sequence.get() <= through.value());
    let next = stream
        .tombstones
        .checked_add(removed)
        .filter(|next| *next <= tombstones.get())
        .ok_or(MemoryJournalError::TombstoneCapacity)?;
    for (_, location) in stream.by_id.as_mut_slice() {
        if matches!(*location, Location::Retained(sequence) if sequence.get() <= through.value()) {
            *location = Location::Compacted;
        }
    }
    stream
        .entries
        .retain(|entry| entry.sequence.get() > through.value());
    stream.tombstones = next;
    stream.first_available = boundary;
    Ok(CompactionOutcome::Compacted {
        removed,
        first_available: JournalCursor::new(boundary),
    })
}
