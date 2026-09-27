use crate::internal::notification::notify_all;
use crate::internal::runtime::{self, Step};
use crate::{Effect, Exit};
use event_listener::Event;
use futures_util::future::select;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn make<E: Send + 'static, R: Send + Sync + 'static>() -> Effect<Self, E, R> {
        Effect::sync(Self::new)
    }

    /// True only for the first opening. All current/future waits can proceed.
    pub fn open<E: Send + 'static, R: Send + Sync + 'static>(&self) -> Effect<bool, E, R> {
        let latch = self.clone();
        Effect::sync(move || {
            let first = !latch.state.opened.swap(true, Ordering::AcqRel);
            if first {
                notify_all(&latch.state.changed);
            }
            first
        })
    }

    pub fn await_open<E: Send + 'static, R: Send + Sync + 'static>(&self) -> Effect<(), E, R> {
        let latch = self.clone();
        Effect::new(move |_, context, finish| {
            let latch = latch.clone();
            Step::Suspended(Box::pin(async move {
                loop {
                    let changed = latch.state.changed.listen();
                    if context.cancelled() {
                        return runtime::finish(finish, Exit::Interrupted);
                    }
                    if latch.state.opened.load(Ordering::Acquire) {
                        return runtime::finish(finish, Exit::Success(()));
                    }
                    let changed = std::pin::pin!(changed);
                    let cancelled = std::pin::pin!(context.cancellation.cancelled());
                    let _ = select(changed, cancelled).await;
                }
            }))
        })
    }
}
