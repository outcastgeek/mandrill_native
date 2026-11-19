//! Startup-owned native extension loan controls.
use crate::host::NativeSlot;
use effect::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use std::cell::UnsafeCell;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

struct Payload<E: Send + 'static, R: Send + Sync + 'static> {
    loans: Mutex<usize>,
    slot: UnsafeCell<Option<NativeSlot<E, R>>>,
}
// SAFETY: slot installation and removal occur only with the loan gate held.
// A frozen shared reference exists only through a live loan, so removal cannot
// overlap a borrow. Arc::get_mut synchronizes final loan retirement with reuse.
unsafe impl<E: Send + 'static, R: Send + Sync + 'static> Sync for Payload<E, R> where
    NativeSlot<E, R>: Send + Sync
{
}

struct Control<E: Send + 'static, R: Send + Sync + 'static> {
    payload: SharedSlot<'static, Payload<E, R>>,
}
type Controls<E, R> = Mutex<FixedVector<'static, Arc<Control<E, R>>>>;

pub(crate) struct NativeSlotRegistry<E: Send + 'static, R: Send + Sync + 'static> {
    controls: SharedSlot<'static, Controls<E, R>>,
}
pub(crate) struct NativeSlotRef<E: Send + 'static, R: Send + Sync + 'static> {
    control: Arc<Control<E, R>>,
}
pub(crate) struct NativeSlotVacancy<E: Send + 'static, R: Send + Sync + 'static> {
    control: Arc<Control<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> NativeSlotRegistry<E, R> {
    /// Region payload only; `count` standard Arc controls are startup System allocations.
    pub(crate) fn storage_bytes(count: usize) -> Result<Bytes, MemoryError> {
        let entries = std::alloc::Layout::array::<Arc<Control<E, R>>>(count)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Payload<E, R>>::storage_bytes(count)?
            .checked_add(SlotPool::<Controls<E, R>>::storage_bytes(1)?)?
            .checked_add(Bytes::new(entries.size()))
    }
    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        count: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(count)?;
        let payloads = memory.slots(region, count, |_| {
            Ok(Payload {
                loans: Mutex::new(0),
                slot: UnsafeCell::new(None),
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
            controls: registry.checkout()?.share(),
        })
    }
    pub(crate) fn reserve(&self) -> Result<NativeSlotVacancy<E, R>, MemoryError> {
        let mut controls = crate::try_admission_lock(&self.controls)?;
        for control in controls.as_mut_slice() {
            if Arc::get_mut(control).is_none() {
                continue;
            }
            return Ok(NativeSlotVacancy {
                control: Arc::clone(control),
            });
        }
        Err(MemoryError::NoSlot)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> NativeSlotVacancy<E, R> {
    pub(crate) fn publish(self, slot: NativeSlot<E, R>) -> NativeSlotRef<E, R> {
        let mut loans = self
            .control
            .payload
            .loans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: the affine vacancy owns the only external Arc and no loans exist.
        unsafe {
            *self.control.payload.slot.get() = Some(slot);
        }
        *loans = 1;
        drop(loans);
        NativeSlotRef {
            control: self.control,
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for NativeSlotRef<E, R> {
    fn clone(&self) -> Self {
        let control = Arc::clone(&self.control);
        {
            let mut loans = control
                .payload
                .loans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Every loan owns an Arc; Arc's own count limit is reached first.
            *loans += 1;
        }
        Self { control }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Deref for NativeSlotRef<E, R> {
    type Target = NativeSlot<E, R>;
    #[allow(clippy::expect_used)] // Every live loan retains an initialized frozen slot.
    fn deref(&self) -> &Self::Target {
        // SAFETY: this loan prevents removal until its borrow has ended.
        unsafe {
            (&*self.control.payload.slot.get())
                .as_ref()
                .expect("live native slot loan")
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for NativeSlotRef<E, R> {
    fn drop(&mut self) {
        let retired = {
            let mut loans = self
                .control
                .payload
                .loans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *loans -= 1;
            if *loans == 0 {
                // SAFETY: no other loan can borrow the slot after the last decrement.
                unsafe { (&mut *self.control.payload.slot.get()).take() }
            } else {
                None
            }
        };
        // This control Arc remains live throughout payload destruction.
        drop(retired);
    }
}
