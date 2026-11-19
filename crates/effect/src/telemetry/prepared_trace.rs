use super::TraceContext;
use crate::memory::{Buffer, Bytes, MemoryError, PreparedMemory, RegionId, SharedBuffer, SlotPool};
use std::fmt;

const LENGTH_BYTES: usize = core::mem::size_of::<usize>();
// A valid Rust str length cannot use this sentinel.
const NO_SPAN: usize = usize::MAX;

/// Startup storage selected for prepared trace snapshots.
#[derive(Clone, Copy, Debug)]
pub struct PreparedTraceCapacity {
    /// Number of simultaneously retained snapshots.
    pub snapshots: usize,
    /// Maximum number of distinct annotation keys per snapshot.
    pub annotations: usize,
    /// Bytes per snapshot for span text, keys, values and two native-word lengths per key.
    /// Snapshot metadata is included separately in the storage demand.
    pub bytes_per_snapshot: Bytes,
}

impl PreparedTraceCapacity {
    /// Region payload bytes, including the snapshot pool and each complete backing buffer.
    /// Region allocation headers and alignment are additional.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        SlotPool::<Buffer<'static>>::storage_bytes(self.snapshots)?
            .checked_add(self.buffer_bytes()?.checked_mul(self.snapshots)?)
    }

    fn buffer_bytes(self) -> Result<Bytes, MemoryError> {
        self.bytes_per_snapshot
            .checked_add(Bytes::new(LENGTH_BYTES))
    }
}

/// A fixed bank of physical trace snapshots. Published views retain their whole
/// byte backing until the final consumer releases it.
pub struct PreparedTracePool {
    snapshots: SlotPool<'static, Buffer<'static>>,
    annotations: usize,
}

impl PreparedTracePool {
    /// Provision the actual snapshot backing before sealing the selected memory.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedTraceCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.storage_bytes()?;
        Ok(Self {
            snapshots: memory.buffers(region, capacity.snapshots, capacity.buffer_bytes()?)?,
            annotations: capacity.annotations,
        })
    }

    /// Retain this bank without provisioning another one.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            snapshots: self.snapshots.try_clone()?,
            annotations: self.annotations,
        })
    }

    /// Copy the trace text into one admitted snapshot while borrowing the source.
    /// The result does not retain the source map or its string owners.
    pub fn try_capture(&self, trace: &TraceContext) -> Result<PreparedTraceContext, MemoryError> {
        self.pack(
            trace.span.as_ref().map(super::SpanId::as_str),
            trace
                .annotations
                .iter()
                .map(|(key, value)| (key.as_ref(), value.as_ref())),
        )
    }

    /// Capture borrowed text directly. Annotations are visited in key order;
    /// when a key occurs more than once the last supplied value is retained.
    pub fn try_store(
        &self,
        span: Option<&str>,
        annotations: &[(&str, &str)],
    ) -> Result<PreparedTraceContext, MemoryError> {
        let mut previous = None;
        let ordered = std::iter::from_fn(move || {
            let next = annotations
                .iter()
                .rev()
                .copied()
                .filter(|(key, _)| previous.is_none_or(|previous| *key > previous))
                .min_by(|(left, _), (right, _)| left.cmp(right))?;
            previous = Some(next.0);
            Some(next)
        });
        self.pack(span, ordered)
    }

    fn pack<'a>(
        &self,
        span: Option<&str>,
        annotations: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Result<PreparedTraceContext, MemoryError> {
        let mut storage = self.snapshots.try_checkout()?;
        storage.clear();
        storage.extend_from_slice(&span.map_or(NO_SPAN, str::len).to_ne_bytes())?;
        if let Some(span) = span {
            storage.extend_from_slice(span.as_bytes())?;
        }
        let mut count = 0usize;
        for (key, value) in annotations {
            count = count
                .checked_add(1)
                .ok_or(MemoryError::ArithmeticOverflow)?;
            if count > self.annotations {
                return Err(MemoryError::CapacityExceeded {
                    resource: 0,
                    requested: count,
                    available: self.annotations,
                    control: false,
                });
            }
            append_text(&mut storage, key)?;
            append_text(&mut storage, value)?;
        }
        Ok(PreparedTraceContext {
            bytes: storage.share(),
        })
    }
}

fn append_text(buffer: &mut Buffer<'_>, text: &str) -> Result<(), MemoryError> {
    buffer.extend_from_slice(&text.len().to_ne_bytes())?;
    buffer.extend_from_slice(text.as_bytes())
}

fn take_text<'a>(rest: &mut &'a [u8]) -> Option<&'a str> {
    let (length, data) = rest.split_first_chunk::<LENGTH_BYTES>()?;
    let (text, tail) = data.split_at_checked(usize::from_ne_bytes(*length))?;
    *rest = tail;
    str::from_utf8(text).ok()
}

/// Immutable regional span and annotation snapshot for prepared execution.
/// Borrowed views allocate no text, and checked aliases retain the same snapshot.
pub struct PreparedTraceContext {
    bytes: SharedBuffer<'static>,
}

impl PreparedTraceContext {
    /// Obtain a checked retained copy of this snapshot's backing owner.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            bytes: self.bytes.try_clone()?,
        })
    }

    /// Borrow the active parent span's textual identity.
    #[must_use]
    pub fn span(&self) -> Option<&str> {
        let (span, _) = self.parts()?;
        span.and_then(|bytes| str::from_utf8(bytes).ok())
    }

    fn parts(&self) -> Option<(Option<&[u8]>, &[u8])> {
        let (length, data) = self.bytes.as_slice().split_first_chunk::<LENGTH_BYTES>()?;
        let length = usize::from_ne_bytes(*length);
        if length == NO_SPAN {
            Some((None, data))
        } else {
            let (span, annotations) = data.split_at_checked(length)?;
            Some((Some(span), annotations))
        }
    }

    /// Borrow the stored annotations in key order.
    pub fn annotations(&self) -> impl Iterator<Item = (&str, &str)> {
        let mut bytes = self
            .parts()
            .map(|(_, annotations)| annotations)
            .unwrap_or_default();
        std::iter::from_fn(move || Some((take_text(&mut bytes)?, take_text(&mut bytes)?)))
    }

    /// Borrow the value of one annotation, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.annotations()
            .find_map(|(candidate, value)| (candidate == key).then_some(value))
    }
}

impl fmt::Debug for PreparedTraceContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedTraceContext")
            .field("has_span", &self.span().is_some())
            .field("entries", &self.annotations().count())
            .finish()
    }
}
