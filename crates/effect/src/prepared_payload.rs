//! Physically prepared immutable payload ownership for bounded fanout.
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool};
use std::ops::Deref;
use std::sync::OnceLock;

struct Owner<T: Send + Sync + 'static> {
    slot: Option<SharedSlot<'static, OnceLock<T>>>,
}
impl<T: Send + Sync + 'static> Owner<T> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slot: Some(self.slot.as_ref().ok_or(MemoryError::Closed)?.try_clone()?),
        })
    }
    fn cell(&self) -> &OnceLock<T> {
        #[allow(clippy::expect_used)] // Every live public authority has one slot.
        self.slot.as_deref().expect("live payload authority")
    }
}
impl<T: Send + Sync + 'static> Drop for Owner<T> {
    fn drop(&mut self) {
        if let Some(mut lease) = self.slot.take().and_then(SharedSlot::into_last) {
            // Clear before user destruction, outside pool metadata, while the
            // unique lease prevents the physical slot from returning early.
            let value = lease.take();
            drop(value);
            drop(lease);
        }
    }
}
/// A caller-prepared finite pool of inline immutable payload cells.
pub struct PreparedPayloadPool<T: Send + Sync + 'static> {
    slots: SlotPool<'static, OnceLock<T>>,
}
/// The one affine writer for a reserved, not-yet-published cell.
pub struct PreparedPayloadReservation<T: Send + Sync + 'static> {
    owner: Owner<T>,
}
/// A preclaimed fanout owner that cannot read before publication.
pub struct PreparedPayloadLink<T: Send + Sync + 'static> {
    owner: Owner<T>,
}
/// An immutable view retaining the actual published payload and pool backing.
pub struct PreparedPayloadView<T: Send + Sync + 'static> {
    owner: Owner<T>,
}

impl<T: Send + Sync + 'static> PreparedPayloadPool<T> {
    /// Inline payload and pool metadata bytes; allocator headers, padding, and
    /// storage separately owned by `T` are additional.
    pub fn storage_bytes(capacity: usize) -> Result<Bytes, MemoryError> {
        SlotPool::<OnceLock<T>>::storage_bytes(capacity)
    }
    /// Initialize the caller-selected number of empty payload cells at startup.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            slots: memory.slots(region, capacity, |_| Ok(OnceLock::new()))?,
        })
    }
    /// Fallibly retain the existing pool.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slots: self.slots.try_clone()?,
        })
    }
    /// Acquire the physical cell before constructing its candidate.
    pub fn reserve(&self) -> Result<PreparedPayloadReservation<T>, MemoryError> {
        // Startup initializes empty cells; final-owner retirement clears each
        // cell before its physical slot can return to this pool.
        let lease = self.slots.checkout()?;
        Ok(PreparedPayloadReservation {
            owner: Owner {
                slot: Some(lease.share()),
            },
        })
    }
}
impl<T: Send + Sync + 'static> PreparedPayloadReservation<T> {
    /// Preclaim one actual fanout owner before candidate construction.
    pub fn try_link(&self) -> Result<PreparedPayloadLink<T>, MemoryError> {
        Ok(PreparedPayloadLink {
            owner: self.owner.try_clone()?,
        })
    }
    /// Publish exactly once. A failed invariant returns both authority and value.
    #[allow(clippy::result_large_err)]
    pub fn publish(self, value: T) -> Result<PreparedPayloadView<T>, (Self, T)> {
        match self.owner.cell().set(value) {
            Ok(()) => Ok(PreparedPayloadView { owner: self.owner }),
            Err(value) => Err((self, value)),
        }
    }
}
impl<T: Send + Sync + 'static> PreparedPayloadLink<T> {
    /// Preclaim another actual owner, without exposing the unpublished payload.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            owner: self.owner.try_clone()?,
        })
    }
    /// Convert only after the writer has published; failure returns the link.
    pub fn into_view(self) -> Result<PreparedPayloadView<T>, Self> {
        if self.owner.cell().get().is_some() {
            Ok(PreparedPayloadView { owner: self.owner })
        } else {
            Err(self)
        }
    }
}
impl<T: Send + Sync + 'static> PreparedPayloadView<T> {
    /// Fallibly retain this exact cell and its generation.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            owner: self.owner.try_clone()?,
        })
    }
    /// Compare actual pool and generation identity.
    #[must_use]
    pub fn same_payload(&self, other: &Self) -> bool {
        match (&self.owner.slot, &other.owner.slot) {
            (Some(left), Some(right)) => left.same_slot(right),
            _ => false,
        }
    }
}
impl<T: Send + Sync + 'static> Deref for PreparedPayloadView<T> {
    type Target = T;
    #[allow(clippy::expect_used)] // Only a published cell can construct a view; a live owner prevents clearing.
    fn deref(&self) -> &T {
        self.owner.cell().get().expect("published payload")
    }
}
