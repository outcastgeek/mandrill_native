use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::sync::Arc;

pub struct TxChunk<T: TxValue> {
    cell: TxRef<Vec<Arc<T>>>,
}

impl<T: TxValue> Clone for TxChunk<T> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue> TxChunk<T> {
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(Vec::new()).map(|cell| Self { cell })
    }

    pub fn append<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<(), E> {
        self.cell.update(move |chunk| {
            let mut next = chunk.clone();
            next.push(Arc::clone(&value));
            Ok((next, ()))
        })
    }

    #[must_use]
    pub fn pop<E: Send + 'static>(&self) -> Transaction<Option<Arc<T>>, E> {
        self.cell.update(|chunk| {
            let mut next = chunk.clone();
            let value = next.pop();
            Ok((next, value))
        })
    }

    #[must_use]
    pub fn snapshot<E: Send + 'static>(&self) -> Transaction<Arc<Vec<Arc<T>>>, E> {
        self.cell.read()
    }
}
