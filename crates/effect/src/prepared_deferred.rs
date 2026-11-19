//! Physically prepared completion and shared outcome observation.
use crate::memory::{
    Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool, prepared_notification,
};
use crate::{Exit, LibraryDefect, PreparedCancellationObservation};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard};
use std::task::{Context, Poll};

type OutcomeSlot<A, E> = SharedSlot<'static, Mutex<Option<Exit<A, E>>>>;
struct Outcome<A: Send + 'static, E: Send + 'static> {
    slot: Option<OutcomeSlot<A, E>>,
}
impl<A: Send + 'static, E: Send + 'static> Outcome<A, E> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        match &self.slot {
            Some(slot) => Ok(Self {
                slot: Some(slot.try_clone()?),
            }),
            None => Err(MemoryError::Closed),
        }
    }
}
impl<A: Send + 'static, E: Send + 'static> Drop for Outcome<A, E> {
    fn drop(&mut self) {
        if let Some(lease) = self.slot.take().and_then(SharedSlot::into_last) {
            let value = lease
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(value);
            drop(lease);
        }
    }
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum Completion {
    Pending,
    Ready,
    Shared,
    Consumed,
}
struct State<A: Send + 'static, E: Send + 'static> {
    completion: Mutex<Completion>,
    outcome: Outcome<A, E>,
    changed: prepared_notification::Event,
}
/// A single prepared completion with explicitly bounded observers.
pub struct PreparedDeferred<A: Send + 'static, E: Send + 'static> {
    state: SharedSlot<'static, State<A, E>>,
}
/// A shared owner of the original prepared completion cell.
pub struct PreparedSharedExit<A: Send + 'static, E: Send + 'static> {
    outcome: Outcome<A, E>,
}
/// Immutable access to the actual retained outcome.
pub struct PreparedExitBorrow<'a, A: Send + 'static, E: Send + 'static> {
    guard: MutexGuard<'a, Option<Exit<A, E>>>,
}
impl<A: Send + 'static, E: Send + 'static> PreparedExitBorrow<'_, A, E> {
    /// The outcome is present while this shared owner is live.
    #[must_use]
    pub fn get(&self) -> Option<&Exit<A, E>> {
        self.guard.as_ref()
    }
}
impl<A: Send + 'static, E: Send + 'static> PreparedSharedExit<A, E> {
    /// Fallibly retain the same original outcome cell.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            outcome: self.outcome.try_clone()?,
        })
    }
    /// Borrow without cloning either channel. This guard does not catch callback panics.
    #[must_use]
    pub fn borrow(&self) -> Option<PreparedExitBorrow<'_, A, E>> {
        self.outcome.slot.as_ref().map(|slot| PreparedExitBorrow {
            guard: slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        })
    }
    /// Inspect the retained outcome using a diagnostic reserved before the
    /// callback. The outcome lock is released before panic conversion.
    pub fn with<B>(
        &self,
        reservation: crate::PreparedPanicReservation,
        inspect: impl FnOnce(&Exit<A, E>) -> B,
    ) -> Result<B, crate::Defect> {
        let borrow = self
            .borrow()
            .ok_or(LibraryDefect::DeferredOutcomeConsumed)?;
        let outcome = borrow.get().ok_or(LibraryDefect::DeferredOutcomeConsumed)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inspect(outcome)));
        drop(borrow);
        result.map_err(|panic| reservation.capture(panic))
    }
}
/// Either physical sharing was refused or the outcome was already consumed.
#[derive(Debug)]
pub enum PreparedDeferredError {
    Memory(MemoryError),
    Library(LibraryDefect),
}
/// One physically admitted completion listener.
pub struct PreparedDeferredWait<A: Send + 'static, E: Send + 'static> {
    listener: prepared_notification::Registration,
    state: SharedSlot<'static, State<A, E>>,
}
impl<A: Send + 'static, E: Send + 'static> PreparedDeferred<A, E> {
    /// Pool layouts excluding region allocator headers and padding.
    pub fn storage_bytes(listeners: usize) -> Result<Bytes, MemoryError> {
        SlotPool::<State<A, E>>::storage_bytes(1)?
            .checked_add(SlotPool::<Mutex<Option<Exit<A, E>>>>::storage_bytes(1)?)?
            .checked_add(prepared_notification::Event::storage_bytes(listeners)?)
    }
    /// Prepare the actual outcome cell and caller-selected listener capacity.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        listeners: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(listeners)?;
        let outcomes = memory.slots(region, 1, |_| Ok(Mutex::new(None)))?;
        let outcome = Outcome {
            slot: Some(outcomes.checkout()?.share()),
        };
        let changed = prepared_notification::Event::prepare(memory, region, listeners)?;
        let mut outcome = Some(outcome);
        let mut changed = Some(changed);
        let states = memory.slots(region, 1, |_| {
            Ok(State {
                completion: Mutex::new(Completion::Pending),
                outcome: outcome.take().ok_or(MemoryError::NoSlot)?,
                changed: changed.take().ok_or(MemoryError::NoSlot)?,
            })
        })?;
        Ok(Self {
            state: states.checkout()?.share(),
        })
    }
    /// Fallibly retain this prepared completion control.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            state: self.state.try_clone()?,
        })
    }
    /// Publish once; a duplicate returns the complete original candidate.
    pub fn publish_now(&self, outcome: Exit<A, E>) -> Result<(), Exit<A, E>> {
        let mut completion = self
            .state
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *completion != Completion::Pending {
            return Err(outcome);
        }
        if let Some(slot) = &self.state.outcome.slot {
            let mut stored = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if stored.is_some() {
                return Err(outcome);
            }
            *stored = Some(outcome);
            drop(stored);
            *completion = Completion::Ready;
            drop(completion);
            self.state.changed.notify_all();
            Ok(())
        } else {
            Err(outcome)
        }
    }
    /// Move the original outcome once, refusing while external shared owners exist.
    pub fn try_take_owned(&self) -> Result<Option<Exit<A, E>>, LibraryDefect> {
        let mut completion = self
            .state
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *completion {
            Completion::Pending => Ok(None),
            Completion::Consumed => Err(LibraryDefect::DeferredOutcomeConsumed),
            Completion::Ready | Completion::Shared => {
                let Some(slot) = &self.state.outcome.slot else {
                    return Err(LibraryDefect::DeferredOutcomeConsumed);
                };
                if *completion == Completion::Shared && !slot.is_unique() {
                    return Err(LibraryDefect::DeferredOutcomeShared);
                }
                let value = slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                *completion = Completion::Consumed;
                Ok(value)
            }
        }
    }
    /// Retain an actual shared outcome without cloning A or E.
    pub fn try_share(&self) -> Result<Option<PreparedSharedExit<A, E>>, PreparedDeferredError> {
        let mut completion = self
            .state
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *completion {
            Completion::Pending => Ok(None),
            Completion::Consumed => Err(PreparedDeferredError::Library(
                LibraryDefect::DeferredOutcomeConsumed,
            )),
            Completion::Ready | Completion::Shared => {
                let outcome = self
                    .state
                    .outcome
                    .try_clone()
                    .map_err(PreparedDeferredError::Memory)?;
                *completion = Completion::Shared;
                Ok(Some(PreparedSharedExit { outcome }))
            }
        }
    }
    /// Reserve one actual waiter before starting the operation that will wait.
    pub fn wait(&self) -> Result<PreparedDeferredWait<A, E>, MemoryError> {
        let state = self.state.try_clone()?;
        let listener = state.changed.listen()?;
        Ok(PreparedDeferredWait { listener, state })
    }
}
impl<A: Send + 'static, E: Send + 'static> Future for PreparedDeferredWait<A, E> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.as_ref().get_ref();
        let _notification = this.listener.poll(cx);
        if *this
            .state
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            == Completion::Pending
        {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}
impl<A: Send + 'static, E: Send + 'static> PreparedDeferredWait<A, E> {
    /// Drive this admitted wait with the graph's reserved cancellation listener.
    pub async fn with_context<X: Send + 'static>(
        mut self,
        mut context: crate::__private::prepared_graph::FutureContext<X>,
    ) -> Exit<(), X> {
        std::future::poll_fn(|cx| {
            if context.is_requested() {
                return Poll::Ready(Exit::Interrupted);
            }
            let observed = {
                let cancelled = std::pin::pin!(context.cancelled());
                Future::poll(cancelled, cx)
            };
            if matches!(
                observed,
                Poll::Ready(PreparedCancellationObservation::Requested)
            ) {
                return Poll::Ready(Exit::Interrupted);
            }
            let ready = Future::poll(Pin::new(&mut self), cx).is_ready();
            if context.is_requested() {
                Poll::Ready(Exit::Interrupted)
            } else if ready {
                Poll::Ready(Exit::Success(()))
            } else {
                Poll::Pending
            }
        })
        .await
    }
}
impl<A: Send + 'static, E: Send + 'static> Unpin for PreparedDeferredWait<A, E> {}
