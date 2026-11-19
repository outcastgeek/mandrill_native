//! Checked leaf storage beneath the existing interpreter. Both the pending frame
//! and its completion transition are physically acquired before factory invocation.
use super::{Consume, Once, Pending};
use crate::Exit;
use crate::internal::runtime::{Finish, Step};
use crate::memory::{FramePool, FrameReservation, MemoryError};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::task::{Context, Poll};

struct Completion<A: Send + 'static, E: Send + 'static> {
    // Keep terminal callback guards through user outcome destruction.
    outcome: Exit<A, E>,
    finish: Finish<A, E>,
}
impl<A: Send + 'static, E: Send + 'static> Consume<(), Step> for Completion<A, E> {
    fn consume(self, (): ()) -> Step {
        self.finish.call(self.outcome).unwrap_or(Step::Done)
    }
}
struct PendingExit<F, A: Send + 'static, E: Send + 'static> {
    pending: F,
    finish: Option<Finish<A, E>>,
    completion: Option<FrameReservation<'static, Completion<A, E>>>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<F: Future<Output = Exit<A, E>> + Send + 'static, A: Send + 'static, E: Send + 'static> Future
    for PendingExit<F, A, E>
{
    type Output = Step;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Step> {
        // SAFETY: PendingExit is pinned in its private initialized cell. Only
        // pending is projected as pinned; the Option control fields are Unpin and
        // may be taken. No path moves F after its first placement/poll.
        let this = unsafe { self.get_unchecked_mut() };
        let outcome = match catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: same stable F field, exclusively polled through this frame.
            unsafe { Pin::new_unchecked(&mut this.pending).poll(context) }
        })) {
            Ok(Poll::Pending) => return Poll::Pending,
            Ok(Poll::Ready(outcome)) => outcome,
            Err(payload) => Exit::Die(crate::internal::runtime::panic_defect(
                this.panic.take(),
                payload,
            )),
        };
        match (this.finish.take(), this.completion.take()) {
            (Some(finish), Some(reservation)) => Poll::Ready(Step::More(Once::prepared(
                reservation,
                Completion { finish, outcome },
            ))),
            // The evaluator never repolls a completed frame. Retain its existing
            // missing-outcome invariant behavior if an internal caller violates it.
            (Some(_), None) | (None, Some(_)) | (None, None) => Poll::Ready(Step::Done),
        }
    }
}

/// Uninitialized concrete pending/completion ownership acquired as one rollback
/// bundle. It can be held across resource acquisition; start cannot need capacity.
pub(crate) struct FutureReservation<F, A, E>
where
    F: Future<Output = Exit<A, E>> + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
{
    completion: FrameReservation<'static, Completion<A, E>>,
    pending: FrameReservation<'static, PendingExit<F, A, E>>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<F, A, E> FutureReservation<F, A, E>
where
    F: Future<Output = Exit<A, E>> + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
{
    pub(crate) fn layouts() -> (std::alloc::Layout, std::alloc::Layout) {
        (
            std::alloc::Layout::new::<PendingExit<F, A, E>>(),
            std::alloc::Layout::new::<Completion<A, E>>(),
        )
    }
    pub(crate) fn reserve(pool: &FramePool<'static>) -> Result<Self, MemoryError> {
        let completion = pool.reserve::<Completion<A, E>>()?;
        let pending = pool.reserve::<PendingExit<F, A, E>>()?;
        Ok(Self {
            completion,
            pending,
            panic: None,
        })
    }
    pub(crate) fn with_panic(mut self, panic: Option<crate::PreparedPanicReservation>) -> Self {
        self.panic = panic;
        self
    }
    pub(crate) fn complete(self, outcome: Exit<A, E>, finish: Finish<A, E>) -> Step {
        Step::More(Once::prepared(
            self.completion,
            Completion { finish, outcome },
        ))
    }
    pub(crate) fn start(self, factory: impl FnOnce() -> F, finish: Finish<A, E>) -> Step {
        let Self {
            completion,
            pending,
            panic,
        } = self;
        match catch_unwind(AssertUnwindSafe(factory)) {
            Ok(future) => Step::Suspended(Pending::prepared(
                pending,
                PendingExit {
                    pending: future,
                    finish: Some(finish),
                    completion: Some(completion),
                    panic,
                },
            )),
            Err(payload) => Step::More(Once::prepared(
                completion,
                Completion {
                    finish,
                    outcome: Exit::Die(crate::internal::runtime::panic_defect(panic, payload)),
                },
            )),
        }
    }
}

pub(crate) fn prepare_future<F, A, E>(
    pool: &FramePool<'static>,
    factory: impl FnOnce() -> F,
    finish: Finish<A, E>,
) -> Result<Step, (MemoryError, Finish<A, E>)>
where
    F: Future<Output = Exit<A, E>> + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
{
    match FutureReservation::<F, A, E>::reserve(pool) {
        Ok(reservation) => Ok(reservation.start(factory, finish)),
        Err(error) => Err((error, finish)),
    }
}
