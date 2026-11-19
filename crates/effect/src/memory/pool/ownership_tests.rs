//! Exercise identity fencing that has no public mutation seam.
#![cfg(test)]

use super::*;
use crate::memory::{MemoryPlan, Partition};
use std::cell::Cell;

const TEST: RegionId = RegionId::named("slot-validation");

fn memory() -> Result<PreparedMemory<'static>, MemoryError> {
    let bytes = Bytes::new(4096);
    let partitions = [Partition {
        id: TEST,
        capacity: bytes,
    }];
    MemoryPlan::new(bytes, &partitions)?
        .metadata(TEST)?
        .prepare()
}

#[test]
fn generation_exhaustion_retires_instead_of_reviving_weak_aliases() -> Result<(), MemoryError> {
    let memory = memory()?;
    let pool = memory.slots(TEST, 1, |_| Ok(17_u8))?;
    let initial = pool.checkout()?.share();
    let stale = initial.downgrade()?;
    let identity = initial.id();
    let mut exclusive = initial.into_last().ok_or(MemoryError::NoSlot)?;
    assert!(stale.upgrade().is_none());
    *exclusive = 29;
    let shared = exclusive.share();
    assert_eq!(*shared, 29);
    assert!(stale.upgrade().is_none());
    drop(shared);

    let newer = pool.checkout()?.share();
    assert_eq!(newer.id().index, identity.index);
    assert_ne!(newer.id().generation, identity.generation);
    assert!(stale.upgrade().is_none());
    drop(newer);

    {
        let mut gate = pool.owner.lock();
        // SAFETY: identity came from this one-slot pool. All strong value
        // leases have returned and this guard uniquely serializes metadata.
        unsafe { gate.record(identity.index) }.generation = usize::MAX;
    }
    assert!(matches!(pool.try_checkout(), Err(MemoryError::NoSlot)));
    assert_eq!(pool.retired(), 1);
    assert_eq!(pool.available()?, 0);
    assert!(stale.upgrade().is_none());
    Ok(())
}

struct OnDrop<F: Fn()> {
    callback: F,
}
impl<F: Fn()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        (self.callback)();
    }
}

#[test]
fn failed_slot_factory_drops_initialized_payload_outside_memory_gate() -> Result<(), MemoryError> {
    let memory = memory()?;
    let drops = Cell::new(0);
    let before = memory.usage(TEST)?.active_backing;
    let failed = memory.slots(TEST, 2, |index| {
        if index == 1 {
            return Err(MemoryError::NoSlot);
        }
        Ok(OnDrop {
            callback: || {
                let nested = memory.allocate(TEST, Layout::new::<u8>());
                assert!(nested.is_ok());
                drops.set(drops.get() + 1);
            },
        })
    });
    assert!(matches!(failed, Err(MemoryError::NoSlot)));
    assert_eq!(drops.get(), 1);
    assert_eq!(memory.usage(TEST)?.active_backing, before);
    Ok(())
}
