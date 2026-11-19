//! Versioned transactional references.

use super::TxValue;
use super::internal::journal::{DomainInner, JournalError, TxJournal};
use super::internal::value::ErasedValue;
use super::{Transaction, TransactionError, TxStep};
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A capacity or identity failure in the process-local transaction domain.
pub enum TransactionDomainError {
    /// The configured capacity cannot admit this operation.
    Capacity,
    /// No further identity can be allocated without exceeding its range.
    IdentityExhausted,
    /// The selected prepared storage could not admit the operation.
    Memory(MemoryError),
}

/// Physical limits for a prepared STM domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedTransactionCapacity {
    /// Live transactional cells.
    pub cells: NonZeroUsize,
    /// Distinct read observations per journal, including retry branch unions.
    pub reads: NonZeroUsize,
    /// Distinct staged writes per journal.
    pub writes: NonZeroUsize,
    /// Simultaneous journal pairs, including nested branches and preadmitted
    /// scoped releases.
    pub journals: NonZeroUsize,
    /// Concurrent retry observations and their committed-change registrations.
    pub retry_waiters: NonZeroUsize,
}

/// Process-local synchronization domain. There is no database, async runtime
/// or global singleton; R supplies the domain used by `atomically`.
pub struct TransactionDomain {
    pub(crate) inner: Arc<DomainInner>,
    capacity: NonZeroUsize,
}

impl TransactionDomain {
    /// Regional record and listener storage, excluding region headers/alignment
    /// overhead and caller-owned cell values.
    pub fn storage_bytes(capacity: PreparedTransactionCapacity) -> Result<Bytes, MemoryError> {
        DomainInner::storage_bytes(capacity)
    }

    /// Provision the index, journal tables and retry controls at startup. A
    /// transaction executed through this domain uses the same atomic commit
    /// path and reports a typed memory refusal before publishing any write.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedTransactionCapacity,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            inner: Arc::new(DomainInner::prepare(memory, region, capacity)?),
            capacity: capacity.cells,
        })
    }

    #[must_use]
    /// Construct a `TransactionDomain` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `capacity` (`NonZeroUsize`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            inner: Arc::new(DomainInner::default()),
            capacity,
        }
    }

    /// Borrow the transactional reference underlying this handle.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bounds: `TxValue`.
    ///
    /// # Parameters
    /// - `initial` (`T`): Initial value used before any state transition.
    ///
    /// # Returns
    /// Type: `Result<TxRef<T>, TransactionDomainError>`.
    pub fn reference<T: TxValue>(&self, initial: T) -> Result<TxRef<T>, TransactionDomainError> {
        Ok(TxRef {
            handle: self.create_handle(ErasedValue::ordinary(Arc::new(initial)))?,
            marker: std::marker::PhantomData,
        })
    }

    fn create_handle(
        &self,
        value: ErasedValue,
    ) -> Result<Arc<ReferenceHandle>, TransactionDomainError> {
        let id = self.inner.create_cell(self.capacity.get(), value)?;
        Ok(Arc::new(ReferenceHandle {
            domain: Arc::clone(&self.inner),
            id,
        }))
    }

    pub(super) fn create_prepared_handle(
        &self,
        value: ErasedValue,
    ) -> Result<Arc<ReferenceHandle>, TransactionDomainError> {
        if !self.inner.is_prepared() {
            return Err(TransactionDomainError::Memory(
                MemoryError::UnsupportedRunContract,
            ));
        }
        self.create_handle(value)
    }
}

/// Selects the atomic transaction domain in an application environment.
pub trait HasTransactionDomain {
    /// # Returns
    /// Type: `Arc<TransactionDomain>`.
    fn transaction_domain(&self) -> Arc<TransactionDomain>;
}

/// Immutable values are shared by Arc. A transaction replaces a whole value;
/// it cannot mutate a cell through an aliased interior-mutable reference.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `TxValue`.
pub struct TxRef<T: TxValue> {
    handle: Arc<ReferenceHandle>,
    marker: std::marker::PhantomData<fn() -> T>,
}

/// Typed reference handle used by transactional tx ref.
pub(crate) struct ReferenceHandle {
    pub domain: Arc<DomainInner>,
    pub id: NonZeroU64,
}

impl Drop for ReferenceHandle {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        self.domain.remove_cell(self.id);
    }
}

impl<T: TxValue> Clone for TxRef<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            handle: Arc::clone(&self.handle),
            marker: std::marker::PhantomData,
        }
    }
}

impl<T: TxValue> TxRef<T> {
    #[must_use]
    /// Return the stable identity associated with this value.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `TxValue`.
    ///
    /// # Returns
    /// Type: `NonZeroU64`.
    pub fn id(&self) -> NonZeroU64 {
        self.handle.id
    }

    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Result<(u64, Arc<T>), JournalError>`.
    pub(crate) fn inspect_committed(&self) -> Result<(u64, Arc<T>), JournalError> {
        self.handle.domain.inspect(self.handle.id)
    }

    /// # Type parameters
    /// - `T`: Value or item type. Bound: `TxValue`.
    ///
    /// # Returns
    /// Type: `&event_listener::Event`.
    pub(crate) fn changed(&self) -> &event_listener::Event {
        &self.handle.domain.changed
    }

    #[must_use]
    /// Read the selected state without assuming it has been modified.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Transaction<Arc<T>, E>`.
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

    /// Record the supplied value through the selected state or transport capability.
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
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `update` (`impl Fn(&T) -> Result<(T, B), E> + Send + Sync + 'static`): The update supplied to this operation.
    ///
    /// # Returns
    /// Type: `Transaction<B, E>`.
    pub fn update<B: Send + 'static, E: Send + 'static>(
        &self,
        update: impl Fn(&T) -> Result<(T, B), E> + Send + Sync + 'static,
    ) -> Transaction<B, E> {
        let reference = self.clone();
        Transaction::new(move |journal: &mut TxJournal| {
            let current = match journal.read_for_write::<T>(&reference.handle) {
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
