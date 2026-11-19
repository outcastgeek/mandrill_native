//! Prepared per-key semaphore storage and bounded snapshot retirement.
use crate::PreparedPanicReservation;
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
};
use crate::{
    Exit, PreparedCleanupAccumulator, PreparedCleanupConfig, PreparedPanicPool, PreparedSemaphore,
    PreparedSemaphoreError, SemaphoreError,
};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

struct Entry<K: Ord + Send + 'static> {
    key: K,
    semaphore: SlotLease<'static, PreparedSemaphore>,
}
struct Index<K: Ord + Send + 'static> {
    closed: bool,
    entries: FixedVector<'static, Entry<K>>,
    // A single physically backed batch serializes snapshot retirement.
    _retirement: Option<Retirement<K>>,
}
struct Retirement<K: Ord + Send + 'static> {
    entries: FixedVector<'static, Entry<K>>,
    diagnostics: FixedVector<'static, PreparedPanicReservation>,
}
struct State<K: Ord + Send + 'static> {
    index: Mutex<Index<K>>,
    semaphores: SlotPool<'static, PreparedSemaphore>,
}
struct RetirementGuard<'a, K: Ord + Send + 'static> {
    state: &'a State<K>,
    batch: Option<Retirement<K>>,
    close: bool,
}
struct ContinueRetirement<'a, 'b, K: Ord + Send + 'static> {
    guard: &'a mut RetirementGuard<'b, K>,
    armed: bool,
}
impl<K: Ord + Send + 'static> Drop for ContinueRetirement<'_, '_, K> {
    fn drop(&mut self) {
        if self.armed {
            self.guard.drain_and_restore();
        }
    }
}
impl<K: Ord + Send + 'static> RetirementGuard<'_, K> {
    fn drain_and_restore(&mut self) {
        loop {
            let entry = self
                .batch
                .as_mut()
                .and_then(|batch| batch.entries.remove_first());
            let Some(entry) = entry else { break };
            let mut continuation = ContinueRetirement {
                guard: self,
                armed: true,
            };
            if continuation.guard.close {
                let Entry { key, semaphore } = entry;
                let _ = semaphore.close_now();
                drop(key);
                drop(semaphore);
            } else {
                drop(entry);
            }
            continuation.armed = false;
        }
        if let Some(mut batch) = self.batch.take() {
            while let Some(token) = batch.diagnostics.pop() {
                drop(token);
            }
            let mut index = self
                .state
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            index._retirement = Some(batch);
        }
    }
}
impl<K: Ord + Send + 'static> Drop for RetirementGuard<'_, K> {
    fn drop(&mut self) {
        self.drain_and_restore();
    }
}
/// Per-key semaphore index with caller-provisioned partition and waiter storage.
/// Prune and close require caller-provided diagnostic and cleanup capacity.
pub struct PreparedPartitionedSemaphore<K: Ord + Send + 'static> {
    state: SharedSlot<'static, State<K>>,
}
impl<K: Ord + Send + 'static> PreparedPartitionedSemaphore<K> {
    /// Payload/layout bytes excluding allocator headers and alignment padding.
    pub fn storage_bytes(
        partitions: usize,
        waiters_per_partition: usize,
    ) -> Result<Bytes, MemoryError> {
        let entries = std::alloc::Layout::array::<Entry<K>>(partitions)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let tokens = partitions
            .checked_mul(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let diagnostics = std::alloc::Layout::array::<PreparedPanicReservation>(tokens)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<State<K>>::storage_bytes(1)?
            .checked_add(SlotPool::<PreparedSemaphore>::storage_bytes(partitions)?)?
            .checked_add(Bytes::new(entries.size()).checked_mul(2)?)?
            .checked_add(Bytes::new(diagnostics.size()))?
            .checked_add(
                PreparedSemaphore::storage_bytes(waiters_per_partition)?.checked_mul(partitions)?,
            )
    }
    /// Prepare all partition semaphores and the index/retirement backing.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity_per_partition: NonZeroUsize,
        partitions: usize,
        waiters_per_partition: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(partitions, waiters_per_partition)?;
        let entries = FixedVector::prepare(memory.allocator(region)?, partitions)?;
        let retirement = Retirement {
            entries: FixedVector::prepare(memory.allocator(region)?, partitions)?,
            diagnostics: FixedVector::prepare(
                memory.allocator(region)?,
                partitions
                    .checked_mul(2)
                    .ok_or(MemoryError::ArithmeticOverflow)?,
            )?,
        };
        let semaphores = memory.slots(region, partitions, |_| {
            PreparedSemaphore::prepare(
                memory,
                region,
                capacity_per_partition,
                capacity_per_partition.get(),
                waiters_per_partition,
            )
            .map_err(|error| match error {
                PreparedSemaphoreError::Memory(error) => error,
                PreparedSemaphoreError::Semaphore(_) => MemoryError::UnsupportedRunContract,
            })
        })?;
        let mut entries = Some(entries);
        let mut retirement = Some(retirement);
        let mut semaphores = Some(semaphores);
        let pool = memory.slots(region, 1, |_| {
            Ok(State {
                index: Mutex::new(Index {
                    closed: false,
                    entries: entries.take().ok_or(MemoryError::NoSlot)?,
                    _retirement: retirement.take(),
                }),
                semaphores: semaphores.take().ok_or(MemoryError::NoSlot)?,
            })
        })?;
        Ok(Self {
            state: pool.checkout()?.share(),
        })
    }
    /// Fallibly retain the same prepared index.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            state: self.state.try_clone()?,
        })
    }
    fn retirement(
        &self,
        panic_pool: &PreparedPanicPool,
        cleanup: &PreparedCleanupConfig<SemaphoreError>,
        tokens_per_entry: usize,
    ) -> Result<
        (
            RetirementGuard<'_, K>,
            crate::CleanupFailures<SemaphoreError>,
        ),
        MemoryError,
    > {
        let batch = self
            .state
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            ._retirement
            .take()
            .ok_or(MemoryError::Busy)?;
        let mut guard = RetirementGuard {
            state: &self.state,
            batch: Some(batch),
            close: tokens_per_entry == 2,
        };
        let failures = cleanup.reserve_failures()?;
        let batch = guard
            .batch
            .as_mut()
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let count = batch
            .entries
            .capacity()
            .checked_mul(tokens_per_entry)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        for _ in 0..count {
            batch.diagnostics.push_reserved(panic_pool.try_reserve()?);
        }
        Ok((guard, failures))
    }
    /// Remove the current sorted snapshot of idle partitions. All failure
    /// storage is admitted before the index changes; Busy denotes another drain.
    #[allow(clippy::expect_used)] // Complete token count is reserved before detachment.
    pub fn prune_idle(
        &self,
        panic_pool: &PreparedPanicPool,
        cleanup: &PreparedCleanupConfig<SemaphoreError>,
    ) -> Result<Exit<usize, SemaphoreError>, MemoryError> {
        if self
            .state
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .is_empty()
        {
            return Ok(Exit::Success(0));
        }
        let (mut guard, failures) = self.retirement(panic_pool, cleanup, 1)?;
        let batch = guard
            .batch
            .as_mut()
            .ok_or(MemoryError::UnsupportedRunContract)?;
        {
            let mut index = self
                .state
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut position = 0;
            while position < index.entries.len() {
                if index
                    .entries
                    .as_slice()
                    .get(position)
                    .is_some_and(|entry| entry.semaphore.is_idle_indexed())
                {
                    for current in position + 1..index.entries.len() {
                        index.entries.as_mut_slice().swap(current - 1, current);
                    }
                    if let Some(entry) = index.entries.pop() {
                        batch.entries.push_reserved(entry);
                    }
                } else {
                    position += 1;
                }
            }
        }
        let mut result =
            PreparedCleanupAccumulator::from_reserved(Exit::Success(batch.entries.len()), failures);
        while let Some(entry) = batch.entries.remove_first() {
            let token = batch
                .diagnostics
                .pop()
                .expect("one preadmitted diagnostic per retired partition");
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(entry))) {
                result.append_cleanup(Exit::Die(token.capture(panic)));
            }
        }
        Ok(result.finish())
    }
    /// Close the index and retire its sorted snapshot with preadmitted cleanup.
    #[allow(clippy::expect_used)] // Two tokens per possible entry are reserved before detachment.
    pub fn close_now(
        &self,
        panic_pool: &PreparedPanicPool,
        cleanup: &PreparedCleanupConfig<SemaphoreError>,
    ) -> Result<Exit<bool, SemaphoreError>, MemoryError> {
        if self
            .state
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed
        {
            return Ok(Exit::Success(false));
        }
        let (mut guard, failures) = self.retirement(panic_pool, cleanup, 2)?;
        let batch = guard
            .batch
            .as_mut()
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let first = {
            let mut index = self
                .state
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if index.closed {
                false
            } else {
                index.closed = true;
                while let Some(entry) = index.entries.remove_first() {
                    batch.entries.push_reserved(entry);
                }
                true
            }
        };
        let mut result = PreparedCleanupAccumulator::from_reserved(Exit::Success(first), failures);
        while let Some(Entry { key, semaphore }) = batch.entries.remove_first() {
            let close = batch
                .diagnostics
                .pop()
                .expect("close diagnostic preadmitted");
            let key_drop = batch.diagnostics.pop().expect("key diagnostic preadmitted");
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| {
                let _ = semaphore.close_now();
            })) {
                result.append_cleanup(Exit::Die(close.capture(panic)));
            }
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(key))) {
                result.append_cleanup(Exit::Die(key_drop.capture(panic)));
            }
            drop(semaphore);
        }
        Ok(result.finish())
    }
    /// Find or admit a partition. On refusal the original key is returned.
    /// Key comparisons execute under the index gate, as in the ordinary index.
    pub fn partition(&self, key: K) -> Result<PreparedSemaphore, (MemoryError, K)> {
        let mut index = self
            .state
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if index.closed {
            return Err((MemoryError::Closed, key));
        }
        let position = match index
            .entries
            .as_slice()
            .binary_search_by(|entry| entry.key.cmp(&key))
        {
            Ok(position) => {
                let result = match index.entries.as_slice().get(position) {
                    Some(entry) => entry.semaphore.try_clone(),
                    None => Err(MemoryError::UnsupportedRunContract),
                };
                drop(index);
                return match result {
                    Ok(semaphore) => {
                        drop(key);
                        Ok(semaphore)
                    }
                    Err(error) => Err((error, key)),
                };
            }
            Err(position) => position,
        };
        if index.entries.len() == index.entries.capacity() {
            return Err((MemoryError::NoSlot, key));
        }
        let lease = match self.state.semaphores.checkout() {
            Ok(lease) => lease,
            Err(error) => return Err((error, key)),
        };
        if let Err(error) = lease.reset_idle_partition() {
            drop(index);
            drop(lease);
            return Err((error, key));
        }
        let semaphore = match lease.try_clone() {
            Ok(semaphore) => semaphore,
            Err(error) => {
                drop(index);
                drop(lease);
                return Err((error, key));
            }
        };
        index.entries.push_reserved(Entry {
            key,
            semaphore: lease,
        });
        let entries = index.entries.as_mut_slice();
        for current in (position + 1..entries.len()).rev() {
            entries.swap(current - 1, current);
        }
        drop(index);
        Ok(semaphore)
    }
}
