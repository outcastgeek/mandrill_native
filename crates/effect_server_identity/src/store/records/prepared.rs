use super::{RecordCodec, RecordError, RecordPolicy};
use effect::memory::{Buffer, Bytes, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool};
use std::{
    alloc::Layout,
    num::{NonZeroU16, NonZeroUsize},
    sync::Arc,
};

/// Finite physical storage for private record wire bytes shared by selected adapters.
/// Each live write or read (including a page) holds one buffer.
pub struct PreparedRecordMemory {
    buffers: SlotPool<'static, Buffer<'static>>,
    bytes_per_record: NonZeroUsize,
}

impl PreparedRecordMemory {
    /// Checked demand for reusable buffers and their lease controls.
    pub fn storage_bytes(
        operations: NonZeroUsize,
        bytes_per_record: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        let buffer =
            Layout::array::<u8>(bytes_per_record.get()).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Buffer<'static>>::storage_bytes(operations.get())?
            .checked_add(Bytes::new(buffer.size()).checked_mul(operations.get())?)
    }

    /// Allocate the selected record working set in the supplied startup region.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        operations: NonZeroUsize,
        bytes_per_record: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(operations, bytes_per_record)?;
        Ok(Self {
            buffers: memory.buffers(
                region,
                operations.get(),
                Bytes::new(bytes_per_record.get()),
            )?,
            bytes_per_record,
        })
    }

    fn check_policy(&self, policy: RecordPolicy) -> Result<(), MemoryError> {
        if policy.max_bytes > self.bytes_per_record {
            return Err(MemoryError::ValueTooLarge {
                requested: Bytes::new(policy.max_bytes.get()),
                capacity: Bytes::new(self.bytes_per_record.get()),
            });
        }
        Ok(())
    }

    fn reserve(&self) -> Result<RecordBuffer, MemoryError> {
        Ok(RecordBuffer(self.buffers.try_checkout()?))
    }
}

/// A codec for the selected working set. Decoded owned values belong to the codec
/// and may outlive the reserved borrowed bytes. Supply a bounded native value type
/// or an independently admitted owner when a decoded value must retain its backing.
pub trait PreparedRecordCodec<A>: RecordCodec<A> {
    /// Encode directly into the real admitted record buffer.
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed codec ABI"
    )]
    fn encode_prepared(&self, value: &A, writer: &mut RecordWriter<'_>) -> Result<(), RecordError>;

    /// Validate the version and decode from a borrowed bounded secret buffer.
    #[expect(
        clippy::result_large_err,
        reason = "preserves the established typed codec ABI"
    )]
    fn decode_prepared(&self, version: NonZeroU16, bytes: &[u8]) -> Result<A, RecordError>;
}

/// Append-only view of the preadmitted record backing.
pub struct RecordWriter<'a> {
    buffer: &'a mut Buffer<'static>,
    limit: NonZeroUsize,
    overflow: bool,
}

impl RecordWriter<'_> {
    /// Append bytes atomically against the installed record policy.
    pub fn extend_from_slice(&mut self, bytes: &[u8]) -> Result<(), MemoryError> {
        let overflow = self
            .buffer
            .len()
            .checked_add(bytes.len())
            .ok_or(MemoryError::ArithmeticOverflow)
            .and_then(|total| {
                if total <= self.limit.get() {
                    Ok(())
                } else {
                    Err(MemoryError::ValueTooLarge {
                        requested: Bytes::new(total),
                        capacity: Bytes::new(self.limit.get()),
                    })
                }
            });
        let result = overflow.and_then(|()| self.buffer.extend_from_slice(bytes));
        self.overflow |= result.is_err();
        result
    }
}

pub(in crate::store) struct RecordBuffer(SlotLease<'static, Buffer<'static>>);

impl RecordBuffer {
    fn stage(&mut self, bytes: &[u8]) -> Result<&[u8], MemoryError> {
        // Stale bytes from the previous row stay private until this lease is erased.
        self.0.clear();
        self.0.extend_from_slice(bytes)?;
        Ok(self.0.as_slice())
    }
}

impl Drop for RecordBuffer {
    fn drop(&mut self) {
        self.0.clear_zeroed();
    }
}

pub(super) enum EncodedPayload {
    Ordinary(Vec<u8>),
    Prepared(RecordBuffer),
}

impl EncodedPayload {
    pub(super) fn binding(&mut self) -> PayloadBinding<'_> {
        match self {
            Self::Ordinary(bytes) => PayloadBinding::Ordinary(std::mem::take(bytes)),
            Self::Prepared(buffer) => PayloadBinding::Prepared(buffer.0.as_slice()),
        }
    }
}

pub(super) enum PayloadBinding<'a> {
    Ordinary(Vec<u8>),
    Prepared(&'a [u8]),
}

impl<'a> PayloadBinding<'a> {
    pub(super) fn bind<'q>(
        self,
        query: sqlx::query::Query<'q, sqlx::Turso, sqlx::TursoArguments>,
    ) -> sqlx::query::Query<'q, sqlx::Turso, sqlx::TursoArguments>
    where
        'a: 'q,
    {
        match self {
            Self::Ordinary(bytes) => query.bind(bytes),
            Self::Prepared(bytes) => query.bind(bytes),
        }
    }
}

pub(super) struct PreparedCodec<A, C> {
    memory: Arc<PreparedRecordMemory>,
    encode: fn(&C, &A, &mut RecordWriter<'_>) -> Result<(), RecordError>,
    decode: fn(&C, NonZeroU16, &[u8]) -> Result<A, RecordError>,
}

impl<A, C: PreparedRecordCodec<A>> PreparedCodec<A, C> {
    pub(super) fn new(
        memory: Arc<PreparedRecordMemory>,
        policy: RecordPolicy,
    ) -> Result<Self, MemoryError> {
        memory.check_policy(policy)?;
        Ok(Self {
            memory,
            encode: C::encode_prepared,
            decode: C::decode_prepared,
        })
    }
}

pub(super) fn reserve(
    prepared: Option<&PreparedCodec<impl Sized, impl Sized>>,
) -> Result<Option<RecordBuffer>, MemoryError> {
    prepared.map(|codec| codec.memory.reserve()).transpose()
}

#[expect(
    clippy::result_large_err,
    reason = "preserves the established typed codec ABI"
)]
pub(super) fn encode<A, C: RecordCodec<A>>(
    codec: &C,
    prepared: Option<&PreparedCodec<A, C>>,
    policy: RecordPolicy,
    value: &A,
    buffer: Option<RecordBuffer>,
) -> Result<EncodedPayload, RecordError> {
    match (prepared, buffer) {
        (Some(selected), Some(mut buffer)) => {
            let mut writer = RecordWriter {
                buffer: &mut buffer.0,
                limit: policy.max_bytes,
                overflow: false,
            };
            let result = (selected.encode)(codec, value, &mut writer);
            if writer.overflow {
                return Err(RecordError::Bound);
            }
            result?;
            Ok(EncodedPayload::Prepared(buffer))
        }
        (None, None) => {
            let bytes = codec.encode(value)?;
            if bytes.len() > policy.max_bytes.get() {
                return Err(RecordError::Bound);
            }
            Ok(EncodedPayload::Ordinary(bytes))
        }
        (Some(_), None) | (None, Some(_)) => Err(RecordError::WrongConfiguration),
    }
}

#[expect(
    clippy::result_large_err,
    reason = "preserves the established typed codec ABI"
)]
pub(super) fn decode<A, C: RecordCodec<A>>(
    codec: &C,
    prepared: Option<&PreparedCodec<A, C>>,
    policy: RecordPolicy,
    version: NonZeroU16,
    bytes: &[u8],
    buffer: Option<&mut RecordBuffer>,
) -> Result<A, RecordError> {
    if bytes.len() > policy.max_bytes.get() {
        return Err(RecordError::Corrupt);
    }
    match (prepared, buffer) {
        (Some(selected), Some(buffer)) => {
            let bytes = buffer.stage(bytes).map_err(|_| RecordError::Bound)?;
            (selected.decode)(codec, version, bytes)
        }
        (None, None) => codec.decode(version, bytes),
        (Some(_), None) | (None, Some(_)) => Err(RecordError::WrongConfiguration),
    }
}
