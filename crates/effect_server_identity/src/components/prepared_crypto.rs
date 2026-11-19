//! Application-wide admission for the existing native password workers.

use crate::IdentityError;
use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use effect::{
    AdmissionError, PreparedAdmissionError, PreparedSemaphore, PreparedSemaphoreError,
    PreparedSemaphorePermit, PreparedSharedValuePool, SemaphoreError, SharedValue,
};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Regional capacity shared by selected native user-manager compositions.
/// Native cryptographic algorithms, their internal data, and Tokio blocking
/// workers remain with the supplied providers.
pub struct PreparedIdentityCrypto {
    workers: PreparedSemaphore,
    retained: PreparedSharedValuePool<PreparedSemaphorePermit>,
    maximum: NonZeroUsize,
}

impl PreparedIdentityCrypto {
    /// Actual regional semaphore and retained invocation records.
    pub fn storage_bytes(managers: NonZeroUsize) -> Result<Bytes, MemoryError> {
        PreparedSemaphore::storage_bytes(0)?.checked_add(PreparedSharedValuePool::<
            PreparedSemaphorePermit,
        >::storage_bytes(managers.get())?)
    }

    /// Provision the combined worker quota and affine invocation records.
    /// Share the same owner across definitions using this native worker budget.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        workers: NonZeroUsize,
        managers: NonZeroUsize,
    ) -> Result<Self, IdentityError> {
        Self::storage_bytes(managers).map_err(memory_error)?;
        Ok(Self {
            workers: PreparedSemaphore::prepare(memory, region, workers, workers.get(), 0)
                .map_err(semaphore_error)?,
            retained: PreparedSharedValuePool::prepare(memory, region, managers.get())
                .map_err(memory_error)?,
            maximum: workers,
        })
    }

    pub(super) fn validate(&self, workers: NonZeroUsize) -> Result<(), IdentityError> {
        if workers > self.maximum {
            return Err(IdentityError::Configuration("identity password work limit"));
        }
        Ok(())
    }

    pub(super) fn reserve(
        self: &Arc<Self>,
        workers: NonZeroUsize,
    ) -> Result<PreparedPasswordWork, IdentityError> {
        let reservation = self.retained.reserve().map_err(memory_error)?;
        let permit = self
            .workers
            .try_acquire_now(workers)
            .map_err(semaphore_error)?
            .ok_or(IdentityError::Admission(PreparedAdmissionError::Admission(
                AdmissionError::Full,
            )))?;
        Ok(PreparedPasswordWork {
            permit: reservation.publish(permit),
            owner: Arc::clone(self),
            workers,
        })
    }
}

/// Affine admission token transferred to one selected native manager.
/// The underlying work quota follows any retained native child alias.
pub struct PreparedPasswordWork {
    permit: SharedValue<PreparedSemaphorePermit>,
    owner: Arc<PreparedIdentityCrypto>,
    workers: NonZeroUsize,
}

impl PreparedPasswordWork {
    pub(super) fn admitted_hasher(
        self,
        owner: &Arc<PreparedIdentityCrypto>,
        workers: NonZeroUsize,
    ) -> Result<SharedValue<PreparedSemaphorePermit>, IdentityError> {
        if !Arc::ptr_eq(&self.owner, owner) || self.workers != workers {
            return Err(IdentityError::WrongConfiguration);
        }
        Ok(self.permit)
    }
}

fn memory_error(error: MemoryError) -> IdentityError {
    IdentityError::Admission(PreparedAdmissionError::Memory(error))
}

fn semaphore_error(error: PreparedSemaphoreError) -> IdentityError {
    match error {
        PreparedSemaphoreError::Memory(error) => memory_error(error),
        PreparedSemaphoreError::Semaphore(SemaphoreError::Closed) => {
            IdentityError::Admission(PreparedAdmissionError::Admission(AdmissionError::Closed))
        }
        PreparedSemaphoreError::Semaphore(
            SemaphoreError::RequestExceedsCapacity
            | SemaphoreError::InvalidInitialPermits
            | SemaphoreError::CapacityExceeded
            | SemaphoreError::WaiterIdExhausted
            | SemaphoreError::PermitAccountingViolation
            | SemaphoreError::PartitionKeyConsumed,
        ) => IdentityError::Configuration("identity password work limit"),
    }
}
