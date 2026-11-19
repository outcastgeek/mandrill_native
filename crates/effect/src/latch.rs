//! A one-way synchronization signal with cancellation-aware waiting.

use crate::effect_sync;
use crate::internal::notification::notify_all;
use crate::internal::runtime::{self, Step};
use crate::{Effect, Exit};
use event_listener::Event;
use futures_util::future::select;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
/// Typed state used by latch.
struct State {
    opened: AtomicBool,
    changed: Event,
}

/// An executor-independent, persistent opening with cancellable waiters.
/// Opening broadcasts; cancelling/dropping a waiter does not open the latch.
/// Unlike a payload Deferred this needs no stored outcome or data mutex.
#[derive(Clone, Default)]
pub struct Latch {
    state: Arc<State>,
}

impl Latch {
    #[must_use]
    /// Construct a `Latch` from its required inputs and explicit policies.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a `Latch` through the declared Effect and environment capabilities.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<Self, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make<E: Send + 'static, R: Send + Sync + 'static>() -> Effect<Self, E, R> {
        effect_sync! { body: Self::new, }
    }

    /// True only for the first opening. All current/future waits can proceed.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<bool, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn open<E: Send + 'static, R: Send + Sync + 'static>(&self) -> Effect<bool, E, R> {
        let latch = self.clone();
        effect_sync! { body: move || {
            let first = !latch.state.opened.swap(true, Ordering::AcqRel);
            if first {
                notify_all(&latch.state.changed);
            }
            first
        }, }
    }

    /// Wait cooperatively until the latch opens.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Effect<(), E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn await_open<E: Send + 'static, R: Send + Sync + 'static>(&self) -> Effect<(), E, R> {
        let latch = self.clone();
        Effect::new(move |_, context, finish| {
            let latch = latch.clone();
            Step::suspended(Box::pin(async move {
                loop {
                    let changed = latch.state.changed.listen();
                    if context.cancelled() {
                        return runtime::finish(finish, Exit::Interrupted);
                    }
                    if latch.state.opened.load(Ordering::Acquire) {
                        return runtime::finish(finish, Exit::Success(()));
                    }
                    let changed = pin!(changed);
                    let cancelled = pin!(context.cancellation.cancelled());
                    let _ = select(changed, cancelled).await;
                }
            }))
        })
    }
}

/// Startup-provisioned latch whose listener slots have an explicit finite capacity.
/// It is separate from the ordinary allocating `Latch` and uses fallible sharing.
pub struct PreparedLatch {
    state: crate::memory::SharedSlot<'static, PreparedLatchState>,
}
struct PreparedLatchState {
    opened: AtomicBool,
    changed: crate::memory::prepared_notification::Event,
}
/// One admitted listener. Dropping it returns its physical listener slot.
pub struct PreparedLatchWait {
    listener: crate::memory::prepared_notification::Registration,
    state: crate::memory::SharedSlot<'static, PreparedLatchState>,
}
impl PreparedLatch {
    /// Prepare one latch and its caller-selected number of concurrent listeners.
    pub fn prepare(
        memory: &crate::memory::PreparedMemory<'static>,
        region: crate::memory::RegionId,
        capacity: usize,
    ) -> Result<Self, crate::memory::MemoryError> {
        let changed =
            crate::memory::prepared_notification::Event::prepare(memory, region, capacity)?;
        let mut changed = Some(changed);
        let pool = memory.slots(region, 1, |_| {
            Ok(PreparedLatchState {
                opened: AtomicBool::new(false),
                changed: changed.take().ok_or(crate::memory::MemoryError::NoSlot)?,
            })
        })?;
        Ok(Self {
            state: pool.checkout()?.share(),
        })
    }
    /// Pool layout bytes, excluding partition allocator headers and padding.
    pub fn storage_bytes(
        capacity: usize,
    ) -> Result<crate::memory::Bytes, crate::memory::MemoryError> {
        crate::memory::SlotPool::<PreparedLatchState>::storage_bytes(1)?.checked_add(
            crate::memory::prepared_notification::Event::storage_bytes(capacity)?,
        )
    }
    /// Retain the existing prepared latch without allocating a control block.
    pub fn try_clone(&self) -> Result<Self, crate::memory::MemoryError> {
        Ok(Self {
            state: self.state.try_clone()?,
        })
    }
    /// Reset an exclusively owned latch after all retained handles and waiters
    /// have retired. The listener pool itself remains prepared for reuse.
    pub fn reset(&mut self) -> Result<(), crate::memory::MemoryError> {
        if !self.state.is_unique() {
            return Err(crate::memory::MemoryError::Busy);
        }
        self.state.opened.store(false, Ordering::Release);
        Ok(())
    }
    /// Open once, publishing before notifying current waiters.
    #[must_use]
    pub fn open_now(&self) -> bool {
        let first = !self.state.opened.swap(true, Ordering::AcqRel);
        if first {
            self.state.changed.notify_all();
        }
        first
    }
    /// Reserve one actual listener before the operation that will wait starts.
    /// The returned future alone does not observe cancellation.
    pub fn wait(&self) -> Result<PreparedLatchWait, crate::memory::MemoryError> {
        let state = self.state.try_clone()?;
        let listener = state.changed.listen()?;
        Ok(PreparedLatchWait { listener, state })
    }
}
impl std::future::Future for PreparedLatchWait {
    type Output = ();
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        if this.state.opened.load(Ordering::Acquire) {
            return std::task::Poll::Ready(());
        }
        let _notification = this.listener.poll(cx);
        if this.state.opened.load(Ordering::Acquire) {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    }
}

impl PreparedLatchWait {
    /// Drive an admitted wait with the graph's already-reserved cancellation listener.
    /// Reserve this wait before the operation's business factory runs. The graph's
    /// `effect_future! { exit_with_context; ... }` supplies the prepared context.
    pub async fn with_context<E: Send + 'static>(
        mut self,
        mut context: crate::__private::prepared_graph::FutureContext<E>,
    ) -> Exit<(), E> {
        std::future::poll_fn(|cx| {
            if context.is_requested() {
                return std::task::Poll::Ready(Exit::Interrupted);
            }
            let observation = {
                let cancelled = std::pin::pin!(context.cancelled());
                std::future::Future::poll(cancelled, cx)
            };
            if matches!(
                observation,
                std::task::Poll::Ready(crate::PreparedCancellationObservation::Requested)
            ) {
                return std::task::Poll::Ready(Exit::Interrupted);
            }
            let opened = std::future::Future::poll(std::pin::Pin::new(&mut self), cx).is_ready();
            if context.is_requested() {
                std::task::Poll::Ready(Exit::Interrupted)
            } else if opened {
                std::task::Poll::Ready(Exit::Success(()))
            } else {
                std::task::Poll::Pending
            }
        })
        .await
    }
}
