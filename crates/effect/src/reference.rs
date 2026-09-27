use crate::Effect;
use std::sync::{Arc, Mutex};

/// Mutex-backed atomic compound operations over an owned value.
///
/// The name follows the reference's AtomicRef, not a lock-free guarantee.
/// A needs Send, not Clone or Sync. Callbacks run synchronously under this
/// value's gate and must not re-enter the same reference. Returned values own
/// their data; borrowed references cannot escape into an Effect result.
pub struct AtomicRef<A: Send + 'static> {
    value: Arc<Mutex<A>>,
}

impl<A: Send + 'static> Clone for AtomicRef<A> {
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
        }
    }
}

impl<A: Send + 'static> AtomicRef<A> {
    #[must_use]
    pub fn new(value: A) -> Self {
        Self {
            value: Arc::new(Mutex::new(value)),
        }
    }

    /// Fresh owned initial state on each run, without an A: Clone requirement.
    pub fn make_with<E: Send + 'static, R: Send + Sync + 'static>(
        initial: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<Self, E, R> {
        Effect::sync(move || Self::new(initial()))
    }

    /// Explicitly copy stored initial state per run. Prefer make_with for owned
    /// data that should be moved/created without copying.
    pub fn make<E: Send + 'static, R: Send + Sync + 'static>(initial: A) -> Effect<Self, E, R>
    where
        A: Clone + Sync,
    {
        Self::make_with(move || initial.clone())
    }

    /// Inspect a consistent borrowed value. No stored data is cloned.
    pub fn with<B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        inspect: impl Fn(&A) -> B + Send + Sync + 'static,
    ) -> Effect<B, E, R> {
        let reference = self.clone();
        Effect::sync(move || {
            let value = reference
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            inspect(&value)
        })
    }

    /// Explicit snapshot copying. Borrow with `with` when ownership is unneeded.
    pub fn get<E: Send + 'static, R: Send + Sync + 'static>(&self) -> Effect<A, E, R>
    where
        A: Clone,
    {
        self.with(Clone::clone)
    }

    /// Read current state and atomically install its replacement. The old value
    /// is retained if the callback unwinds before returning a replacement;
    /// mutations through interior mutability are ordinary Rust side effects.
    /// Destruction of a replaced value occurs after releasing the gate.
    pub fn modify<B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        update: impl Fn(&A) -> (B, A) + Send + Sync + 'static,
    ) -> Effect<B, E, R> {
        let reference = self.clone();
        Effect::sync(move || {
            let (result, old) = {
                let mut value = reference
                    .value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let (result, next) = update(&value);
                (result, std::mem::replace(&mut *value, next))
            };
            drop(old);
            result
        })
    }

    pub fn update<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        transform: impl Fn(&A) -> A + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        self.modify(move |value| ((), transform(value)))
    }

    /// Mutate existing storage under the gate, preserving its allocation.
    /// As with ordinary Rust mutation, an unwinding callback can leave partial
    /// changes; this operation does not promise rollback of those changes.
    pub fn modify_in_place<B: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        update: impl Fn(&mut A) -> B + Send + Sync + 'static,
    ) -> Effect<B, E, R> {
        let reference = self.clone();
        Effect::sync(move || {
            let mut value = reference
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            update(&mut value)
        })
    }

    /// Create a replacement before acquiring the gate, then commit atomically.
    /// Destroy the old value outside the gate.
    pub fn set_with<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        let reference = self.clone();
        Effect::sync(move || {
            let next = factory();
            let old = {
                let mut value = reference
                    .value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::replace(&mut *value, next)
            };
            drop(old);
        })
    }

    /// Explicitly copy a stored replacement on each run. `set_with` avoids
    /// copying when the application can produce fresh owned state instead.
    pub fn set<E: Send + 'static, R: Send + Sync + 'static>(&self, value: A) -> Effect<(), E, R>
    where
        A: Clone + Sync,
    {
        self.set_with(move || value.clone())
    }
}
