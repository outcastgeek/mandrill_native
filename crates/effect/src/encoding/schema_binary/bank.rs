//! Reusable admitted union diagnostic builders for one binary operation.
use super::BinaryError;
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
};
use crate::schema::{PreparedJsonArrayBuilder, PreparedJsonArrayPool};
use std::sync::Mutex;

type Builder = PreparedJsonArrayBuilder<BinaryError>;
type List = FixedVector<'static, Builder>;
type Cell = Mutex<Option<List>>;
/// Explicit simultaneous banks and builders admitted into each bank.
#[derive(Clone, Copy, Debug)]
pub struct PreparedBinaryUnionCapacity {
    /// Number of concurrent encoding bundles.
    pub banks: usize,
    /// Actual diagnostic array builders held per bundle.
    pub builders_per_bank: usize,
}
impl PreparedBinaryUnionCapacity {
    /// Region controls and vector slots. Error arrays and their frame backing are separate.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let layout = std::alloc::Layout::array::<Builder>(self.builders_per_bank)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Cell>::storage_bytes(self.banks)?
            .checked_add(Bytes::new(layout.size()).checked_mul(self.banks)?)
    }
}
/// Region-backed banks; each admission also claims its real error-array builders.
pub struct PreparedBinaryUnionPool {
    cells: SlotPool<'static, Cell>,
}
impl PreparedBinaryUnionPool {
    /// Prepare bank control cells and fixed builder storage.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedBinaryUnionCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.storage_bytes()?;
        let cells = memory.slots(region, capacity.banks, |_| {
            Ok(Mutex::new(Some(FixedVector::prepare(
                memory.allocator(region)?,
                capacity.builders_per_bank,
            )?)))
        })?;
        Ok(Self { cells })
    }
    pub(super) fn admit(
        &self,
        errors: &PreparedJsonArrayPool<BinaryError>,
    ) -> Result<Bank, MemoryError> {
        let slot = self.cells.checkout()?;
        let mut cleanup = Drain::take(slot)?;
        drain_builders(cleanup.values.as_mut().ok_or(MemoryError::Closed)?);
        let count = cleanup
            .values
            .as_ref()
            .ok_or(MemoryError::Closed)?
            .capacity();
        let slot = cleanup.restore()?;
        let bank = Bank {
            slot: Some(slot.share()),
        };
        for _ in 0..count {
            let builder = errors.reserve_shared()?;
            let rejected = {
                let slot = bank.slot.as_ref().ok_or(MemoryError::Closed)?;
                let mut guard = slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match guard.as_mut() {
                    Some(values) => values.push(builder).err(),
                    None => Some(builder),
                }
            };
            if let Some(builder) = rejected {
                drop(builder);
                return Err(MemoryError::NoSlot);
            }
        }
        Ok(bank)
    }
}
// On unwind, return the emptied actual fixed vector before returning its slot.
fn drain_builders(values: &mut List) {
    let mut continuation = std::mem::DropGuard::new(values, |values| {
        while let Some(builder) = values.pop() {
            drop(builder);
        }
    });
    while let Some(builder) = continuation.pop() {
        drop(builder);
    }
    let _ = std::mem::DropGuard::dismiss(continuation);
}
struct Drain {
    slot: Option<SlotLease<'static, Cell>>,
    values: Option<List>,
}
impl Drain {
    fn take(slot: SlotLease<'static, Cell>) -> Result<Self, MemoryError> {
        let values = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or(MemoryError::Closed)?;
        Ok(Self {
            slot: Some(slot),
            values: Some(values),
        })
    }
    fn restore(mut self) -> Result<SlotLease<'static, Cell>, MemoryError> {
        let slot = self.slot.take().ok_or(MemoryError::Closed)?;
        let values = self.values.take().ok_or(MemoryError::Closed)?;
        *slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(values);
        Ok(slot)
    }
}
impl Drop for Drain {
    fn drop(&mut self) {
        if let (Some(slot), Some(values)) = (self.slot.take(), self.values.take()) {
            *slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(values);
            drop(slot);
        }
    }
}
pub(super) struct Bank {
    slot: Option<SharedSlot<'static, Cell>>,
}
impl Bank {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slot: Some(self.slot.as_ref().ok_or(MemoryError::Closed)?.try_clone()?),
        })
    }
    pub(super) fn checkout(&self) -> Result<BankBuilder, MemoryError> {
        let bank = self.try_clone()?;
        let builder = {
            let slot = self.slot.as_ref().ok_or(MemoryError::Closed)?;
            let mut guard = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard
                .as_mut()
                .ok_or(MemoryError::Closed)?
                .pop()
                .ok_or(MemoryError::NoSlot)?
        };
        Ok(BankBuilder {
            builder: Some(builder),
            bank,
        })
    }
}
impl Drop for Bank {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take()
            && let Some(slot) = slot.into_last()
            && let Ok(mut drain) = Drain::take(slot)
            && let Some(values) = drain.values.as_mut()
        {
            drain_builders(values);
        }
    }
}
pub(super) struct BankBuilder {
    builder: Option<Builder>,
    bank: Bank,
}
impl BankBuilder {
    pub(super) fn capacity(&self) -> usize {
        self.builder.as_ref().map_or(0, Builder::capacity)
    }
    pub(super) fn try_push(&mut self, error: BinaryError) -> Result<(), BinaryError> {
        match &mut self.builder {
            Some(builder) => builder.try_push(error),
            None => Err(error),
        }
    }
    pub(super) fn publish(
        mut self,
    ) -> Result<crate::schema::SharedPreparedJsonArray<BinaryError>, MemoryError> {
        Ok(self.builder.take().ok_or(MemoryError::Closed)?.publish())
    }
}
impl Drop for BankBuilder {
    fn drop(&mut self) {
        if let Some(mut builder) = self.builder.take() {
            builder.clear();
            let remaining = if let Some(slot) = &self.bank.slot {
                let mut guard = slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match guard.as_mut() {
                    Some(list) => list.push(builder).err(),
                    None => Some(builder),
                }
            } else {
                Some(builder)
            };
            drop(remaining);
        }
    }
}
