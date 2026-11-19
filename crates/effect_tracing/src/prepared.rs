//! Prepared native span records and checked redaction output.

use super::LiveSpan;
use effect::memory::{
    Buffer, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease,
    SlotPool,
};
use effect::{Annotations, Defect, LibraryDefect, SpanId, SpanStart};
use std::alloc::Layout;
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::Duration;

const WORD: usize = std::mem::size_of::<usize>();

/// Caller-selected live span and per-record metadata storage.
#[derive(Clone, Copy, Debug)]
pub struct PreparedTracingCapacity {
    /// Live spans, accepted starts, and spans held by an active native callback.
    pub spans: NonZeroUsize,
    /// Maximum UTF-8 bytes in one span name.
    pub name_bytes: Bytes,
    /// Maximum number of source and redacted annotations.
    pub annotations: usize,
    /// Maximum packed annotation bytes, including two native-word lengths per pair.
    pub annotation_bytes: Bytes,
}

impl PreparedTracingCapacity {
    fn record_bytes(self) -> Result<Bytes, MemoryError> {
        let bytes = self.name_bytes.checked_add(self.annotation_bytes)?;
        Layout::array::<u8>(bytes.get()).map_err(|_| MemoryError::InvalidLayout)?;
        Ok(bytes)
    }

    /// Actual span records and metadata payload, excluding region allocator overhead.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let spans = self.spans.get();
        let index = Layout::array::<SpanOwner>(spans).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<SpanRecord>::storage_bytes(spans)?
            .checked_add(Bytes::new(index.size()))?
            .checked_add(self.record_bytes()?.checked_mul(spans)?)
    }
}

struct SpanRecord {
    metadata: Buffer<'static>,
    id: Option<SpanId>,
    native: Option<LiveSpan>,
}

pub(super) struct SpanOwner(Option<SharedSlot<'static, SpanRecord>>);

impl SpanOwner {
    pub(super) fn try_clone(&self) -> Result<Self, MemoryError> {
        self.0
            .as_ref()
            .ok_or(MemoryError::Closed)?
            .try_clone()
            .map(|slot| Self(Some(slot)))
    }

    pub(super) fn native(&self) -> Option<&tracing::Span> {
        self.0
            .as_ref()
            .and_then(|slot| slot.native.as_ref().map(|span| &span.native))
    }

    fn id(&self) -> Option<&SpanId> {
        self.0.as_ref().and_then(|slot| slot.id.as_ref())
    }
}

impl Drop for SpanOwner {
    fn drop(&mut self) {
        if let Some(mut record) = self.0.take().and_then(SharedSlot::into_last) {
            let native = record.native.take();
            record.id = None;
            record.metadata.clear();
            drop(native);
        }
    }
}

pub(super) struct SpanVacancy {
    record: SlotLease<'static, SpanRecord>,
    capacity: PreparedTracingCapacity,
}

impl SpanVacancy {
    pub(super) fn write<T>(
        &mut self,
        info: &SpanStart,
        redact: impl FnOnce(&Annotations, &mut PreparedSpanAnnotations<'_>) -> Result<T, Defect>,
    ) -> Result<T, Defect> {
        validate_source(info, self.capacity).map_err(capacity_defect)?;
        let name = info.name.as_bytes();
        self.record.metadata.clear();
        self.record
            .metadata
            .extend_from_slice(name)
            .map_err(capacity_defect)?;
        let mut output = PreparedSpanAnnotations {
            buffer: &mut self.record.metadata,
            name: name.len(),
            limit: self.capacity.annotation_bytes.get(),
            maximum: self.capacity.annotations,
            count: 0,
            failure: None,
        };
        let result = redact(&info.annotations, &mut output)?;
        output
            .failure
            .map_or(Ok(result), |error| Err(capacity_defect(error)))
    }

    pub(super) fn info<'a>(&'a self, info: &'a SpanStart) -> PreparedSpanStart<'a> {
        let (name, annotations) = self.record.metadata.as_slice().split_at(info.name.len());
        PreparedSpanStart {
            name: str::from_utf8(name).unwrap_or_default(),
            parent: info.parent.as_ref(),
            time: info.time,
            annotations,
        }
    }

    pub(super) fn publish(mut self, id: SpanId, native: LiveSpan) -> SpanOwner {
        self.record.id = Some(id);
        self.record.native = Some(native);
        SpanOwner(Some(self.record.share()))
    }
}

pub(super) struct PreparedSpans {
    records: SlotPool<'static, SpanRecord>,
    index: Mutex<FixedVector<'static, SpanOwner>>,
    capacity: PreparedTracingCapacity,
}

impl PreparedSpans {
    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedTracingCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.storage_bytes()?;
        let bytes = capacity.record_bytes()?;
        Ok(Self {
            records: memory.slots(region, capacity.spans.get(), |_| {
                Ok(SpanRecord {
                    metadata: memory.buffer(region, bytes)?,
                    id: None,
                    native: None,
                })
            })?,
            index: Mutex::new(FixedVector::prepare(
                memory.allocator(region)?,
                capacity.spans.get(),
            )?),
            capacity,
        })
    }

    pub(super) fn reserve(&self) -> Result<SpanVacancy, Defect> {
        let record = self.records.try_checkout().map_err(capacity_defect)?;
        Ok(SpanVacancy {
            record,
            capacity: self.capacity,
        })
    }

    pub(super) fn active(&self) -> usize {
        self.index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub(super) fn lookup(&self, id: &str) -> Option<SpanOwner> {
        self.index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_slice()
            .iter()
            .find(|entry| entry.id().is_some_and(|candidate| candidate.as_str() == id))
            .and_then(|entry| entry.try_clone().ok())
    }

    pub(super) fn insert(&self, id: &SpanId, entry: SpanOwner) -> Result<(), Defect> {
        let mut index = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = remove(&mut index, id);
        let inserted = index.push(entry);
        drop(index);
        drop(previous);
        inserted.map_err(|_| capacity_defect(MemoryError::NoSlot))
    }

    pub(super) fn remove(&self, id: &SpanId) -> Option<SpanOwner> {
        let mut index = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        remove(&mut index, id)
    }
}

fn remove(index: &mut FixedVector<'static, SpanOwner>, id: &SpanId) -> Option<SpanOwner> {
    let position = index
        .as_slice()
        .iter()
        .position(|entry| entry.id() == Some(id))?;
    index.as_mut_slice().split_at_mut(position).1.rotate_left(1);
    index.pop()
}

fn validate_source(info: &SpanStart, capacity: PreparedTracingCapacity) -> Result<(), MemoryError> {
    let requested = Bytes::new(info.name.len());
    if requested > capacity.name_bytes {
        return Err(MemoryError::ValueTooLarge {
            requested,
            capacity: capacity.name_bytes,
        });
    }
    if info.annotations.len() > capacity.annotations {
        return Err(MemoryError::CollectionReservationFailed);
    }
    let bytes = info
        .annotations
        .iter()
        .try_fold(Bytes::new(0), |bytes, (key, value)| {
            bytes.checked_add(pair_bytes(key, value)?)
        })?;
    if bytes > capacity.annotation_bytes {
        return Err(MemoryError::ValueTooLarge {
            requested: bytes,
            capacity: capacity.annotation_bytes,
        });
    }
    Ok(())
}

fn pair_bytes(key: &str, value: &str) -> Result<Bytes, MemoryError> {
    Bytes::new(2 * WORD)
        .checked_add(Bytes::new(key.len()))?
        .checked_add(Bytes::new(value.len()))
}

pub(super) fn capacity_defect(_: MemoryError) -> Defect {
    Defect::from(LibraryDefect::ProviderCapacityFull)
}

/// Checked destination for a redactor to write metadata before native span entry.
pub struct PreparedSpanAnnotations<'a> {
    buffer: &'a mut Buffer<'static>,
    name: usize,
    limit: usize,
    maximum: usize,
    count: usize,
    failure: Option<MemoryError>,
}

impl PreparedSpanAnnotations<'_> {
    /// Write one pair in the selected annotation storage. A refused write
    /// prevents publication even if the callback does not propagate its error.
    pub fn push(&mut self, key: &str, value: &str) -> Result<(), MemoryError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let result = self.push_checked(key, value);
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }

    fn push_checked(&mut self, key: &str, value: &str) -> Result<(), MemoryError> {
        if self.count == self.maximum {
            return Err(MemoryError::CollectionReservationFailed);
        }
        let requested =
            Bytes::new(self.buffer.len() - self.name).checked_add(pair_bytes(key, value)?)?;
        let capacity = Bytes::new(self.limit);
        if requested > capacity {
            return Err(MemoryError::ValueTooLarge {
                requested,
                capacity,
            });
        }
        self.buffer.extend_from_slice(&key.len().to_ne_bytes())?;
        self.buffer.extend_from_slice(key.as_bytes())?;
        self.buffer.extend_from_slice(&value.len().to_ne_bytes())?;
        self.buffer.extend_from_slice(value.as_bytes())?;
        self.count += 1;
        Ok(())
    }
}

/// Borrowed, validated and redacted metadata for one admitted native span.
pub struct PreparedSpanStart<'a> {
    pub name: &'a str,
    pub parent: Option<&'a SpanId>,
    pub time: Duration,
    annotations: &'a [u8],
}

impl PreparedSpanStart<'_> {
    /// Borrow annotations in the order written by the redactor.
    pub fn annotations(&self) -> impl Iterator<Item = (&str, &str)> {
        let mut bytes = self.annotations;
        std::iter::from_fn(move || Some((take_text(&mut bytes)?, take_text(&mut bytes)?)))
    }
}

impl fmt::Debug for PreparedSpanStart<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // As with ordinary SharedAnnotations, do not expose annotated values.
        formatter
            .debug_struct("PreparedSpanStart")
            .field("annotations", &self.annotations().count())
            .finish_non_exhaustive()
    }
}

fn take_text<'a>(bytes: &mut &'a [u8]) -> Option<&'a str> {
    let (length, rest) = bytes.split_first_chunk::<WORD>()?;
    let (text, tail) = rest.split_at_checked(usize::from_ne_bytes(*length))?;
    *bytes = tail;
    str::from_utf8(text).ok()
}
