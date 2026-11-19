//! Startup-prepared request base and immutable metadata snapshot owners.
use crate::input::{RequestBase, RequestState};
use effect::memory::{
    Admission, AdmissionBundle, Bytes, CapacityUnit, FixedVector, MemoryError, PreparedMemory,
    RegionId, SharedSlot, SlotPool,
};
use std::cell::UnsafeCell;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

struct Payload<T> {
    loans: Arc<Mutex<LoanState>>,
    value: UnsafeCell<Option<T>>,
}
struct LoanState {
    count: usize,
    quota: Option<AdmissionBundle<'static, 1>>,
}
// SAFETY: payload publication/removal holds the loan gate. A shared reference
// is available only through a live loan, so the last-loan removal cannot race
// with a borrow. Arc::get_mut synchronizes retirement before the next checkout.
unsafe impl<T: Send + Sync> Sync for Payload<T> {}
struct Control<T> {
    payload: SharedSlot<'static, Payload<T>>,
}
type Registry<T> = Mutex<FixedVector<'static, Arc<Control<T>>>>;
pub(crate) struct Pool<T: Send + Sync + 'static> {
    registry: SharedSlot<'static, Registry<T>>,
}
pub(crate) struct Vacancy<T: Send + Sync + 'static> {
    control: Arc<Control<T>>,
    quota: Option<AdmissionBundle<'static, 1>>,
}
pub(crate) struct Loan<T: Send + Sync + 'static> {
    control: Option<Arc<Control<T>>>,
}
impl<T: Send + Sync + 'static> Pool<T> {
    pub(crate) fn storage_bytes(count: usize) -> Result<Bytes, MemoryError> {
        let vector = std::alloc::Layout::array::<Arc<Control<T>>>(count)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Payload<T>>::storage_bytes(count)?
            .checked_add(SlotPool::<Registry<T>>::storage_bytes(1)?)?
            .checked_add(Bytes::new(vector.size()))
    }
    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
    ) -> Result<Self, MemoryError> {
        let payloads = memory.slots(region, count, |_| {
            Ok(Payload {
                loans: Arc::new(Mutex::new(LoanState {
                    count: 0,
                    quota: None,
                })),
                value: UnsafeCell::new(None),
            })
        })?;
        let mut controls = FixedVector::prepare(memory.allocator(region)?, count)?;
        for _ in 0..count {
            let control = Arc::new(Control {
                payload: payloads.checkout()?.share(),
            });
            if let Err(control) = controls.push(control) {
                drop(control);
                return Err(MemoryError::NoSlot);
            }
        }
        let mut controls = Some(controls);
        let registry = memory.slots(region, 1, |_| {
            Ok(Mutex::new(controls.take().ok_or(MemoryError::NoSlot)?))
        })?;
        Ok(Self {
            registry: registry.checkout()?.share(),
        })
    }
    pub(crate) fn reserve(
        &self,
        admission: Option<&Admission<'static, 1>>,
    ) -> Result<Vacancy<T>, MemoryError> {
        let control = {
            let mut registry = crate::try_admission_lock(&self.registry)?;
            let mut selected = None;
            for control in registry.as_mut_slice() {
                if Arc::get_mut(control).is_some() {
                    selected = Some(Arc::clone(control));
                    break;
                }
            }
            selected
        }
        .ok_or(MemoryError::NoSlot)?;
        let quota = admission
            .map(|admission| admission.reserve([1], [0]))
            .transpose()?;
        Ok(Vacancy { control, quota })
    }
}
impl<T: Send + Sync + 'static> Vacancy<T> {
    #[allow(clippy::result_large_err)] // Preserve the original payload on invariant refusal.
    pub(crate) fn publish(mut self, value: T) -> Result<Loan<T>, T> {
        let mut loans = self
            .control
            .payload
            .loans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: vacancy is the only external Arc and publication holds the gate.
        let slot = unsafe { &mut *self.control.payload.value.get() };
        if slot.is_some() || loans.count != 0 {
            return Err(value);
        }
        *slot = Some(value);
        loans.count = 1;
        loans.quota = self.quota.take();
        drop(loans);
        Ok(Loan {
            control: Some(self.control),
        })
    }
}
impl<T: Send + Sync + 'static> Clone for Loan<T> {
    #[expect(
        clippy::unreachable,
        reason = "a live request loan retains its control until Drop"
    )]
    fn clone(&self) -> Self {
        let Some(current) = &self.control else {
            unreachable!("live request storage loan");
        };
        let control = Arc::clone(current);
        // Each loan holds an Arc first; its count limit precedes usize overflow.
        control
            .payload
            .loans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .count += 1;
        Self {
            control: Some(control),
        }
    }
}
impl<T: Send + Sync + 'static> Deref for Loan<T> {
    type Target = T;
    #[expect(
        clippy::unreachable,
        reason = "a live request loan retains its control until Drop"
    )]
    fn deref(&self) -> &T {
        // SAFETY: publication precedes the first loan and the payload remains
        // immutable until the last live loan has retired; this borrow holds one.
        let Some(control) = &self.control else {
            unreachable!("live request storage loan");
        };
        unsafe { (&*control.payload.value.get()).as_ref().unwrap_unchecked() }
    }
}
impl<T: Send + Sync + 'static> Drop for Loan<T> {
    fn drop(&mut self) {
        // Quota is declared first so control and value retire before it on unwind.
        let quota;
        let Some(control) = self.control.take() else {
            return;
        };
        let gate = Arc::clone(&control.payload.loans);
        let value = {
            let mut loans = gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if loans.count != 1 {
                drop(control);
                loans.count -= 1;
                return;
            }
            loans.count = 0;
            quota = loans.quota.take();
            // SAFETY: the last loan removes the value under its independent gate.
            unsafe { (&mut *control.payload.value.get()).take() }
        };
        drop(value);
        drop(control);
        drop(quota);
    }
}
/// Caller-selected simultaneously retained base requests and metadata snapshots.
#[derive(Clone, Copy, Debug)]
pub struct PreparedRequestStorageCapacity {
    /// Simultaneously retained request bodies and extension sets.
    pub requests: usize,
    /// Simultaneously retained immutable metadata snapshots.
    pub snapshots: usize,
}
impl PreparedRequestStorageCapacity {
    /// Minimum builtin metadata slots for these concurrent routing flows.
    /// Additional callback-retained snapshots require additional capacity.
    pub fn builtin_snapshots_for(requests: usize) -> Result<usize, MemoryError> {
        requests
            .checked_mul(4)
            .ok_or(MemoryError::ArithmeticOverflow)
    }
}
/// Physical region payload and startup Arc control accounting. Arc payload
/// bytes exclude standard-library headers and allocator rounding; HTTP-native
/// metadata and arbitrary pointees are separate.
pub struct PreparedRequestStorageDemand {
    pub region_bytes: Bytes,
    pub startup_arc_controls: usize,
    pub startup_arc_payload_bytes: Bytes,
}
/// Finite request storage. Native HTTP container and cancellation allocations
/// remain separately owned and are not certified by these records.
pub struct PreparedRequestStoragePool {
    roots: Pool<RequestBase>,
    snapshots: Pool<RequestState>,
    capacity: PreparedRequestStorageCapacity,
    admission: Option<Admission<'static, 1>>,
}
impl PreparedRequestStoragePool {
    pub fn demand(
        capacity: PreparedRequestStorageCapacity,
    ) -> Result<PreparedRequestStorageDemand, MemoryError> {
        let controls = capacity
            .requests
            .checked_add(capacity.snapshots)
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| v.checked_add(1))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let root = Bytes::new(std::mem::size_of::<Control<RequestBase>>())
            .checked_mul(capacity.requests)?;
        let snapshots = Bytes::new(std::mem::size_of::<Control<RequestState>>())
            .checked_mul(capacity.snapshots)?;
        Ok(PreparedRequestStorageDemand {
            region_bytes: Pool::<RequestBase>::storage_bytes(capacity.requests)?
                .checked_add(Pool::<RequestState>::storage_bytes(capacity.snapshots)?)?,
            startup_arc_controls: controls,
            startup_arc_payload_bytes: root
                .checked_add(snapshots)?
                .checked_add(
                    Bytes::new(std::mem::size_of::<Mutex<LoanState>>()).checked_mul(
                        capacity
                            .requests
                            .checked_add(capacity.snapshots)
                            .ok_or(MemoryError::ArithmeticOverflow)?,
                    )?,
                )?
                .checked_add(Bytes::new(std::mem::size_of::<Self>()))?,
        })
    }
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedRequestStorageCapacity,
    ) -> Result<Arc<Self>, MemoryError> {
        let _ = Self::demand(capacity)?;
        let roots = Pool::prepare(memory, region, capacity.requests)?;
        let snapshots = Pool::prepare(memory, region, capacity.snapshots)?;
        Ok(Arc::new(Self {
            roots,
            snapshots,
            capacity,
            admission: None,
        }))
    }
    /// Couple each selected root or snapshot control to one Slots work unit.
    /// HTTP containers and their native or caller-owned payloads are separate.
    pub fn with_admission(
        mut self: Arc<Self>,
        admission: Admission<'static, 1>,
    ) -> Result<Arc<Self>, MemoryError> {
        if admission.capacities()[0].unit != CapacityUnit::Slots {
            return Err(MemoryError::UnsupportedRunContract);
        }
        Arc::get_mut(&mut self).ok_or(MemoryError::Busy)?.admission = Some(admission);
        Ok(self)
    }
    /// Selected physical capacities; no implicit request/snapshot limit is added.
    #[must_use]
    pub fn capacity(&self) -> PreparedRequestStorageCapacity {
        self.capacity
    }
    pub(crate) fn reserve_base(&self) -> Result<BaseVacancy, MemoryError> {
        self.roots.reserve(self.admission.as_ref()).map(BaseVacancy)
    }
    pub(crate) fn reserve_snapshot(&self) -> Result<SnapshotVacancy, MemoryError> {
        self.snapshots
            .reserve(self.admission.as_ref())
            .map(SnapshotVacancy)
    }
}
pub(crate) struct BaseVacancy(Vacancy<RequestBase>);
pub(crate) struct SnapshotVacancy(Vacancy<RequestState>);
pub(crate) type BaseLoan = Loan<RequestBase>;
pub(crate) type SnapshotLoan = Loan<RequestState>;
impl BaseVacancy {
    #[allow(clippy::result_large_err)] // Preserve the actual initialized base on refusal.
    pub(crate) fn publish(self, value: RequestBase) -> Result<BaseLoan, RequestBase> {
        self.0.publish(value)
    }
}
impl SnapshotVacancy {
    #[allow(clippy::result_large_err)] // Preserve the actual initialized snapshot on refusal.
    pub(crate) fn publish(self, value: RequestState) -> Result<SnapshotLoan, RequestState> {
        self.0.publish(value)
    }
}
/// One actual base and metadata snapshot reserved before native request work.
pub(crate) struct PreparedRequestStorageReservation {
    pub(crate) base: BaseVacancy,
    pub(crate) snapshot: SnapshotVacancy,
    pub(crate) pool: Arc<PreparedRequestStoragePool>,
}
impl PreparedRequestStoragePool {
    /// Reserve the complete initial request backing without invoking a factory.
    pub(crate) fn reserve_request(
        self: &Arc<Self>,
    ) -> Result<PreparedRequestStorageReservation, MemoryError> {
        let base = self.reserve_base()?;
        let snapshot = self.reserve_snapshot()?;
        Ok(PreparedRequestStorageReservation {
            base,
            snapshot,
            pool: Arc::clone(self),
        })
    }
}
