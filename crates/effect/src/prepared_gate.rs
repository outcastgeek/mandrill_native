//! A single-permit gate using the prepared FIFO semaphore.
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use crate::{PreparedSemaphore, PreparedSemaphoreAcquire, PreparedSemaphoreError};
use std::num::NonZeroUsize;

/// A bounded gate with physically prepared waiter storage.
/// An admitted entry returns a semaphore acquisition whose permit releases on
/// drop; its `with_context` adapter uses the graph's cancellation listener.
pub struct PreparedGate {
    semaphore: PreparedSemaphore,
}

impl PreparedGate {
    /// Payload and pool bytes; allocator headers and alignment padding are extra.
    pub fn storage_bytes(waiters: usize) -> Result<Bytes, MemoryError> {
        PreparedSemaphore::storage_bytes(waiters)
    }

    /// Prepare the gate and caller-selected concurrent waiter capacity.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        waiters: usize,
    ) -> Result<Self, PreparedSemaphoreError> {
        let capacity = NonZeroUsize::MIN;
        Ok(Self {
            semaphore: PreparedSemaphore::prepare(memory, region, capacity, 1, waiters)?,
        })
    }

    /// Retain the existing prepared gate without creating a new allocation.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            semaphore: self.semaphore.try_clone()?,
        })
    }

    /// Admit one FIFO entry before polling; dropping it cancels its reservation.
    pub fn enter(&self) -> Result<PreparedSemaphoreAcquire, PreparedSemaphoreError> {
        self.semaphore.acquire(NonZeroUsize::MIN)
    }

    /// Close new and queued entries and wake admitted waiters.
    #[must_use]
    pub fn close_now(&self) -> bool {
        self.semaphore.close_now()
    }
}
