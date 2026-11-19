//! Physical native-allocation ownership and its fallible domain boundary.
use std::alloc::Layout;
use std::ptr::NonNull;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRefusal {
    Closed,
    Capacity,
    Layout,
    OwnerLimit,
    Busy,
    Backing,
    Unsupported,
}

/// One exclusive initialized-byte allocation. The owner callback retains the
/// entire backing and its metadata until physical retirement; no detached pointer
/// is returned by a safe API. Container integrations must keep this object until
/// their values and queued destruction descendants have finished.
pub struct NativeAllocation {
    pointer: NonNull<u8>,
    layout: Layout,
    owner: NonNull<()>,
    release: unsafe fn(NonNull<()>),
}
// SAFETY: from_owned_raw requires exclusive transferable payload storage and a
// thread-safe owner/release protocol. Shared access exposes only immutable bytes.
unsafe impl Send for NativeAllocation {}
unsafe impl Sync for NativeAllocation {}
impl NativeAllocation {
    /// # Safety
    /// pointer must denote exclusive, initialized storage for layout, valid until
    /// release(owner), with no other mutable aliases. owner must be an actual
    /// retained backing owner, not a copied identity/count. release is called
    /// exactly once, on any thread, and must defer physical reuse past queued
    /// payload descendants. Neither pointer nor owner may require a caller borrow.
    pub unsafe fn from_owned_raw(
        pointer: NonNull<u8>,
        layout: Layout,
        owner: NonNull<()>,
        release: unsafe fn(NonNull<()>),
    ) -> Self {
        Self {
            pointer,
            layout,
            owner,
            release,
        }
    }
    #[must_use]
    pub fn layout(&self) -> Layout {
        self.layout
    }
    /// Raw address for a container's private typed initialization. It does not
    /// transfer ownership or permit access after this allocation is released.
    #[must_use]
    pub fn pointer(&self) -> NonNull<u8> {
        self.pointer
    }
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: initialized exclusive allocation, borrowed through live owner.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.layout.size()) }
    }
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: unique allocation borrow; no mutable alias can escape it.
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.layout.size()) }
    }
}
impl Drop for NativeAllocation {
    fn drop(&mut self) {
        unsafe { (self.release)(self.owner) }
    }
}

/// A fallible physical checkout before native construction. Ordinary native
/// allocations do not automatically become domain-owned by creating this trait.
/// # Safety
/// Each success must supply unique actual backing covering layout/alignment and
/// all allocation metadata, without fallback. Implementations must distinguish
/// closed/capacity/layout failure and keep accepted allocations valid after close.
pub unsafe trait NativeDomain: Send + Sync {
    fn try_allocate(&self, layout: Layout) -> Result<NativeAllocation, NativeRefusal>;
}
