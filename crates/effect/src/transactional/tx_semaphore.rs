use super::{
    HasTransactionDomain, Transaction, TransactionDomain, TransactionDomainError, TransactionError,
    TransactionPolicy, TxRef,
};
use crate::{Effect, Release, ResourceValue, Scope};
use std::num::NonZeroUsize;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxSemaphoreError {
    Overflow,
}

pub struct TxSemaphore {
    cell: TxRef<usize>,
}

impl Clone for TxSemaphore {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl TxSemaphore {
    pub fn new(domain: &TransactionDomain, permits: usize) -> Result<Self, TransactionDomainError> {
        domain.reference(permits).map(|cell| Self { cell })
    }

    #[must_use]
    pub fn available<E: Send + 'static>(&self) -> Transaction<usize, E> {
        self.cell.read().map(|value| *value)
    }

    #[must_use]
    pub fn acquire<E: Send + 'static>(&self, permits: NonZeroUsize) -> Transaction<(), E> {
        let cell = self.cell.clone();
        self.cell.read().flat_map(move |available| {
            if *available < permits.get() {
                Transaction::retry()
            } else {
                cell.write(Arc::new(*available - permits.get()))
            }
        })
    }

    #[must_use]
    pub fn release(&self, permits: NonZeroUsize) -> Transaction<(), TxSemaphoreError> {
        self.cell.update(move |available| {
            available
                .checked_add(permits.get())
                .map(|next| (next, ()))
                .ok_or(TxSemaphoreError::Overflow)
        })
    }

    /// Scope owns the permit from acquisition through final release,
    /// including a cancellation racing successful transaction commit.
    pub fn acquire_scoped<R>(
        &self,
        scope: &Scope<TransactionError<TxSemaphoreError>>,
        permits: NonZeroUsize,
        policy: TransactionPolicy,
    ) -> Effect<ResourceValue<()>, TransactionError<TxSemaphoreError>, R>
    where
        R: HasTransactionDomain + Send + Sync + 'static,
    {
        let semaphore = self.clone();
        scope.acquire(
            self.acquire::<TxSemaphoreError>(permits).atomically(policy),
            move |()| Release::from_effect(semaphore.release(permits).atomically(policy)),
        )
    }
}
