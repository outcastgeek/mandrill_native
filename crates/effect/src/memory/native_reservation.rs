//! Complete native allocation vectors acquired before parsing/factory work.
//! Each run owns its actual parent allocations and pre-held recycling metadata;
//! accepted allocations retain the run after its serving/task owner closes.
use super::*;
use super::{NativeAllocation, NativeDomain as Domain, NativeRefusal as Refusal};
use crate::internal::destruction::{DestructionNode, enqueue_prepared};
use std::cell::UnsafeCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

struct Cell {
    layout: Layout,
    backing: UnsafeCell<Option<NativeAllocation>>,
    leased: UnsafeCell<bool>,
    run: UnsafeCell<*const Run>,
    node: UnsafeCell<DestructionNode>,
}
// SAFETY: control fields/backing ownership are serialized by Run.state. Only an
// exclusive allocation accesses its data. Node stays leased through retirement.
unsafe impl Send for Cell {}
unsafe impl Sync for Cell {}
struct State {
    open: bool,
    close_drained: bool,
    live: usize,
}
struct RunOwner {
    _lease: SlotLease<'static, Run>,
    quota: Option<AdmissionBundle<'static, 2>>,
}
struct Run {
    cells: FixedVector<'static, Cell>,
    state: Mutex<State>,
    owner: UnsafeCell<MaybeUninit<RunOwner>>,
}
// SAFETY: owner is installed on exclusive checkout and taken once under state;
// cells use that same state gate. Actual byte access follows Domain's ownership.
unsafe impl Send for Run {}
unsafe impl Sync for Run {}
unsafe fn empty(_: NonNull<DestructionNode>) {}
unsafe fn finish_cell(node: NonNull<DestructionNode>) {
    // SAFETY: exact stable cell/run installed during admission; this queued
    // allocation still contributes to live, retaining the actual run lease.
    let cell = unsafe { &*DestructionNode::context(node).cast::<Cell>() };
    let run = unsafe { &*(*cell.run.get()) };
    let (backing, owner) = {
        let mut state = run
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        unsafe {
            *cell.leased.get() = false;
        }
        state.live -= 1;
        let backing = if state.open {
            None
        } else {
            unsafe { (*cell.backing.get()).take() }
        };
        let owner = if !state.open && state.close_drained && state.live == 0 {
            Some(unsafe { (*run.owner.get()).assume_init_read() })
        } else {
            None
        };
        (backing, owner)
    };
    // No userdata/native owner drop or physical return under the state gate.
    // Nothing touches cell/run after the possible last metadata-lease return.
    drop(backing);
    drop(owner);
}
unsafe fn release_cell(pointer: NonNull<()>) {
    // SAFETY: paired allocation owns this cell once; queued live keeps the run
    // and every control address stable until the native descendants finish.
    let cell = unsafe { pointer.cast::<Cell>().as_ref() };
    unsafe {
        enqueue_prepared(NonNull::new_unchecked(cell.node.get()));
    }
}
impl Run {
    fn close(&self) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.open = false;
        }
        // Advance by ordinary iteration before each owner Drop. The run lease
        // cannot return until close_drained, even if another cell retires now.
        struct Close<'a> {
            run: &'a Run,
            next: usize,
        }
        impl Close<'_> {
            fn drain(&mut self) {
                while self.next < self.run.cells.len() {
                    let index = self.next;
                    self.next += 1;
                    let cell = match self.run.cells.as_slice().get(index) {
                        Some(cell) => cell,
                        None => continue,
                    };
                    let backing = {
                        let _state = self
                            .run
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if unsafe { *cell.leased.get() } {
                            None
                        } else {
                            unsafe { (*cell.backing.get()).take() }
                        }
                    };
                    drop(backing);
                }
            }
        }
        impl Drop for Close<'_> {
            fn drop(&mut self) {
                self.drain();
            }
        }
        // Finalize after the unwind-resuming close cursor, including on panic.
        struct Finished<'a>(&'a Run);
        impl Drop for Finished<'_> {
            fn drop(&mut self) {
                let owner = {
                    let mut state = self
                        .0
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.close_drained = true;
                    if state.live == 0 {
                        Some(unsafe { (*self.0.owner.get()).assume_init_read() })
                    } else {
                        None
                    }
                };
                drop(owner);
            }
        }
        let _finished = Finished(self);
        let mut close = Close { run: self, next: 0 };
        close.drain();
    }
}
/// Startup metadata for complete vectors of native allocations. The layouts
/// describe actual data/control/header blocks, not a byte counter or demand label.
pub struct NativeReservationPool {
    runs: SlotPool<'static, Run>,
    closed: AtomicBool,
    admission: Option<Admission<'static, 2>>,
}
impl NativeReservationPool {
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        metadata: RegionId,
        capacity: usize,
        layouts: &[Layout],
    ) -> Result<Self, MemoryError> {
        let runs = SlotPool::prepare(memory, metadata, capacity, |_| {
            let mut cells = FixedVector::prepare(memory.allocator(metadata)?, layouts.len())?;
            for layout in layouts {
                if cells
                    .push(Cell {
                        layout: *layout,
                        backing: UnsafeCell::new(None),
                        leased: UnsafeCell::new(false),
                        run: UnsafeCell::new(std::ptr::null()),
                        node: UnsafeCell::new(DestructionNode::new(empty, empty, finish_cell)),
                    })
                    .is_err()
                {
                    return Err(MemoryError::NoSlot);
                }
            }
            Ok(Run {
                cells,
                state: Mutex::new(State {
                    open: false,
                    close_drained: true,
                    live: 0,
                }),
                owner: UnsafeCell::new(MaybeUninit::uninit()),
            })
        })?;
        Ok(Self {
            runs,
            closed: AtomicBool::new(false),
            admission: None,
        })
    }
    /// Couple each retained run and its actual parent allocation sizes to an
    /// admission coordinator with ordered Slots and Bytes capacities. Metadata,
    /// class slack and allocations from other domains are separate. The charge
    /// lasts through parent allocation release callbacks; a parent may defer
    /// recycling its backing beyond those callbacks.
    pub fn with_admission(mut self, admission: Admission<'static, 2>) -> Result<Self, MemoryError> {
        let capacities = admission.capacities();
        if capacities[0].unit != CapacityUnit::Slots || capacities[1].unit != CapacityUnit::Bytes {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.admission = Some(admission);
        Ok(self)
    }
    pub fn reserve(&self, parent: &dyn Domain) -> Result<NativeReservation, Refusal> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Refusal::Closed);
        }
        let mut owner = match self.runs.try_checkout() {
            Ok(owner) => owner,
            Err(MemoryError::Closed) => return Err(Refusal::Closed),
            Err(MemoryError::OwnerLimit) => return Err(Refusal::OwnerLimit),
            Err(MemoryError::Busy) => return Err(Refusal::Busy),
            Err(MemoryError::NoSlot) => return Err(Refusal::Capacity),
            Err(_) => return Err(Refusal::Backing),
        };
        *owner
            .state
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = State {
            open: true,
            close_drained: false,
            live: 0,
        };
        let pointer = NonNull::from(&*owner);
        // SAFETY: exclusive checked-out run; prior return completed all cell
        // retirement. Publish self owner BEFORE any parent allocation callback.
        unsafe {
            let run = pointer.as_ref();
            (*run.owner.get()).write(RunOwner {
                _lease: owner,
                quota: None,
            });
            for cell in run.cells.as_slice() {
                *cell.run.get() = pointer.as_ptr();
                (*cell.node.get()).set_context((cell as *const Cell).cast_mut().cast());
            }
        }
        let reservation = NativeReservation { run: pointer };
        let mut bytes = 0usize;
        for cell in unsafe { pointer.as_ref() }.cells.as_slice() {
            let allocation = parent.try_allocate(cell.layout)?;
            if self.admission.is_some() {
                bytes = bytes
                    .checked_add(allocation.layout().size())
                    .ok_or(Refusal::Capacity)?;
            }
            // SAFETY: not exposed yet, no allocation can access this free cell.
            unsafe {
                *cell.backing.get() = Some(allocation);
            }
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(Refusal::Closed);
        }
        if let Some(admission) = &self.admission {
            let quota = admission.reserve([1, bytes], [0, 0]).map_err(|error| {
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
            })?;
            // SAFETY: the initialized owner remains exclusively unpublished;
            // no callback or state gate is active during quota attachment.
            unsafe { (*(*pointer.as_ref().owner.get()).as_mut_ptr()).quota = Some(quota) };
        }
        Ok(reservation)
    }
    /// Actual metadata layouts, excluding partition headers/padding reported by
    /// MemoryUsage, and excluding the separately owned parent backing vector.
    pub fn storage_bytes(capacity: usize, allocation_count: usize) -> Result<Bytes, MemoryError> {
        let cells =
            Layout::array::<Cell>(allocation_count).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Run>::storage_bytes(capacity)?
            .checked_add(Bytes::new(cells.size()).checked_mul(capacity)?)
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.runs.close();
    }
}
/// Actual vector ownership for one admitted native activity. Drop closes only
/// new construction, not the lifetime of an accepted buffer/frame/header view.
pub struct NativeReservation {
    run: NonNull<Run>,
}
// SAFETY: run/state and its pre-held cells are synchronized; reservation owns
// its unique serving handle, while each live allocation retains the self lease.
unsafe impl Send for NativeReservation {}
unsafe impl Sync for NativeReservation {}
impl Drop for NativeReservation {
    fn drop(&mut self) {
        unsafe { self.run.as_ref() }.close();
    }
}
// SAFETY: only free actual parent backing is handed out; no late allocation or
// fallback occurs. All retired bytes wait through the common phased drain.
unsafe impl Domain for NativeReservation {
    fn try_allocate(&self, layout: Layout) -> Result<NativeAllocation, Refusal> {
        let run = unsafe { self.run.as_ref() };
        let mut state = run.state.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => Refusal::Busy,
            std::sync::TryLockError::Poisoned(_) => Refusal::Backing,
        })?;
        if !state.open {
            return Err(Refusal::Closed);
        }
        let mut suitable = false;
        for cell in run.cells.as_slice() {
            if cell.layout.size() < layout.size() || cell.layout.align() < layout.align() {
                continue;
            }
            suitable = true;
            if unsafe { *cell.leased.get() } {
                continue;
            }
            let backing = unsafe { (*cell.backing.get()).as_ref() }.ok_or(Refusal::Backing)?;
            let pointer = backing.pointer();
            unsafe {
                *cell.leased.get() = true;
                pointer.as_ptr().write_bytes(0, layout.size());
            }
            state.live += 1;
            // SAFETY: exclusive cell, actual initialized/aligned parent backing,
            // stable live run lease; release is paired with this exact cell.
            return Ok(unsafe {
                NativeAllocation::from_owned_raw(
                    pointer,
                    layout,
                    NonNull::from(cell).cast(),
                    release_cell,
                )
            });
        }
        Err(if suitable {
            Refusal::Capacity
        } else {
            Refusal::Layout
        })
    }
}
