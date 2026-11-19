//! Startup-backed per-request observer batches.
use effect::Effect;
use effect::memory::{
    Admission, AdmissionBundle, Bytes, CapacityUnit, FixedVector, MemoryError, PreparedMemory,
    RegionId, SlotLease, SlotPool,
};

pub(crate) struct ObserverBatchPool<E: Send + 'static, R: Send + Sync + 'static> {
    slots: SlotPool<'static, FixedVector<'static, Effect<(), E, R>>>,
    admission: Option<Admission<'static, 1>>,
}
pub(crate) struct ObserverBatch<E: Send + 'static, R: Send + Sync + 'static> {
    slot: SlotLease<'static, FixedVector<'static, Effect<(), E, R>>>,
    _quota: Option<AdmissionBundle<'static, 1>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> ObserverBatchPool<E, R> {
    pub(crate) fn arc_payload_layout() -> std::alloc::Layout {
        std::alloc::Layout::new::<Self>()
    }
    pub(crate) fn storage_bytes(batches: usize, observers: usize) -> Result<Bytes, MemoryError> {
        let values = std::alloc::Layout::array::<Effect<(), E, R>>(observers)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<FixedVector<'static, Effect<(), E, R>>>::storage_bytes(batches)?
            .checked_add(Bytes::new(values.size()).checked_mul(batches)?)
    }
    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        batches: usize,
        observers: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(batches, observers)?;
        let slots = memory.slots(region, batches, |_| {
            FixedVector::prepare(memory.allocator(region)?, observers)
        })?;
        Ok(Self {
            slots,
            admission: None,
        })
    }
    pub(crate) fn install_admission(
        &mut self,
        admission: Admission<'static, 1>,
    ) -> Result<(), MemoryError> {
        if admission.capacities()[0].unit != CapacityUnit::Slots {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.admission = Some(admission);
        Ok(())
    }
    pub(crate) fn reserve(&self) -> Result<ObserverBatch<E, R>, MemoryError> {
        let slot = self.slots.try_checkout()?;
        let quota = self
            .admission
            .as_ref()
            .map(|admission| admission.reserve([1], [0]))
            .transpose()?;
        Ok(ObserverBatch {
            slot,
            _quota: quota,
        })
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> ObserverBatch<E, R> {
    pub(crate) fn capacity(&self) -> usize {
        self.slot.capacity()
    }
    pub(crate) fn push(&mut self, effect: Effect<(), E, R>) -> Result<(), Effect<(), E, R>> {
        self.slot.push(effect)
    }
    pub(crate) fn pop_front(&mut self) -> Option<Effect<(), E, R>> {
        self.slot.remove_first()
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for ObserverBatch<E, R> {
    fn drop(&mut self) {
        self.slot.clear();
    }
}
