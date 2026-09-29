use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::collections::VecDeque;
use std::sync::Arc;

pub struct TxQueue<T: TxValue> {
    cell: TxRef<VecDeque<Arc<T>>>,
}

impl<T: TxValue> Clone for TxQueue<T> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue> TxQueue<T> {
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(VecDeque::new()).map(|cell| Self { cell })
    }

    pub fn offer<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<(), E> {
        self.cell.update(move |queue| {
            let mut next = queue.clone();
            next.push_back(Arc::clone(&value));
            Ok((next, ()))
        })
    }

    #[must_use]
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
    pub fn len<E: Send + 'static>(&self) -> Transaction<usize, E> {
        self.cell.read().map(|queue| queue.len())
    }
}
