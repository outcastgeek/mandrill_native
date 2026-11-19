use super::RequestResolverError;
use crate::memory::{
    Bytes, FixedVector, FramePool, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease,
    SlotPool,
};
use crate::{
    CleanupFailures, Exit, Finalization, PreparedCleanupConfig, PreparedCleanupPool,
    PreparedExitMapper, SharedValue,
};
use std::ops::Range;
use std::sync::{Mutex, MutexGuard};

pub(super) struct Position<Q, A, E: Send + 'static> {
    pub(super) request: Option<Q>,
    outcome: Option<Exit<A, E>>,
    completion: Option<CompletionStorage<A, E>>,
}

impl<Q, A, E: Send + 'static> Position<Q, A, E> {
    pub(super) fn new(completion: CompletionStorage<A, E>) -> Self {
        Self {
            request: None,
            outcome: None,
            completion: Some(completion),
        }
    }
}

type PayloadSlot<T> = SlotLease<'static, Option<Mutex<T>>>;
type PayloadPool<T> = SlotPool<'static, Option<Mutex<T>>>;
type Errors<E> = FixedVector<'static, PayloadSlot<E>>;

pub(super) type ErrorLists<E> = SlotPool<'static, Errors<E>>;

struct ReservedErrors<E: Send + 'static>(SlotLease<'static, Errors<E>>);

impl<E: Send + 'static> Drop for ReservedErrors<E> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

pub(super) struct CompletionPools<A, E: Send + 'static> {
    pub(super) values: PayloadPool<A>,
    pub(super) errors: PayloadPool<E>,
    pub(super) lists: ErrorLists<E>,
    pub(super) cleanup: PreparedCleanupConfig<RequestResolverError<E>>,
    pub(super) retained: usize,
}

impl<A, E: Send + 'static> CompletionPools<A, E> {
    pub(super) fn storage_bytes(
        values: usize,
        cleanup_copies: usize,
        retained: usize,
    ) -> Result<Bytes, MemoryError> {
        let per_outcome = retained
            .checked_add(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let error_slots = values
            .checked_mul(per_outcome)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let list = std::alloc::Layout::array::<PayloadSlot<E>>(per_outcome)
            .map_err(|_| MemoryError::InvalidLayout)?;
        PayloadPool::<A>::storage_bytes(values)?
            .checked_add(PayloadPool::<E>::storage_bytes(error_slots)?)?
            .checked_add(ErrorLists::<E>::storage_bytes(values)?)?
            .checked_add(Bytes::new(list.size()).checked_mul(values)?)?
            .checked_add(
                PreparedCleanupPool::<RequestResolverError<E>>::storage_bytes(
                    cleanup_copies,
                    retained,
                )?,
            )?
            .checked_add(FramePool::storage_bytes(
                cleanup_copies,
                PreparedCleanupPool::<RequestResolverError<E>>::frame_layout(),
            )?)
    }

    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        values: usize,
        cleanup_copies: usize,
        retained: usize,
    ) -> Result<Self, MemoryError> {
        let per_outcome = retained
            .checked_add(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let error_slots = values
            .checked_mul(per_outcome)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let values = memory.slots(region, values, |_| Ok(None))?;
        let lists = memory.slots(region, values.capacity(), |_| {
            FixedVector::prepare(memory.allocator(region)?, per_outcome)
        })?;
        let errors = memory.slots(region, error_slots, |_| Ok(None))?;
        let pool = PreparedCleanupPool::prepare(memory, region, cleanup_copies, retained)?;
        let frames = memory.frames(
            region,
            cleanup_copies,
            PreparedCleanupPool::<RequestResolverError<E>>::frame_layout(),
        )?;
        Ok(Self {
            values,
            errors,
            lists,
            cleanup: PreparedCleanupConfig::new(pool, frames),
            retained,
        })
    }

    pub(super) fn reserve(&self) -> Result<CompletionStorage<A, E>, MemoryError> {
        let mut errors = ReservedErrors(self.lists.try_checkout()?);
        for _ in 0..errors.0.capacity() {
            errors.0.push_reserved(self.errors.try_checkout()?);
        }
        Ok(CompletionStorage {
            value: self.values.try_checkout()?,
            errors,
            diagnostics: PreparedExitMapper::admit(&self.cleanup)?,
            retained: self.retained,
        })
    }

    pub(super) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            values: self.values.try_clone()?,
            errors: self.errors.try_clone()?,
            lists: self.lists.try_clone()?,
            cleanup: self.cleanup.try_clone()?,
            retained: self.retained,
        })
    }

    pub(super) fn reserve_diagnostics(&self) -> Result<Diagnostics<E>, MemoryError> {
        self.cleanup.reserve_failures()
    }
}

pub(super) type Diagnostics<E> = CleanupFailures<RequestResolverError<E>>;

pub(super) struct CompletionStorage<A, E: Send + 'static> {
    value: PayloadSlot<A>,
    errors: ReservedErrors<E>,
    diagnostics: PreparedExitMapper<RequestResolverError<E>>,
    retained: usize,
}

impl<A, E: Send + 'static> CompletionStorage<A, E> {
    pub(super) fn share<B>(
        self,
        outcome: Exit<B, E>,
        error: impl Fn(SharedValue<E>) -> RequestResolverError<E>,
    ) -> (Exit<B, RequestResolverError<E>>, PayloadSlot<A>) {
        let Self {
            value,
            mut errors,
            diagnostics,
            retained,
        } = self;
        #[allow(clippy::expect_used)]
        // Primary, first, and each retained secondary have preheld cells.
        let result = diagnostics.map_retained(outcome, retained, |reason| {
            let slot = errors
                .0
                .pop()
                .expect("admitted resolver expected-error cell");
            error(SharedValue::from_reserved(slot, reason))
        });
        (result, value)
    }

    pub(super) fn into_diagnostics(self) -> Diagnostics<E> {
        self.diagnostics.into_rest()
    }
}

type Positions<Q, A, E> = FixedVector<'static, Position<Q, A, E>>;
pub(super) type ValueTable<Q, A, E> = Mutex<Positions<Q, A, E>>;

pub(super) struct BatchValues<Q: 'static, A: 'static, E: Send + 'static>(
    Option<SharedSlot<'static, ValueTable<Q, A, E>>>,
);

impl<Q, A, E: Send + 'static> BatchValues<Q, A, E> {
    pub(super) fn new(values: SharedSlot<'static, ValueTable<Q, A, E>>) -> Self {
        Self(Some(values))
    }

    fn table(&self) -> Result<MutexGuard<'_, Positions<Q, A, E>>, MemoryError> {
        Ok(self
            .0
            .as_ref()
            .ok_or(MemoryError::MissingRuntimeStorage)?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    fn try_clone(&self) -> Result<Self, MemoryError> {
        self.0
            .as_ref()
            .ok_or(MemoryError::MissingRuntimeStorage)?
            .try_clone()
            .map(Self::new)
    }

    pub(super) fn view(
        &self,
        range: Range<usize>,
        first_copy: Diagnostics<E>,
    ) -> Result<PreparedValueBatch<Q, A, E>, MemoryError> {
        self.table()?
            .as_slice()
            .get(range.clone())
            .ok_or(MemoryError::InvalidRange)?;
        Ok(PreparedValueBatch {
            first_copy,
            expected: BatchIdentity {
                values: self.try_clone()?,
                range: range.clone(),
            },
            writer: PreparedRequestBatch {
                values: self.try_clone()?,
                range,
                read: 0,
                written: 0,
                failure: None,
            },
        })
    }
}

impl<Q, A, E: Send + 'static> Drop for BatchValues<Q, A, E> {
    fn drop(&mut self) {
        if let Some(shared) = self.0.take()
            && let Some(mut lease) = shared.into_last()
        {
            lease
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
        }
    }
}

pub(super) struct PreparedValueBatch<Q: 'static, A: 'static, E: Send + 'static> {
    pub(super) first_copy: Diagnostics<E>,
    pub(super) expected: BatchIdentity<Q, A, E>,
    pub(super) writer: PreparedRequestBatch<Q, A, E>,
}

pub(super) fn clone_with_diagnostics<A: Clone, E: Clone>(
    outcome: &Exit<A, E>,
    destination: &mut Option<CleanupFailures<E>>,
) -> Exit<A, E> {
    if let Exit::FinalizationFailed(source) = outcome
        && let Some(mut rest) = destination.take()
    {
        rest.extend(source.rest.iter().cloned());
        Exit::FinalizationFailed(Finalization {
            primary: source.primary.clone(),
            first: source.first.clone(),
            rest,
        })
    } else {
        outcome.clone()
    }
}

pub(super) struct BatchIdentity<Q: 'static, A: 'static, E: Send + 'static> {
    values: BatchValues<Q, A, E>,
    range: Range<usize>,
}

impl<Q, A, E: Send + 'static> BatchIdentity<Q, A, E> {
    pub(super) fn accepts(&self, batch: &PreparedRequestBatch<Q, A, E>) -> bool {
        self.range == batch.range
            && self
                .values
                .0
                .as_ref()
                .zip(batch.values.0.as_ref())
                .is_some_and(|(expected, actual)| expected.same_slot(actual))
    }

    pub(super) fn take_completion(&self, offset: usize) -> Option<CompletionStorage<A, E>> {
        let index = self.range.clone().nth(offset)?;
        self.values
            .table()
            .ok()?
            .as_mut_slice()
            .get_mut(index)?
            .completion
            .take()
    }

    pub(super) fn verify_completions(&self) -> bool {
        self.values.table().ok().is_some_and(|values| {
            values
                .as_slice()
                .get(self.range.clone())
                .is_some_and(|positions| {
                    positions
                        .iter()
                        .all(|position| position.completion.is_some())
                })
        })
    }
}

/// The owned inputs and fixed result writer for one admitted resolver group.
/// Return this batch from the loader after writing one outcome per input in order.
pub struct PreparedRequestBatch<Q: 'static, A: 'static, E: Send + 'static> {
    values: BatchValues<Q, A, E>,
    range: Range<usize>,
    read: usize,
    written: usize,
    failure: Option<MemoryError>,
}

impl<Q, A, E: Send + 'static> PreparedRequestBatch<Q, A, E> {
    /// The number of requested outcomes in this group.
    #[must_use]
    pub fn len(&self) -> usize {
        self.range.len()
    }

    /// Whether the batch has no requests.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }

    /// Take the next owned request in its original group order.
    pub fn next_request(&mut self) -> Option<Q> {
        let index = self.range.clone().nth(self.read)?;
        let request = self
            .values
            .table()
            .ok()?
            .as_mut_slice()
            .get_mut(index)?
            .request
            .take();
        self.read += 1;
        request
    }

    /// Append an outcome. A refused append prevents publication of the batch.
    pub fn push(&mut self, outcome: Exit<A, E>) -> Result<(), MemoryError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let Some(index) = self.range.clone().nth(self.written) else {
            let error = MemoryError::CapacityExceeded {
                resource: 0,
                requested: self.written.saturating_add(1),
                available: self.len(),
                control: true,
            };
            self.failure = Some(error);
            return Err(error);
        };
        let appended = (|| {
            let mut values = self.values.table()?;
            let position = values
                .as_mut_slice()
                .get_mut(index)
                .ok_or(MemoryError::InvalidRange)?;
            if position.outcome.is_some() {
                return Err(MemoryError::InvalidRange);
            }
            position.outcome = Some(outcome);
            Ok(())
        })();
        if let Err(error) = appended {
            self.failure = Some(error);
            return Err(error);
        }
        self.written += 1;
        Ok(())
    }

    pub(super) fn written(&self) -> Result<usize, MemoryError> {
        self.failure.map_or(Ok(self.written), Err)
    }

    pub(super) fn take_outcome(&mut self, offset: usize) -> Option<Exit<A, E>> {
        let index = self.range.clone().nth(offset)?;
        self.values
            .table()
            .ok()?
            .as_mut_slice()
            .get_mut(index)?
            .outcome
            .take()
    }

    pub(super) fn verify_outcomes(&self) -> bool {
        self.values.table().ok().is_some_and(|values| {
            values
                .as_slice()
                .get(self.range.clone())
                .is_some_and(|positions| {
                    positions
                        .iter()
                        .all(|position| position.outcome.is_some() && position.completion.is_some())
                })
        })
    }
}
