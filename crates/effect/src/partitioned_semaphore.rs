use crate::internal::runtime;
use crate::{
    CleanupFailure, Defect, Effect, Exit, Finalization, PrimaryOutcome, Semaphore, SemaphoreError,
    SemaphorePermit,
};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

struct State<K> {
    closed: bool,
    partitions: BTreeMap<K, Semaphore>,
}

struct Inner<K> {
    state: Mutex<State<K>>,
    capacity: NonZeroUsize,
}

/// Independent FIFO admission per typed key. Keys move into the index without
/// a Clone bound. Partitions remain indexed until explicit idle pruning or
/// close; no hidden TTL, eviction cadence or unbounded background task exists.
pub struct PartitionedSemaphore<K: Ord + Send + 'static> {
    inner: Arc<Inner<K>>,
}

impl<K: Ord + Send + 'static> Clone for PartitionedSemaphore<K> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<K: Ord + Send + 'static> PartitionedSemaphore<K> {
    #[must_use]
    pub fn new(capacity_per_partition: NonZeroUsize) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    closed: false,
                    partitions: BTreeMap::new(),
                }),
                capacity: capacity_per_partition,
            }),
        }
    }

    pub fn make<R: Send + Sync + 'static>(
        capacity_per_partition: NonZeroUsize,
    ) -> Effect<Self, SemaphoreError, R> {
        Effect::sync(move || Self::new(capacity_per_partition))
    }

    /// Move one key into the first started execution. A reusable recipe uses
    /// `acquire_with` to create a fresh owned key for each run.
    pub fn acquire<R: Send + Sync + 'static>(
        &self,
        key: K,
        count: NonZeroUsize,
    ) -> Effect<SemaphorePermit, SemaphoreError, R> {
        let key = Mutex::new(Some(key));
        self.acquire_candidate(
            move || {
                key.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .ok_or(SemaphoreError::PartitionKeyConsumed)
            },
            count,
        )
    }

    pub fn acquire_with<R: Send + Sync + 'static>(
        &self,
        key: impl Fn() -> K + Send + Sync + 'static,
        count: NonZeroUsize,
    ) -> Effect<SemaphorePermit, SemaphoreError, R> {
        self.acquire_candidate(move || Ok(key()), count)
    }

    fn acquire_candidate<R: Send + Sync + 'static>(
        &self,
        key: impl Fn() -> Result<K, SemaphoreError> + Send + Sync + 'static,
        count: NonZeroUsize,
    ) -> Effect<SemaphorePermit, SemaphoreError, R> {
        let partitions = self.clone();
        Effect::new(move |env, context, finish| {
            if count > partitions.inner.capacity {
                return runtime::finish(finish, Exit::Fail(SemaphoreError::RequestExceedsCapacity));
            }
            let key = match catch_unwind(AssertUnwindSafe(&key)) {
                Ok(Ok(key)) => key,
                Ok(Err(error)) => return runtime::finish(finish, Exit::Fail(error)),
                Err(panic) => return runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            };
            match catch_unwind(AssertUnwindSafe(|| partitions.partition(key))) {
                Ok(Ok(semaphore)) => semaphore.acquire(count).execute(env, context, finish),
                Ok(Err(error)) => runtime::finish(finish, Exit::Fail(error)),
                Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            }
        })
    }

    fn partition(&self, key: K) -> Result<Semaphore, SemaphoreError> {
        let (semaphore, unused_key) = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                (None, Some(key))
            } else if let Some(existing) = state.partitions.get(&key) {
                (Some(existing.clone()), Some(key))
            } else {
                let semaphore = Semaphore::full(self.inner.capacity);
                state.partitions.insert(key, semaphore.clone());
                (Some(semaphore), None)
            }
        };
        drop(unused_key);
        semaphore.ok_or(SemaphoreError::Closed)
    }

    /// Remove only partitions that have no outstanding permit, waiter or
    /// separately owned recipe. The caller controls when this scan is paid.
    pub fn prune_idle<R: Send + Sync + 'static>(&self) -> Effect<usize, SemaphoreError, R> {
        let partitions = self.clone();
        Effect::ready(move |_, _| {
            let removed = {
                let mut state = partitions
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut kept = BTreeMap::new();
                let mut removed = Vec::new();
                for (key, semaphore) in std::mem::take(&mut state.partitions) {
                    if semaphore.is_idle_indexed() {
                        removed.push((key, semaphore));
                    } else {
                        kept.insert(key, semaphore);
                    }
                }
                state.partitions = kept;
                removed
            };
            let count = removed.len();
            let mut defects = Vec::new();
            for entry in removed {
                if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(entry))) {
                    defects.push(Defect::from_panic(panic));
                }
            }
            cleanup_outcome(count, defects)
        })
    }

    pub fn close<R: Send + Sync + 'static>(&self) -> Effect<bool, SemaphoreError, R> {
        let partitions = self.clone();
        Effect::ready(move |_, _| {
            let active = {
                let mut state = partitions
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    return Exit::Success(false);
                }
                state.closed = true;
                std::mem::take(&mut state.partitions)
            };
            let mut defects = Vec::new();
            for (key, semaphore) in active {
                if let Err(panic) = catch_unwind(AssertUnwindSafe(|| semaphore.close_now())) {
                    defects.push(Defect::from_panic(panic));
                }
                if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(key))) {
                    defects.push(Defect::from_panic(panic));
                }
            }
            cleanup_outcome(true, defects)
        })
    }
}

fn cleanup_outcome<A>(value: A, defects: Vec<Defect>) -> Exit<A, SemaphoreError> {
    let mut failures = defects.into_iter().map(CleanupFailure::ReleaseDefect);
    match failures.next() {
        None => Exit::Success(value),
        Some(first) => {
            drop(value);
            Exit::FinalizationFailed(Finalization {
                primary: PrimaryOutcome::Succeeded,
                first,
                rest: failures.collect(),
            })
        }
    }
}
