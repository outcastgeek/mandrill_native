//! Fixed decimal long-division workspaces.
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
/// Explicit concurrent operations and maximum divisor digits.
#[derive(Clone, Copy, Debug)]
pub struct PreparedNumericScratchCapacity {
    /// Simultaneous arithmetic operations.
    pub operations: usize,
    /// Maximum normalized divisor coefficient digits per operation.
    pub digits: usize,
}
/// Caller-owned region backing; allocator rounding and Arc wrappers are separate.
#[derive(Clone, Copy, Debug)]
pub struct PreparedNumericScratchDemand {
    /// Total region slots and digit vectors.
    pub region_bytes: Bytes,
}
pub(super) struct Scratch {
    pub divisor: FixedVector<'static, u8>,
    pub remainder: FixedVector<'static, u8>,
}
/// Fixed workspaces for prepared exact-number divisibility.
pub struct PreparedNumericScratchPool {
    pool: SlotPool<'static, Scratch>,
}
pub(super) struct ScratchLease(SlotLease<'static, Scratch>);
impl std::ops::Deref for ScratchLease {
    type Target = Scratch;
    fn deref(&self) -> &Scratch {
        &self.0
    }
}
impl std::ops::DerefMut for ScratchLease {
    fn deref_mut(&mut self) -> &mut Scratch {
        &mut self.0
    }
}
impl Drop for ScratchLease {
    fn drop(&mut self) {
        self.0.divisor.clear();
        self.0.remainder.clear();
    }
}
impl PreparedNumericScratchCapacity {
    /// Compute checked region payload, excluding separately supplied owner metadata.
    pub fn demand(self) -> Result<PreparedNumericScratchDemand, MemoryError> {
        let length = self
            .digits
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let vectors = Bytes::new(length)
            .checked_mul(2)?
            .checked_mul(self.operations)?;
        Ok(PreparedNumericScratchDemand {
            region_bytes: SlotPool::<Scratch>::storage_bytes(self.operations)?
                .checked_add(vectors)?,
        })
    }
}
impl PreparedNumericScratchPool {
    /// Prepare both fixed digit vectors for every operation.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedNumericScratchCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand()?;
        let length = capacity
            .digits
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let pool = memory.slots(region, capacity.operations, |_| {
            Ok(Scratch {
                divisor: FixedVector::prepare(memory.allocator(region)?, length)?,
                remainder: FixedVector::prepare(memory.allocator(region)?, length)?,
            })
        })?;
        Ok(Self { pool })
    }
    pub(super) fn checkout(&self) -> Result<ScratchLease, MemoryError> {
        self.pool.checkout().map(ScratchLease)
    }
}
