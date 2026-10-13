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
            Step::Suspended(Box::pin(async move {
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
