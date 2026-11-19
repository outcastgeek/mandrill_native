//! Typed prepared slots. Control, free-list records, generations and values all
//! occupy a single allocation charged to the selected memory partition.
use super::{Allocation, Bytes, MemoryError, PreparedMemory, RegionId};
use futures_util::task::AtomicWaker;
use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};
use std::task::Waker;

const NONE: usize = usize::MAX;
// Default generic pools track their slab only, never arbitrary T pointees.
unsafe fn slab_only(_: NonNull<u8>, _: bool) {}
// Keep this helper non-generic in T/F: the zero-listener recursive case has
// one concrete monomorphization rather than nesting the caller's factory type.
fn prepare_return_listeners<'a>(
    memory: &PreparedMemory<'a>,
    region: RegionId,
    count: usize,
) -> Result<SlotPool<'a, AtomicWaker>, MemoryError> {
    SlotPool::prepare(memory, region, count, |_| Ok(AtomicWaker::new()))
}
struct Record {
    next: usize,
    generation: usize,
    references: usize,
    retained: bool,
    weak: WeakAccess,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum WeakAccess {
    Unshared,
    Shared,
    Published,
    Retired,
}
struct State {
    head: usize,
    closed: bool,
    active: usize,
    retired: usize,
    retained_slots: usize,
}
struct Header<'a> {
    owner: ManuallyDrop<Allocation<'a>>,
    references: AtomicUsize,
    locked: AtomicBool,
    state: UnsafeCell<State>,
    count: usize,
    records: usize,
    values: usize,
    track: unsafe fn(NonNull<u8>, bool),
    listeners: Option<SlotPool<'a, AtomicWaker>>,
}
struct Owner<'a, T> {
    pointer: NonNull<Header<'a>>,
    value: PhantomData<T>,
}
impl<'a, T> Owner<'a, T> {
    fn header(&self) -> &Header<'a> {
        // SAFETY: each Owner is a counted reference retaining initialized header.
        unsafe { self.pointer.as_ref() }
    }
    fn try_clone(&self) -> Result<Self, MemoryError> {
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
            value: PhantomData,
        })
    }
    fn try_lock(&self) -> Result<Locked<'_, '_, T>, MemoryError> {
        self.header()
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| MemoryError::Busy)?;
        Ok(Locked { owner: self })
    }
    fn lock(&self) -> Locked<'_, '_, T> {
        while self
            .header()
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
        Locked { owner: self }
    }
    unsafe fn value(&self, index: usize) -> *mut T {
        // SAFETY: callers prove index < initialized count. Values layout/alignment
        // was checked before initialization, including inhabited ZST construction.
        unsafe {
            self.pointer
                .cast::<u8>()
                .as_ptr()
                .add(self.header().values)
                .cast::<T>()
                .add(index)
        }
    }
    fn retain(&self, index: usize, shared: bool) {
        let mut lock = self.lock();
        // SAFETY: a unique live lease supplies this initialized slot index.
        let record = unsafe { lock.record(index) };
        if shared && record.weak == WeakAccess::Unshared {
            record.weak = WeakAccess::Shared;
        }
        if !record.retained {
            record.retained = true;
            lock.state().retained_slots += 1;
            self.header().owner.set_retained(true);
            // SAFETY: shim is selected at pool preparation for exactly T. It
            // changes only owned allocation metadata, never invokes user code.
            unsafe {
                (self.header().track)(NonNull::new_unchecked(self.value(index).cast()), true)
            };
        }
    }
    fn release(&self, index: usize) {
        let mut lock = self.lock();
        // SAFETY: each lease has a constructor-checked index and counted reference.
        let record = unsafe { lock.record(index) };
        record.references -= 1;
        let returned = record.references == 0;
        if returned {
            let retained = record.retained;
            record.retained = false;
            record.weak = WeakAccess::Unshared;
            if retained {
                // SAFETY: private tracking shims access only owned Allocation
                // metadata; last lease retains T and no payload destruction runs.
                unsafe {
                    (self.header().track)(NonNull::new_unchecked(self.value(index).cast()), false)
                };
                lock.state().retained_slots -= 1;
                if lock.state().retained_slots == 0 {
                    self.header().owner.set_retained(false);
                }
            }
            let state = lock.state();
            let head = state.head;
            state.head = index;
            state.active -= 1;
            // SAFETY: same checked index; mutable record borrow above has ended.
            unsafe { lock.record(index).next = head };
        }
        drop(lock);
        if returned {
            self.notify_returns();
        }
    }
    fn notify_returns(&self) {
        if let Some(listeners) = &self.header().listeners {
            NotifyReturns { listeners, next: 0 }.run();
        }
    }
}
// Advance before each wake, so the remaining bounded notifications continue on unwind.
struct NotifyReturns<'s, 'a> {
    listeners: &'s SlotPool<'a, AtomicWaker>,
    next: usize,
}
impl NotifyReturns<'_, '_> {
    fn run(&mut self) {
        while self.next < self.listeners.capacity() {
            let index = self.next;
            self.next += 1;
            // SAFETY: listener pool initializes every AtomicWaker at startup and
            // retains that backing throughout this scan; wake is internally synchronized.
            unsafe {
                (&*self.listeners.owner.value(index)).wake();
            }
        }
    }
}
impl Drop for NotifyReturns<'_, '_> {
    fn drop(&mut self) {
        self.run();
    }
}
struct NotifyRetirement<'s, 'a, T> {
    owner: &'s Owner<'a, T>,
    retired: bool,
}
impl<T> Drop for NotifyRetirement<'_, '_, T> {
    fn drop(&mut self) {
        if self.retired {
            self.owner.notify_returns();
        }
    }
}
impl<T> Drop for Owner<'_, T> {
    fn drop(&mut self) {
        if self.header().references.fetch_sub(1, Ordering::Release) == 1 {
            fence(Ordering::Acquire);
            // SAFETY: no pool handles, leases, views or metadata locks remain.
            // Move backing ownership out before dropping T: it stays alive during
            // reentrant destructors/unwind. Slice drop glue drops remaining values
            // even when an element's destructor unwinds.
            unsafe {
                let values = self.value(0);
                let count = self.header().count;
                let allocation = ManuallyDrop::take(&mut self.pointer.as_mut().owner);
                std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(values, count));
                std::ptr::drop_in_place(self.pointer.as_ptr());
                drop(allocation);
            }
        }
    }
}
struct Locked<'s, 'a, T> {
    owner: &'s Owner<'a, T>,
}
impl<T> Drop for Locked<'_, '_, T> {
    fn drop(&mut self) {
        self.owner.header().locked.store(false, Ordering::Release);
    }
}
impl<T> Locked<'_, '_, T> {
    fn state(&mut self) -> &mut State {
        // SAFETY: unique atomic metadata lock; this state never aliases payload.
        unsafe { &mut *self.owner.header().state.get() }
    }
    unsafe fn record(&mut self, index: usize) -> &mut Record {
        // SAFETY: caller proves valid index; lock exclusively owns record access.
        unsafe {
            &mut *(*self
                .owner
                .pointer
                .cast::<u8>()
                .as_ptr()
                .add(self.owner.header().records)
                .cast::<UnsafeCell<Record>>()
                .add(index))
            .get()
        }
    }
}

struct Initializing<'a, T> {
    allocation: Allocation<'a>,
    values: usize,
    initialized: usize,
    value: PhantomData<T>,
}
impl<T> Drop for Initializing<'_, T> {
    fn drop(&mut self) {
        // SAFETY: only successful factory results increased initialized; remaining
        // slots are never read. allocation field drops afterwards, also on unwind.
        unsafe {
            let values = self
                .allocation
                .pointer
                .as_ptr()
                .add(self.values)
                .cast::<T>();
            std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(values, self.initialized));
        }
    }
}

/// A prepared fixed-capacity pool. Values are constructed only at startup, never
/// at checkout; returning a lease preserves its value for the next borrower.
/// Sharing pool handles is fallible only at the representable owner-count limit.
pub struct SlotPool<'a, T> {
    owner: Owner<'a, T>,
}
// SAFETY: disjoint values transfer via unique leases; metadata is locked. T may
// be !Sync: sharing a pool grants no shared reference to a checked-out value.
unsafe impl<T: Send> Send for SlotPool<'_, T> {}
// SAFETY: lock serializes checkout; each unique lease requires T: Send to transfer.
unsafe impl<T: Send> Sync for SlotPool<'_, T> {}
impl<'a, T> SlotPool<'a, T> {
    pub(super) fn prepare<F>(
        memory: &PreparedMemory<'a>,
        region: RegionId,
        count: usize,
        mut factory: F,
    ) -> Result<Self, MemoryError>
    where
        F: FnMut(usize) -> Result<T, MemoryError>,
    {
        Self::prepare_tracked(memory, region, count, &mut factory, slab_only)
    }
    /// Private concrete-owner shims account for known separately allocated
    /// backing. They must not call user code, destroy payloads or acquire a pool
    /// lock; transitions run under this pool's lock before region metadata locks.
    pub(super) fn prepare_tracked<F>(
        memory: &PreparedMemory<'a>,
        region: RegionId,
        count: usize,
        factory: F,
        track: unsafe fn(NonNull<u8>, bool),
    ) -> Result<Self, MemoryError>
    where
        F: FnMut(usize) -> Result<T, MemoryError>,
    {
        Self::prepare_tracked_with_listeners(memory, region, count, 0, factory, track)
    }
    pub(super) fn prepare_tracked_with_listeners<F>(
        memory: &PreparedMemory<'a>,
        region: RegionId,
        count: usize,
        listener_count: usize,
        mut factory: F,
        track: unsafe fn(NonNull<u8>, bool),
    ) -> Result<Self, MemoryError>
    where
        F: FnMut(usize) -> Result<T, MemoryError>,
    {
        let listeners = if listener_count == 0 {
            None
        } else {
            Some(prepare_return_listeners(memory, region, listener_count)?)
        };
        let records =
            Layout::array::<UnsafeCell<Record>>(count).map_err(|_| MemoryError::InvalidLayout)?;
        let values = Layout::array::<T>(count).map_err(|_| MemoryError::InvalidLayout)?;
        let (layout, records_offset) = Layout::new::<Header<'a>>()
            .extend(records)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let (layout, values_offset) = layout
            .extend(values)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let allocation = memory.allocate(region, layout)?;
        let mut pending = Initializing::<T> {
            allocation,
            values: values_offset,
            initialized: 0,
            value: PhantomData,
        };
        for index in 0..count {
            let value = factory(index)?;
            // SAFETY: slot index is in checked layout and uninitialized. Factory
            // produces a real T even when zero-sized; no conjured capabilities.
            unsafe {
                pending
                    .allocation
                    .pointer
                    .as_ptr()
                    .add(values_offset)
                    .cast::<T>()
                    .add(index)
                    .write(value)
            };
            pending.initialized += 1;
        }
        let pending = ManuallyDrop::new(pending);
        // SAFETY: all slots initialized; no fallible/user code occurs before
        // publication. Moving allocation transfers its backing owner into header.
        unsafe {
            let allocation = std::ptr::read(&pending.allocation);
            let base = allocation.pointer;
            for index in 0..count {
                base.as_ptr()
                    .add(records_offset)
                    .cast::<UnsafeCell<Record>>()
                    .add(index)
                    .write(UnsafeCell::new(Record {
                        next: if index + 1 == count { NONE } else { index + 1 },
                        generation: 0,
                        references: 0,
                        retained: false,
                        weak: WeakAccess::Unshared,
                    }));
            }
            base.cast::<Header<'a>>().as_ptr().write(Header {
                owner: ManuallyDrop::new(allocation),
                references: AtomicUsize::new(1),
                locked: AtomicBool::new(false),
                state: UnsafeCell::new(State {
                    head: if count == 0 { NONE } else { 0 },
                    closed: false,
                    active: 0,
                    retired: 0,
                    retained_slots: 0,
                }),
                count,
                records: records_offset,
                values: values_offset,
                track,
                listeners,
            });
            Ok(Self {
                owner: Owner {
                    pointer: base.cast(),
                    value: PhantomData,
                },
            })
        }
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            owner: self.owner.try_clone()?,
        })
    }
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.owner.header().count
    }
    #[must_use]
    pub fn same_pool(&self, other: &Self) -> bool {
        self.owner.pointer == other.owner.pointer
    }
    pub(crate) fn owns_shared(&self, slot: &SharedSlot<'a, T>) -> bool {
        self.owner.pointer == slot.owner.pointer
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.owner.lock().state().active
    }
    /// Records retired at generation exhaustion are observable, never reused.
    #[must_use]
    pub fn retired(&self) -> usize {
        self.owner.lock().state().retired
    }
    /// Prevents new checkout across aliases. Outstanding leases/views retain
    /// their values and backing until actual last-owner release.
    pub fn close(&self) {
        self.owner.lock().state().closed = true;
        self.owner.notify_returns();
    }
    /// Acquire one startup-provisioned, unique return-listener slot.
    pub fn subscribe_returns(&self) -> Result<ReturnSubscription<'a, T>, MemoryError> {
        let owner = self.owner.try_clone()?;
        let listeners = self
            .owner
            .header()
            .listeners
            .as_ref()
            .ok_or(MemoryError::NoSlot)?;
        // Listener admission is startup-only; wait for the short metadata gate
        // rather than reporting transient contention as missing capacity.
        let listener = listeners.checkout()?;
        Ok(ReturnSubscription {
            listener,
            _owner: owner,
        })
    }
    /// Available physical slots, excluding permanently retired generations.
    pub fn available(&self) -> Result<usize, MemoryError> {
        let mut lock = self.owner.lock();
        let state = lock.state();
        if state.closed {
            return Err(MemoryError::Closed);
        }
        Ok(self.capacity() - state.active - state.retired)
    }
    /// Largest number of slots that can ever be concurrently checked out.
    #[must_use]
    pub fn reusable_capacity(&self) -> usize {
        self.capacity() - self.retired()
    }
    pub fn checkout(&self) -> Result<SlotLease<'a, T>, MemoryError> {
        self.checkout_inner(false)
    }
    /// Nonblocking metadata admission for multi-resource reservation. Busy
    /// releases the temporary owner without retaining a partial resource wait.
    pub fn try_checkout(&self) -> Result<SlotLease<'a, T>, MemoryError> {
        self.checkout_inner(true)
    }
    fn checkout_inner(&self, nonblocking: bool) -> Result<SlotLease<'a, T>, MemoryError> {
        let owner = self.owner.try_clone()?;
        // Declared before the metadata guard, so notification runs after unlock.
        let mut retirement = NotifyRetirement {
            owner: &self.owner,
            retired: false,
        };
        let mut lock = if nonblocking {
            self.owner.try_lock()?
        } else {
            self.owner.lock()
        };
        if lock.state().closed {
            return Err(MemoryError::Closed);
        }
        loop {
            let index = lock.state().head;
            if index == NONE {
                return Err(MemoryError::NoSlot);
            }
            // SAFETY: free-list links were initialized in range and only returned
            // by the unique final lease owner under this lock.
            let record = unsafe { lock.record(index) };
            let next = record.next;
            let generation = record.generation.checked_add(1);
            lock.state().head = next;
            if let Some(generation) = generation {
                // SAFETY: same valid index, previous record borrow has ended.
                let record = unsafe { lock.record(index) };
                record.generation = generation;
                record.references = 1;
                lock.state().active += 1;
                return Ok(SlotLease {
                    owner,
                    id: SlotId { index, generation },
                });
            }
            lock.state().retired += 1;
            retirement.retired = true;
            // Never wrap generations. An exhausted record is retired, reducing
            // capacity rather than letting a stale identifier name a new lease.
        }
    }
    /// Identity-only query; identifiers cannot be used to forge storage access.
    #[must_use]
    pub fn is_leased(&self, id: SlotId) -> bool {
        if id.index >= self.capacity() {
            return false;
        }
        // SAFETY: index checked, lock serializes generation/reference updates.
        let mut lock = self.owner.lock();
        let record = unsafe { lock.record(id.index) };
        record.references != 0 && record.generation == id.generation
    }
    /// Bytes for pool control, records and T slots, excluding partition allocator
    /// headers/padding (those appear in MemoryUsage::active_backing).
    pub fn storage_bytes(count: usize) -> Result<Bytes, MemoryError> {
        let (layout, _) = Layout::new::<Header<'a>>()
            .extend(
                Layout::array::<UnsafeCell<Record>>(count)
                    .map_err(|_| MemoryError::InvalidLayout)?,
            )
            .map_err(|_| MemoryError::InvalidLayout)?;
        let (layout, _) = layout
            .extend(Layout::array::<T>(count).map_err(|_| MemoryError::InvalidLayout)?)
            .map_err(|_| MemoryError::InvalidLayout)?;
        Ok(Bytes::new(layout.size()))
    }
    /// Actual pool and separately prepared bounded return-listener storage.
    pub fn storage_bytes_with_return_listeners(
        count: usize,
        listeners: usize,
    ) -> Result<Bytes, MemoryError> {
        let base = Self::storage_bytes(count)?;
        if listeners == 0 {
            Ok(base)
        } else {
            base.checked_add(SlotPool::<AtomicWaker>::storage_bytes(listeners)?)
        }
    }
}
/// Cancellation-safe unique listener; its slot returns before the watched pool owner.
pub struct ReturnSubscription<'a, T> {
    listener: SlotLease<'a, AtomicWaker>,
    _owner: Owner<'a, T>,
}
// SAFETY: the token has unique listener ownership; the watched pool owner is
// transferable under the same T: Send bound as its SlotLease.
unsafe impl<T: Send> Send for ReturnSubscription<'_, T> {}
impl<T> ReturnSubscription<'_, T> {
    pub fn register(&self, waker: &Waker) {
        self.listener.register(waker);
    }
    pub(crate) fn take_waker(&self) -> Option<Waker> {
        self.listener.take()
    }
}
impl<T> Drop for ReturnSubscription<'_, T> {
    fn drop(&mut self) {
        drop(self.listener.take());
    }
}

/// Generation-bearing identity local to one pool, not a cross-pool address.
/// It never grants read/write access on its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlotId {
    pub index: usize,
    pub generation: usize,
}

/// Unique checkout of an already initialized value. Non-Clone; mutable references
/// cannot coexist with other leases to the same slot. No allocation at checkout.
pub struct SlotLease<'a, T> {
    owner: Owner<'a, T>,
    id: SlotId,
}
// SAFETY: unique T ownership transfers, retaining the counted pool owner.
unsafe impl<T: Send> Send for SlotLease<'_, T> {}
// SAFETY: shared access grants only &T; mutation requires an exclusive lease borrow.
unsafe impl<T: Sync> Sync for SlotLease<'_, T> {}
impl<'a, T> SlotLease<'a, T> {
    pub(crate) fn belongs_to(&self, pool: &SlotPool<'a, T>) -> bool {
        self.owner.pointer == pool.owner.pointer
    }
    #[must_use]
    pub fn id(&self) -> SlotId {
        self.id
    }
    // Private queue/last-owner transfers retain the same slot through payload
    // destruction; final return clears this mark, not the pointer move itself.
    pub(super) fn mark_retained(&self) {
        self.owner.retain(self.id.index, false);
    }
    /// Convert unique ownership to immutable sharing without allocating metadata.
    #[must_use]
    pub fn share(self) -> SharedSlot<'a, T> {
        self.owner.retain(self.id.index, true);
        let lease = ManuallyDrop::new(self);
        // SAFETY: consumed unique lease cannot be dropped/accessed afterwards;
        // its one counted owner/reference transfers unchanged to SharedSlot.
        unsafe {
            SharedSlot {
                owner: std::ptr::read(&lease.owner),
                id: lease.id,
            }
        }
    }
}
impl<T> Deref for SlotLease<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: unique checked-out slot is initialized and owner retains backing.
        unsafe { &*self.owner.value(self.id.index) }
    }
}
impl<T> DerefMut for SlotLease<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: lease is unique; no sharing exists before it is consumed by share.
        unsafe { &mut *self.owner.value(self.id.index) }
    }
}
impl<T> Drop for SlotLease<'_, T> {
    fn drop(&mut self) {
        self.owner.release(self.id.index);
    }
}

/// Immutable views share the entire backing owner; the last view, not the
/// handler or original lease, returns the slot. No per-view heap allocation.
pub struct SharedSlot<'a, T> {
    owner: Owner<'a, T>,
    id: SlotId,
}
// SAFETY: shared immutable access across threads requires T: Sync, and last-drop
// can run on another thread so T must also be Send.
unsafe impl<T: Send + Sync> Send for SharedSlot<'_, T> {}
// SAFETY: same immutable-access and cross-thread destructor constraints.
unsafe impl<T: Send + Sync> Sync for SharedSlot<'_, T> {}
impl<'a, T> SharedSlot<'a, T> {
    /// Internal exclusive-consumer check. A true result remains exclusive while
    /// the sole handle is owned by the caller and no weak authority is reachable.
    pub(crate) fn is_unique(&self) -> bool {
        let mut lock = self.owner.lock();
        // SAFETY: this counted view retains its checked live slot generation.
        let record = unsafe { lock.record(self.id.index) };
        record.references == 1 && record.weak != WeakAccess::Published
    }
    /// Transfer the final shared owner to unique retirement without publishing
    /// the slot as free. Non-final owners release only their reference. This lets
    /// frame payload destructors finish outside metadata gates before reuse.
    #[must_use]
    pub fn into_last(self) -> Option<SlotLease<'a, T>> {
        let this = ManuallyDrop::new(self);
        // SAFETY: the consumed view will not drop; move its one owner exactly
        // once. The local owner remains live on every return/unwind path.
        let owner = unsafe { std::ptr::read(&this.owner) };
        let last = {
            let mut lock = owner.lock();
            // SAFETY: this retained view has the constructor-checked live index.
            let record = unsafe { lock.record(this.id.index) };
            if record.references == 1 {
                // Invalidate every weak authority before granting mutable
                // access. Sharing this lease again must not revive them.
                record.weak = WeakAccess::Retired;
                true
            } else {
                record.references -= 1;
                false
            }
        };
        if last {
            Some(SlotLease { owner, id: this.id })
        } else {
            None
        }
    }

    /// Compare retained slot identity, including pool and generation; identifiers
    /// alone cannot prove that two contexts belong to the same owner.
    #[must_use]
    pub fn same_slot<U>(&self, other: &SharedSlot<'_, U>) -> bool {
        self.owner.pointer.cast::<()>() == other.owner.pointer.cast::<()>() && self.id == other.id
    }
    #[must_use]
    pub fn id(&self) -> SlotId {
        self.id
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        let owner = self.owner.try_clone()?;
        let mut lock = self.owner.lock();
        // SAFETY: this live view retains the checked-out slot with matching index.
        let record = unsafe { lock.record(self.id.index) };
        record.references = record
            .references
            .checked_add(1)
            .filter(|n| *n <= isize::MAX as usize)
            .ok_or(MemoryError::OwnerLimit)?;
        Ok(Self { owner, id: self.id })
    }

    pub(crate) fn downgrade(&self) -> Result<WeakSlot<'a, T>, MemoryError> {
        let owner = self.owner.try_clone()?;
        let mut lock = self.owner.lock();
        // SAFETY: this retained view has the constructor-checked live index.
        let record = unsafe { lock.record(self.id.index) };
        if !matches!(record.weak, WeakAccess::Shared | WeakAccess::Published) {
            return Err(MemoryError::Closed);
        }
        record.weak = WeakAccess::Published;
        Ok(WeakSlot { owner, id: self.id })
    }
}
impl<T> Deref for SharedSlot<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: slot cannot be returned/reused until final shared view drops;
        // shared views expose no mutable access to this initialized T.
        unsafe { &*self.owner.value(self.id.index) }
    }
}
impl<T> Drop for SharedSlot<'_, T> {
    fn drop(&mut self) {
        self.owner.release(self.id.index);
    }
}

/// Generation-bearing weak authority. It retains pool storage without keeping a
/// physical slot checked out.
pub(crate) struct WeakSlot<'a, T> {
    owner: Owner<'a, T>,
    id: SlotId,
}
// SAFETY: the pool owns every initialized T. An upgrade yields only shared T.
unsafe impl<T: Send + Sync> Send for WeakSlot<'_, T> {}
// SAFETY: the checked upgrade is serialized with checkout and exclusive retirement.
unsafe impl<T: Send + Sync> Sync for WeakSlot<'_, T> {}
impl<'a, T> WeakSlot<'a, T> {
    pub(crate) fn is_live(&self) -> bool {
        let mut lock = self.owner.lock();
        // SAFETY: the token retains its pool and constructor-checked index.
        let record = unsafe { lock.record(self.id.index) };
        record.generation == self.id.generation && record.weak == WeakAccess::Published
    }

    pub(crate) fn same_slot(&self, other: &Self) -> bool {
        self.owner.pointer == other.owner.pointer && self.id == other.id
    }

    pub(crate) fn matches_shared(&self, other: &SharedSlot<'_, T>) -> bool {
        self.owner.pointer.cast::<()>() == other.owner.pointer.cast::<()>() && self.id == other.id
    }

    pub(crate) fn upgrade(&self) -> Option<SharedSlot<'a, T>> {
        let owner = self.owner.try_clone().ok()?;
        let mut lock = self.owner.lock();
        // SAFETY: the token retains its pool and constructor-checked index;
        // access checks and pool reference acquisition use the same gate.
        let record = unsafe { lock.record(self.id.index) };
        if record.generation != self.id.generation || record.weak != WeakAccess::Published {
            return None;
        }
        record.references = record
            .references
            .checked_add(1)
            .filter(|n| *n <= isize::MAX as usize)?;
        Some(SharedSlot { owner, id: self.id })
    }
}

impl<'a> PreparedMemory<'a> {
    pub fn slots<T, F>(
        &self,
        region: RegionId,
        count: usize,
        factory: F,
    ) -> Result<SlotPool<'a, T>, MemoryError>
    where
        F: FnMut(usize) -> Result<T, MemoryError>,
    {
        SlotPool::prepare(self, region, count, factory)
    }
    /// Prepare physical listener slots alongside the ordinary typed pool.
    pub fn slots_with_return_listeners<T, F>(
        &self,
        region: RegionId,
        count: usize,
        listener_count: usize,
        factory: F,
    ) -> Result<SlotPool<'a, T>, MemoryError>
    where
        F: FnMut(usize) -> Result<T, MemoryError>,
    {
        SlotPool::prepare_tracked_with_listeners(
            self,
            region,
            count,
            listener_count,
            factory,
            slab_only,
        )
    }
}

// A reserved broadcast control for prepared synchronization. Its listener pool
// is private: leases are never mutably dereferenced while a scan can occur.
#[allow(dead_code)] // Connected as prepared synchronization consumers are migrated.
pub(crate) mod prepared_notification {
    use super::{Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool};
    use futures_util::task::AtomicWaker;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll};

    struct Listener {
        notified: AtomicBool,
        waker: AtomicWaker,
    }
    struct State {
        closed: Mutex<bool>,
        listeners: SlotPool<'static, Listener>,
    }
    pub(crate) struct Event {
        state: SharedSlot<'static, State>,
    }
    pub(crate) struct Registration {
        listener: SlotLease<'static, Listener>,
        state: SharedSlot<'static, State>,
    }
    impl Event {
        pub(crate) fn prepare(
            memory: &PreparedMemory<'static>,
            region: RegionId,
            capacity: usize,
        ) -> Result<Self, MemoryError> {
            let listeners = memory.slots(region, capacity, |_| {
                Ok(Listener {
                    notified: AtomicBool::new(false),
                    waker: AtomicWaker::new(),
                })
            })?;
            let mut listeners = Some(listeners);
            let owner = memory.slots(region, 1, |_| {
                Ok(State {
                    closed: Mutex::new(false),
                    listeners: listeners.take().ok_or(MemoryError::NoSlot)?,
                })
            })?;
            Ok(Self {
                state: owner.checkout()?.share(),
            })
        }
        pub(crate) fn storage_bytes(capacity: usize) -> Result<Bytes, MemoryError> {
            SlotPool::<State>::storage_bytes(1)?
                .checked_add(SlotPool::<Listener>::storage_bytes(capacity)?)
        }
        pub(crate) fn try_clone(&self) -> Result<Self, MemoryError> {
            Ok(Self {
                state: self.state.try_clone()?,
            })
        }
        pub(crate) fn listen(&self) -> Result<Registration, MemoryError> {
            let state = self.state.try_clone()?;
            let listener = state.listeners.checkout()?;
            let closed = state
                .closed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *closed {
                drop(closed);
                return Err(MemoryError::Closed);
            }
            listener.notified.store(false, Ordering::Release);
            drop(closed);
            Ok(Registration { state, listener })
        }
        pub(crate) fn notify_all(&self) {
            self.change(false);
        }
        pub(crate) fn close(&self) {
            self.change(true);
        }
        fn change(&self, close: bool) {
            let mut closed = self
                .state
                .closed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *closed |= close;
            for index in 0..self.state.listeners.capacity() {
                // SAFETY: every slot is initialized for the pool lifetime; this
                // private pool never mutably dereferences a listener lease.
                let listener = unsafe { &*self.state.listeners.owner.value(index) };
                listener.notified.store(true, Ordering::Release);
            }
            drop(closed);
            WakeRemaining {
                listeners: &self.state.listeners,
                next: 0,
            }
            .run();
        }
    }
    struct WakeRemaining<'a> {
        listeners: &'a SlotPool<'static, Listener>,
        next: usize,
    }
    impl WakeRemaining<'_> {
        fn run(&mut self) {
            while self.next < self.listeners.capacity() {
                let index = self.next;
                self.next += 1;
                // SAFETY: the pool retains permanently initialized listener cells;
                // no mutable listener references exist in this private module.
                unsafe {
                    (&*self.listeners.owner.value(index)).waker.wake();
                }
            }
        }
    }
    impl Drop for WakeRemaining<'_> {
        fn drop(&mut self) {
            self.run();
        }
    }
    impl Registration {
        pub(crate) fn reset(&self) {
            let closed = self
                .state
                .closed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.listener.notified.store(false, Ordering::Release);
            drop(closed);
        }
        pub(crate) fn poll(&self, cx: &mut Context<'_>) -> Poll<()> {
            self.listener.waker.register(cx.waker());
            let closed = self
                .state
                .closed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let result = if *closed || self.listener.notified.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            };
            drop(closed);
            result
        }
    }
    impl Drop for Registration {
        fn drop(&mut self) {
            drop(self.listener.waker.take());
        }
    }
}

#[cfg(test)]
mod ownership_tests;
