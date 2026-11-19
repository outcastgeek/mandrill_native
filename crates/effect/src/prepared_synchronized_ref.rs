//! Startup-backed synchronized references and admitted native update futures.
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool};
use crate::{
    CleanupFailures, Exit, PreparedCleanupAccumulator, PreparedCleanupConfig, PreparedPanicPool,
    PreparedPanicReservation, PreparedPayloadPool, PreparedPayloadView, PreparedSemaphore,
    PreparedSemaphoreError, PreparedSemaphorePermit,
};
use futures_util::FutureExt;
use std::future::{Future, poll_fn};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::task::Poll;

type Context<E> = crate::internal::runtime::prepared_graph::FutureContext<E>;
struct State<T: Send + Sync + 'static> {
    current: Mutex<Option<PreparedPayloadView<T>>>,
    versions: PreparedPayloadPool<T>,
    gate: PreparedSemaphore,
    panics: PreparedPanicPool,
}
/// A checked shared owner of one prepared current value and admission gate.
pub struct PreparedSynchronizedRef<T: Send + Sync + 'static> {
    state: Option<SharedSlot<'static, State<T>>>,
}
/// Physical state demand; cleanup and panic authorities are separately supplied.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSynchronizedRefDemand {
    /// Region state, version cells and semaphore metadata, excluding allocator overhead and T pointees.
    pub region_bytes: Bytes,
    /// Simultaneous diagnostic reservations per admitted update.
    pub diagnostics_per_update: usize,
    /// Independently owned cleanup carrier per admitted update.
    pub cleanup_buffers_per_update: usize,
}
impl<T: Send + Sync + 'static> PreparedSynchronizedRef<T> {
    /// Report actual fixed region demand for caller-selected versions and waiters.
    pub fn demand(
        versions: usize,
        waiters: usize,
    ) -> Result<PreparedSynchronizedRefDemand, MemoryError> {
        Ok(PreparedSynchronizedRefDemand {
            region_bytes: SlotPool::<State<T>>::storage_bytes(1)?
                .checked_add(PreparedPayloadPool::<T>::storage_bytes(versions)?)?
                .checked_add(PreparedSemaphore::storage_bytes(waiters)?)?,
            diagnostics_per_update: 2,
            cleanup_buffers_per_update: 1,
        })
    }
    /// Prepare all state before consuming the original initial value.
    #[allow(clippy::result_large_err)]
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        versions: usize,
        waiters: usize,
        panics: &PreparedPanicPool,
        initial: T,
    ) -> Result<Self, (PreparedSemaphoreError, T)> {
        let provision = (|| {
            let _ = Self::demand(versions, waiters).map_err(PreparedSemaphoreError::Memory)?;
            let versions = PreparedPayloadPool::prepare(memory, region, versions)
                .map_err(PreparedSemaphoreError::Memory)?;
            let gate = PreparedSemaphore::prepare(memory, region, NonZeroUsize::MIN, 1, waiters)?;
            let panics = panics.try_clone().map_err(PreparedSemaphoreError::Memory)?;
            let mut record = Some(State {
                current: Mutex::new(None),
                versions,
                gate,
                panics,
            });
            let root = memory
                .slots(region, 1, |_| record.take().ok_or(MemoryError::NoSlot))
                .map_err(PreparedSemaphoreError::Memory)?;
            let lease = root.checkout().map_err(PreparedSemaphoreError::Memory)?;
            let writer = lease
                .versions
                .reserve()
                .map_err(PreparedSemaphoreError::Memory)?;
            Ok::<_, PreparedSemaphoreError>((lease, writer))
        })();
        let (lease, writer) = match provision {
            Ok(value) => value,
            Err(error) => return Err((error, initial)),
        };
        let view = match writer.publish(initial) {
            Ok(view) => view,
            Err((_writer, value)) => {
                return Err((
                    PreparedSemaphoreError::Memory(MemoryError::ConstructionAlreadyBound),
                    value,
                ));
            }
        };
        *lease
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(view);
        Ok(Self {
            state: Some(lease.share()),
        })
    }
    /// Retain the prepared current state without allocating.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            state: Some(
                self.state
                    .as_ref()
                    .ok_or(MemoryError::Closed)?
                    .try_clone()?,
            ),
        })
    }
    #[allow(clippy::expect_used)] // Every live handle retains its initialized root.
    fn state(&self) -> &State<T> {
        self.state.as_deref().expect("live synchronized reference")
    }
    /// Retain the current immutable value and its actual version cell.
    #[allow(clippy::expect_used)]
    pub fn read_now(&self) -> Result<PreparedPayloadView<T>, MemoryError> {
        let current = self
            .state()
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        current
            .as_ref()
            .expect("initialized synchronized value")
            .try_clone()
    }
}
impl<T: Send + Sync + 'static> Drop for PreparedSynchronizedRef<T> {
    fn drop(&mut self) {
        if let Some(lease) = self.state.take().and_then(SharedSlot::into_last) {
            let value = lease
                .current
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(value);
            drop(lease);
        }
    }
}
// Context Closed denotes observation termination, not cancellation. Register
// the existing listener on each poll, then use its authoritative requested flag.
async fn wait_context<E: Send + 'static, F: Future>(
    context: &mut Context<E>,
    future: F,
) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    poll_fn(|cx| {
        if context.is_requested() {
            return Poll::Ready(None);
        }
        {
            let mut cancelled = std::pin::pin!(context.cancelled());
            let _ = cancelled.as_mut().poll(cx);
        }
        if context.is_requested() {
            return Poll::Ready(None);
        }
        future.as_mut().poll(cx).map(Some)
    })
    .await
}
impl<T: Send + Sync + 'static> PreparedSynchronizedRef<T> {
    /// Admit all update owners before the callback. A refusal returns its
    /// original context, callback and mapper without invoking user code.
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn admit_update<B, E, F, Fut, M>(
        &self,
        context: Context<E>,
        cleanup: &PreparedCleanupConfig<E>,
        update: F,
        map_error: M,
    ) -> Result<
        impl Future<Output = Exit<B, E>> + Send + 'static,
        (PreparedSemaphoreError, Context<E>, F, M),
    >
    where
        B: Send + 'static,
        E: Send + 'static,
        F: FnOnce(PreparedPayloadView<T>) -> Fut + Send + 'static,
        Fut: Future<Output = Exit<(B, T), E>> + Send + 'static,
        M: Fn(PreparedSemaphoreError) -> E + Send + 'static,
    {
        let admitted = (|| {
            let candidate = self
                .state()
                .versions
                .reserve()
                .map_err(PreparedSemaphoreError::Memory)?;
            let owner = self.try_clone().map_err(PreparedSemaphoreError::Memory)?;
            let cleanup = cleanup
                .reserve_failures()
                .map_err(PreparedSemaphoreError::Memory)?;
            let first = self
                .state()
                .panics
                .try_reserve()
                .map_err(PreparedSemaphoreError::Memory)?;
            let second = self
                .state()
                .panics
                .try_reserve()
                .map_err(PreparedSemaphoreError::Memory)?;
            let acquisition = self.state().gate.acquire(NonZeroUsize::MIN)?;
            Ok::<_, PreparedSemaphoreError>((candidate, owner, cleanup, first, second, acquisition))
        })();
        let (candidate, owner, cleanup, first, second, acquisition) = match admitted {
            Ok(value) => value,
            Err(error) => return Err((error, context, update, map_error)),
        };
        Ok(async move {
            let mut context = context;
            let mut map_error = map_error;
            let mut permit: Option<PreparedSemaphorePermit> = None;
            let body = std::panic::AssertUnwindSafe(async {
                let map_error = &mut map_error;
                let acquired = wait_context(&mut context, acquisition).await;
                match acquired {
                    None => return Exit::Interrupted,
                    Some(Err(error)) => {
                        return Exit::Fail(map_error(PreparedSemaphoreError::Semaphore(error)));
                    }
                    Some(Ok(value)) => permit = Some(value),
                }
                if context.is_requested() {
                    return Exit::Interrupted;
                }
                let current = match owner.read_now() {
                    Ok(current) => current,
                    Err(error) => {
                        return Exit::Fail(map_error(PreparedSemaphoreError::Memory(error)));
                    }
                };
                if context.is_requested() {
                    drop(current);
                    return Exit::Interrupted;
                }
                // The pinned update is destroyed within this catch before commit or cancellation completes.
                let outcome = wait_context(&mut context, update(current)).await;
                match outcome {
                    None => Exit::Interrupted,
                    Some(Exit::Success((result, next))) => {
                        if context.is_requested() {
                            drop((result, next));
                            return Exit::Interrupted;
                        }
                        let next = match candidate.publish(next) {
                            Ok(next) => next,
                            Err((_candidate, next)) => {
                                drop((result, next));
                                return Exit::Fail(map_error(PreparedSemaphoreError::Memory(
                                    MemoryError::ConstructionAlreadyBound,
                                )));
                            }
                        };
                        let old = {
                            let mut current = owner
                                .state()
                                .current
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            current.replace(next)
                        };
                        drop(old);
                        Exit::Success(result)
                    }
                    Some(Exit::Fail(error)) => Exit::Fail(error),
                    Some(Exit::Die(defect)) => Exit::Die(defect),
                    Some(Exit::Interrupted) => Exit::Interrupted,
                    Some(Exit::FinalizationFailed(details)) => Exit::FinalizationFailed(details),
                }
            })
            .catch_unwind()
            .await;
            let (primary, spare_first) = match body {
                Ok(outcome) => (outcome, Some(first)),
                Err(payload) => (Exit::Die(first.capture(payload)), None),
            };
            let Some(mut permit) = permit else {
                return primary;
            };
            let (release_token, mut spare_second) = match spare_first {
                Some(first) => (first, Some(second)),
                None => (second, None),
            };
            let release = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                permit
                    .release_now()
                    .map(|_| ())
                    .map_err(|error| map_error(PreparedSemaphoreError::Semaphore(error)))
            }));
            let release = match release {
                Ok(Ok(())) => {
                    drop(release_token);
                    return primary;
                }
                Ok(Err(error)) => {
                    drop(spare_second);
                    spare_second = Some(release_token);
                    Exit::Fail(error)
                }
                Err(payload) => Exit::Die(release_token.capture(payload)),
            };
            finish_with_release(primary, release, cleanup, spare_second)
        })
    }
}
#[allow(clippy::unreachable, clippy::expect_used)]
fn finish_with_release<B, E: Send + 'static>(
    primary: Exit<B, E>,
    release: Exit<(), E>,
    cleanup: CleanupFailures<E>,
    token: Option<PreparedPanicReservation>,
) -> Exit<B, E> {
    match primary {
        Exit::Success(value) => {
            let mut accumulator =
                PreparedCleanupAccumulator::from_reserved(Exit::Success(()), cleanup);
            accumulator.append_cleanup(release);
            if std::mem::needs_drop::<B>() {
                let token = token.expect("preadmitted success-drop diagnostic");
                if let Err(payload) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(value)))
                {
                    accumulator.append_cleanup(Exit::Die(token.capture(payload)));
                }
            } else {
                drop(value);
            }
            match accumulator.finish() {
                Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
                Exit::Success(()) | Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted => {
                    unreachable!("failed release creates cleanup details")
                }
            }
        }
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            let mut accumulator = PreparedCleanupAccumulator::from_reserved(other, cleanup);
            accumulator.append_cleanup(release);
            accumulator.finish()
        }
    }
}

/// Private-grammar write authority retained by the canonical prepared Resource.
/// It never owns a ResourceValue; the permit protects the entire lexical body.
#[doc(hidden)]
pub struct PreparedSynchronizedWrite<T: Send + Sync + 'static, M, E: Send + 'static> {
    reference: PreparedSynchronizedRef<T>,
    permit: Mutex<Option<PreparedSemaphorePermit>>,
    candidate: Mutex<Option<crate::PreparedPayloadReservation<T>>>,
    map_error: M,
    marker: std::marker::PhantomData<fn() -> E>,
}
impl<T: Send + Sync + 'static> PreparedSynchronizedRef<T> {
    /// Reserve a candidate, root and semaphore operation before a graph Resource body.
    #[doc(hidden)]
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn admit_write<E, M>(
        &self,
        context: Context<E>,
        map_error: M,
    ) -> Result<
        impl Future<Output = Exit<PreparedSynchronizedWrite<T, M, E>, E>>
        + Send
        + 'static
        + use<T, E, M>,
        (PreparedSemaphoreError, Context<E>, M),
    >
    where
        E: Send + 'static,
        M: Fn(PreparedSemaphoreError) -> E + Send + Sync + 'static,
    {
        let admitted = (|| {
            let candidate = self
                .state()
                .versions
                .reserve()
                .map_err(PreparedSemaphoreError::Memory)?;
            let reference = self.try_clone().map_err(PreparedSemaphoreError::Memory)?;
            let acquisition = self.state().gate.acquire(NonZeroUsize::MIN)?;
            Ok::<_, PreparedSemaphoreError>((candidate, reference, acquisition))
        })();
        let (candidate, reference, acquisition) = match admitted {
            Ok(value) => value,
            Err(error) => return Err((error, context, map_error)),
        };
        Ok(async move {
            let mut context = context;
            match wait_context(&mut context, acquisition).await {
                None => Exit::Interrupted,
                Some(Err(error)) => {
                    if context.is_requested() {
                        Exit::Interrupted
                    } else {
                        Exit::Fail(map_error(PreparedSemaphoreError::Semaphore(error)))
                    }
                }
                Some(Ok(permit)) => {
                    if context.is_requested() {
                        drop(permit);
                        Exit::Interrupted
                    } else {
                        Exit::Success(PreparedSynchronizedWrite {
                            reference,
                            permit: Mutex::new(Some(permit)),
                            candidate: Mutex::new(Some(candidate)),
                            map_error,
                            marker: std::marker::PhantomData,
                        })
                    }
                }
            }
        })
    }
}
impl<T, M, E> PreparedSynchronizedWrite<T, M, E>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    M: Fn(PreparedSemaphoreError) -> E + Send + Sync + 'static,
{
    /// Retain the latest value after acquiring the write permit.
    #[doc(hidden)]
    pub fn snapshot(&self) -> Result<PreparedPayloadView<T>, E> {
        self.reference
            .read_now()
            .map_err(|error| (self.map_error)(PreparedSemaphoreError::Memory(error)))
    }
    /// Commit exactly one reserved replacement; retire its predecessor outside the value gate.
    #[doc(hidden)]
    pub fn commit(&self, next: T) -> Result<(), E> {
        let candidate = self
            .candidate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(candidate) = candidate else {
            drop(next);
            return Err((self.map_error)(PreparedSemaphoreError::Memory(
                MemoryError::ConstructionUnbound,
            )));
        };
        let next = match candidate.publish(next) {
            Ok(next) => next,
            Err((_candidate, next)) => {
                drop(next);
                return Err((self.map_error)(PreparedSemaphoreError::Memory(
                    MemoryError::ConstructionAlreadyBound,
                )));
            }
        };
        let old = self
            .reference
            .state()
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(next);
        drop(old);
        Ok(())
    }
    /// Return the permit through the Resource's protected release descriptor.
    #[doc(hidden)]
    pub fn release(self) -> Result<(), E> {
        let mut permit = self
            .permit
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match permit.as_mut() {
            Some(permit) => permit
                .release_now()
                .map(|_| ())
                .map_err(|error| (self.map_error)(PreparedSemaphoreError::Semaphore(error))),
            None => Ok(()),
        }
    }
}
