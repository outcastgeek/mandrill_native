//! Transactional priority ordering and dequeue.

use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use crate::transaction;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

/// A priority queue whose insertion and removal participate in a transaction.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `TxValue + Ord`.
/// - `V`: Value associated with a key. Bound or type: `TxValue`.
pub struct TxPriorityQueue<K: TxValue + Ord, V: TxValue> {
    cell: TxRef<BTreeMap<Arc<K>, VecDeque<Arc<V>>>>,
}

impl<K: TxValue + Ord, V: TxValue> Clone for TxPriorityQueue<K, V> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<K: TxValue + Ord, V: TxValue> TxPriorityQueue<K, V> {
    /// Construct a `TxPriorityQueue` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `TxValue + Ord`.
    /// - `V`: Value associated with a key. Bound: `TxValue`.
    ///
    /// # Parameters
    /// - `domain` (`&TransactionDomain`): Stable identity or provider of the operation’s transaction domain.
    ///
    /// # Returns
    /// Type: `Result<Self, TransactionDomainError>`.
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(BTreeMap::new()).map(|cell| Self { cell })
    }

    /// Submit a value for delivery under the queue’s identity and capacity rules.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `K`: Type supplied by the enclosing impl or trait.
    /// - `V`: Stored value type.
    ///
    /// # Parameters
    /// - `priority` (`Arc<K>`): The priority supplied to this operation.
    /// - `value` (`Arc<V>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Transaction<(), E>`.
    pub fn offer<E: Send + 'static>(&self, priority: Arc<K>, value: Arc<V>) -> Transaction<(), E> {
        transaction! {
        modify; reference: self.cell, modify: move |queue| {
                    let mut next = queue.clone();
                    next.entry(Arc::clone(&priority))
                        .or_default()
                        .push_back(Arc::clone(&value));
                    Ok((next, ()))
                },
        }
    }

    #[must_use]
    /// Transfer the owned value out when the current state permits it.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `K`: Type supplied by the enclosing impl or trait.
    /// - `V`: Stored value type.
    ///
    /// # Returns
    /// Type: `Transaction<(Arc<K>, Arc<V>), E>`.
    pub fn take<E: Send + 'static>(&self) -> Transaction<(Arc<K>, Arc<V>), E> {
        let cell = self.cell.clone();
        {
            let __dsl_transaction = self.cell.read();
            transaction! {
            let queue = __dsl_transaction.clone()?; (|| {
                        let mut next = queue.as_ref().clone();
                        let Some(priority) = next.keys().next().cloned() else {
                            return Transaction::retry();
                        };
                        let values = next.get_mut(&priority);
                        let Some(value) = values.and_then(VecDeque::pop_front) else {
                            return Transaction::retry();
                        };
                        if next.get(&priority).is_some_and(VecDeque::is_empty) {
                            next.remove(&priority);
                        }
                        { let __dsl_transaction = cell.write(Arc::new(next)); transaction! {
            let () = __dsl_transaction.clone()?; {
                            let priority = Arc::clone(&priority);
                            let value = Arc::clone(&value);
                            Transaction::succeed_with(move || (Arc::clone(&priority), Arc::clone(&value)))
                        }?
            } }
                    })()?
            }
        }
    }
}
