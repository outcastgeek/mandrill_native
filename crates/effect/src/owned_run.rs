use crate::LibraryDefect;
use crate::internal::notification::notify_all;
use crate::{Defect, Exit};
use event_listener::{Event, EventListener};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

type RunFuture<A, E> = Pin<Box<dyn Future<Output = Exit<A, E>> + Send>>;

struct Completion<A, E> {
    outcome: Mutex<Option<Exit<A, E>>>,
    changed: Event,
}

impl<A, E> Completion<A, E> {
    fn publish(&self, outcome: Exit<A, E>) {
        *self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
        notify_all(&self.changed);
    }

    fn take(&self) -> Option<Exit<A, E>> {
        self.outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// Explicit execution owner, separate from the observer of its result.
///
/// The host must keep polling this driver through settlement. Dropping its
/// observer does not abandon execution. Dropping the driver before settlement
/// reports a defect to its observer and cannot promise asynchronous cleanup.
#[must_use = "keep driving the owned run through settlement"]
pub struct RunDriver<A, E> {
    pending: Option<RunFuture<A, E>>,
    completion: Arc<Completion<A, E>>,
    settled: bool,
}

/// Single-consumer observation of a run's complete outcome, without a Clone
/// or Sync requirement on success/error values. The host retains its driver.
#[must_use = "await the observer to handle the complete run outcome"]
pub struct RunObserver<A, E> {
    completion: Arc<Completion<A, E>>,
    listener: Option<Pin<Box<EventListener>>>,
    consumed: bool,
}

pub(crate) fn owned<A, E>(
    pending: impl Future<Output = Exit<A, E>> + Send + 'static,
) -> (RunDriver<A, E>, RunObserver<A, E>) {
    let completion = Arc::new(Completion {
        outcome: Mutex::new(None),
        changed: Event::new(),
    });
    (
        RunDriver {
            pending: Some(Box::pin(pending)),
            completion: Arc::clone(&completion),
            settled: false,
        },
        RunObserver {
            completion,
            listener: None,
            consumed: false,
        },
    )
}

impl<A, E> Future for RunDriver<A, E> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let Some(pending) = &mut this.pending else {
            return Poll::Ready(());
        };
        match pending.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(outcome) => {
                this.settled = true;
                this.pending = None;
                this.completion.publish(outcome);
                Poll::Ready(())
            }
        }
    }
}

impl<A, E> Drop for RunDriver<A, E> {
    fn drop(&mut self) {
        if !self.settled {
            drop(self.pending.take());
            self.completion
                .publish(Exit::Die(Defect::from(LibraryDefect::RunDriverDropped)));
        }
    }
}

impl<A, E> Future for RunObserver<A, E> {
    type Output = Exit<A, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.consumed {
            return Poll::Ready(Exit::Die(Defect::from(LibraryDefect::RunOutcomeConsumed)));
        }
        loop {
            // Register before inspecting completion, including if it arrived
            // before the observer was polled.
            let listener = this
                .listener
                .get_or_insert_with(|| Box::pin(this.completion.changed.listen()));
            if let Some(outcome) = this.completion.take() {
                this.consumed = true;
                this.listener = None;
                return Poll::Ready(outcome);
            }
            match listener.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(()) => this.listener = None,
            }
        }
    }
}
