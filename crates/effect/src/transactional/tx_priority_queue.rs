use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

pub struct TxPriorityQueue<K: TxValue + Ord, V: TxValue> {
    cell: TxRef<BTreeMap<Arc<K>, VecDeque<Arc<V>>>>,
}

impl<K: TxValue + Ord, V: TxValue> Clone for TxPriorityQueue<K, V> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<K: TxValue + Ord, V: TxValue> TxPriorityQueue<K, V> {
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(BTreeMap::new()).map(|cell| Self { cell })
    }

    pub fn offer<E: Send + 'static>(&self, priority: Arc<K>, value: Arc<V>) -> Transaction<(), E> {
        self.cell.update(move |queue| {
            let mut next = queue.clone();
            next.entry(Arc::clone(&priority))
                .or_default()
                .push_back(Arc::clone(&value));
            Ok((next, ()))
        })
    }

    #[must_use]
    pub fn take<E: Send + 'static>(&self) -> Transaction<(Arc<K>, Arc<V>), E> {
        let cell = self.cell.clone();
        self.cell.read().flat_map(move |queue| {
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
            cell.write(Arc::new(next)).flat_map(move |()| {
                let priority = Arc::clone(&priority);
                let value = Arc::clone(&value);
                Transaction::succeed_with(move || (Arc::clone(&priority), Arc::clone(&value)))
            })
        })
    }
}
