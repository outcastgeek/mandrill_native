//! Transactional set membership.

use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Arc;

/// A hash set whose membership changes commit with the surrounding transaction.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue + Eq + Hash`.
pub struct TxHashSet<T: TxValue + Eq + Hash> {
    cell: TxRef<HashSet<Arc<T>>>,
}

impl<T: TxValue + Eq + Hash> Clone for TxHashSet<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue + Eq + Hash> TxHashSet<T> {
    /// Construct a `TxHashSet` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `TxValue + Eq + Hash`.
    ///
    /// # Parameters
    /// - `domain` (`&TransactionDomain`): Stable identity or provider of the operation’s transaction domain.
    ///
    /// # Returns
    /// Type: `Result<Self, TransactionDomainError>`.
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(HashSet::new()).map(|cell| Self { cell })
    }

    /// Check whether the selected identity or value is present.
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
    pub fn contains<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<bool, E> {
        self.cell
            .read()
            .map(move |set| set.contains(value.as_ref()))
    }

    /// Insert the selected value under the collection’s identity rules.
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
    pub fn insert<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<bool, E> {
        self.cell.update(move |set| {
            let mut next = set.clone();
            let inserted = next.insert(Arc::clone(&value));
            Ok((next, inserted))
        })
    }

    /// Remove the selected entry through this collection or store contract.
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
    pub fn remove<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<bool, E> {
        self.cell.update(move |set| {
            let mut next = set.clone();
            let removed = next.remove(value.as_ref());
            Ok((next, removed))
        })
    }
}
