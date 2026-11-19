//! Finite region-owned byte cells shared by prepared framing consumers.
use crate::memory::{Buffer, Bytes, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool};

/// Caller-selected simultaneous payloads and bytes per payload.
#[derive(Clone, Copy, Debug)]
pub struct PreparedByteCapacity {
    /// Simultaneous in-progress or escaped payload buffers.
    pub buffers: usize,
    /// Maximum physical bytes in one payload.
    pub bytes_per_buffer: usize,
}
impl PreparedByteCapacity {
    /// Actual region payload and slot metadata, excluding allocator overhead.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        SlotPool::<Buffer<'static>>::storage_bytes(self.buffers)?
            .checked_add(Bytes::new(self.bytes_per_buffer).checked_mul(self.buffers)?)
    }
}
/// Startup-backed byte cells; escaped outputs keep their actual cell occupied.
pub struct PreparedBytePool {
    slots: SlotPool<'static, Buffer<'static>>,
}
impl PreparedBytePool {
    /// Prepare all fixed payload cells in the selected region.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedByteCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.storage_bytes()?;
        Ok(Self {
            slots: memory.slots(region, capacity.buffers, |_| {
                memory.buffer(region, Bytes::new(capacity.bytes_per_buffer))
            })?,
        })
    }
    /// Retain the same actual pool owner without a new allocation.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slots: self.slots.try_clone()?,
        })
    }
    pub(super) fn checkout(&self) -> Result<PreparedBytes, MemoryError> {
        Ok(PreparedBytes {
            slot: Some(self.slots.checkout()?),
        })
    }
}
/// Owned payload retaining the physical bytes until dropped.
pub struct PreparedBytes {
    slot: Option<SlotLease<'static, Buffer<'static>>>,
}
impl std::fmt::Debug for PreparedBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}
impl PreparedBytes {
    pub(super) fn empty() -> Self {
        Self { slot: None }
    }
    /// Borrow the payload while retaining its backing.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.slot.as_ref().map_or(&[], |slot| slot.as_slice())
    }
    pub(super) fn buffer_mut(&mut self) -> Option<&mut Buffer<'static>> {
        self.slot.as_deref_mut()
    }
}
impl Drop for PreparedBytes {
    fn drop(&mut self) {
        if let Some(slot) = &mut self.slot {
            slot.clear();
        }
    }
}
