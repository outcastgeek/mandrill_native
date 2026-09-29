//! Transactional ordered chunks.

use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::sync::Arc;

/// An ordered collection updated atomically as part of a transaction.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
pub struct TxChunk<T: TxValue> {
    cell: TxRef<Vec<Arc<T>>>,
}

impl<T: TxValue> Clone for TxChunk<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue> TxChunk<T> {
    /// Construct a `TxChunk` from its required inputs and explicit policies.
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
        domain.reference(Vec::new()).map(|cell| Self { cell })
    }

    /// Add an item to the transactional chunk in the current journal.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`Arc<T>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Transaction<(), E>`.
    pub fn append<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<(), E> {
        self.cell.update(move |chunk| {
            let mut next = chunk.clone();
            next.push(Arc::clone(&value));
            Ok((next, ()))
        })
    }

    #[must_use]
    /// Remove the last transactional chunk item in the current journal.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Transaction<Option<Arc<T>>, E>`.
    pub fn pop<E: Send + 'static>(&self) -> Transaction<Option<Arc<T>>, E> {
        self.cell.update(|chunk| {
            let mut next = chunk.clone();
            let value = next.pop();
            Ok((next, value))
        })
    }

    #[must_use]
    /// Read a consistent state and position for later observation or replay.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Transaction<Arc<Vec<Arc<T>>>, E>`.
    pub fn snapshot<E: Send + 'static>(&self) -> Transaction<Arc<Vec<Arc<T>>>, E> {
        self.cell.read()
    }
}
