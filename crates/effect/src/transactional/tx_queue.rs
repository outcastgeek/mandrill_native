//! Transactional FIFO enqueue and dequeue.

use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::collections::VecDeque;
use std::sync::Arc;

/// A queue whose offer and take operations participate in one atomic transaction.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
pub struct TxQueue<T: TxValue> {
    cell: TxRef<VecDeque<Arc<T>>>,
}

impl<T: TxValue> Clone for TxQueue<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue> TxQueue<T> {
    /// Construct a `TxQueue` from its required inputs and explicit policies.
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
        domain.reference(VecDeque::new()).map(|cell| Self { cell })
    }

    /// Submit a value for delivery under the queue’s identity and capacity rules.
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
    pub fn offer<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<(), E> {
        self.cell.update(move |queue| {
            let mut next = queue.clone();
            next.push_back(Arc::clone(&value));
            Ok((next, ()))
        })
    }

    #[must_use]
    /// Transfer the owned value out when the current state permits it.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Transaction<Arc<T>, E>`.
    pub fn take<E: Send + 'static>(&self) -> Transaction<Arc<T>, E> {
        let cell = self.cell.clone();
        self.cell.read().flat_map(move |queue| {
            let mut next = queue.as_ref().clone();
            match next.pop_front() {
                Some(value) => cell.write(Arc::new(next)).flat_map(move |()| {
                    let value = Arc::clone(&value);
                    Transaction::succeed_with(move || Arc::clone(&value))
                }),
                None => Transaction::retry(),
            }
        })
    }

    #[must_use]
    /// Return the number of values currently retained.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `TxValue`.
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `Transaction<usize, E>`.
    pub fn len<E: Send + 'static>(&self) -> Transaction<usize, E> {
        self.cell.read().map(|queue| queue.len())
    }
}
