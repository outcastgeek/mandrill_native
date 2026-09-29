use super::{Transaction, TransactionDomain, TransactionDomainError, TxRef, TxValue};
use std::sync::Arc;

pub struct TxDeferred<T: TxValue> {
    cell: TxRef<Option<Arc<T>>>,
}

impl<T: TxValue> Clone for TxDeferred<T> {
    fn clone(&self) -> Self {
        Self {
            cell: self.cell.clone(),
        }
    }
}

impl<T: TxValue> TxDeferred<T> {
    pub fn new(domain: &TransactionDomain) -> Result<Self, TransactionDomainError> {
        domain.reference(None).map(|cell| Self { cell })
    }

    pub fn complete<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<bool, E> {
        let cell = self.cell.clone();
        self.cell
            .read()
            .flat_map(move |current| match current.as_ref() {
                Some(_) => Transaction::succeed_with(|| false),
                None => cell
                    .write(Arc::new(Some(Arc::clone(&value))))
                    .map(|()| true),
            })
    }

    #[must_use]
    pub fn await_value<E: Send + 'static>(&self) -> Transaction<Arc<T>, E> {
        self.cell.read().flat_map(|value| match value.as_ref() {
            Some(value) => {
                let value = Arc::clone(value);
                Transaction::succeed_with(move || Arc::clone(&value))
            }
            None => Transaction::retry(),
        })
    }
}
