use super::TxValue;
use super::internal::journal::{Cell, DomainInner, JournalError, TxJournal};
use super::{Transaction, TransactionError, TxStep};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionDomainError {
    Capacity,
    IdentityExhausted,
}

/// Process-local synchronization domain. There is no database, async runtime
/// or global singleton; R supplies the domain used by `atomically`.
pub struct TransactionDomain {
    pub(crate) inner: Arc<DomainInner>,
    capacity: NonZeroUsize,
}

impl TransactionDomain {
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            inner: Arc::new(DomainInner::default()),
            capacity,
        }
    }

    pub fn reference<T: TxValue>(&self, initial: T) -> Result<TxRef<T>, TransactionDomainError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.cells.len() >= self.capacity.get() {
            return Err(TransactionDomainError::Capacity);
        }
        let raw = state
            .next_id
            .checked_add(1)
            .ok_or(TransactionDomainError::IdentityExhausted)?;
        let id = NonZeroU64::new(raw).ok_or(TransactionDomainError::IdentityExhausted)?;
        state.next_id = raw;
        state.cells.insert(
            id,
            Cell {
                version: 0,
                value: Arc::new(initial),
            },
        );
        Ok(TxRef {
            handle: Arc::new(ReferenceHandle {
                domain: Arc::clone(&self.inner),
                id,
            }),
            marker: std::marker::PhantomData,
        })
    }
}

pub trait HasTransactionDomain {
    fn transaction_domain(&self) -> Arc<TransactionDomain>;
}

/// Immutable values are shared by Arc. A transaction replaces a whole value;
/// it cannot mutate a cell through an aliased interior-mutable reference.
pub struct TxRef<T: TxValue> {
    handle: Arc<ReferenceHandle>,
    marker: std::marker::PhantomData<fn() -> T>,
}

pub(crate) struct ReferenceHandle {
    pub domain: Arc<DomainInner>,
    pub id: NonZeroU64,
}

impl Drop for ReferenceHandle {
    fn drop(&mut self) {
        let removed = self
            .domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cells
            .remove(&self.id);
        // A value's destructor can reenter the domain; run it outside the gate.
        drop(removed);
    }
}

impl<T: TxValue> Clone for TxRef<T> {
    fn clone(&self) -> Self {
        Self {
            handle: Arc::clone(&self.handle),
            marker: std::marker::PhantomData,
        }
    }
}

impl<T: TxValue> TxRef<T> {
    #[must_use]
    pub fn id(&self) -> NonZeroU64 {
        self.handle.id
    }

    pub(crate) fn inspect_committed(&self) -> Result<(u64, Arc<T>), JournalError> {
        let state = self
            .handle
            .domain
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cell = state
            .cells
            .get(&self.handle.id)
            .ok_or(JournalError::MissingCell)?;
        let value = Arc::downcast::<T>(Arc::clone(&cell.value))
            .map_err(|_| JournalError::WrongValueType)?;
        Ok((cell.version, value))
    }

    pub(crate) fn changed(&self) -> &event_listener::Event {
        &self.handle.domain.changed
    }

    #[must_use]
    pub fn read<E: Send + 'static>(&self) -> Transaction<Arc<T>, E> {
        let reference = self.clone();
        Transaction::new(move |journal: &mut TxJournal| {
            match journal.read::<T>(&reference.handle) {
                Ok(value) => TxStep::Success(value),
                Err(JournalError::Conflict) => TxStep::Conflict,
                Err(error) => TxStep::Invalid(TransactionError::from_journal(error)),
            }
        })
    }

    pub fn write<E: Send + 'static>(&self, value: Arc<T>) -> Transaction<(), E> {
        let reference = self.clone();
        Transaction::new(move |journal: &mut TxJournal| {
            match journal.write::<T>(&reference.handle, Arc::clone(&value)) {
                Ok(()) => TxStep::Success(()),
                Err(JournalError::Conflict) => TxStep::Conflict,
                Err(error) => TxStep::Invalid(TransactionError::from_journal(error)),
            }
        })
    }

    /// Replace a value from a borrowed immutable snapshot and return an
    /// operation result. Only the returned replacement is journaled; a typed
    /// failure leaves the cell untouched.
    pub fn update<B: Send + 'static, E: Send + 'static>(
        &self,
        update: impl Fn(&T) -> Result<(T, B), E> + Send + Sync + 'static,
    ) -> Transaction<B, E> {
        let reference = self.clone();
        Transaction::new(move |journal: &mut TxJournal| {
            let current = match journal.read::<T>(&reference.handle) {
                Ok(value) => value,
                Err(JournalError::Conflict) => return TxStep::Conflict,
                Err(error) => return TxStep::Invalid(TransactionError::from_journal(error)),
            };
            let (next, output) = match update(&current) {
                Ok(value) => value,
                Err(error) => return TxStep::Failure(error),
            };
            match journal.write::<T>(&reference.handle, Arc::new(next)) {
                Ok(()) => TxStep::Success(output),
                Err(JournalError::Conflict) => TxStep::Conflict,
                Err(error) => TxStep::Invalid(TransactionError::from_journal(error)),
            }
        })
    }
}
