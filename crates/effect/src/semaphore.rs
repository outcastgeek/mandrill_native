use crate::internal::notification::notify_all;
use crate::internal::runtime::{self, Step};
use crate::{Cancellation, Defect, Effect, Exit, Release, ResourceValue, Scope};
use event_listener::Event;
use futures_util::{FutureExt, future::select};
use std::collections::BTreeMap;
use std::num::{NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemaphoreError {
    InvalidInitialPermits,
    CapacityExceeded,
    RequestExceedsCapacity,
    WaiterIdExhausted,
    PermitAccountingViolation,
    PartitionKeyConsumed,
    Closed,
}

impl std::fmt::Display for SemaphoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for SemaphoreError {}

struct State {
    available: usize,
    issued: usize,
    waiters: BTreeMap<u64, NonZeroUsize>,
    next_id: Option<u64>,
    closed: bool,
}

struct Inner {
    state: Mutex<State>,
    changed: Event,
    capacity: NonZeroUsize,
}

/// Runtime-neutral counting semaphore. Requests acquire in registration order;
/// a large front request can intentionally hold later smaller requests. No
/// executor, polling interval or admission batch is created by this type.
#[derive(Clone)]
pub struct Semaphore {
    inner: Arc<Inner>,
}

/// Exclusive ownership of a selected number of permits. Dropping this guard
/// returns them once; no application value or error needs Clone.
pub struct SemaphorePermit {
    inner: Option<Arc<Inner>>,
    count: NonZeroUsize,
}

impl Semaphore {
    #[must_use]
    pub fn full(capacity: NonZeroUsize) -> Self {
        Self::with_available(capacity, capacity.get())
    }

    fn with_available(capacity: NonZeroUsize, available: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    available,
                    issued: available,
                    waiters: BTreeMap::new(),
                    next_id: Some(u64::default()),
                    closed: false,
                }),
                changed: Event::new(),
                capacity,
            }),
        }
    }

    pub fn new(capacity: NonZeroUsize, initially_available: usize) -> Result<Self, SemaphoreError> {
        if initially_available > capacity.get() {
            return Err(SemaphoreError::InvalidInitialPermits);
        }
        Ok(Self::with_available(capacity, initially_available))
    }

    pub fn make<R: Send + Sync + 'static>(
        capacity: NonZeroUsize,
        initially_available: usize,
    ) -> Effect<Self, SemaphoreError, R> {
        Effect::try_sync(
            move || Self::new(capacity, initially_available),
            |error| error,
        )
    }

    #[must_use]
    pub fn capacity(&self) -> NonZeroUsize {
        self.inner.capacity
    }

    pub(crate) fn is_idle_indexed(&self) -> bool {
        if Arc::strong_count(&self.inner) != NonZeroUsize::MIN.get() {
            return false;
        }
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.available == self.inner.capacity.get() && state.waiters.is_empty()
    }

    pub fn available<R: Send + Sync + 'static>(&self) -> Effect<usize, SemaphoreError, R> {
        let semaphore = self.clone();
        Effect::sync(move || {
            semaphore
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .available
        })
    }

    /// Mint permits only within the caller-declared capacity. This can open a
    /// semaphore constructed with fewer initially available permits.
    pub fn add_permits<R: Send + Sync + 'static>(
        &self,
        count: NonZeroUsize,
    ) -> Effect<(), SemaphoreError, R> {
        let semaphore = self.clone();
        Effect::ready(move |_, context| {
            let mut state = semaphore
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if context.cancelled() {
                return Exit::Interrupted;
            }
            if state.closed {
                return Exit::Fail(SemaphoreError::Closed);
            }
            let Some(issued) = state.issued.checked_add(count.get()) else {
                return Exit::Fail(SemaphoreError::CapacityExceeded);
            };
            if issued > semaphore.inner.capacity.get() {
                return Exit::Fail(SemaphoreError::CapacityExceeded);
            }
            state.issued = issued;
            state.available += count.get();
            drop(state);
            notify_all(&semaphore.inner.changed);
            Exit::Success(())
        })
    }

    pub fn try_acquire<R: Send + Sync + 'static>(
        &self,
        count: NonZeroUsize,
    ) -> Effect<Option<SemaphorePermit>, SemaphoreError, R> {
        let semaphore = self.clone();
        Effect::ready(move |_, context| {
            if count > semaphore.inner.capacity {
                return Exit::Fail(SemaphoreError::RequestExceedsCapacity);
            }
            let mut state = semaphore
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if context.cancelled() {
                return Exit::Interrupted;
            }
            if state.closed {
                return Exit::Fail(SemaphoreError::Closed);
            }
            if !state.waiters.is_empty() || state.available < count.get() {
                return Exit::Success(None);
            }
            state.available -= count.get();
            Exit::Success(Some(SemaphorePermit {
                inner: Some(Arc::clone(&semaphore.inner)),
                count,
            }))
        })
    }

    pub fn acquire<R: Send + Sync + 'static>(
        &self,
        count: NonZeroUsize,
    ) -> Effect<SemaphorePermit, SemaphoreError, R> {
        let semaphore = self.clone();
        Effect::new(move |_, context, finish| {
            let semaphore = semaphore.clone();
            Step::Suspended(Box::pin(async move {
                let outcome =
                    AssertUnwindSafe(semaphore.acquire_owned(count, context.cancellation))
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }

    pub fn acquire_scoped<R: Send + Sync + 'static>(
        &self,
        scope: &Scope<SemaphoreError>,
        count: NonZeroUsize,
    ) -> Effect<ResourceValue<SemaphorePermit>, SemaphoreError, R> {
        scope.acquire(self.acquire(count), |mut permit| {
            Release::try_sync(move || permit.release_now().map(|_| ()))
        })
    }

    /// Closing rejects pending/new acquisition, but already acquired guards
    /// retain ownership and can release normally. The first close wins.
    pub fn close<R: Send + Sync + 'static>(&self) -> Effect<bool, SemaphoreError, R> {
        let semaphore = self.clone();
        Effect::sync(move || semaphore.close_now())
    }

    pub(crate) fn close_now(&self) -> bool {
        let first = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                false
            } else {
                state.closed = true;
                state.waiters.clear();
                true
            }
        };
        if first {
            notify_all(&self.inner.changed);
        }
        first
    }

    async fn acquire_owned(
        &self,
        count: NonZeroUsize,
        cancellation: Cancellation,
    ) -> Exit<SemaphorePermit, SemaphoreError> {
        if count > self.inner.capacity {
            return Exit::Fail(SemaphoreError::RequestExceedsCapacity);
        }
        let id = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if cancellation.is_requested() {
                return Exit::Interrupted;
            }
            if state.closed {
                return Exit::Fail(SemaphoreError::Closed);
            }
            if state.waiters.is_empty() && state.available >= count.get() {
                state.available -= count.get();
                return Exit::Success(SemaphorePermit {
                    inner: Some(Arc::clone(&self.inner)),
                    count,
                });
            }
            let Some(id) = state.next_id else {
                return Exit::Fail(SemaphoreError::WaiterIdExhausted);
            };
            state.next_id = id.checked_add(NonZeroU64::MIN.get());
            state.waiters.insert(id, count);
            id
        };
        let mut registration = WaitRegistration {
            inner: Arc::clone(&self.inner),
            id: Some(id),
        };
        loop {
            let changed = self.inner.changed.listen();
            let result = {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if cancellation.is_requested() {
                    state.waiters.remove(&id);
                    Some(Exit::Interrupted)
                } else if state.closed {
                    state.waiters.remove(&id);
                    Some(Exit::Fail(SemaphoreError::Closed))
                } else if state
                    .waiters
                    .first_key_value()
                    .is_some_and(|(first, _)| *first == id)
                    && state.available >= count.get()
                {
                    state.waiters.remove(&id);
                    state.available -= count.get();
                    Some(Exit::Success(SemaphorePermit {
                        inner: Some(Arc::clone(&self.inner)),
                        count,
                    }))
                } else {
                    None
                }
            };
            if let Some(result) = result {
                registration.id = None;
                notify_all(&self.inner.changed);
                return result;
            }
            let changed = std::pin::pin!(changed);
            let cancelled = std::pin::pin!(cancellation.cancelled());
            let _ = select(changed, cancelled).await;
        }
    }
}

struct WaitRegistration {
    inner: Arc<Inner>,
    id: Option<u64>,
}

impl Drop for WaitRegistration {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let removed = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .waiters
                .remove(&id)
                .is_some();
            if removed {
                let _ = catch_unwind(AssertUnwindSafe(|| notify_all(&self.inner.changed)));
            }
        }
    }
}

impl SemaphorePermit {
    pub fn release_now(&mut self) -> Result<bool, SemaphoreError> {
        let Some(inner) = self.inner.take() else {
            return Ok(false);
        };
        {
            let mut state = inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(available) = state.available.checked_add(self.count.get()) else {
                self.inner = Some(Arc::clone(&inner));
                return Err(SemaphoreError::PermitAccountingViolation);
            };
            if available > state.issued {
                self.inner = Some(Arc::clone(&inner));
                return Err(SemaphoreError::PermitAccountingViolation);
            }
            state.available = available;
        }
        notify_all(&inner.changed);
        Ok(true)
    }
}

impl Drop for SemaphorePermit {
    fn drop(&mut self) {
        let _ = catch_unwind(AssertUnwindSafe(|| self.release_now()));
    }
}
