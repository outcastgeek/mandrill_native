//! A region-backed mutable reference with ordinary atomic callback semantics.
//! Callbacks invoked under its value gate must not reenter the same reference.
//! These synchronous helpers leave panic conversion to their prepared caller.
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool};
use std::sync::Mutex;

/// A single prepared cell; handles share its actual backing without a new Arc.
pub struct PreparedAtomicRef<T: Send + 'static> {
    slot: Option<SharedSlot<'static, Mutex<Option<T>>>>,
}
impl<T: Send + 'static> PreparedAtomicRef<T> {
    /// Inline cell and pool metadata; allocator overhead and T-owned pointees are additional.
    pub fn storage_bytes() -> Result<Bytes, MemoryError> {
        SlotPool::<Mutex<Option<T>>>::storage_bytes(1)
    }
    /// Provision the complete empty cell before consuming the initial value.
    #[allow(clippy::result_large_err)]
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        initial: T,
    ) -> Result<Self, (MemoryError, T)> {
        let pool = match memory.slots(region, 1, |_| Ok(Mutex::new(None))) {
            Ok(pool) => pool,
            Err(error) => return Err((error, initial)),
        };
        let lease = match pool.checkout() {
            Ok(lease) => lease,
            Err(error) => return Err((error, initial)),
        };
        *lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(initial);
        Ok(Self {
            slot: Some(lease.share()),
        })
    }
    /// Fallibly retain the same cell.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slot: Some(self.slot.as_ref().ok_or(MemoryError::Closed)?.try_clone()?),
        })
    }
    #[allow(clippy::expect_used)] // Each live handle owns the initialized cell.
    fn value(&self) -> &Mutex<Option<T>> {
        self.slot
            .as_deref()
            .expect("live prepared atomic reference")
    }
    /// Inspect under the value gate. The callback must not reenter this reference.
    #[allow(clippy::expect_used)]
    pub fn with_now<B: Send + 'static>(&self, inspect: impl FnOnce(&T) -> B) -> B {
        let value = self
            .value()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inspect(value.as_ref().expect("initialized atomic value"))
    }
    /// Explicitly clone a snapshot. Cloning T can itself allocate.
    pub fn get_now(&self) -> T
    where
        T: Clone,
    {
        self.with_now(Clone::clone)
    }
    /// Atomically compute a replacement; callback unwind preserves the installed
    /// value and identity, though interior mutations are ordinary Rust effects.
    /// The replaced value is destroyed after releasing the value gate.
    #[allow(clippy::expect_used)]
    pub fn modify_now<B: Send + 'static>(&self, update: impl FnOnce(&T) -> (B, T)) -> B {
        let (result, old) = {
            let mut value = self
                .value()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let (result, next) = update(value.as_ref().expect("initialized atomic value"));
            let old = value.replace(next);
            (result, old)
        };
        drop(old);
        result
    }
    /// Atomically replace using a callback under the gate.
    pub fn update_now(&self, update: impl FnOnce(&T) -> T) {
        self.modify_now(|value| ((), update(value)));
    }
    /// Mutate under the gate; unwind may leave partial mutations as in ordinary Rust.
    #[allow(clippy::expect_used)]
    pub fn modify_in_place_now<B: Send + 'static>(&self, update: impl FnOnce(&mut T) -> B) -> B {
        let mut value = self
            .value()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        update(value.as_mut().expect("initialized atomic value"))
    }
    /// Run the factory before locking, then replace and retire the old value outside the gate.
    pub fn set_with_now(&self, factory: impl FnOnce() -> T) {
        let next = factory();
        let old = self
            .value()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(next);
        drop(old);
    }
}
impl<T: Send + 'static> Drop for PreparedAtomicRef<T> {
    fn drop(&mut self) {
        if let Some(lease) = self.slot.take().and_then(SharedSlot::into_last) {
            let value = lease
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(value);
            drop(lease);
        }
    }
}
