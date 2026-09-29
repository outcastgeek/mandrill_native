//! Transactional one-shot completion and waiting.

use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::sync::Arc;

/// A one-shot value completed and observed within transactional state.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
pub struct TxDeferred<T: TxValue> {
    cell: TxRef<Option<Arc<T>>>,
}

impl<T: TxValue> Clone for TxDeferred<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue> TxDeferred<T> {
    /// Construct a `TxDeferred` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `TxValue`.
    ///
    /// # Parameters
    /// - `domain` (`&TransactionDomain`): Stable identity or provider of the operation’s transaction domain.
    ///
    /// # Returns
    /// Type: `Result<Self, TransactionDomainError>`.
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(None).map(|cell| Self { cell })
    }

    /// Record a terminal result once and publish it to current observers.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`Arc<T>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Transaction<bool, E>`.
    pub fn complete<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<bool, E> {
        let cell = self.cell.clone();
        self.cell
            .read()
            .flat_map(move |current| match current.as_ref() {
                Some(_) => Transaction::succeed_with(|| false),
                None => cell
                    .write(Arc::new(Some(Arc::clone(&value))))
                    .map(|()| true),
            })
    }

    #[must_use]
    /// Wait transactionally until this deferred value has been completed.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Transaction<Arc<T>, E>`.
    pub fn await_value<E: Send + 'static>(&self) -> Transaction<Arc<T>, E> {
        self.cell.read().flat_map(|value| match value.as_ref() {
            Some(value) => {
                let value = Arc::clone(value);
                Transaction::succeed_with(move || Arc::clone(&value))
            }
            None => Transaction::retry(),
        })
    }
}
