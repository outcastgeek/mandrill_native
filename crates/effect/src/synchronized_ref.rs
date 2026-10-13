//! Effectful updates serialized through one reference.

use crate::effect_sync;
use crate::{Effect, Release, ResourceBinding, Semaphore, SemaphoreError};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// Typed inner used by synchronized ref.
///
/// # Type parameters
/// - `A`: Application value type.
struct Inner<A> {
    value: Mutex<Arc<A>>,
    admission: Semaphore,
}

/// Serializes effectful updates without holding a synchronous lock across an
/// await. Readers borrow a shared immutable snapshot; failed updates leave the
/// previous value installed. Each update owns its permit through Effect's
/// protected resource finalization.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + Sync + 'static`.
pub struct SynchronizedRef<A: Send + Sync + 'static> {
    inner: Arc<Inner<A>>,
}

impl<A: Send + Sync + 'static> Clone for SynchronizedRef<A> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<A: Send + Sync + 'static> SynchronizedRef<A> {
    #[must_use]
    /// Construct a `SynchronizedRef` from its required inputs and explicit policies.
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
            inner: Arc::new(Inner {
                value: Mutex::new(Arc::new(value)),
                admission: Semaphore::full(NonZeroUsize::MIN),
            }),
        }
    }

    /// Create a `SynchronizedRef` using the supplied factory and capability environment.
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
        effect_sync! { body: move || Self::new(initial()), }
    }

    /// Sharing the current snapshot never clones A and does not wait for a
    /// writer's effect. A reader sees either the old or committed new value.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<Arc<A>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn read<E: Send + 'static, R: Send + Sync + 'static>(&self) -> Effect<Arc<A>, E, R> {
        let reference = self.clone();
        effect_sync! { body: move || {
            Arc::clone(
                &reference
                    .inner
                    .value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
        }, }
    }

    /// Compute and commit a replacement reference value through an Effect.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type. Additional bound: `Send + 'static`.
    /// - `E`: Domain-error type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `update` (`impl Fn(Arc<A>) -> Effect<(B, A), E, R> + Send + Sync + 'static`): The update supplied to this operation.
    /// - `on_admission_error` (`impl Fn(SemaphoreError) -> E + Send + Sync + 'static`): The on admission error supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<B, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn modify_effect<B, E, R>(
        &self,
        update: impl Fn(Arc<A>) -> Effect<(B, A), E, R> + Send + Sync + 'static,
        on_admission_error: impl Fn(SemaphoreError) -> E + Send + Sync + 'static,
    ) -> Effect<B, E, R>
    where
        B: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let reference = self.clone();
        let on_admission_error = Arc::new(on_admission_error);
        let release_error = Arc::clone(&on_admission_error);
        let acquisition = self
            .inner
            .admission
            .acquire(NonZeroUsize::MIN)
            .map_error(move |error| on_admission_error(error));
        ResourceBinding::new(acquisition, move |mut permit, _| {
            let release_error = Arc::clone(&release_error);
            Release::try_sync(move || {
                permit
                    .release_now()
                    .map(|_| ())
                    .map_err(move |error| release_error(error))
            })
        })
        .use_with(move |_| {
            let current = Arc::clone(
                &reference
                    .inner
                    .value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            let reference = reference.clone();
            update(current).map(move |(result, next)| {
                let old = {
                    let mut value = reference
                        .inner
                        .value
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    std::mem::replace(&mut *value, Arc::new(next))
                };
                drop(old);
                result
            })
        })
    }
}
