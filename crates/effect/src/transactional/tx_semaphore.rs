//! Transactional permit accounting.

use super::{
    HasTransactionDomain, Transaction, TransactionDomain, TransactionDomainError, TransactionError,
    TransactionPolicy, TxRef,
};
use crate::{Effect, Release, ResourceValue, Scope};
use std::num::NonZeroUsize;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A permit or lifecycle failure in a transactional semaphore.
pub enum TxSemaphoreError {
    /// A bounded counter or arithmetic operation overflowed.
    Overflow,
}

/// A permit count read and updated atomically with other transaction state.
pub struct TxSemaphore {
    cell: TxRef<usize>,
}

impl Clone for TxSemaphore {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl TxSemaphore {
    /// Construct a `TxSemaphore` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `domain` (`&TransactionDomain`): Stable identity or provider of the operation’s transaction domain.
    /// - `permits` (`usize`): The permits supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, TransactionDomainError>`.
    pub fn new(domain: &TransactionDomain, permits: usize) -> Result<Self, TransactionDomainError> {
        domain.reference(permits).map(|cell| Self { cell })
    }

    #[must_use]
    /// Report the capacity currently available for new work.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `Transaction<usize, E>`.
    pub fn available<E: Send + 'static>(&self) -> Transaction<usize, E> {
        self.cell.read().map(|value| *value)
    }

    #[must_use]
    /// Acquire a resource and register the matching release obligation.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    ///
    /// # Parameters
    /// - `permits` (`NonZeroUsize`): The permits supplied to this operation.
    ///
    /// # Returns
    /// Type: `Transaction<(), E>`.
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
    /// End this claim or lease and return its settlement status.
    ///
    /// # Parameters
    /// - `permits` (`NonZeroUsize`): The permits supplied to this operation.
    ///
    /// # Returns
    /// Type: `Transaction<(), TxSemaphoreError>`.
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
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasTransactionDomain + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `scope` (`&Scope<TransactionError<TxSemaphoreError>>`): Owner that joins child work and releases resources at close.
    /// - `permits` (`NonZeroUsize`): The permits supplied to this operation.
    /// - `policy` (`TransactionPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Effect<ResourceValue<()>, TransactionError<TxSemaphoreError>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
