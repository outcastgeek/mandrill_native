//! Private fixed backing owner. Free-list metadata lives only in free ranges;
//! allocated ranges carry an inaccessible prefix recording their full charge.
use super::{Bytes, MemoryError, MemoryPolicy, MemoryReport, MemoryUsage, Partition, RegionId};
use std::alloc::{Allocator, Layout};
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::{DropGuard, MaybeUninit, align_of, size_of};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};

const NONE: usize = usize::MAX;
const BLOCK_ALIGN: usize = align_of::<FreeBlock>();

#[derive(Clone, Copy)]
struct FreeBlock {
    length: usize,
    next: usize,
}
#[derive(Clone, Copy)]
struct AllocationHeader {
    start: usize,
    length: usize,
    retained: bool,
}
struct PartitionState {
    id: RegionId,
    head: usize,
    usage: MemoryUsage,
}
struct Control {
    sealed: bool,
    closed: bool,
}
struct Header {
    references: AtomicUsize,
    locked: AtomicBool,
    control: UnsafeCell<Control>,
    policy: MemoryPolicy,
    report: MemoryReport,
    partitions: usize,
    count: usize,
    allocator: usize,
    layout: Layout,
    release: unsafe fn(NonNull<Header>),
}

pub(super) struct Shared<'a> {
    pointer: NonNull<Header>,
    // An owned instance uses 'static. Borrowed instances retain the exclusive
    // input borrow until EVERY cloned handle/allocator/allocation is gone.
    lifetime: PhantomData<&'a mut [MaybeUninit<u8>]>,
}
// SAFETY: only immutable header fields are read without the atomic lock. Every
// mutable metadata access takes that lock. Payload is accessed exclusively by
// allocator clients, under Allocator's rules, never through the shared owner.
// Owned allocators are required to be Send + Sync; borrowed bytes are Send.
unsafe impl Send for Shared<'_> {}
// SAFETY: same synchronization and payload-disjointness invariant as Send.
unsafe impl Sync for Shared<'_> {}

impl Shared<'_> {
    pub(super) fn try_clone(&self) -> Result<Self, MemoryError> {
        self.header()
            .references
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                if count < isize::MAX as usize {
                    Some(count + 1)
                } else {
                    None
                }
            })
            .map_err(|_| MemoryError::OwnerLimit)?;
        Ok(Self {
            pointer: self.pointer,
            lifetime: PhantomData,
        })
    }
}
impl Drop for Shared<'_> {
    fn drop(&mut self) {
        if self.header().references.fetch_sub(1, Ordering::Release) == 1 {
            fence(Ordering::Acquire);
            // SAFETY: this was the last owner; every operation/lock borrows an
            // owner, so neither live metadata access nor client storage remains.
            unsafe { (self.header().release)(self.pointer) };
        }
    }
}

pub(super) fn align_up(value: usize, alignment: usize) -> Result<usize, MemoryError> {
    if !alignment.is_power_of_two() {
        return Err(MemoryError::InvalidLayout);
    }
    value
        .checked_add(alignment - 1)
        .map(|v| v & !(alignment - 1))
        .ok_or(MemoryError::ArithmeticOverflow)
}

struct Shape {
    layout: Layout,
    allocator: usize,
    partitions: usize,
    payload: usize,
    prefix: usize,
    metadata: Option<RegionId>,
    report: MemoryReport,
}
impl Shape {
    // Called only after checked total/control arithmetic and Layout validation.
    // Every partition's declared allowance includes its leading/trailing padding.
    fn span(&self, partitions: &[Partition], index: usize) -> (usize, usize) {
        let metadata = self
            .metadata
            .and_then(|id| partitions.iter().find(|p| p.id == id));
        let alignment = self.report.alignment.max(BLOCK_ALIGN);
        let mut cursor = metadata.map_or(self.payload, |p| p.capacity.get() - self.prefix);
        for (position, partition) in partitions.iter().enumerate() {
            let (begin, end) = if Some(partition.id) == self.metadata {
                (self.payload, partition.capacity.get() - self.prefix)
            } else {
                let begin = cursor;
                cursor += partition.capacity.get();
                (begin, cursor)
            };
            if position == index {
                let start = (begin + alignment - 1) & !(alignment - 1);
                let usable = end.saturating_sub(start) & !(BLOCK_ALIGN - 1);
                return (
                    start,
                    if usable >= size_of::<FreeBlock>() {
                        usable
                    } else {
                        0
                    },
                );
            }
        }
        (0, 0)
    }
}
fn shape(
    total: Bytes,
    alignment: usize,
    partitions: &[Partition],
    metadata: Option<RegionId>,
    allocator: Layout,
    prefix: usize,
) -> Result<Shape, MemoryError> {
    if !alignment.is_power_of_two() {
        return Err(MemoryError::InvalidLayout);
    }
    let base_alignment = alignment.max(align_of::<Header>()).max(allocator.align());
    let (with_allocator, allocator_offset) = Layout::new::<Header>()
        .extend(allocator)
        .map_err(|_| MemoryError::InvalidLayout)?;
    let descriptors = Layout::array::<UnsafeCell<PartitionState>>(partitions.len())
        .map_err(|_| MemoryError::InvalidLayout)?;
    let (control, partitions_offset) = with_allocator
        .extend(descriptors)
        .map_err(|_| MemoryError::InvalidLayout)?;
    let control_with_prefix = control
        .size()
        .checked_add(prefix)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    let mut required = if let Some(id) = metadata {
        let partition = partitions
            .iter()
            .find(|p| p.id == id)
            .ok_or(MemoryError::UnknownRegion(id))?;
        if partition.capacity.get() < control_with_prefix {
            return Err(MemoryError::BudgetExceeded {
                required: Bytes::new(control_with_prefix),
                available: partition.capacity,
            });
        }
        0
    } else {
        control_with_prefix
    };
    for partition in partitions {
        required = required
            .checked_add(partition.capacity.get())
            .ok_or(MemoryError::ArithmeticOverflow)?;
    }
    if required > total.get() {
        return Err(MemoryError::BudgetExceeded {
            required: Bytes::new(required),
            available: total,
        });
    }
    let layout = Layout::from_size_align(total.get() - prefix, base_alignment)
        .map_err(|_| MemoryError::InvalidLayout)?;
    let control_bytes = size_of::<Header>()
        .checked_add(allocator.size())
        .and_then(|v| v.checked_add(descriptors.size()))
        .ok_or(MemoryError::ArithmeticOverflow)?;
    let mut shape = Shape {
        layout,
        allocator: allocator_offset,
        partitions: partitions_offset,
        payload: control.size(),
        prefix,
        metadata,
        report: MemoryReport {
            reserved: total,
            committed: total,
            backing: total,
            control: Bytes::new(control_bytes),
            partition_storage: Bytes::new(0),
            padding: Bytes::new(0),
            unassigned: Bytes::new(total.get() - required),
            alignment,
        },
    };
    let mut payload = 0;
    for index in 0..partitions.len() {
        payload += shape.span(partitions, index).1;
    }
    shape.report.partition_storage = Bytes::new(payload);
    shape.report.padding = Bytes::new(required - control_bytes - payload);
    Ok(shape)
}

unsafe fn release_owned<A: Allocator>(pointer: NonNull<Header>) {
    // SAFETY: last Shared owns the complete initialized block. Move A out before
    // freeing its containing block, and deallocate with the original layout/A.
    unsafe {
        let header = pointer.as_ref();
        let layout = header.layout;
        let allocator = pointer
            .cast::<u8>()
            .as_ptr()
            .add(header.allocator)
            .cast::<A>()
            .read();
        std::ptr::drop_in_place(pointer.as_ptr());
        allocator.deallocate(pointer.cast(), layout);
    }
}
unsafe fn release_borrowed(pointer: NonNull<Header>) {
    // SAFETY: last Shared ends initialized control lifetime. Caller owns backing;
    // no free occurs. Descriptors contain only atomics/plain values, no drops.
    unsafe { std::ptr::drop_in_place(pointer.as_ptr()) };
}

/// Initialize without reading any input backing bytes. The caller has checked
/// the complete shape and owns disjoint writable storage for that shape.
unsafe fn initialize<'a>(
    pointer: NonNull<u8>,
    shape: &Shape,
    policy: MemoryPolicy,
    partitions: &[Partition],
    release: unsafe fn(NonNull<Header>),
) -> Shared<'a> {
    // SAFETY: offsets were layout checked before allocation; all fields written
    // below are initialized before publishing the handle. There is no user code
    // or fallible operation in this initialization loop.
    unsafe {
        pointer.cast::<Header>().as_ptr().write(Header {
            references: AtomicUsize::new(1),
            locked: AtomicBool::new(false),
            control: UnsafeCell::new(Control {
                sealed: false,
                closed: false,
            }),
            policy,
            report: shape.report,
            partitions: shape.partitions,
            count: partitions.len(),
            allocator: shape.allocator,
            layout: shape.layout,
            release,
        });
        for (index, partition) in partitions.iter().enumerate() {
            let (cursor, usable) = shape.span(partitions, index);
            let head = if usable >= size_of::<FreeBlock>() {
                pointer
                    .as_ptr()
                    .add(cursor)
                    .cast::<FreeBlock>()
                    .write(FreeBlock {
                        length: usable,
                        next: NONE,
                    });
                cursor
            } else {
                NONE
            };
            pointer
                .as_ptr()
                .add(shape.partitions)
                .cast::<UnsafeCell<PartitionState>>()
                .add(index)
                .write(UnsafeCell::new(PartitionState {
                    id: partition.id,
                    head,
                    usage: MemoryUsage {
                        reserved: partition.capacity,
                        committed: Bytes::new(usable),
                        ..MemoryUsage::default()
                    },
                }));
        }
        Shared {
            pointer: pointer.cast(),
            lifetime: PhantomData,
        }
    }
}

pub(super) fn owned<A: Allocator + Send + Sync + 'static>(
    total: Bytes,
    alignment: usize,
    policy: MemoryPolicy,
    partitions: &[Partition],
    metadata: Option<RegionId>,
    allocator: A,
) -> Result<Shared<'static>, MemoryError> {
    let shape = shape(
        total,
        alignment,
        partitions,
        metadata,
        Layout::new::<A>(),
        0,
    )?;
    let allocation = allocator
        .allocate(shape.layout)
        .map_err(|_| MemoryError::BackingAllocationFailed)?;
    let guard = DropGuard::new((allocation, allocator), |(pointer, allocator)| {
        // SAFETY: guard exclusively owns the live, not-yet-published allocation.
        unsafe { allocator.deallocate(pointer.cast(), shape.layout) };
    });
    if allocation.len() > total.get() {
        return Err(MemoryError::BackingExceedsBudget {
            returned: Bytes::new(allocation.len()),
            budget: total,
        });
    }
    let pointer = allocation.cast::<u8>();
    // SAFETY: successful allocator contract provides the complete aligned block.
    let shared = unsafe { initialize(pointer, &shape, policy, partitions, release_owned::<A>) };
    // No unwinding operation occurs from dismissal through initialization of A.
    let (_, allocator) = DropGuard::dismiss(guard);
    // SAFETY: shape reserved space/alignment for A; that location is not payload.
    unsafe {
        pointer
            .as_ptr()
            .add(shape.allocator)
            .cast::<A>()
            .write(allocator)
    };
    Ok(shared)
}

pub(super) fn borrowed<'a>(
    total: Bytes,
    alignment: usize,
    policy: MemoryPolicy,
    partitions: &[Partition],
    metadata: Option<RegionId>,
    backing: &'a mut [MaybeUninit<u8>],
) -> Result<Shared<'a>, MemoryError> {
    if backing.len() < total.get() {
        return Err(MemoryError::BudgetExceeded {
            required: total,
            available: Bytes::new(backing.len()),
        });
    }
    let effective = alignment.max(align_of::<Header>());
    if !effective.is_power_of_two() {
        return Err(MemoryError::InvalidLayout);
    }
    let prefix = backing.as_ptr().align_offset(effective);
    let shape = shape(
        total,
        alignment,
        partitions,
        metadata,
        Layout::new::<()>(),
        prefix,
    )?;
    // SAFETY: shape includes prefix within the caller's exclusive borrowed slice;
    // initialized control/descriptor/payload ranges remain within total bytes.
    let shared = unsafe {
        let pointer = NonNull::new_unchecked(backing.as_mut_ptr().cast::<u8>().add(prefix));
        initialize(pointer, &shape, policy, partitions, release_borrowed)
    };
    Ok(shared)
}

struct Locked<'s, 'a> {
    shared: &'s Shared<'a>,
}
impl Drop for Locked<'_, '_> {
    fn drop(&mut self) {
        self.shared.header().locked.store(false, Ordering::Release);
    }
}
impl Locked<'_, '_> {
    fn control(&mut self) -> &mut Control {
        // SAFETY: this guard holds the unique metadata lock until dropped.
        unsafe { &mut *self.shared.header().control.get() }
    }
    unsafe fn partition_unchecked(&mut self, index: usize) -> &mut PartitionState {
        // SAFETY: caller proves index < initialized descriptor count; guard owns
        // the metadata lock. Descriptor storage is separate from user payloads.
        unsafe {
            &mut *(*self
                .shared
                .pointer
                .cast::<u8>()
                .as_ptr()
                .add(self.shared.header().partitions)
                .cast::<UnsafeCell<PartitionState>>()
                .add(index))
            .get()
        }
    }
    fn partition(&mut self, index: usize) -> Option<&mut PartitionState> {
        let header = self.shared.header();
        // SAFETY: initialized descriptor slice is disjoint from payload/header;
        // the atomic lock serializes all mutable descriptor access.
        unsafe {
            let descriptors = std::slice::from_raw_parts(
                self.shared
                    .pointer
                    .cast::<u8>()
                    .as_ptr()
                    .add(header.partitions)
                    .cast::<UnsafeCell<PartitionState>>(),
                header.count,
            );
            descriptors.get(index).map(|cell| &mut *cell.get())
        }
    }
}
impl Shared<'_> {
    pub(super) fn policy(&self) -> MemoryPolicy {
        self.header().policy
    }
    fn header(&self) -> &Header {
        // SAFETY: every Shared retains a reference to the initialized header.
        unsafe { self.pointer.as_ref() }
    }
    fn lock(&self) -> Locked<'_, '_> {
        while self
            .header()
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
        Locked { shared: self }
    }
    pub(super) fn report(&self) -> MemoryReport {
        self.header().report
    }
    pub(super) fn same_owner(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }
    pub(super) fn index(&self, id: RegionId) -> Result<usize, MemoryError> {
        let mut lock = self.lock();
        for index in 0..self.header().count {
            if lock.partition(index).is_some_and(|p| p.id == id) {
                return Ok(index);
            }
        }
        Err(MemoryError::UnknownRegion(id))
    }
    pub(super) fn usage(&self, index: usize) -> Option<MemoryUsage> {
        self.lock().partition(index).map(|p| p.usage)
    }
    pub(super) fn seal(&self) {
        self.lock().control().sealed = true;
    }
    pub(super) fn close(&self) {
        self.lock().control().closed = true;
    }
    pub(super) fn execution_allocator(&self) -> Result<(), MemoryError> {
        if self.header().policy == MemoryPolicy::PreparedReuse {
            return Err(MemoryError::PreparedOnly);
        }
        if !self.lock().control().sealed {
            return Err(MemoryError::NotSealed);
        }
        Ok(())
    }
    pub(super) fn allocate(
        &self,
        index: usize,
        layout: Layout,
        preparation: bool,
    ) -> Result<NonNull<[u8]>, MemoryError> {
        let mut lock = self.lock();
        let control = lock.control();
        let error = if control.closed {
            Some(MemoryError::Closed)
        } else if preparation && control.sealed {
            Some(MemoryError::Sealed)
        } else if !preparation && !control.sealed {
            Some(MemoryError::NotSealed)
        } else if !preparation && self.header().policy == MemoryPolicy::PreparedReuse {
            Some(MemoryError::PreparedOnly)
        } else if layout.align() > self.header().report.alignment {
            Some(MemoryError::UnsupportedAlignment {
                requested: layout.align(),
                supported: self.header().report.alignment,
            })
        } else {
            None
        };
        let partition = lock.partition(index).ok_or(MemoryError::InvalidLayout)?;
        if let Some(error) = error {
            partition.usage.rejected_allocations =
                partition.usage.rejected_allocations.saturating_add(1);
            return Err(error);
        }
        if layout.size() == 0 {
            return Ok(NonNull::slice_from_raw_parts(
                NonNull::without_provenance(
                    std::num::NonZeroUsize::new(layout.align())
                        .ok_or(MemoryError::InvalidLayout)?,
                ),
                0,
            ));
        }
        let base = self.pointer.cast::<u8>().as_ptr();
        let mut previous = NONE;
        let mut current = partition.head;
        while current != NONE {
            // SAFETY: the sorted free list is initialized and accessed only under
            // lock; live clients have no access to any of its free ranges.
            let block = unsafe { base.add(current).cast::<FreeBlock>().read() };
            let payload = align_up(
                current
                    .checked_add(size_of::<AllocationHeader>())
                    .ok_or(MemoryError::ArithmeticOverflow)?,
                layout.align().max(align_of::<AllocationHeader>()),
            )?;
            let end = align_up(
                payload
                    .checked_add(layout.size())
                    .ok_or(MemoryError::ArithmeticOverflow)?,
                BLOCK_ALIGN,
            )?;
            let block_end = current + block.length;
            if end <= block_end {
                let remainder = block_end - end;
                let consumed = if remainder >= size_of::<FreeBlock>() {
                    end - current
                } else {
                    block.length
                };
                let next = if remainder >= size_of::<FreeBlock>() {
                    end
                } else {
                    block.next
                };
                // SAFETY: split nodes and allocated prefix are within the chosen
                // free block; payload alignment was checked. Ranges are disjoint.
                unsafe {
                    if remainder >= size_of::<FreeBlock>() {
                        base.add(end).cast::<FreeBlock>().write(FreeBlock {
                            length: remainder,
                            next: block.next,
                        });
                    }
                    if previous == NONE {
                        partition.head = next;
                    } else {
                        (*base.add(previous).cast::<FreeBlock>()).next = next;
                    }
                    base.add(payload - size_of::<AllocationHeader>())
                        .cast::<AllocationHeader>()
                        .write(AllocationHeader {
                            start: current,
                            length: consumed,
                            retained: false,
                        });
                }
                partition.usage.active_payload =
                    Bytes::new(partition.usage.active_payload.get() + layout.size());
                partition.usage.active_backing =
                    Bytes::new(partition.usage.active_backing.get() + consumed);
                partition.usage.active_allocations += 1;
                partition.usage.high_water_backing = partition
                    .usage
                    .high_water_backing
                    .max(partition.usage.active_backing);
                // SAFETY: nonzero aligned payload is inside the exclusive chosen
                // block. It remains reserved until matching client deallocation.
                return Ok(NonNull::slice_from_raw_parts(
                    unsafe { NonNull::new_unchecked(base.add(payload)) },
                    layout.size(),
                ));
            }
            previous = current;
            current = block.next;
        }
        partition.usage.rejected_allocations =
            partition.usage.rejected_allocations.saturating_add(1);
        Err(MemoryError::Exhausted {
            region: partition.id,
            requested: Bytes::new(layout.size()),
        })
    }
    /// The caller retains the unique Allocation capability, or its immutable
    /// embedding pool owner under that pool's metadata lock. No client payload
    /// or destructor is accessed here. Lock order is pool -> region only.
    pub(super) unsafe fn set_retained(
        &self,
        index: usize,
        pointer: NonNull<u8>,
        layout: Layout,
        retained: bool,
    ) {
        if layout.size() == 0 {
            return;
        }
        let mut lock = self.lock();
        // SAFETY: caller proves the matching live allocation and partition.
        unsafe {
            let partition = lock.partition_unchecked(index);
            let header = &mut *pointer
                .as_ptr()
                .sub(size_of::<AllocationHeader>())
                .cast::<AllocationHeader>();
            if header.retained != retained {
                if retained {
                    partition.usage.retained_backing =
                        Bytes::new(partition.usage.retained_backing.get() + header.length);
                    partition.usage.high_water_retained = partition
                        .usage
                        .high_water_retained
                        .max(partition.usage.retained_backing);
                } else {
                    partition.usage.retained_backing =
                        Bytes::new(partition.usage.retained_backing.get() - header.length);
                }
                header.retained = retained;
            }
        }
    }
    pub(super) unsafe fn deallocate(&self, index: usize, pointer: NonNull<u8>, layout: Layout) {
        if layout.size() == 0 {
            return;
        }
        let mut lock = self.lock();
        // SAFETY: every allocator index came from Shared::index on this owner;
        // descriptors are immutable in number/identity for the owner's lifetime.
        let partition = unsafe { lock.partition_unchecked(index) };
        let base = self.pointer.cast::<u8>().as_ptr();
        // SAFETY: caller supplies a live pointer/layout allocated by this exact
        // partition; the inaccessible prefix is initialized and exclusively ours.
        unsafe {
            let offset = pointer.as_ptr().offset_from(base) as usize;
            let allocation = base
                .add(offset - size_of::<AllocationHeader>())
                .cast::<AllocationHeader>()
                .read();
            if allocation.retained {
                partition.usage.retained_backing =
                    Bytes::new(partition.usage.retained_backing.get() - allocation.length);
            }
            let mut previous = NONE;
            let mut current = partition.head;
            while current != NONE && current < allocation.start {
                previous = current;
                current = (*base.add(current).cast::<FreeBlock>()).next;
            }
            let mut block = FreeBlock {
                length: allocation.length,
                next: current,
            };
            if current != NONE && allocation.start + block.length == current {
                let successor = base.add(current).cast::<FreeBlock>().read();
                block.length += successor.length;
                block.next = successor.next;
            }
            if previous != NONE
                && previous + (*base.add(previous).cast::<FreeBlock>()).length == allocation.start
            {
                let predecessor = &mut *base.add(previous).cast::<FreeBlock>();
                predecessor.length += block.length;
                predecessor.next = block.next;
            } else {
                base.add(allocation.start).cast::<FreeBlock>().write(block);
                if previous == NONE {
                    partition.head = allocation.start;
                } else {
                    (*base.add(previous).cast::<FreeBlock>()).next = allocation.start;
                }
            }
            partition.usage.active_payload =
                Bytes::new(partition.usage.active_payload.get() - layout.size());
            partition.usage.active_backing =
                Bytes::new(partition.usage.active_backing.get() - allocation.length);
            partition.usage.active_allocations -= 1;
        }
    }
}
