//! Atomic references that preserve owned values.

use crate::Effect;
use std::sync::{Arc, Mutex};

/// Mutex-backed atomic compound operations over an owned value.
///
/// The name follows the reference's AtomicRef, not a lock-free guarantee.
/// A needs Send, not Clone or Sync. Callbacks run synchronously under this
/// value's gate and must not re-enter the same reference. Returned values own
/// their data; borrowed references cannot escape into an Effect result.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
pub struct AtomicRef<A: Send + 'static> {
    value: Arc<Mutex<A>>,
}

impl<A: Send + 'static> Clone for AtomicRef<A> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
        }
    }
}

impl<A: Send + 'static> AtomicRef<A> {
    #[must_use]
    /// Construct a `AtomicRef` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(value: A) -> Self {
        Self {
            value: Arc::new(Mutex::new(value)),
        }
    }

    /// Fresh owned initial state on each run, without an A: Clone requirement.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `initial` (`impl Fn() -> A + Send + Sync + 'static`): Initial value used before any state transition.
    ///
    /// # Returns
    /// Type: `Effect<Self, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make_with<E: Send + 'static, R: Send + Sync + 'static>(
        initial: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<Self, E, R> {
        Effect::sync(move || Self::new(initial()))
    }

    /// Explicitly copy stored initial state per run. Prefer make_with for owned
    /// data that should be moved/created without copying.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `initial` (`A`): Initial value used before any state transition.
    ///
    /// # Returns
    /// Type: `Effect<Self, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make<E: Send + 'static, R: Send + Sync + 'static>(initial: A) -> Effect<Self, E, R>
    where
        A: Clone + Sync,
    {
        Self::make_with(move || initial.clone())
    }

    /// Inspect a consistent borrowed value. No stored data is cloned.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `inspect` (`impl Fn(&A) -> B + Send + Sync + 'static`): The inspect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<B, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<A, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `update` (`impl Fn(&A) -> (B, A) + Send + Sync + 'static`): The update supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<B, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    /// Replace the atomic reference using the supplied update function.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `transform` (`impl Fn(&A) -> A + Send + Sync + 'static`): Callback applied to the selected value or error.
    ///
    /// # Returns
    /// Type: `Effect<(), E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn update<E: Send + 'static, R: Send + Sync + 'static>(
        &self,
        transform: impl Fn(&A) -> A + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        self.modify(move |value| ((), transform(value)))
    }

    /// Mutate existing storage under the gate, preserving its allocation.
    /// As with ordinary Rust mutation, an unwinding callback can leave partial
    /// changes; this operation does not promise rollback of those changes.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `update` (`impl Fn(&mut A) -> B + Send + Sync + 'static`): The update supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<B, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn() -> A + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Effect<(), E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn set<E: Send + 'static, R: Send + Sync + 'static>(&self, value: A) -> Effect<(), E, R>
    where
        A: Clone + Sync,
    {
        self.set_with(move || value.clone())
    }
}
