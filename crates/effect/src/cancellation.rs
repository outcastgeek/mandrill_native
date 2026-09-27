use crate::internal::notification::notify_all;
use event_listener::Event;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

#[derive(Default)]
struct State {
    requested: AtomicBool,
    changed: Event,
    children: Mutex<Vec<Weak<State>>>,
    parent: Mutex<Option<FlatDrop<Arc<State>>>>,
}

impl Drop for State {
    fn drop(&mut self) {
        let parent = self
            .parent
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        unlink(std::ptr::from_ref(self), &parent);
    }
}

fn unlink(address: *const State, parent: &Option<FlatDrop<Arc<State>>>) {
    if let Some(parent) = parent.as_ref().and_then(FlatDrop::as_ref) {
        parent
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|child| child.as_ptr() != address);
    }
}

/// Executor-independent, persistent cancellation request with broadcast notification.
#[derive(Clone, Default)]
pub struct Cancellation {
    state: Arc<State>,
}

impl Cancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A derived request source. Parent requests reach this child and its
    /// descendants; requesting the child does not request its parent/siblings.
    /// Parent registration stays alive until the child is dropped. Teardown
    /// unregisters the weak child link and destroys ancestors iteratively.
    #[must_use]
    pub fn child(&self) -> Self {
        let mut state = State::default();
        state.parent = Mutex::new(Some(FlatDrop::new(Arc::clone(&self.state))));
        let child = Self {
            state: Arc::new(state),
        };
        let requested = {
            let mut children = self
                .state
                .children
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            children.push(Arc::downgrade(&child.state));
            self.is_requested()
        };
        if requested {
            let _ = child.request();
        }
        child
    }

    /// Returns true only for the first request. Every registered waiter wakes.
    #[must_use]
    pub fn request(&self) -> bool {
        let first = !self.state.requested.swap(true, Ordering::AcqRel);
        if !first {
            return false;
        }
        let mut pending = vec![Arc::clone(&self.state)];
        let mut changed = Vec::new();
        // Iterative propagation keeps deep child trees stack-safe. Weak links
        // are detached before notifications/descendant requests, and no user
        // wake callback runs while a registration gate is held.
        while let Some(state) = pending.pop() {
            let children = std::mem::take(
                &mut *state
                    .children
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            for child in children.into_iter().filter_map(|child| child.upgrade()) {
                if !child.requested.swap(true, Ordering::AcqRel) {
                    pending.push(child);
                }
            }
            changed.push(state);
        }
        // Record the entire linked request before invoking any wake callback.
        for state in changed {
            notify_all(&state.changed);
        }
        true
    }

    pub(crate) fn unlink(&self) {
        let parent = self
            .state
            .parent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        unlink(Arc::as_ptr(&self.state), &parent);
    }

    #[must_use]
    pub fn is_requested(&self) -> bool {
        self.state.requested.load(Ordering::Acquire)
    }

    /// Registration precedes the flag check, so an intervening request cannot be lost.
    pub async fn cancelled(&self) {
        loop {
            let listener = self.state.changed.listen();
            if self.is_requested() {
                return;
            }
            listener.await;
        }
    }
}
use crate::internal::destruction::FlatDrop;
