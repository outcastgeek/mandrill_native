//! Application-selected fixed memory, independent of any runtime or global
//! allocator. [`MemoryPlan`] describes startup storage; [`PreparedMemory`]
//! permits provisioning; [`Memory`] is the sealed execution capability.
//!
//! Prepared-reuse denies new allocation requests after sealing. Bounded-allocation
//! explicitly permits fallible allocation/recycling inside the existing backing.
//! Neither silently falls back to the global heap. Existing Effect execution does
//! not automatically use this capability: domain adapters must opt into it.
//!
//! One fallible allocation holds the shared control owner, allocator value,
//! partition descriptors and payload regions. Reports include their layout costs.
//! Borrowed storage holds the same control structures in the supplied bytes, with
//! a lifetime preventing reuse until the last owner disappears. Allocator-visible
//! sizes are not measurements of native malloc overhead or whole-process memory.
#![doc = include_str!("README.md")]

mod admission;
mod buffer;
mod collection;
mod configuration;
mod frame;
mod index;
mod native_allocation;
mod native_domain;
mod native_reservation;
mod pool;
mod storage;
mod types;

pub use crate::internal::runtime_storage::PreparedContextualDemand;
pub use admission::{
    Admission, AdmissionBundle, AdmissionUsage, BackedAdmission, CapacityUnit, ResourceCapacity,
};
pub use buffer::{Buffer, BufferLease, BufferView, SharedBuffer, Text, TextFormatError, TextLease};
pub use collection::{FixedVector, InitializationError};
pub use configuration::MemoryConfiguration;
pub(crate) use frame::FrameReturnSubscription;
pub(crate) use frame::FrameVacancy;
pub use frame::{
    FramePool, FrameReservation, PinnedValue, PreparedScopedUnitFutureReservation,
    PreparedUnitFuture, RetainedValue, RetainedValueReservation, RunFrames,
};
pub(crate) use frame::{RetainedFrameValue, RuntimeFrame, SharedRuntimeFrame};
pub use index::FixedIndex;
pub use native_allocation::{NativeAllocation, NativeDomain, NativeRefusal};
pub use native_domain::{NativeAllocationClass, NativeAllocationDomain};
pub use native_reservation::{NativeReservation, NativeReservationPool};
pub(crate) use pool::{ReturnSubscription, WeakSlot};
pub(crate) use pool::prepared_notification;
pub use pool::{SharedSlot, SlotId, SlotLease, SlotPool};

use std::alloc::{AllocError, Allocator, Layout, System};
use std::fmt;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
pub use types::{Bytes, MemoryError, MemoryPolicy, MemoryReport, MemoryUsage, Partition, RegionId};

/// Inert configuration borrowing caller-owned descriptors. Preparing copies the
/// descriptors into backing; no String/Vec/Arc metadata allocation is hidden here.
#[derive(Clone, Copy, Debug)]
pub struct MemoryPlan<'p> {
    total: Bytes,
    partitions: &'p [Partition],
    alignment: usize,
    policy: MemoryPolicy,
    metadata: Option<RegionId>,
}
impl<'p> MemoryPlan<'p> {
    pub fn new(total: Bytes, partitions: &'p [Partition]) -> Result<Self, MemoryError> {
        for (index, partition) in partitions.iter().enumerate() {
            if partitions
                .iter()
                .take(index)
                .any(|earlier| earlier.id == partition.id)
            {
                return Err(MemoryError::DuplicateRegion(partition.id));
            }
        }
        Ok(Self {
            total,
            partitions,
            alignment: std::mem::align_of::<usize>(),
            policy: MemoryPolicy::PreparedReuse,
            metadata: None,
        })
    }
    #[must_use]
    pub const fn policy(mut self, policy: MemoryPolicy) -> Self {
        self.policy = policy;
        self
    }
    /// Charge the shared control/descriptors to this partition's allowance,
    /// instead of requiring additional bytes outside the partition allowances.
    /// Its remaining space is still available for ordinary bounded allocations.
    pub fn metadata(mut self, region: RegionId) -> Result<Self, MemoryError> {
        if !self.partitions.iter().any(|p| p.id == region) {
            return Err(MemoryError::UnknownRegion(region));
        }
        self.metadata = Some(region);
        Ok(self)
    }
    /// Select supported payload alignment. Any valid power-of-two alignment may
    /// be selected, subject to the platform's checked Layout/total-size limits.
    pub fn alignment(mut self, alignment: usize) -> Result<Self, MemoryError> {
        Layout::from_size_align(self.total.get(), alignment)
            .map_err(|_| MemoryError::InvalidLayout)?;
        self.alignment = alignment;
        Ok(self)
    }
    /// Fallible startup provisioning through System, with no secondary allocation.
    pub fn prepare(self) -> Result<PreparedMemory<'static>, MemoryError> {
        self.prepare_in(System)
    }
    /// The allocator value is stored in the same counted allocation. Any external
    /// resources already owned by a supplied allocator remain its caller's domain.
    pub fn prepare_in<A: Allocator + Send + Sync + 'static>(
        self,
        allocator: A,
    ) -> Result<PreparedMemory<'static>, MemoryError> {
        storage::owned(
            self.total,
            self.alignment,
            self.policy,
            self.partitions,
            self.metadata,
            allocator,
        )
        .map(|shared| PreparedMemory { shared })
    }
    /// Uses only the first `total` caller-provided bytes. Leading alignment loss
    /// counts against that total; no heap owner or allocation is introduced.
    pub fn prepare_borrowed<'a>(
        self,
        backing: &'a mut [MaybeUninit<u8>],
    ) -> Result<PreparedMemory<'a>, MemoryError> {
        storage::borrowed(
            self.total,
            self.alignment,
            self.policy,
            self.partitions,
            self.metadata,
            backing,
        )
        .map(|shared| PreparedMemory { shared })
    }
}

/// Startup allocation authority. Previously obtained allocator aliases observe
/// sealing atomically and cannot allocate after this owner is sealed.
pub struct PreparedMemory<'a> {
    shared: storage::Shared<'a>,
}
impl<'a> PreparedMemory<'a> {
    #[must_use]
    pub fn policy(&self) -> MemoryPolicy {
        self.shared.policy()
    }
    #[must_use]
    pub fn report(&self) -> MemoryReport {
        self.shared.report()
    }
    pub fn allocator(&self, region: RegionId) -> Result<RegionAllocator<'a>, MemoryError> {
        let index = self.shared.index(region)?;
        Ok(RegionAllocator {
            shared: self.shared.try_clone()?,
            index,
            preparation: true,
        })
    }
    pub fn allocate(
        &self,
        region: RegionId,
        layout: Layout,
    ) -> Result<Allocation<'a>, MemoryError> {
        self.allocator(region)?.allocate_bytes(layout)
    }
    pub fn usage(&self, region: RegionId) -> Result<MemoryUsage, MemoryError> {
        self.shared
            .usage(self.shared.index(region)?)
            .ok_or(MemoryError::UnknownRegion(region))
    }
    #[must_use]
    pub fn seal(self) -> Memory<'a> {
        self.shared.seal();
        Memory {
            shared: self.shared,
        }
    }
}

/// Sealed execution service. Fallible sharing retains the same backing without
/// allocation. Allocation authority requires explicit bounded-allocation policy.
pub struct Memory<'a> {
    shared: storage::Shared<'a>,
}
impl<'a> Memory<'a> {
    /// Shares ownership without allocation. The owner count is bounded by
    /// `isize::MAX`; saturation returns OwnerLimit without changing identity.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            shared: self.shared.try_clone()?,
        })
    }
    #[must_use]
    pub fn report(&self) -> MemoryReport {
        self.shared.report()
    }
    #[must_use]
    pub fn same_owner(&self, other: &Self) -> bool {
        self.shared.same_owner(&other.shared)
    }
    pub fn usage(&self, region: RegionId) -> Result<MemoryUsage, MemoryError> {
        self.shared
            .usage(self.shared.index(region)?)
            .ok_or(MemoryError::UnknownRegion(region))
    }
    /// Close admission across aliases. Existing owners remain valid and may
    /// deallocate even after closure; closure is not acknowledgment of settlement.
    pub fn close(&self) {
        self.shared.close();
    }
    pub fn allocator(&self, region: RegionId) -> Result<RegionAllocator<'a>, MemoryError> {
        self.shared.execution_allocator()?;
        let index = self.shared.index(region)?;
        Ok(RegionAllocator {
            shared: self.shared.try_clone()?,
            index,
            preparation: false,
        })
    }
    pub fn allocate(
        &self,
        region: RegionId,
        layout: Layout,
    ) -> Result<Allocation<'a>, MemoryError> {
        self.allocator(region)?.allocate_bytes(layout)
    }
}

/// `R` supplies its selected capability explicitly, never through ambient state.
/// The associated lifetime permits an owned `'static` region or borrowed backing.
pub trait HasMemory<'a> {
    fn memory(&self) -> &Memory<'a>;
}

/// A typed pool selector supplied by R. Pool marker values select explicit
/// prepared capabilities; the macro never chooses an ambient/global pool.
/// Effect outputs require owned 'static storage under the existing run contract;
/// borrowed pools remain usable directly within their caller's storage lifetime.
pub trait HasMemoryPool<P> {
    type Value: Send + 'static;
    fn memory_pool(&self, pool: &P) -> &SlotPool<'static, Self::Value>;
}

/// Allocator identity is the shared backing and partition. Clones can free each
/// other's allocations. All allocation headers, alignment and rounded tails are
/// charged to that partition; zero-size requests consume no payload storage.
pub struct RegionAllocator<'a> {
    shared: storage::Shared<'a>,
    index: usize,
    preparation: bool,
}
impl<'a> RegionAllocator<'a> {
    /// Shares the same allocator identity up to `isize::MAX` live owners.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            shared: self.shared.try_clone()?,
            index: self.index,
            preparation: self.preparation,
        })
    }
    pub fn allocate_bytes(&self, layout: Layout) -> Result<Allocation<'a>, MemoryError> {
        // Obtain ownership first: saturation must not leave an unowned payload.
        let allocator = self.try_clone()?;
        let pointer = allocator
            .shared
            .allocate(allocator.index, layout, allocator.preparation)?
            .cast::<u8>();
        // SAFETY: allocator reserved this unique payload range, or returned a
        // correctly aligned non-null zero-length pointer. No references exist yet.
        unsafe { pointer.as_ptr().write_bytes(0, layout.size()) };
        Ok(Allocation {
            pointer,
            layout,
            allocator,
        })
    }
    #[must_use]
    pub fn same_owner(&self, other: &Self) -> bool {
        self.index == other.index && self.shared.same_owner(&other.shared)
    }
}
// SAFETY: storage serializes metadata and reserves disjoint live payload ranges.
// Deallocation returns only the caller's live range to this same partition;
// clones retain backing and can deallocate each other's allocations. No fallback
// occurs. Default grow/shrink allocate from the same fixed backing and preserve
// the old allocation on failure. Sealing restricts NEW requests, not deallocation.
unsafe impl Allocator for RegionAllocator<'_> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.shared
            .allocate(self.index, layout, self.preparation)
            .map_err(|_| AllocError)
    }
    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        // SAFETY: Allocator caller supplies the matching live allocation/layout.
        unsafe { self.shared.deallocate(self.index, pointer, layout) };
    }
}

/// Unique initialized byte allocation. It retains backing beyond environment or
/// preparation lifetime, and returns its complete charge exactly once on drop.
/// This primitive is not yet a prepared buffer pool or an admission transaction.
pub struct Allocation<'a> {
    pointer: NonNull<u8>,
    layout: Layout,
    allocator: RegionAllocator<'a>,
}
// SAFETY: payload ownership is unique and bytes are Send. Shared metadata is locked.
unsafe impl Send for Allocation<'_> {}
// SAFETY: shared access exposes only initialized immutable bytes, never mutation.
unsafe impl Sync for Allocation<'_> {}
impl Allocation<'_> {
    /// Transfer this unique allocation into retained ownership. Its complete
    /// charge (not visible length) remains a subset of active_backing until drop.
    /// Use this explicit boundary for caller-owned output/storage; moving or
    /// cloning an unrelated pointer is never interpreted as an escape.
    #[must_use]
    pub fn retain(self) -> Self {
        self.set_retained(true);
        self
    }
    pub(super) fn set_retained(&self, retained: bool) {
        // SAFETY: this capability owns the exact live pointer/layout. Internal
        // pool users serialize transitions and keep the allocation alive.
        unsafe {
            self.allocator.shared.set_retained(
                self.allocator.index,
                self.pointer,
                self.layout,
                retained,
            )
        };
    }
}
impl Deref for Allocation<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // SAFETY: this owner retains its unique initialized payload and backing.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.layout.size()) }
    }
}
impl DerefMut for Allocation<'_> {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: &mut self gives exclusive access to this non-Clone payload owner.
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.layout.size()) }
    }
}
impl Drop for Allocation<'_> {
    fn drop(&mut self) {
        // SAFETY: pointer/layout come from our retained allocator and are never
        // exposed for safe ownership reconstruction or duplicated by cloning.
        unsafe { self.allocator.deallocate(self.pointer, self.layout) };
    }
}
impl fmt::Debug for Allocation<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Allocation")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}
