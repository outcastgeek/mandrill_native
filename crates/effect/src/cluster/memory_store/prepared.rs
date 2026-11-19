use super::{
    ActivationTable, InternalState, MemoryClusterMessageStore, MemoryState, MessageKey,
    ObserverState, Record, disposition, expiry, verify_claim,
};
use crate::cluster::{
    AcceptancePolicy, ClaimOptions, ClaimToken, ClusterAddress, ClusterObserverId,
    ClusterRequestId, ClusterStoreError, MessageAcceptance, MessageClaim, MessageDisposition,
    MessageScan, MessageStoreCapacity, ObserverCursor, PayloadConflictPolicy,
    PreparedClusterActivationCapacity, ReplyKind, SavedChunk, SavedReply, StoreCompletion,
    StoreMutation,
};
use crate::memory::{
    Buffer, BufferView, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease,
    SlotPool,
};
use crate::stream::{PreparedChunkPool, PreparedChunkValues};
use crate::{Effect, UtcMillis};
use std::alloc::Layout;
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex};

/// Startup dimensions for the selected volatile cluster store.
#[derive(Clone, Copy, Debug)]
pub struct PreparedClusterStoreCapacity {
    /// Stable entity addresses retained by the shared activation fence table.
    pub activation: PreparedClusterActivationCapacity,
    /// Accepted records, including pending and retained terminal records.
    pub records: NonZeroUsize,
    /// Chunks per record before another chunk is refused.
    pub chunks_per_record: NonZeroUsize,
    /// Observers per record, including acknowledgement cursors.
    pub observers_per_record: NonZeroUsize,
    /// Combined caller-owned address and request identity bytes per record.
    pub identity_bytes_per_record: Bytes,
    /// Caller-owned stable observer identifier bytes per cursor.
    pub identity_bytes_per_observer: Bytes,
    /// Concurrent payload reservations plus retained payload generations.
    /// Request, idempotency key, chunk, terminal reply and outstanding
    /// snapshot owners all share these physical buffers.
    pub payloads: NonZeroUsize,
    /// Bytes in one request, idempotency key, chunk or terminal reply buffer.
    pub bytes_per_payload: Bytes,
    /// Page slots in each of the claim and scan pools. The underlying
    /// history and cursor snapshot pools each share this count times
    /// records_per_page across all prepared readers.
    pub snapshots: NonZeroUsize,
    /// Maximum items per prepared claim or scan.
    pub records_per_page: NonZeroUsize,
}

/// A selected store rejection with exact physical-storage diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedClusterStoreError {
    /// The existing cluster contract rejected this transition.
    Store(ClusterStoreError),
    /// Physical storage or the selected run contract could not admit it.
    Memory(MemoryError),
}

impl From<ClusterStoreError> for PreparedClusterStoreError {
    fn from(error: ClusterStoreError) -> Self {
        Self::Store(error)
    }
}

impl PreparedClusterStoreError {
    pub(super) fn into_legacy(self) -> ClusterStoreError {
        match self {
            Self::Store(error) => error,
            Self::Memory(_) => ClusterStoreError::CapacityExceeded,
        }
    }
}

type PreparedPayload = SlotLease<'static, Buffer<'static>>;

/// Exclusive store-owned payload storage. Dropping an unused reservation
/// returns it to its original store.
pub struct PreparedClusterPayloadReservation {
    store: Arc<Mutex<MemoryState>>,
    buffer: PreparedPayload,
}

impl PreparedClusterPayloadReservation {
    /// Append to the fixed backing without taking ownership of caller storage.
    pub fn extend_from_slice(&mut self, bytes: &[u8]) -> Result<(), MemoryError> {
        self.buffer.extend_from_slice(bytes)
    }

    /// Physical byte limit of this reservation.
    #[must_use]
    pub fn capacity(&self) -> Bytes {
        self.buffer.capacity()
    }

    pub(crate) fn buffer_mut(&mut self) -> &mut Buffer<'static> {
        &mut self.buffer
    }
}

/// Acceptance metadata paired with a separate store-owned request payload.
pub struct PreparedAcceptedRequest {
    /// Destination entity address.
    pub address: ClusterAddress,
    /// Stable request identity.
    pub id: ClusterRequestId,
    /// Caller-provided RPC idempotency identity. The store copies its bytes.
    pub idempotency_key: Option<Arc<[u8]>>,
    /// Caller-supplied time.
    pub accepted_at: UtcMillis,
    /// Deduplication, retention and retry policy.
    pub policy: AcceptancePolicy,
}

pub(super) enum RetainedBytes {
    Ordinary(Arc<[u8]>),
    Prepared(BufferView<'static>),
}

impl RetainedBytes {
    fn view(&self) -> Result<BufferView<'static>, PreparedClusterStoreError> {
        match self {
            Self::Prepared(view) => view.try_clone().map_err(PreparedClusterStoreError::Memory),
            Self::Ordinary(_) => Err(PreparedClusterStoreError::Memory(
                MemoryError::UnsupportedRunContract,
            )),
        }
    }

    pub(super) fn legacy(&self) -> Arc<[u8]> {
        match self {
            Self::Ordinary(bytes) => Arc::clone(bytes),
            Self::Prepared(view) => Arc::from(view.as_ref()),
        }
    }
}

impl AsRef<[u8]> for RetainedBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Ordinary(bytes) => bytes,
            Self::Prepared(view) => view.as_ref(),
        }
    }
}

pub(super) struct StoredChunk {
    pub(super) sequence: NonZeroU64,
    pub(super) payload: RetainedBytes,
}

impl StoredChunk {
    pub(super) fn legacy(&self) -> SavedChunk {
        SavedChunk {
            sequence: self.sequence,
            payload: self.payload.legacy(),
        }
    }
}

pub(super) struct StoredReply {
    pub(super) kind: ReplyKind,
    pub(super) payload: RetainedBytes,
}

impl StoredReply {
    pub(super) fn legacy(&self) -> SavedReply {
        SavedReply {
            kind: self.kind,
            payload: self.payload.legacy(),
        }
    }

    pub(super) fn matches(&self, kind: ReplyKind, payload: &[u8]) -> bool {
        self.kind == kind && self.payload.as_ref() == payload
    }

    fn snapshot(&self) -> Result<PreparedClusterReply, PreparedClusterStoreError> {
        Ok(PreparedClusterReply {
            kind: self.kind,
            payload: self.payload.view()?,
        })
    }
}

pub(super) struct StoredRequest {
    pub(super) info: PreparedAcceptedRequest,
    pub(super) payload: RetainedBytes,
    pub(super) key: Option<RetainedBytes>,
}

pub(super) enum History {
    Ordinary(Vec<StoredChunk>),
    Prepared(PreparedChunkValues<StoredChunk>),
}

impl History {
    pub(super) fn push(&mut self, value: StoredChunk) -> Result<(), ClusterStoreError> {
        match self {
            Self::Ordinary(values) => {
                values.push(value);
                Ok(())
            }
            Self::Prepared(values) => values
                .try_push(value)
                .map_err(|_| ClusterStoreError::CapacityExceeded),
        }
    }

    pub(super) fn has_room(&self) -> bool {
        match self {
            Self::Ordinary(_) => true,
            Self::Prepared(values) => values.len() < values.capacity(),
        }
    }
}

impl std::ops::Deref for History {
    type Target = [StoredChunk];

    fn deref(&self) -> &[StoredChunk] {
        match self {
            Self::Ordinary(values) => values,
            Self::Prepared(values) => values.as_slice(),
        }
    }
}

pub(super) enum Observers {
    Ordinary(BTreeMap<ClusterObserverId, ObserverState>),
    Prepared(PreparedChunkValues<(ClusterObserverId, ObserverState)>),
}

impl Observers {
    pub(super) fn iter(&self) -> impl Iterator<Item = (&ClusterObserverId, &ObserverState)> {
        let (ordinary, prepared) = match self {
            Self::Ordinary(values) => (Some(values.iter()), None),
            Self::Prepared(values) => (None, Some(values.as_slice().iter())),
        };
        ordinary.into_iter().flatten().chain(
            prepared
                .into_iter()
                .flatten()
                .map(|(id, cursor)| (id, cursor)),
        )
    }

    pub(super) fn insert(
        &mut self,
        observer: ClusterObserverId,
        max_bytes: Bytes,
    ) -> Result<(), ClusterStoreError> {
        let cursor = ObserverState {
            last_acknowledged: None,
            reply_acknowledged: false,
        };
        match self {
            Self::Ordinary(values) => {
                values.entry(observer).or_insert(cursor);
            }
            Self::Prepared(values) => {
                if values.as_slice().iter().any(|(id, _)| id == &observer) {
                    return Ok(());
                }
                if observer.as_bytes().len() > max_bytes.get() {
                    return Err(ClusterStoreError::CapacityExceeded);
                }
                values
                    .try_push((observer, cursor))
                    .map_err(|_| ClusterStoreError::CapacityExceeded)?;
                values
                    .as_mut_slice()
                    .sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            }
        }
        Ok(())
    }

    pub(super) fn get_mut(&mut self, observer: &ClusterObserverId) -> Option<&mut ObserverState> {
        match self {
            Self::Ordinary(values) => values.get_mut(observer),
            Self::Prepared(values) => values
                .as_mut_slice()
                .iter_mut()
                .find(|(id, _)| id == observer)
                .map(|(_, cursor)| cursor),
        }
    }

    pub(super) fn remove(&mut self, observer: &ClusterObserverId) -> bool {
        match self {
            Self::Ordinary(values) => values.remove(observer).is_some(),
            Self::Prepared(values) => values
                .as_slice()
                .iter()
                .position(|(id, _)| id == observer)
                .and_then(|index| values.remove_at(index))
                .is_some(),
        }
    }

    pub(super) fn clear(&mut self) {
        match self {
            Self::Ordinary(values) => values.clear(),
            Self::Prepared(values) => drop(values.take_items()),
        }
    }
}

/// Prepared entries are always stored in acceptance order, including replacements.
pub(super) enum RecordIndex {
    Ordinary(BTreeMap<MessageKey, Record>),
    Prepared(FixedVector<'static, (MessageKey, Record)>),
}

impl RecordIndex {
    pub(super) fn get(&self, key: &MessageKey) -> Option<&Record> {
        match self {
            Self::Ordinary(values) => values.get(key),
            Self::Prepared(values) => values
                .as_slice()
                .iter()
                .find(|(id, _)| id == key)
                .map(|(_, record)| record),
        }
    }

    pub(super) fn get_mut(&mut self, key: &MessageKey) -> Option<&mut Record> {
        match self {
            Self::Ordinary(values) => values.get_mut(key),
            Self::Prepared(values) => values
                .as_mut_slice()
                .iter_mut()
                .find(|(id, _)| id == key)
                .map(|(_, record)| record),
        }
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Ordinary(values) => values.len(),
            Self::Prepared(values) => values.len(),
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&MessageKey, &Record)> {
        let (ordinary, prepared) = match self {
            Self::Ordinary(values) => (Some(values.iter()), None),
            Self::Prepared(values) => (None, Some(values.as_slice().iter())),
        };
        ordinary.into_iter().flatten().chain(
            prepared
                .into_iter()
                .flatten()
                .map(|(key, record)| (key, record)),
        )
    }

    fn insert_reserved(&mut self, key: MessageKey, record: Record) {
        match self {
            Self::Ordinary(values) => {
                values.insert(key, record);
            }
            Self::Prepared(values) => values.push_reserved((key, record)),
        }
    }

    pub(super) fn remove(&mut self, key: &MessageKey) -> Option<Record> {
        match self {
            Self::Ordinary(values) => values.remove(key),
            Self::Prepared(values) => values
                .as_slice()
                .iter()
                .position(|(id, _)| id == key)
                .and_then(|position| values.remove_at(position))
                .map(|(_, record)| record),
        }
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&Record) -> bool) {
        match self {
            Self::Ordinary(values) => values.retain(|_, record| keep(record)),
            Self::Prepared(values) => values.retain(|(_, record)| keep(record)),
        }
    }
}

pub(super) struct PreparedStorage {
    pub(super) capacity: PreparedClusterStoreCapacity,
    pub(super) payloads: SlotPool<'static, Buffer<'static>>,
    histories: PreparedChunkPool<StoredChunk>,
    observers: PreparedChunkPool<(ClusterObserverId, ObserverState)>,
    snapshot_chunks: PreparedChunkPool<PreparedClusterChunk>,
    snapshot_cursors: PreparedChunkPool<ObserverCursor>,
    claims: PreparedChunkPool<PreparedClusterClaim>,
    scans: PreparedChunkPool<PreparedClusterSnapshot>,
}

impl PreparedClusterStoreCapacity {
    fn snapshot_records(self) -> Result<NonZeroUsize, MemoryError> {
        self.snapshots
            .checked_mul(self.records_per_page)
            .ok_or(MemoryError::ArithmeticOverflow)
    }

    /// Checked inline storage demand. Include region allocator headers and
    /// alignment padding when assigning total region capacity.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let snapshot_records = self.snapshot_records()?;
        let records = Layout::array::<(MessageKey, Record)>(self.records.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let payload = Layout::array::<u8>(self.bytes_per_payload.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        self.activation
            .storage_bytes()?
            .checked_add(Bytes::new(records.size()))?
            .checked_add(SlotPool::<Buffer<'static>>::storage_bytes(
                self.payloads.get(),
            )?)?
            .checked_add(Bytes::new(payload.size()).checked_mul(self.payloads.get())?)?
            .checked_add(PreparedChunkPool::<StoredChunk>::storage_bytes(
                self.records,
                self.chunks_per_record,
            )?)?
            .checked_add(
                PreparedChunkPool::<(ClusterObserverId, ObserverState)>::storage_bytes(
                    self.records,
                    self.observers_per_record,
                )?,
            )?
            .checked_add(PreparedChunkPool::<PreparedClusterChunk>::storage_bytes(
                snapshot_records,
                self.chunks_per_record,
            )?)?
            .checked_add(PreparedChunkPool::<ObserverCursor>::storage_bytes(
                snapshot_records,
                self.observers_per_record,
            )?)?
            .checked_add(PreparedChunkPool::<PreparedClusterClaim>::storage_bytes(
                self.snapshots,
                self.records_per_page,
            )?)?
            .checked_add(PreparedChunkPool::<PreparedClusterSnapshot>::storage_bytes(
                self.snapshots,
                self.records_per_page,
            )?)
    }
}

fn checkout_payload(
    pool: &SlotPool<'static, Buffer<'static>>,
) -> Result<PreparedPayload, MemoryError> {
    let mut owned = pool.try_checkout()?;
    owned.clear();
    Ok(owned)
}

fn retain_payload(
    pool: Option<&SlotPool<'static, Buffer<'static>>>,
    source: RetainedBytes,
) -> Result<RetainedBytes, PreparedClusterStoreError> {
    match (pool, source) {
        (Some(pool), RetainedBytes::Ordinary(bytes)) => {
            let mut owned = checkout_payload(pool).map_err(PreparedClusterStoreError::Memory)?;
            owned
                .extend_from_slice(&bytes)
                .map_err(PreparedClusterStoreError::Memory)?;
            Ok(RetainedBytes::Prepared(BufferView::new(owned.share())))
        }
        (_, retained) => Ok(retained),
    }
}

impl PreparedStorage {
    fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedClusterStoreCapacity,
    ) -> Result<Self, MemoryError> {
        let snapshot_records = capacity.snapshot_records()?;
        Ok(Self {
            capacity,
            payloads: memory.buffers(
                region,
                capacity.payloads.get(),
                capacity.bytes_per_payload,
            )?,
            histories: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.records,
                capacity.chunks_per_record,
            )?,
            observers: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.records,
                capacity.observers_per_record,
            )?,
            snapshot_chunks: PreparedChunkPool::prepare(
                memory,
                region,
                snapshot_records,
                capacity.chunks_per_record,
            )?,
            snapshot_cursors: PreparedChunkPool::prepare(
                memory,
                region,
                snapshot_records,
                capacity.observers_per_record,
            )?,
            claims: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.snapshots,
                capacity.records_per_page,
            )?,
            scans: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.snapshots,
                capacity.records_per_page,
            )?,
        })
    }

    fn identity_fits(&self, info: &PreparedAcceptedRequest) -> bool {
        info.address
            .identity_bytes()
            .and_then(|address| address.checked_add(Bytes::new(info.id.as_bytes().len())))
            .is_ok_and(|bytes| bytes.get() <= self.capacity.identity_bytes_per_record.get())
    }

    fn chunks(
        &self,
        record: &Record,
        after: Option<NonZeroU64>,
    ) -> Result<PreparedChunkValues<PreparedClusterChunk>, PreparedClusterStoreError> {
        let mut chunks = self
            .snapshot_chunks
            .try_checkout()
            .map_err(PreparedClusterStoreError::Memory)?;
        for chunk in record
            .chunks
            .iter()
            .filter(|chunk| after.is_none_or(|last| chunk.sequence > last))
        {
            chunks
                .try_push(PreparedClusterChunk {
                    sequence: chunk.sequence,
                    payload: chunk.payload.view()?,
                })
                .map_err(|_| ClusterStoreError::CapacityExceeded)?;
        }
        Ok(chunks)
    }

    fn snapshot(
        &self,
        record: &Record,
    ) -> Result<PreparedClusterSnapshot, PreparedClusterStoreError> {
        let mut observers = self
            .snapshot_cursors
            .try_checkout()
            .map_err(PreparedClusterStoreError::Memory)?;
        for (observer, cursor) in record.observers.iter() {
            observers
                .try_push(ObserverCursor {
                    observer: observer.clone(),
                    last_acknowledged: cursor.last_acknowledged,
                    reply_acknowledged: cursor.reply_acknowledged,
                })
                .map_err(|_| ClusterStoreError::CapacityExceeded)?;
        }
        let state = match &record.state {
            InternalState::Pending => PreparedClusterMessageState::Pending,
            InternalState::Claimed(token) => PreparedClusterMessageState::Claimed(token.clone()),
            InternalState::Completed {
                reply,
                completed_at,
                retained_until,
                ..
            } => PreparedClusterMessageState::Completed {
                reply: reply.snapshot()?,
                completed_at: *completed_at,
                retained_until: *retained_until,
            },
            InternalState::DeadLetter { attempts } => PreparedClusterMessageState::DeadLetter {
                attempts: *attempts,
            },
        };
        Ok(PreparedClusterSnapshot {
            address: record.request.info.address.clone(),
            request: record.request.info.id.clone(),
            payload: record.request.payload.view()?,
            accepted_at: record.request.info.accepted_at,
            policy: record.request.info.policy,
            state,
            chunks: self.chunks(record, None)?,
            observers,
        })
    }
}

pub(super) fn accept_bytes(
    state: &mut MemoryState,
    capacity: MessageStoreCapacity,
    mut info: PreparedAcceptedRequest,
    payload: RetainedBytes,
) -> Result<MessageAcceptance, PreparedClusterStoreError> {
    let key = (info.address.clone(), info.id.clone());
    let existing = state.records.get(&key);
    if let Some(record) = existing
        && !expiry(record, info.accepted_at)
    {
        if record.request.payload.as_ref() == payload.as_ref()
            && record.request.key.as_ref().map(AsRef::as_ref) == info.idempotency_key.as_deref()
            && record.request.info.policy == info.policy
        {
            return Ok(MessageAcceptance::Duplicate {
                disposition: disposition(&record.state),
            });
        }
        return match info.policy.payload_conflict {
            PayloadConflictPolicy::Reject => Err(ClusterStoreError::PayloadConflict.into()),
            PayloadConflictPolicy::ReportExisting => Ok(MessageAcceptance::PayloadConflict {
                disposition: disposition(&record.state),
            }),
        };
    }
    if existing.is_none()
        && matches!(capacity, MessageStoreCapacity::Bounded(limit) if state.records.len() >= limit.get())
    {
        return Err(ClusterStoreError::CapacityExceeded.into());
    }
    let order = state
        .next_order
        .ok_or(ClusterStoreError::SequenceExhausted)?;
    let storage = state.prepared.as_ref();
    if storage.is_some_and(|storage| !storage.identity_fits(&info)) {
        return Err(ClusterStoreError::CapacityExceeded.into());
    }
    let buffers = storage.map(|storage| &storage.payloads);
    let payload = retain_payload(buffers, payload)?;
    let idempotency_key = info
        .idempotency_key
        .take()
        .map(|bytes| retain_payload(buffers, RetainedBytes::Ordinary(bytes)))
        .transpose()?;

    let (chunks, observers) = match (storage, state.records.remove(&key)) {
        (None, _) => (
            History::Ordinary(Vec::new()),
            Observers::Ordinary(BTreeMap::new()),
        ),
        (Some(_), Some(previous)) => {
            let mut chunks = previous.chunks;
            let mut observers = previous.observers;
            match &mut chunks {
                History::Ordinary(values) => values.clear(),
                History::Prepared(values) => drop(values.take_items()),
            }
            observers.clear();
            (chunks, observers)
        }
        (Some(storage), None) => (
            History::Prepared(
                storage
                    .histories
                    .try_checkout()
                    .map_err(PreparedClusterStoreError::Memory)?,
            ),
            Observers::Prepared(
                storage
                    .observers
                    .try_checkout()
                    .map_err(PreparedClusterStoreError::Memory)?,
            ),
        ),
    };
    // A replacement vacated one entry; a new record checked the limit above.
    state.records.insert_reserved(
        key,
        Record {
            request: StoredRequest {
                info,
                payload,
                key: idempotency_key,
            },
            order,
            attempts: None,
            state: InternalState::Pending,
            chunks,
            observers,
        },
    );
    state.next_order = order.get().checked_add(1).and_then(NonZeroU64::new);
    Ok(MessageAcceptance::Accepted { order })
}

pub(super) fn save_chunk_bytes(
    state: &mut MemoryState,
    token: ClaimToken,
    sequence: NonZeroU64,
    payload: RetainedBytes,
    now: UtcMillis,
) -> Result<StoreMutation, PreparedClusterStoreError> {
    let storage = state.prepared.as_ref().map(|storage| &storage.payloads);
    let record = verify_claim(&mut state.records, &state.current_owner, &token, now)?;
    let current = usize::try_from(sequence.get()).map_err(|_| ClusterStoreError::ChunkGap)?;
    if current <= record.chunks.len() {
        let recorded = record
            .chunks
            .get(current - 1)
            .ok_or(ClusterStoreError::ChunkGap)?;
        return if recorded.payload.as_ref() == payload.as_ref() {
            Ok(StoreMutation::AlreadyStored)
        } else {
            Err(ClusterStoreError::ChunkConflict.into())
        };
    }
    let next = record
        .chunks
        .len()
        .checked_add(1)
        .ok_or(ClusterStoreError::SequenceExhausted)?;
    if current != next {
        return Err(ClusterStoreError::ChunkGap.into());
    }
    if !record.chunks.has_room() {
        return Err(ClusterStoreError::CapacityExceeded.into());
    }
    let payload = retain_payload(storage, payload)?;
    record.chunks.push(StoredChunk { sequence, payload })?;
    Ok(StoreMutation::Stored)
}

pub(super) fn complete_bytes(
    state: &mut MemoryState,
    token: ClaimToken,
    kind: ReplyKind,
    payload: RetainedBytes,
    completed_at: UtcMillis,
) -> Result<StoreCompletion, PreparedClusterStoreError> {
    let key = (token.address.clone(), token.request.clone());
    if let Some(record) = state.records.get(&key)
        && let InternalState::Completed {
            token: saved_token,
            reply,
            ..
        } = &record.state
    {
        return if saved_token == &token && reply.matches(kind, payload.as_ref()) {
            Ok(StoreCompletion::AlreadyCompleted)
        } else {
            Err(ClusterStoreError::ReplyConflict.into())
        };
    }
    let record = verify_claim(
        &mut state.records,
        &state.current_owner,
        &token,
        completed_at,
    )?;
    let retention = record
        .request
        .info
        .policy
        .dedup_retention
        .max(record.request.info.policy.reply_retention);
    let retained_until = completed_at
        .checked_add(retention)
        .map_err(|_| ClusterStoreError::RetentionOverflow)?;
    let payload = retain_payload(
        state.prepared.as_ref().map(|storage| &storage.payloads),
        payload,
    )?;
    record.state = InternalState::Completed {
        token,
        reply: StoredReply { kind, payload },
        completed_at,
        retained_until,
    };
    Ok(StoreCompletion::Completed)
}

/// One retained prepared reply chunk.
pub struct PreparedClusterChunk {
    /// Contiguous reply sequence.
    pub sequence: NonZeroU64,
    /// Byte owner, retained even after the original record is retired.
    pub payload: BufferView<'static>,
}

/// Prepared terminal reply and its retained bytes.
pub struct PreparedClusterReply {
    /// Terminal outcome.
    pub kind: ReplyKind,
    /// The final reply bytes.
    pub payload: BufferView<'static>,
}

/// A prepared claim owns its request bytes and prior reply history.
pub struct PreparedClusterClaim {
    /// Current owner and fenced attempt.
    pub token: ClaimToken,
    /// Accepted request bytes.
    pub payload: BufferView<'static>,
    /// Original acceptance instant.
    pub accepted_at: UtcMillis,
    chunks: PreparedChunkValues<PreparedClusterChunk>,
}

impl PreparedClusterClaim {
    /// Retained chunks from prior delivery attempts, in sequence order.
    #[must_use]
    pub fn chunks(&self) -> &[PreparedClusterChunk] {
        self.chunks.as_slice()
    }

    fn into_legacy(self) -> MessageClaim {
        MessageClaim {
            token: self.token,
            payload: Arc::from(self.payload.as_ref()),
            accepted_at: self.accepted_at,
            chunks: self
                .chunks
                .as_slice()
                .iter()
                .map(|chunk| SavedChunk {
                    sequence: chunk.sequence,
                    payload: Arc::from(chunk.payload.as_ref()),
                })
                .collect(),
        }
    }
}

/// Bounded, affine page of claims ordered by acceptance.
pub struct PreparedClusterClaims(PreparedChunkValues<PreparedClusterClaim>);

impl PreparedClusterClaims {
    /// Access this page without copying its payloads or history.
    #[must_use]
    pub fn as_slice(&self) -> &[PreparedClusterClaim] {
        self.0.as_slice()
    }

    /// Move the oldest claim out of the page.
    pub fn pop_front(&mut self) -> Option<PreparedClusterClaim> {
        self.0.pop_front()
    }

    pub(super) fn into_legacy(mut self) -> Vec<MessageClaim> {
        self.0
            .take_items()
            .map(PreparedClusterClaim::into_legacy)
            .collect()
    }
}

/// Observer snapshot retaining only the requested suffix of ordered history.
pub struct PreparedClusterObservation {
    /// Terminal reply, once completed.
    pub reply: Option<PreparedClusterReply>,
    /// Delivery state at observation.
    pub disposition: MessageDisposition,
    chunks: PreparedChunkValues<PreparedClusterChunk>,
}

impl PreparedClusterObservation {
    /// Ordered chunks newer than the caller's cursor.
    #[must_use]
    pub fn chunks(&self) -> &[PreparedClusterChunk] {
        self.chunks.as_slice()
    }
}

/// Prepared record state at the time of inspection.
pub enum PreparedClusterMessageState {
    /// Awaiting a claim.
    Pending,
    /// Current fenced delivery attempt.
    Claimed(ClaimToken),
    /// A retained terminal outcome.
    Completed {
        /// Recorded outcome.
        reply: PreparedClusterReply,
        /// When this claim completed.
        completed_at: UtcMillis,
        /// Earliest possible reclamation, subject to observer acknowledgements.
        retained_until: UtcMillis,
    },
    /// Delivery attempts were exhausted.
    DeadLetter {
        /// Number of recorded attempts.
        attempts: NonZeroU32,
    },
}

/// Prepared inspection snapshot, independent of future store mutations.
pub struct PreparedClusterSnapshot {
    /// Original destination.
    pub address: ClusterAddress,
    /// Request identity.
    pub request: ClusterRequestId,
    /// Accepted request bytes.
    pub payload: BufferView<'static>,
    /// Original acceptance instant.
    pub accepted_at: UtcMillis,
    /// Recorded acceptance policy.
    pub policy: AcceptancePolicy,
    /// Delivery state when this record was inspected.
    pub state: PreparedClusterMessageState,
    chunks: PreparedChunkValues<PreparedClusterChunk>,
    observers: PreparedChunkValues<ObserverCursor>,
}

impl PreparedClusterSnapshot {
    /// Recorded chunks in sequence order.
    #[must_use]
    pub fn chunks(&self) -> &[PreparedClusterChunk] {
        self.chunks.as_slice()
    }

    /// Observer cursors in identifier order.
    #[must_use]
    pub fn observers(&self) -> &[ObserverCursor] {
        self.observers.as_slice()
    }
}

/// Bounded inspection page in acceptance order.
pub struct PreparedClusterSnapshots(PreparedChunkValues<PreparedClusterSnapshot>);

impl PreparedClusterSnapshots {
    /// Borrow the page without copying its snapshots.
    #[must_use]
    pub fn as_slice(&self) -> &[PreparedClusterSnapshot] {
        self.0.as_slice()
    }
}

fn claim_candidate(record: &Record, options: &ClaimOptions) -> bool {
    record.request.info.address == options.owner.address
        && match &record.state {
            InternalState::Pending => true,
            InternalState::Claimed(token) => {
                token.owner != options.owner || options.now >= token.lease_until
            }
            InternalState::Completed { .. } | InternalState::DeadLetter { .. } => false,
        }
}

pub(super) fn claim(
    state: &mut MemoryState,
    options: ClaimOptions,
) -> Result<PreparedClusterClaims, PreparedClusterStoreError> {
    let storage = state
        .prepared
        .as_ref()
        .ok_or(PreparedClusterStoreError::Memory(
            MemoryError::UnsupportedRunContract,
        ))?;
    super::validate_claim_options(&state.current_owner, &options)?;

    // Stage every payload owner and sequence before applying any claim or
    // dead-letter transition. The index stays locked across both passes.
    let mut claims = storage
        .claims
        .try_checkout()
        .map_err(PreparedClusterStoreError::Memory)?;
    let records = match &mut state.records {
        RecordIndex::Prepared(records) => records,
        RecordIndex::Ordinary(_) => {
            return Err(PreparedClusterStoreError::Memory(
                MemoryError::UnsupportedRunContract,
            ));
        }
    };
    let mut next_generation = state.next_claim_generation;
    let cap = options.limit.get();
    if cap > claims.capacity() {
        return Err(ClusterStoreError::CapacityExceeded.into());
    }
    for (_, record) in records
        .as_slice()
        .iter()
        .filter(|(_, record)| claim_candidate(record, &options))
        .take(cap)
    {
        let attempt = match record.attempts {
            Some(previous) if previous >= record.request.info.policy.maximum_attempts => continue,
            Some(previous) => previous
                .get()
                .checked_add(1)
                .and_then(NonZeroU32::new)
                .ok_or(ClusterStoreError::AttemptExhausted)?,
            None => NonZeroU32::MIN,
        };
        let generation = next_generation.ok_or(ClusterStoreError::ClaimGenerationExhausted)?;
        next_generation = generation.get().checked_add(1).and_then(NonZeroU64::new);
        claims.push_reserved(PreparedClusterClaim {
            token: ClaimToken {
                address: record.request.info.address.clone(),
                request: record.request.info.id.clone(),
                owner: options.owner.clone(),
                claim_generation: generation,
                attempt,
                lease_until: options.lease_until,
            },
            payload: record.request.payload.view()?,
            accepted_at: record.request.info.accepted_at,
            chunks: storage.chunks(record, None)?,
        });
    }

    let mut staged = claims.as_slice().iter();
    for (_, record) in records
        .as_mut_slice()
        .iter_mut()
        .filter(|(_, record)| claim_candidate(record, &options))
        .take(cap)
    {
        if let Some(attempts) = record.attempts
            && attempts >= record.request.info.policy.maximum_attempts
        {
            record.state = InternalState::DeadLetter { attempts };
        } else if let Some(claim) = staged.next() {
            record.attempts = Some(claim.token.attempt);
            record.state = InternalState::Claimed(claim.token.clone());
        }
    }
    state.next_claim_generation = next_generation;
    Ok(PreparedClusterClaims(claims))
}

fn observe(
    state: &mut MemoryState,
    key: MessageKey,
    observer: ClusterObserverId,
    after: Option<NonZeroU64>,
) -> Result<PreparedClusterObservation, PreparedClusterStoreError> {
    let storage = state
        .prepared
        .as_ref()
        .ok_or(PreparedClusterStoreError::Memory(
            MemoryError::UnsupportedRunContract,
        ))?;
    let record = state
        .records
        .get_mut(&key)
        .ok_or(ClusterStoreError::MissingRequest)?;
    let chunks = storage.chunks(record, after)?;
    let reply = match &record.state {
        InternalState::Completed { reply, .. } => Some(reply.snapshot()?),
        InternalState::Pending | InternalState::Claimed(_) | InternalState::DeadLetter { .. } => {
            None
        }
    };
    record
        .observers
        .insert(observer, storage.capacity.identity_bytes_per_observer)?;
    Ok(PreparedClusterObservation {
        reply,
        disposition: disposition(&record.state),
        chunks,
    })
}

impl MemoryClusterMessageStore {
    /// Provision physical record, history, observer and payload storage in the
    /// caller-selected region. All selected paths share the activation fence.
    ///
    /// The legacy trait still works through its ordinary owned output types.
    /// Use the prepared methods to retain snapshots without copying bytes or
    /// allocating a native result collection.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedClusterStoreCapacity,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        Ok(Self {
            state: Arc::new(Mutex::new(MemoryState {
                records: RecordIndex::Prepared(FixedVector::prepare(
                    memory.allocator(region)?,
                    capacity.records.get(),
                )?),
                current_owner: ActivationTable::prepare(memory, region, capacity.activation)?,
                prepared: Some(PreparedStorage::prepare(memory, region, capacity)?),
                next_order: Some(NonZeroU64::MIN),
                next_claim_generation: Some(NonZeroU64::MIN),
            })),
            capacity: MessageStoreCapacity::Bounded(capacity.records),
        })
    }

    /// Reserve byte backing before invoking an encoder or provider callback.
    pub fn reserve_prepared_payload(
        &self,
    ) -> Result<PreparedClusterPayloadReservation, MemoryError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let storage = state
            .prepared
            .as_ref()
            .ok_or(MemoryError::UnsupportedRunContract)?;
        Ok(PreparedClusterPayloadReservation {
            store: Arc::clone(&self.state),
            buffer: checkout_payload(&storage.payloads)?,
        })
    }

    fn take_payload(
        &self,
        payload: PreparedClusterPayloadReservation,
    ) -> Result<RetainedBytes, PreparedClusterStoreError> {
        if !Arc::ptr_eq(&self.state, &payload.store) {
            return Err(PreparedClusterStoreError::Memory(
                MemoryError::UnsupportedRunContract,
            ));
        }
        Ok(RetainedBytes::Prepared(BufferView::new(
            payload.buffer.share(),
        )))
    }

    /// Commit a prepared request without copying its reserved payload.
    pub fn accept_prepared_now(
        &self,
        request: PreparedAcceptedRequest,
        payload: PreparedClusterPayloadReservation,
    ) -> Result<MessageAcceptance, PreparedClusterStoreError> {
        let payload = self.take_payload(payload)?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        accept_bytes(&mut state, self.capacity, request, payload)
    }

    /// Append a contiguous chunk under the exact activation and claim fence.
    pub fn save_chunk_prepared_now(
        &self,
        token: ClaimToken,
        sequence: NonZeroU64,
        payload: PreparedClusterPayloadReservation,
        now: UtcMillis,
    ) -> Result<StoreMutation, PreparedClusterStoreError> {
        let payload = self.take_payload(payload)?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        save_chunk_bytes(&mut state, token, sequence, payload, now)
    }

    /// Commit the prepared reply under the exact activation and claim fence.
    pub fn complete_prepared_now(
        &self,
        token: ClaimToken,
        kind: ReplyKind,
        payload: PreparedClusterPayloadReservation,
        completed_at: UtcMillis,
    ) -> Result<StoreCompletion, PreparedClusterStoreError> {
        let payload = self.take_payload(payload)?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        complete_bytes(&mut state, token, kind, payload, completed_at)
    }

    pub(in crate::cluster) fn check_prepared_claim_options(
        &self,
        options: &ClaimOptions,
    ) -> Result<(), PreparedClusterStoreError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.prepared.is_none() {
            return Err(PreparedClusterStoreError::Memory(
                MemoryError::UnsupportedRunContract,
            ));
        }
        super::validate_claim_options(&state.current_owner, options).map_err(Into::into)
    }

    /// Claim a bounded page, synchronously, for a prepared native runner.
    pub fn claim_prepared_now(
        &self,
        options: ClaimOptions,
    ) -> Result<PreparedClusterClaims, PreparedClusterStoreError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        claim(&mut state, options)
    }

    /// Claim a bounded page when the returned effect executes.
    pub fn claim_prepared(
        &self,
        options: ClaimOptions,
    ) -> Effect<PreparedClusterClaims, PreparedClusterStoreError> {
        self.transact(options, claim)
    }

    /// Retain observer history and the current typed terminal reply synchronously.
    pub fn observe_prepared_now(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        after: Option<NonZeroU64>,
    ) -> Result<PreparedClusterObservation, PreparedClusterStoreError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        observe(&mut state, (address, request), observer, after)
    }

    /// Retain observer history and terminal reply when the effect executes.
    pub fn observe_prepared(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
        observer: ClusterObserverId,
        after: Option<NonZeroU64>,
    ) -> Effect<PreparedClusterObservation, PreparedClusterStoreError> {
        self.transact(
            ((address, request), observer, after),
            |state, (key, observer, after)| observe(state, key, observer, after),
        )
    }

    /// Retain one record snapshot without copying its byte owners.
    pub fn inspect_prepared(
        &self,
        address: ClusterAddress,
        request: ClusterRequestId,
    ) -> Effect<Option<PreparedClusterSnapshot>, PreparedClusterStoreError> {
        self.transact((address, request), |state, key| {
            let storage = state
                .prepared
                .as_ref()
                .ok_or(PreparedClusterStoreError::Memory(
                    MemoryError::UnsupportedRunContract,
                ))?;
            state
                .records
                .get(&key)
                .map(|record| storage.snapshot(record))
                .transpose()
        })
    }

    /// Retain a bounded record page in original acceptance order.
    pub fn scan_prepared(
        &self,
        scan: MessageScan,
    ) -> Effect<PreparedClusterSnapshots, PreparedClusterStoreError> {
        self.transact(scan, |state, scan| {
            let storage = state
                .prepared
                .as_ref()
                .ok_or(PreparedClusterStoreError::Memory(
                    MemoryError::UnsupportedRunContract,
                ))?;
            let mut page = storage
                .scans
                .try_checkout()
                .map_err(PreparedClusterStoreError::Memory)?;
            if scan.limit.get() > page.capacity() {
                return Err(ClusterStoreError::CapacityExceeded.into());
            }
            for (_, record) in state
                .records
                .iter()
                .filter(|(_, record)| record.request.info.address == scan.address)
                .take(scan.limit.get())
            {
                page.push_reserved(storage.snapshot(record)?);
            }
            Ok(PreparedClusterSnapshots(page))
        })
    }
}
