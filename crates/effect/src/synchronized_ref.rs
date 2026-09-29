use crate::{Effect, Release, ResourceBinding, Semaphore, SemaphoreError};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

struct Inner<A> {
    value: Mutex<Arc<A>>,
    admission: Semaphore,
}

/// Serializes effectful updates without holding a synchronous lock across an
/// await. Readers borrow a shared immutable snapshot; failed updates leave the
/// previous value installed. Each update owns its permit through Effect's
/// protected resource finalization.
pub struct SynchronizedRef<A: Send + Sync + 'static> {
    inner: Arc<Inner<A>>,
}

impl<A: Send + Sync + 'static> Clone for SynchronizedRef<A> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<A: Send + Sync + 'static> SynchronizedRef<A> {
    #[must_use]
    pub fn new(value: A) -> Self {
        Self {
            inner: Arc::new(Inner {
                value: Mutex::new(Arc::new(value)),
                admission: Semaphore::full(NonZeroUsize::MIN),
            }),
        }
    }

    pub fn make_with<E: Send + 'static, R: Send + Sync + 'static>(
        initial: impl Fn() -> A + Send + Sync + 'static,
    ) -> Effect<Self, E, R> {
        Effect::sync(move || Self::new(initial()))
    }

    /// Sharing the current snapshot never clones A and does not wait for a
    /// writer's effect. A reader sees either the old or committed new value.
    pub fn read<E: Send + 'static, R: Send + Sync + 'static>(&self) -> Effect<Arc<A>, E, R> {
        let reference = self.clone();
        Effect::sync(move || {
            Arc::clone(
                &reference
                    .inner
                    .value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
        })
    }

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
