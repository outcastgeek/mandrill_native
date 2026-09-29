use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Arc;

pub struct TxHashSet<T: TxValue + Eq + Hash> {
    cell: TxRef<HashSet<Arc<T>>>,
}

impl<T: TxValue + Eq + Hash> Clone for TxHashSet<T> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue + Eq + Hash> TxHashSet<T> {
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(HashSet::new()).map(|cell| Self { cell })
    }

    pub fn contains<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<bool, E> {
        self.cell
            .read()
            .map(move |set| set.contains(value.as_ref()))
    }

    pub fn insert<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<bool, E> {
        self.cell.update(move |set| {
            let mut next = set.clone();
            let inserted = next.insert(Arc::clone(&value));
            Ok((next, inserted))
        })
    }

    pub fn remove<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<bool, E> {
        self.cell.update(move |set| {
            let mut next = set.clone();
            let removed = next.remove(value.as_ref());
            Ok((next, removed))
        })
    }
}
