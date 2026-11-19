//! Weighted FIFO semaphore with explicitly prepared waiters.
use crate::internal::waiting::{Position, Waiting};
use crate::memory::{
    Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool, prepared_notification,
};
use crate::{Exit, PreparedCancellationObservation, SemaphoreError};
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

struct Waiter {
    id: u64,
    count: NonZeroUsize,
}
struct Value {
    available: usize,
    issued: usize,
    next_id: Option<u64>,
    closed: bool,
    waiters: Waiting<Waiter>,
}
struct State {
    value: Mutex<Value>,
    capacity: NonZeroUsize,
    changed: prepared_notification::Event,
}
/// Bounded synchronization admission failure, separate from semaphore semantics.
#[derive(Debug)]
pub enum PreparedSemaphoreError {
    Memory(MemoryError),
    Semaphore(SemaphoreError),
}
/// Weighted FIFO semaphore backed by prepared physical waiters.
pub struct PreparedSemaphore {
    state: SharedSlot<'static, State>,
}
/// A physically admitted acquisition; dropping it cancels and unlinks the waiter.
pub struct PreparedSemaphoreAcquire {
    listener: Option<prepared_notification::Registration>,
    state: Option<SharedSlot<'static, State>>,
    position: Option<Position>,
    id: u64,
    count: NonZeroUsize,
    granted: bool,
}
/// An acquired permit retaining its actual semaphore owner.
pub struct PreparedSemaphorePermit {
    state: Option<SharedSlot<'static, State>>,
    count: NonZeroUsize,
}
impl PreparedSemaphore {
    /// Layout payload bytes excluding region allocator headers and padding.
    pub fn storage_bytes(waiters: usize) -> Result<Bytes, MemoryError> {
        SlotPool::<State>::storage_bytes(1)?
            .checked_add(Waiting::<Waiter>::payload_bytes(waiters)?)?
            .checked_add(prepared_notification::Event::storage_bytes(waiters)?)
    }
    /// Prepare the selected permit capacity and number of concurrent waiters.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: NonZeroUsize,
        initially_available: usize,
        waiters: usize,
    ) -> Result<Self, PreparedSemaphoreError> {
        if initially_available > capacity.get() {
            return Err(PreparedSemaphoreError::Semaphore(
                SemaphoreError::InvalidInitialPermits,
            ));
        }
        let _ = Self::storage_bytes(waiters).map_err(PreparedSemaphoreError::Memory)?;
        let waiting =
            Waiting::prepare(memory, region, waiters).map_err(PreparedSemaphoreError::Memory)?;
        let changed = prepared_notification::Event::prepare(memory, region, waiters)
            .map_err(PreparedSemaphoreError::Memory)?;
        let mut waiting = Some(waiting);
        let mut changed = Some(changed);
        let states = memory
            .slots(region, 1, |_| {
                Ok(State {
                    value: Mutex::new(Value {
                        available: initially_available,
                        issued: initially_available,
                        next_id: Some(0),
                        closed: false,
                        waiters: waiting.take().ok_or(MemoryError::NoSlot)?,
                    }),
                    capacity,
                    changed: changed.take().ok_or(MemoryError::NoSlot)?,
                })
            })
            .map_err(PreparedSemaphoreError::Memory)?;
        Ok(Self {
            state: states
                .checkout()
                .map_err(PreparedSemaphoreError::Memory)?
                .share(),
        })
    }
    /// Retain the same prepared semaphore without allocating.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            state: self.state.try_clone()?,
        })
    }
    // The partition index holds the only inner owner while it decides whether
    // an idle entry can be retired and its outer lease reused.
    pub(crate) fn is_idle_indexed(&self) -> bool {
        if !self.state.is_unique() {
            return false;
        }
        let value = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        value.available == self.state.capacity.get() && value.waiters.is_empty()
    }
    pub(crate) fn reset_idle_partition(&self) -> Result<(), MemoryError> {
        if !self.is_idle_indexed() {
            return Err(MemoryError::Busy);
        }
        let mut value = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        value.closed = false;
        value.next_id = Some(0);
        value.issued = self.state.capacity.get();
        Ok(())
    }
    /// The configured total permit capacity.
    #[must_use]
    pub fn capacity(&self) -> NonZeroUsize {
        self.state.capacity
    }
    /// Currently available permits.
    #[must_use]
    pub fn available_now(&self) -> usize {
        self.state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .available
    }
    /// Add permits without exceeding the configured total capacity.
    pub fn add_permits_now(&self, count: NonZeroUsize) -> Result<(), SemaphoreError> {
        let mut state = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(SemaphoreError::Closed);
        }
        let issued = state
            .issued
            .checked_add(count.get())
            .filter(|n| *n <= self.state.capacity.get())
            .ok_or(SemaphoreError::CapacityExceeded)?;
        state.issued = issued;
        state.available += count.get();
        drop(state);
        self.state.changed.notify_all();
        Ok(())
    }
    /// Close admission and notify all queued waiters.
    #[must_use]
    pub fn close_now(&self) -> bool {
        let mut state = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let first = !state.closed;
        state.closed = true;
        while state.waiters.pop_front().is_some() {}
        drop(state);
        if first {
            self.state.changed.notify_all();
        }
        first
    }
    /// Acquire immediately only if no earlier waiter would be bypassed.
    pub fn try_acquire_now(
        &self,
        count: NonZeroUsize,
    ) -> Result<Option<PreparedSemaphorePermit>, PreparedSemaphoreError> {
        if count > self.state.capacity {
            return Err(PreparedSemaphoreError::Semaphore(
                SemaphoreError::RequestExceedsCapacity,
            ));
        }
        let owner = self
            .state
            .try_clone()
            .map_err(PreparedSemaphoreError::Memory)?;
        let mut state = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(PreparedSemaphoreError::Semaphore(SemaphoreError::Closed));
        }
        if !state.waiters.is_empty() || state.available < count.get() {
            return Ok(None);
        }
        state.available -= count.get();
        drop(state);
        Ok(Some(PreparedSemaphorePermit {
            state: Some(owner),
            count,
        }))
    }
    /// Admit an acquisition, retaining its permit owner and, when needed, waiter.
    pub fn acquire(
        &self,
        count: NonZeroUsize,
    ) -> Result<PreparedSemaphoreAcquire, PreparedSemaphoreError> {
        if count > self.state.capacity {
            return Err(PreparedSemaphoreError::Semaphore(
                SemaphoreError::RequestExceedsCapacity,
            ));
        }
        let owner = self
            .state
            .try_clone()
            .map_err(PreparedSemaphoreError::Memory)?;
        {
            let mut state = self
                .state
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                return Err(PreparedSemaphoreError::Semaphore(SemaphoreError::Closed));
            }
            if state.waiters.is_empty() && state.available >= count.get() {
                state.available -= count.get();
                return Ok(PreparedSemaphoreAcquire {
                    listener: None,
                    state: Some(owner),
                    position: None,
                    id: 0,
                    count,
                    granted: true,
                });
            }
        }
        let listener = self
            .state
            .changed
            .listen()
            .map_err(PreparedSemaphoreError::Memory)?;
        let mut state = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(PreparedSemaphoreError::Semaphore(SemaphoreError::Closed));
        }
        if state.waiters.is_empty() && state.available >= count.get() {
            state.available -= count.get();
            drop(state);
            drop(listener);
            return Ok(PreparedSemaphoreAcquire {
                listener: None,
                state: Some(owner),
                position: None,
                id: 0,
                count,
                granted: true,
            });
        }
        let id = state.next_id.ok_or(PreparedSemaphoreError::Semaphore(
            SemaphoreError::WaiterIdExhausted,
        ))?;
        let position = state
            .waiters
            .try_push_back(Waiter { id, count })
            .map_err(|(error, _)| PreparedSemaphoreError::Memory(error))?;
        state.next_id = id.checked_add(1);
        drop(state);
        Ok(PreparedSemaphoreAcquire {
            listener: Some(listener),
            state: Some(owner),
            position: Some(position),
            id,
            count,
            granted: false,
        })
    }
}
impl Future for PreparedSemaphoreAcquire {
    type Output = Result<PreparedSemaphorePermit, SemaphoreError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(listener) = &this.listener {
            listener.reset();
            let _ = listener.poll(cx);
        }
        let Some(owner) = this.state.as_ref() else {
            return Poll::Pending;
        };
        let mut state = owner
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed && !this.granted {
            if let Some(position) = this.position.take() {
                let _ = state
                    .waiters
                    .remove_where(position, |waiter| waiter.id == this.id);
            }
            return Poll::Ready(Err(SemaphoreError::Closed));
        }
        let mut granted_from_queue = false;
        if !this.granted
            && state
                .waiters
                .front()
                .is_some_and(|waiter| waiter.id == this.id && waiter.count.get() <= state.available)
        {
            let _ = state.waiters.pop_front();
            this.position = None;
            state.available -= this.count.get();
            this.granted = true;
            granted_from_queue = true;
        }
        if !this.granted {
            return Poll::Pending;
        }
        drop(state);
        let Some(owner) = this.state.take() else {
            return Poll::Pending;
        };
        let permit = PreparedSemaphorePermit {
            state: Some(owner),
            count: this.count,
        };
        let listener = this.listener.take();
        drop(listener);
        if granted_from_queue && let Some(owner) = &permit.state {
            owner.changed.notify_all();
        }
        Poll::Ready(Ok(permit))
    }
}
impl PreparedSemaphoreAcquire {
    /// Run an admitted acquisition using the prepared graph cancellation listener.
    pub async fn with_context(
        mut self,
        mut context: crate::__private::prepared_graph::FutureContext<SemaphoreError>,
    ) -> Exit<PreparedSemaphorePermit, SemaphoreError> {
        std::future::poll_fn(|cx| {
            if context.is_requested() {
                return Poll::Ready(Exit::Interrupted);
            }
            let observation = {
                let cancelled = std::pin::pin!(context.cancelled());
                Future::poll(cancelled, cx)
            };
            if matches!(
                observation,
                Poll::Ready(PreparedCancellationObservation::Requested)
            ) {
                return Poll::Ready(Exit::Interrupted);
            }
            match Future::poll(Pin::new(&mut self), cx) {
                Poll::Ready(Ok(permit)) => {
                    if context.is_requested() {
                        drop(permit);
                        Poll::Ready(Exit::Interrupted)
                    } else {
                        Poll::Ready(Exit::Success(permit))
                    }
                }
                Poll::Ready(Err(error)) => {
                    if context.is_requested() {
                        Poll::Ready(Exit::Interrupted)
                    } else {
                        Poll::Ready(Exit::Fail(error))
                    }
                }
                Poll::Pending => Poll::Pending,
            }
        })
        .await
    }
}
impl Drop for PreparedSemaphoreAcquire {
    fn drop(&mut self) {
        let Some(owner) = self.state.as_ref() else {
            return;
        };
        let mut state = owner
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = if let Some(position) = self.position.take() {
            state
                .waiters
                .remove_where(position, |waiter| waiter.id == self.id)
                .is_some()
        } else if self.granted {
            state.available += self.count.get();
            self.granted = false;
            true
        } else {
            false
        };
        drop(state);
        if changed {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                owner.changed.notify_all()
            }));
        }
    }
}
impl PreparedSemaphorePermit {
    /// Return permits exactly once, retaining the owner if accounting fails.
    pub fn release_now(&mut self) -> Result<bool, SemaphoreError> {
        let Some(owner) = self.state.as_ref() else {
            return Ok(false);
        };
        let mut state = owner
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let available = state
            .available
            .checked_add(self.count.get())
            .filter(|n| *n <= state.issued)
            .ok_or(SemaphoreError::PermitAccountingViolation)?;
        state.available = available;
        drop(state);
        let owner = self.state.take();
        if let Some(owner) = owner {
            owner.changed.notify_all();
        }
        Ok(true)
    }
}
impl Drop for PreparedSemaphorePermit {
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.release_now()));
    }
}
