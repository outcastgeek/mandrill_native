//! Native transport allocation classes use actual startup-owned byte blocks.
//! Returning a raw native allocation enters the same phased destruction lane;
//! no allocation/metadata is acquired after startup, and close cannot free live
//! blocks. This is a backing primitive, NOT certification of a native consumer.
use super::*;
use super::{NativeAllocation, NativeDomain as Domain, NativeRefusal as Refusal};
use crate::internal::destruction::{DestructionNode, enqueue_prepared};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy)]
pub struct NativeAllocationClass {
    pub region: RegionId,
    pub layout: Layout,
    pub slots: usize,
}
struct Block {
    bytes: Allocation<'static>,
    owner: UnsafeCell<MaybeUninit<BlockOwner>>,
    node: UnsafeCell<DestructionNode>,
}
struct BlockOwner {
    _lease: SlotLease<'static, Block>,
    _quota: Option<AdmissionBundle<'static, 2>>,
}
// SAFETY: pool values are private. A checkout moves its sole lease into owner,
// and only that allocation accesses bytes until its one release queues the node.
// The queue retains the lease through descendants before returning the slot.
unsafe impl Send for Block {}
unsafe impl Sync for Block {}
unsafe fn empty(_: NonNull<DestructionNode>) {}
unsafe fn return_block(node: NonNull<DestructionNode>) {
    // SAFETY: exact block pointer installed on checkout, queued once, not reused
    // until this release callback after payload/footer descendants have finished.
    let block = unsafe { &*DestructionNode::context(node).cast::<Block>() };
    let owner = unsafe { (*block.owner.get()).assume_init_read() };
    drop(owner);
}
unsafe fn release(pointer: NonNull<()>) {
    // SAFETY: NativeAllocation's exact private constructor owns this live block;
    // it calls release once. The saved lease remains present until return_block.
    let block = unsafe { pointer.cast::<Block>().as_ref() };
    let node = unsafe { NonNull::new_unchecked(block.node.get()) };
    // SAFETY: this initialized node owns its saved lease and is not already queued.
    unsafe { enqueue_prepared(node) };
}
unsafe fn track_block(pointer: NonNull<u8>, retained: bool) {
    // SAFETY: only the matching private Block pool installs this callback.
    unsafe { pointer.cast::<Block>().as_ref() }
        .bytes
        .set_retained(retained);
}
struct Class {
    layout: Layout,
    pool: SlotPool<'static, Block>,
}
/// Reusable physical byte and allocation-control storage for native containers.
/// No Global allocator interception or default native allocation is performed.
/// Applications select class sizes/counts; the transport must explicitly opt in.
pub struct NativeAllocationDomain {
    classes: FixedVector<'static, Class>,
    closed: AtomicBool,
    admission: Option<Admission<'static, 2>>,
}
impl NativeAllocationDomain {
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        metadata: RegionId,
        classes: &[NativeAllocationClass],
    ) -> Result<Self, MemoryError> {
        let mut prepared = FixedVector::prepare(memory.allocator(metadata)?, classes.len())?;
        for class in classes {
            let allocator = memory.allocator(class.region)?;
            let pool = SlotPool::prepare_tracked(
                memory,
                metadata,
                class.slots,
                |_| {
                    Ok::<_, MemoryError>(Block {
                        bytes: allocator.allocate_bytes(class.layout)?,
                        owner: UnsafeCell::new(MaybeUninit::uninit()),
                        node: UnsafeCell::new(DestructionNode::new(empty, empty, return_block)),
                    })
                },
                track_block,
            )?;
            // Destination capacity was physically provisioned for every class.
            if prepared
                .push(Class {
                    layout: class.layout,
                    pool,
                })
                .is_err()
            {
                return Err(MemoryError::NoSlot);
            }
        }
        Ok(Self {
            classes: prepared,
            closed: AtomicBool::new(false),
            admission: None,
        })
    }
    /// Couple each class slot and its full class payload size to ordered Slots
    /// and Bytes admission. Metadata and allocator rounding are separate. Using
    /// the same coordinator for a nested reservation charges both layers.
    pub fn with_admission(mut self, admission: Admission<'static, 2>) -> Result<Self, MemoryError> {
        let capacities = admission.capacities();
        if capacities[0].unit != CapacityUnit::Slots || capacities[1].unit != CapacityUnit::Bytes {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.admission = Some(admission);
        Ok(self)
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        for class in self.classes.as_slice() {
            class.pool.close();
        }
    }
}
// SAFETY: successful allocations use exclusive live pool slots and initialized
// backing with requested alignment. Both control and bytes are startup-owned;
// the lease travels with the native allocation and release is phased, not quota.
unsafe impl Domain for NativeAllocationDomain {
    fn try_allocate(&self, layout: Layout) -> Result<NativeAllocation, Refusal> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Refusal::Closed);
        }
        let mut suitable = false;
        for class in self.classes.as_slice() {
            if class.layout.size() < layout.size() || class.layout.align() < layout.align() {
                continue;
            }
            suitable = true;
            let lease = match class.pool.try_checkout() {
                Ok(lease) => lease,
                Err(MemoryError::NoSlot) => continue,
                Err(MemoryError::Closed) => return Err(Refusal::Closed),
                Err(MemoryError::OwnerLimit) => return Err(Refusal::OwnerLimit),
                Err(MemoryError::Busy) => return Err(Refusal::Busy),
                Err(_) => return Err(Refusal::Backing),
            };
            // Linearize admission against whole-domain close, including a
            // class not yet visited by close's pool loop. No native construction
            // has run, so this rollback returns only the acquired physical slot.
            if self.closed.load(Ordering::Acquire) {
                return Err(Refusal::Closed);
            }
            let quota = if let Some(admission) = &self.admission {
                Some(
                    admission
                        .reserve([1, class.layout.size()], [0, 0])
                        .map_err(|error| {
                            if matches!(error, MemoryError::Closed) {
                                Refusal::Closed
                            } else if matches!(error, MemoryError::OwnerLimit) {
                                Refusal::OwnerLimit
                            } else if matches!(error, MemoryError::Busy) {
                                Refusal::Busy
                            } else if matches!(
                                error,
                                MemoryError::NoSlot | MemoryError::CapacityExceeded { .. }
                            ) {
                                Refusal::Capacity
                            } else {
                                Refusal::Backing
                            }
                        })?,
                )
            } else {
                None
            };
            let pointer = NonNull::from(&*lease);
            // SAFETY: exclusive checkout; no earlier allocation/queued owner is
            // live. Data is zeroed before publication, including reused blocks.
            unsafe {
                let block = pointer.as_ref();
                let bytes = block.bytes.pointer;
                bytes.as_ptr().write_bytes(0, layout.size());
                (*block.node.get()).set_context(pointer.as_ptr().cast());
                (*block.owner.get()).write(BlockOwner {
                    _lease: lease,
                    _quota: quota,
                });
                return Ok(NativeAllocation::from_owned_raw(
                    bytes,
                    layout,
                    pointer.cast(),
                    release,
                ));
            }
        }
        Err(if suitable {
            Refusal::Capacity
        } else {
            Refusal::Layout
        })
    }
}
