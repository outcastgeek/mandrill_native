//! Typed regional payloads in the ordinary transaction domain and commit protocol.

use super::internal::journal::JournalError;
use super::internal::value::ErasedValue;
use super::tx_ref::ReferenceHandle;
use super::{
    Transaction, TransactionDomain, TransactionDomainError, TransactionError, TxStep, TxValue,
};
use crate::memory::{Bytes, FramePool, FrameReservation, MemoryError, PreparedMemory, RegionId};
use std::alloc::Layout;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::sync::Arc;

pub use super::internal::value::PreparedTxValue;

/// Startup pool of immutable transaction payload frames for one value type.
/// Size for committed values, pending replacements, and old snapshots still
/// retained by transactions or readers. Any pointees owned separately by `T`
/// retain their own allocation and lifetime contract.
pub struct PreparedTxValuePool<T: TxValue> {
    frames: FramePool<'static>,
    marker: PhantomData<fn() -> T>,
}

impl<T: TxValue> PreparedTxValuePool<T> {
    /// Actual payload and frame bookkeeping bytes. Region allocator overhead is additional.
    pub fn storage_bytes(values: NonZeroUsize) -> Result<Bytes, MemoryError> {
        FramePool::storage_bytes(values.get(), Layout::new::<T>())
    }

    /// Prepare all payload backing in the selected region before execution.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        values: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(values)?;
        let frames = memory.frames(region, values.get(), Layout::new::<T>())?;
        Ok(Self {
            frames,
            marker: PhantomData,
        })
    }

    /// Retain the same prepared pool.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            frames: self.frames.try_clone()?,
            marker: PhantomData,
        })
    }

    fn reserve(&self) -> Result<FrameReservation<'static, T>, MemoryError> {
        self.frames.reserve()
    }

    pub(crate) fn owns(&self, value: &PreparedTxValue<T>) -> bool {
        value.belongs_to(&self.frames)
    }

    /// Move an immutable value into a reserved frame, returning it on refusal.
    pub fn try_store(&self, value: T) -> Result<PreparedTxValue<T>, (MemoryError, T)> {
        match self.reserve() {
            Ok(reservation) => Ok(PreparedTxValue::publish(reservation, value)),
            Err(error) => Err((error, value)),
        }
    }
}

/// A transactional reference whose immutable payloads occupy prepared frames.
/// Reads and writes use the same domain and commit gate as ordinary `TxRef`.
pub struct PreparedTxRef<T: TxValue> {
    handle: Arc<ReferenceHandle>,
    values: PreparedTxValuePool<T>,
}

impl TransactionDomain {
    /// Create a reference at startup with an actual regional initial payload.
    /// Later journal evaluations reuse the selected pool and reference handle.
    pub fn reference_prepared<T: TxValue>(
        &self,
        initial: T,
        values: PreparedTxValuePool<T>,
    ) -> Result<PreparedTxRef<T>, TransactionDomainError> {
        let initial = values
            .try_store(initial)
            .map_err(|(error, _)| TransactionDomainError::Memory(error))?;
        let handle = self.create_prepared_handle(initial.into_erased())?;
        Ok(PreparedTxRef { handle, values })
    }
}

impl<T: TxValue> PreparedTxRef<T> {
    /// Retain the same reference and value pool without copying its value.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            handle: Arc::clone(&self.handle),
            values: self.values.try_clone()?,
        })
    }

    /// Build a reusable read description. Each attempt retains the exact value
    /// it observes using a checked frame alias.
    #[must_use]
    pub fn read<E: Send + 'static>(&self) -> Transaction<PreparedTxValue<T>, E> {
        let handle = Arc::clone(&self.handle);
        Transaction::new(move |journal| {
            classify(
                journal
                    .read_erased(&handle, false)
                    .and_then(ErasedValue::into_prepared),
            )
        })
    }

    /// Build a reusable write description from a prepared owned replacement.
    /// Each attempt stages an alias of the same immutable payload. The input
    /// keeps its originating pool even if it comes from a different region.
    /// Replacements constructed by `update` use this reference's value pool.
    #[must_use]
    pub fn write<E: Send + 'static>(&self, value: PreparedTxValue<T>) -> Transaction<(), E> {
        let handle = Arc::clone(&self.handle);
        Transaction::new(move |journal| {
            classify(
                value
                    .try_clone()
                    .map_err(JournalError::Memory)
                    .and_then(|value| journal.write_erased(&handle, value.into_erased())),
            )
        })
    }

    /// Build a reusable update description. Every attempt reserves the replacement
    /// frame and its read/write journal space before invoking `update`.
    pub fn update<B: Send + 'static, E: Send + 'static>(
        &self,
        update: impl Fn(&T) -> Result<(T, B), E> + Send + Sync + 'static,
    ) -> Result<Transaction<B, E>, MemoryError> {
        self.update_step(move |current| match update(current) {
            Ok(value) => TxStep::Success(value),
            Err(error) => TxStep::Failure(error),
        })
    }

    pub(crate) fn update_step<B: Send + 'static, E: Send + 'static>(
        &self,
        update: impl Fn(&T) -> TxStep<(T, B), E> + Send + Sync + 'static,
    ) -> Result<Transaction<B, E>, MemoryError> {
        let handle = Arc::clone(&self.handle);
        let values = self.values.try_clone()?;
        Ok(Transaction::new(move |journal| {
            let reserved = (|| {
                let replacement = values.reserve().map_err(JournalError::Memory)?;
                let current = journal.read_erased(&handle, true)?.into_prepared::<T>()?;
                Ok((replacement, current))
            })();
            let (replacement, current) = match reserved {
                Ok(reserved) => reserved,
                Err(error) => return classify(Err(error)),
            };
            match update(&current) {
                TxStep::Success((next, output)) => {
                    let value = PreparedTxValue::publish(replacement, next);
                    let result = journal.write_erased(&handle, value.into_erased());
                    classify(result.map(|()| output))
                }
                TxStep::Failure(error) => TxStep::Failure(error),
                TxStep::Retry => TxStep::Retry,
                TxStep::Conflict => TxStep::Conflict,
                TxStep::Invalid(error) => TxStep::Invalid(error),
            }
        }))
    }
}

fn classify<A, E>(result: Result<A, JournalError>) -> TxStep<A, E> {
    match result {
        Ok(value) => TxStep::Success(value),
        Err(JournalError::Conflict) => TxStep::Conflict,
        Err(error) => TxStep::Invalid(TransactionError::from_journal(error)),
    }
}
