use super::{Bytes, MemoryError, MemoryPlan, MemoryPolicy, Partition, PreparedMemory, RegionId};
use std::alloc::{Allocator, Layout};
use std::mem::MaybeUninit;

/// An owned, inert descriptor array for macro/configuration code. It keeps all
/// startup configuration on the caller's stack/owner, without temporary-borrow
/// escape or implicit heap allocation; preparation copies descriptors to backing.
#[derive(Clone, Copy, Debug)]
pub struct MemoryConfiguration<const N: usize> {
    total: Bytes,
    partitions: [Partition; N],
    policy: MemoryPolicy,
    alignment: usize,
    metadata: Option<RegionId>,
}
impl<const N: usize> MemoryConfiguration<N> {
    pub fn new(total: Bytes, partitions: [Partition; N]) -> Result<Self, MemoryError> {
        MemoryPlan::new(total, &partitions)?;
        Ok(Self {
            total,
            partitions,
            policy: MemoryPolicy::PreparedReuse,
            alignment: std::mem::align_of::<usize>(),
            metadata: None,
        })
    }
    #[must_use]
    pub const fn policy(mut self, policy: MemoryPolicy) -> Self {
        self.policy = policy;
        self
    }
    pub fn alignment(mut self, alignment: usize) -> Result<Self, MemoryError> {
        Layout::from_size_align(self.total.get(), alignment)
            .map_err(|_| MemoryError::InvalidLayout)?;
        self.alignment = alignment;
        Ok(self)
    }
    pub fn metadata(mut self, region: RegionId) -> Result<Self, MemoryError> {
        if !self.partitions.iter().any(|p| p.id == region) {
            return Err(MemoryError::UnknownRegion(region));
        }
        self.metadata = Some(region);
        Ok(self)
    }
    pub fn plan(&self) -> Result<MemoryPlan<'_>, MemoryError> {
        let mut plan = MemoryPlan::new(self.total, &self.partitions)?
            .policy(self.policy)
            .alignment(self.alignment)?;
        if let Some(region) = self.metadata {
            plan = plan.metadata(region)?;
        }
        Ok(plan)
    }
    pub fn prepare(self) -> Result<PreparedMemory<'static>, MemoryError> {
        self.plan()?.prepare()
    }
    pub fn prepare_in<A: Allocator + Send + Sync + 'static>(
        self,
        allocator: A,
    ) -> Result<PreparedMemory<'static>, MemoryError> {
        self.plan()?.prepare_in(allocator)
    }
    pub fn prepare_borrowed<'a>(
        self,
        backing: &'a mut [MaybeUninit<u8>],
    ) -> Result<PreparedMemory<'a>, MemoryError> {
        self.plan()?.prepare_borrowed(backing)
    }
}

impl RegionId {
    /// Deterministic allocation-free name mapping. Colliding names are rejected
    /// as duplicate region IDs by plan validation; names are not retained.
    #[must_use]
    pub const fn named(name: &str) -> Self {
        let mut bytes = name.as_bytes();
        let mut hash = 0x811c9dc5_u32;
        while let Some((byte, rest)) = bytes.split_first() {
            hash = (hash ^ (*byte as u32)).wrapping_mul(0x01000193);
            bytes = rest;
        }
        Self(hash)
    }
}
