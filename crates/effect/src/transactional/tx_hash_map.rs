use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

/// Persistent immutable map values; updates clone only map structure and Arc
/// handles, never K/V payloads. All operations use the domain's one journal.
pub struct TxHashMap<K: TxValue + Eq + Hash, V: TxValue> {
    cell: TxRef<HashMap<Arc<K>, Arc<V>>>,
}

impl<K: TxValue + Eq + Hash, V: TxValue> Clone for TxHashMap<K, V> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<K: TxValue + Eq + Hash, V: TxValue> TxHashMap<K, V> {
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(HashMap::new()).map(|cell| Self { cell })
    }

    pub fn get<E: Send + 'static>(&self, key: Arc<K>) -> Transaction<Option<Arc<V>>, E> {
        self.cell
            .read()
            .map(move |map| map.get(key.as_ref()).cloned())
    }

    pub fn insert<E: Send + 'static>(
        &self,
        key: Arc<K>,
        value: Arc<V>,
    ) -> Transaction<Option<Arc<V>>, E> {
        self.cell.update(move |map| {
            let mut next = map.clone();
            let previous = next.insert(Arc::clone(&key), Arc::clone(&value));
            Ok((next, previous))
        })
    }

    pub fn remove<E: Send + 'static>(&self, key: Arc<K>) -> Transaction<Option<Arc<V>>, E> {
        self.cell.update(move |map| {
            let mut next = map.clone();
            let previous = next.remove(key.as_ref());
            Ok((next, previous))
        })
    }

    #[must_use]
    pub fn len<E: Send + 'static>(&self) -> Transaction<usize, E> {
        self.cell.read().map(|map| map.len())
    }
}
