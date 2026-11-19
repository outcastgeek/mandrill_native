//! Fixed-size resource-vector admission. One prepared coordinator reserves the
//! complete vector atomically; no partial acquisition waits on another pool.
use super::{MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease};
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityUnit {
    Bytes,
    Slots,
    Frames,
    Finalizers,
    Children,
    OutputFrames,
    Connections,
    Streams,
    DependencySlots,
}

/// Work and cleanup/control capacities are independent. Work cannot borrow the
/// cleanup reserve, even under saturation. Counts use the declared resource unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceCapacity {
    pub unit: CapacityUnit,
    pub work: usize,
    pub control: usize,
}
struct Counters<const N: usize> {
    work: [usize; N],
    control: [usize; N],
    closed: bool,
    high_water_work: [usize; N],
    high_water_control: [usize; N],
}
struct State<const N: usize> {
    locked: AtomicBool,
    capacities: [ResourceCapacity; N],
    counters: UnsafeCell<Counters<N>>,
    rejected_work: AtomicUsize,
    rejected_control: AtomicUsize,
}
// SAFETY: all counter access is serialized; capacities are immutable. No client
// callbacks run under lock and mutable references never escape a guard.
unsafe impl<const N: usize> Sync for State<N> {}
struct Guard<'s, const N: usize> {
    state: &'s State<N>,
}
impl<const N: usize> Drop for Guard<'_, N> {
    fn drop(&mut self) {
        self.state.locked.store(false, Ordering::Release);
    }
}
impl<const N: usize> Guard<'_, N> {
    fn counters(&mut self) -> &mut Counters<N> {
        // SAFETY: this guard uniquely owns the atomic metadata lock.
        unsafe { &mut *self.state.counters.get() }
    }
}
impl<const N: usize> State<N> {
    fn try_lock(&self) -> Result<Guard<'_, N>, MemoryError> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| MemoryError::Busy)?;
        Ok(Guard { state: self })
    }
    fn lock(&self) -> Guard<'_, N> {
        loop {
            if let Ok(guard) = self.try_lock() {
                return guard;
            }
            std::hint::spin_loop();
        }
    }
    fn restore(&self, work: &mut [usize; N], control: &mut [usize; N]) {
        let mut guard = self.lock();
        let counters = guard.counters();
        for (available, held) in counters.work.iter_mut().zip(work) {
            *available += *held;
            *held = 0;
        }
        for (available, held) in counters.control.iter_mut().zip(control) {
            *available += *held;
            *held = 0;
        }
    }
}

/// Prepared coordinator. The coordinator and its metadata occupy bounded memory;
/// handle/reservation sharing adds no heap owner or dynamically growing queue.
pub struct Admission<'a, const N: usize> {
    state: SharedSlot<'a, State<N>>,
}
impl<'a, const N: usize> Admission<'a, N> {
    pub fn prepare(
        memory: &PreparedMemory<'a>,
        metadata: RegionId,
        capacities: [ResourceCapacity; N],
    ) -> Result<Self, MemoryError> {
        let pool = memory.slots(metadata, 1, |_| {
            Ok(State {
                locked: AtomicBool::new(false),
                capacities,
                rejected_work: AtomicUsize::new(0),
                rejected_control: AtomicUsize::new(0),
                counters: UnsafeCell::new(Counters {
                    work: capacities.map(|c| c.work),
                    control: capacities.map(|c| c.control),
                    closed: false,
                    high_water_work: [0; N],
                    high_water_control: [0; N],
                }),
            })
        })?;
        Ok(Self {
            state: pool.checkout()?.share(),
        })
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            state: self.state.try_clone()?,
        })
    }
    /// Immutable startup capacity specification.
    #[must_use]
    pub fn capacities(&self) -> &[ResourceCapacity; N] {
        &self.state.capacities
    }
    /// Reserve both vectors before work starts. Busy/refusal leaves every
    /// capacity/high-water counter unchanged (one refusal statistic increases).
    /// The returned owner must follow actual work/output and
    /// async-cleanup settlement; its destructor does not itself await settlement.
    pub fn reserve(
        &self,
        work: [usize; N],
        control: [usize; N],
    ) -> Result<AdmissionBundle<'a, N>, MemoryError> {
        self.reserve_inner(work, control, false)
    }
    /// Separate emergency/control path, usable after ordinary admission closes.
    /// It draws only from reserved control capacity and cannot consume work quota.
    pub fn reserve_control(
        &self,
        control: [usize; N],
    ) -> Result<AdmissionBundle<'a, N>, MemoryError> {
        self.reserve_inner([0; N], control, true)
    }
    /// Acquire actual owners before committing the work and control vectors.
    ///
    /// `acquire` must only perform bounded, nonblocking raw resource acquisition.
    /// It runs outside the counter lock. A physical refusal leaves all counters
    /// unchanged; a quota refusal drops the provisional owner after the lock is
    /// released. Retain the returned value through actual owner settlement.
    pub fn reserve_backed<A>(
        &self,
        work: [usize; N],
        control: [usize; N],
        acquire: impl FnOnce() -> Result<A, MemoryError>,
    ) -> Result<BackedAdmission<'a, N, A>, MemoryError> {
        let owner = acquire()?;
        let bundle = self.reserve(work, control)?;
        Ok(BackedAdmission { owner, bundle })
    }
    /// Acquire actual owners before committing control-only capacity, including
    /// after work admission has closed. The same acquisition rules as
    /// [`Self::reserve_backed`] apply.
    pub fn reserve_control_backed<A>(
        &self,
        control: [usize; N],
        acquire: impl FnOnce() -> Result<A, MemoryError>,
    ) -> Result<BackedAdmission<'a, N, A>, MemoryError> {
        let owner = acquire()?;
        let bundle = self.reserve_control(control)?;
        Ok(BackedAdmission { owner, bundle })
    }
    fn reserve_inner(
        &self,
        work: [usize; N],
        control: [usize; N],
        control_only: bool,
    ) -> Result<AdmissionBundle<'a, N>, MemoryError> {
        let result = self.try_reserve_inner(work, control, control_only);
        if result.is_err() {
            let rejected = if control_only {
                &self.state.rejected_control
            } else {
                &self.state.rejected_work
            };
            // Even Busy/OwnerLimit refusals are counted without taking the
            // contended coordinator lock or allocating diagnostic storage.
            let _ = rejected.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_add(1))
            });
        }
        result
    }
    fn try_reserve_inner(
        &self,
        work: [usize; N],
        control: [usize; N],
        control_only: bool,
    ) -> Result<AdmissionBundle<'a, N>, MemoryError> {
        let state = self.state.try_clone()?;
        let mut guard = self.state.try_lock()?;
        let counters = guard.counters();
        if counters.closed && !control_only {
            return Err(MemoryError::Closed);
        }
        for (index, (requested, available)) in work.iter().zip(&counters.work).enumerate() {
            if requested > available {
                return Err(MemoryError::CapacityExceeded {
                    resource: index,
                    requested: *requested,
                    available: *available,
                    control: false,
                });
            }
        }
        for (index, (requested, available)) in control.iter().zip(&counters.control).enumerate() {
            if requested > available {
                return Err(MemoryError::CapacityExceeded {
                    resource: index,
                    requested: *requested,
                    available: *available,
                    control: true,
                });
            }
        }
        // All resources were checked under one lock, so commit cannot fail.
        for (available, requested) in counters.work.iter_mut().zip(&work) {
            *available -= *requested;
        }
        for (available, requested) in counters.control.iter_mut().zip(&control) {
            *available -= *requested;
        }
        for ((peak, available), capacity) in counters
            .high_water_work
            .iter_mut()
            .zip(&counters.work)
            .zip(&self.state.capacities)
        {
            *peak = (*peak).max(capacity.work - available);
        }
        for ((peak, available), capacity) in counters
            .high_water_control
            .iter_mut()
            .zip(&counters.control)
            .zip(&self.state.capacities)
        {
            *peak = (*peak).max(capacity.control - available);
        }
        Ok(AdmissionBundle {
            state,
            work,
            control,
        })
    }
    pub fn close(&self) {
        self.state.lock().counters().closed = true;
    }
    #[must_use]
    pub fn usage(&self) -> AdmissionUsage<N> {
        let mut guard = self.state.lock();
        let counters = guard.counters();
        AdmissionUsage {
            capacities: self.state.capacities,
            available_work: counters.work,
            available_control: counters.control,
            closed: counters.closed,
            high_water_work: counters.high_water_work,
            high_water_control: counters.high_water_control,
            rejected_work: self.state.rejected_work.load(Ordering::Relaxed),
            rejected_control: self.state.rejected_control.load(Ordering::Relaxed),
        }
    }
}

/// Actual physical ownership and its numeric admission move together.
///
/// The owner is dropped before the quota bundle, including during unwinding.
/// Move the whole value to transfer both; it does not expose a detached bundle.
#[must_use = "dropping this value releases its physical owner and admission"]
pub struct BackedAdmission<'a, const N: usize, A> {
    owner: A,
    bundle: AdmissionBundle<'a, N>,
}
impl<const N: usize, A> BackedAdmission<'_, N, A> {
    /// Borrow the actual checked-out owner without separating it from its quota.
    #[must_use]
    pub fn owner(&self) -> &A {
        &self.owner
    }

    /// Work units retained by this owner.
    #[must_use]
    pub fn work(&self) -> &[usize; N] {
        self.bundle.work()
    }

    /// Control units retained by this owner.
    #[must_use]
    pub fn control(&self) -> &[usize; N] {
        self.bundle.control()
    }
}
impl<const N: usize, T> BackedAdmission<'_, N, SlotLease<'_, T>> {
    /// Mutate the leased payload while keeping its actual slot and quota owned.
    pub fn slot_value_mut(&mut self) -> &mut T {
        &mut self.owner
    }
}

/// Availability and high-water arrays are one locked snapshot. Rejection totals
/// are independently atomic samples: simultaneous refusals can advance them
/// while this snapshot is read. Split/release never count as new reservations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionUsage<const N: usize> {
    pub capacities: [ResourceCapacity; N],
    pub available_work: [usize; N],
    pub available_control: [usize; N],
    pub closed: bool,
    /// Maximum simultaneously held units per resource, not cumulative demand.
    pub high_water_work: [usize; N],
    pub high_water_control: [usize; N],
    /// Saturating refused reserve calls (including Busy/Closed/OwnerLimit).
    /// A call is counted once, not once per failing vector component. Ordinary
    /// reserve calls use rejected_work even if control capacity caused refusal.
    pub rejected_work: usize,
    /// Refused reserve_control calls only; independent of ordinary admission.
    pub rejected_control: usize,
}

/// A complete reservation transaction; drop rolls back or releases the complete
/// retained vector. Non-Clone: splitting explicitly transfers already-held quota.
/// These quotas are admission accounting, not a second physical memory region;
/// adapters must tie their actual leases and resource demand to the same vector.
pub struct AdmissionBundle<'a, const N: usize> {
    state: SharedSlot<'a, State<N>>,
    work: [usize; N],
    control: [usize; N],
}
impl<const N: usize> AdmissionBundle<'_, N> {
    #[must_use]
    pub fn work(&self) -> &[usize; N] {
        &self.work
    }
    #[must_use]
    pub fn control(&self) -> &[usize; N] {
        &self.control
    }
    /// Transfer a subset to a child/output owner without reacquiring capacity.
    pub fn split(&mut self, work: [usize; N], control: [usize; N]) -> Result<Self, MemoryError> {
        for (index, (requested, held)) in work.iter().zip(&self.work).enumerate() {
            if requested > held {
                return Err(MemoryError::CapacityExceeded {
                    resource: index,
                    requested: *requested,
                    available: *held,
                    control: false,
                });
            }
        }
        for (index, (requested, held)) in control.iter().zip(&self.control).enumerate() {
            if requested > held {
                return Err(MemoryError::CapacityExceeded {
                    resource: index,
                    requested: *requested,
                    available: *held,
                    control: true,
                });
            }
        }
        let state = self.state.try_clone()?;
        for (held, moved) in self.work.iter_mut().zip(&work) {
            *held -= *moved;
        }
        for (held, moved) in self.control.iter_mut().zip(&control) {
            *held -= *moved;
        }
        Ok(Self {
            state,
            work,
            control,
        })
    }
    /// Work has actually settled; keep cleanup quota until its owner settles too.
    pub fn release_work(&mut self) {
        self.state.restore(&mut self.work, &mut [0; N]);
    }
}
impl<const N: usize> Drop for AdmissionBundle<'_, N> {
    fn drop(&mut self) {
        self.state.restore(&mut self.work, &mut self.control);
    }
}
