//! Caller-selected native byte backing for prepared streaming copies.
use bytes::{Bytes, BytesMut};
use effect::memory::{
    Bytes as MemoryBytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, TryLockError};

/// Explicit count and maximum visible length of native backing buffers.
#[derive(Clone, Copy, Debug)]
pub struct PreparedNativeBytesCapacity {
    /// Number of independently reusable native backings.
    pub buffers: usize,
    /// Maximum visible copied fragment; actual allocator capacity may be larger.
    pub bytes_per_buffer: usize,
}
/// Startup region metadata and separately classified native allocations.
#[derive(Clone, Copy, Debug)]
pub struct PreparedNativeBytesDemand {
    /// Region-backed index and metadata, excluding region allocator overhead.
    pub region_bytes: MemoryBytes,
    /// Number of native backing buffers and separately allocated Bytes shared controls.
    pub native_buffers: usize,
    /// Requested native backing bytes; actual capacity is reported by the pool.
    pub requested_native_bytes: MemoryBytes,
    /// Startup pool Arc controls, excluding headers and allocator overhead.
    pub startup_system_arc_controls: usize,
    /// Requested pool Arc payload bytes, excluding headers and allocator rounding.
    pub startup_system_arc_payload_bytes: MemoryBytes,
}
struct State {
    cells: FixedVector<'static, Mutex<BytesMut>>,
    cursor: AtomicUsize,
    actual_bytes: usize,
}
/// A finite native backing pool. Retained `Bytes` clones prevent overlapping reuse.
/// This does not provide a last-alias notification or bound foreign transport copies.
pub struct PreparedNativeBytesPool {
    state: SharedSlot<'static, State>,
    capacity: PreparedNativeBytesCapacity,
}
/// One DATA frame that owns a fragment of the prepared native backing.
pub struct PreparedNativeFrame(Bytes);
impl PreparedNativeFrame {
    pub(crate) fn into_bytes(self) -> Bytes {
        self.0
    }
}
impl PreparedNativeBytesCapacity {
    /// Compute selected-region metadata and separately requested native storage.
    pub fn demand(self) -> Result<PreparedNativeBytesDemand, MemoryError> {
        let cells = std::alloc::Layout::array::<Mutex<BytesMut>>(self.buffers)
            .map_err(|_| MemoryError::InvalidLayout)?;
        Ok(PreparedNativeBytesDemand {
            region_bytes: SlotPool::<State>::storage_bytes(1)?
                .checked_add(MemoryBytes::new(cells.size()))?,
            native_buffers: if self.bytes_per_buffer == 0 {
                0
            } else {
                self.buffers
            },
            requested_native_bytes: MemoryBytes::new(self.bytes_per_buffer)
                .checked_mul(self.buffers)?,
            startup_system_arc_controls: 1,
            startup_system_arc_payload_bytes: MemoryBytes::new(std::mem::size_of::<
                PreparedNativeBytesPool,
            >()),
        })
    }
}
impl PreparedNativeBytesPool {
    /// Prepare metadata before backing. Vec reservation is fallible; upstream Bytes
    /// shared-control promotion and a caller's startup Arc remain infallible native allocations.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedNativeBytesCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand()?;
        let cells = FixedVector::prepare(memory.allocator(region)?, capacity.buffers)?;
        let mut cells = Some(cells);
        let slots = memory.slots(region, 1, |_| {
            Ok(State {
                cells: cells.take().ok_or(MemoryError::NoSlot)?,
                cursor: AtomicUsize::new(0),
                actual_bytes: 0,
            })
        })?;
        let mut state = slots.checkout()?;
        for _ in 0..capacity.buffers {
            let mut value = if capacity.bytes_per_buffer == 0 {
                BytesMut::new()
            } else {
                let mut native = Vec::new();
                native
                    .try_reserve_exact(capacity.bytes_per_buffer)
                    .map_err(|_| MemoryError::BackingAllocationFailed)?;
                let actual = native.capacity();
                native.resize(actual, 0);
                let mut value = Bytes::from(native)
                    .try_into_mut()
                    .map_err(|_| MemoryError::UnsupportedRunContract)?;
                value.clear();
                drop(value.split_to(0));
                value
            };
            state.actual_bytes = state
                .actual_bytes
                .checked_add(value.capacity())
                .ok_or(MemoryError::ArithmeticOverflow)?;
            value.clear();
            state
                .cells
                .push(Mutex::new(value))
                .map_err(|_| MemoryError::NoSlot)?;
        }
        Ok(Self {
            state: state.share(),
            capacity,
        })
    }
    /// Sum of actual native vector capacities; control headers and allocator rounding are excluded.
    #[must_use]
    pub fn actual_native_bytes(&self) -> MemoryBytes {
        MemoryBytes::new(self.state.actual_bytes)
    }
    /// Copy into available disjoint tail or uniquely reclaimed backing; never waits or grows.
    pub fn try_copy(&self, input: &[u8]) -> Result<Bytes, MemoryError> {
        self.try_fill(input.len(), |output| {
            output.copy_from_slice(input);
            Ok(())
        })
    }
    /// Copy into prepared backing and retain that backing in a typed DATA frame.
    pub fn try_copy_frame(&self, input: &[u8]) -> Result<PreparedNativeFrame, MemoryError> {
        self.try_copy(input).map(PreparedNativeFrame)
    }
    // Only internal, deterministic encoders may use this admitted slice.
    pub(crate) fn try_fill(
        &self,
        len: usize,
        fill: impl FnOnce(&mut [u8]) -> Result<(), MemoryError>,
    ) -> Result<Bytes, MemoryError> {
        if len > self.capacity.bytes_per_buffer {
            return Err(MemoryError::ValueTooLarge {
                requested: MemoryBytes::new(len),
                capacity: MemoryBytes::new(self.capacity.bytes_per_buffer),
            });
        }
        if len == 0 {
            return Ok(Bytes::new());
        }
        let cells = self.state.cells.as_slice();
        if cells.is_empty() {
            return Err(MemoryError::NoSlot);
        }
        let start = self.state.cursor.fetch_add(1, Ordering::Relaxed) % cells.len();
        let mut busy = false;
        for candidate in cells.iter().cycle().skip(start).take(cells.len()) {
            let mut cell = match candidate.try_lock() {
                Ok(cell) => cell,
                Err(TryLockError::Poisoned(error)) => error.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    busy = true;
                    continue;
                }
            };
            if cell.try_reclaim(len) {
                struct Clear<'a>(&'a mut BytesMut);
                impl Drop for Clear<'_> {
                    fn drop(&mut self) {
                        self.0.clear();
                    }
                }
                let reset = Clear(&mut cell);
                reset.0.resize(len, 0);
                fill(reset.0.as_mut())?;
                return Ok(reset.0.split_to(len).freeze());
            }
        }
        Err(if busy {
            MemoryError::Busy
        } else {
            MemoryError::NoSlot
        })
    }
}
