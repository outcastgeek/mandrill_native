//! Transactional keyed values.

use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use crate::transaction;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

/// Persistent immutable map values; updates clone only map structure and Arc
/// handles, never K/V payloads. All operations use the domain's one journal.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `TxValue + Eq + Hash`.
/// - `V`: Value associated with a key. Bound or type: `TxValue`.
pub struct TxHashMap<K: TxValue + Eq + Hash, V: TxValue> {
    cell: TxRef<HashMap<Arc<K>, Arc<V>>>,
}

impl<K: TxValue + Eq + Hash, V: TxValue> Clone for TxHashMap<K, V> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<K: TxValue + Eq + Hash, V: TxValue> TxHashMap<K, V> {
    /// Construct a `TxHashMap` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `TxValue + Eq + Hash`.
    /// - `V`: Value associated with a key. Bound: `TxValue`.
    ///
    /// # Parameters
    /// - `domain` (`&TransactionDomain`): Stable identity or provider of the operation’s transaction domain.
    ///
    /// # Returns
    /// Type: `Result<Self, TransactionDomainError>`.
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(HashMap::new()).map(|cell| Self { cell })
    }

    /// Read the current value through the declared storage or state capability.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `K`: Type supplied by the enclosing impl or trait.
    /// - `V`: Stored value type.
    ///
    /// # Parameters
    /// - `key` (`Arc<K>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Transaction<Option<Arc<V>>, E>`.
    pub fn get<E: Send + 'static>(&self, key: Arc<K>) -> Transaction<Option<Arc<V>>, E> {
        {
            let __dsl_transaction = self.cell.read();
            transaction! {
            let map = __dsl_transaction.clone()?; map.get(key.as_ref()).cloned()
            }
        }
    }

    /// Insert the selected value under the collection’s identity rules.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `K`: Type supplied by the enclosing impl or trait.
    /// - `V`: Stored value type.
    ///
    /// # Parameters
    /// - `key` (`Arc<K>`): Stable key used to locate or identify the selected value.
    /// - `value` (`Arc<V>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Transaction<Option<Arc<V>>, E>`.
    pub fn insert<E: Send + 'static>(
        &self,
        key: Arc<K>,
        value: Arc<V>,
    ) -> Transaction<Option<Arc<V>>, E> {
        transaction! {
        modify; reference: self.cell, modify: move |map| {
                    let mut next = map.clone();
                    let previous = next.insert(Arc::clone(&key), Arc::clone(&value));
                    Ok((next, previous))
                },
        }
    }

    /// Remove the selected entry through this collection or store contract.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `K`: Type supplied by the enclosing impl or trait.
    /// - `V`: Stored value type.
    ///
    /// # Parameters
    /// - `key` (`Arc<K>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Transaction<Option<Arc<V>>, E>`.
    pub fn remove<E: Send + 'static>(&self, key: Arc<K>) -> Transaction<Option<Arc<V>>, E> {
        transaction! {
        modify; reference: self.cell, modify: move |map| {
                    let mut next = map.clone();
                    let previous = next.remove(key.as_ref());
                    Ok((next, previous))
                },
        }
    }

    #[must_use]
    /// Return the number of values currently retained.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `TxValue + Eq + Hash`.
    /// - `V`: Value associated with a key. Bound: `TxValue`.
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `Transaction<usize, E>`.
    pub fn len<E: Send + 'static>(&self) -> Transaction<usize, E> {
        {
            let __dsl_transaction = self.cell.read();
            transaction! {
            let map = __dsl_transaction.clone()?; map.len()
            }
        }
    }
}
