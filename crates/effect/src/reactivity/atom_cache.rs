use super::{Atom, AtomError};
use crate::{Effect, Exit, HasWallClock, Semaphore, SemaphoreError, UtcMillis, WallClock};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug)]
pub enum AtomCacheError<E, ClockError> {
    Atom(AtomError<E>),
    Clock(ClockError),
    Admission(SemaphoreError),
}

/// TTL is a policy of this handle, not a process-global timer. Expiry is
/// checked against the clock in R and never publishes by itself.
pub struct AtomCache<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasWallClock + Send + Sync + 'static,
{
    atom: Atom<A, E, R>,
    ttl: Duration,
    successful_at: Arc<Mutex<Option<(NonZeroU64, UtcMillis)>>>,
    refresh_gate: Semaphore,
}

impl<A, E, R> Clone for AtomCache<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasWallClock + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            atom: self.atom.clone(),
            ttl: self.ttl,
            successful_at: Arc::clone(&self.successful_at),
            refresh_gate: self.refresh_gate.clone(),
        }
    }
}

impl<A, E, R> AtomCache<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasWallClock + Send + Sync + 'static,
{
    #[must_use]
    pub fn new(atom: Atom<A, E, R>, ttl: Duration) -> Self {
        Self {
            atom,
            ttl,
            successful_at: Arc::new(Mutex::new(None)),
            refresh_gate: Semaphore::full(NonZeroUsize::MIN),
        }
    }

    #[must_use]
    pub fn atom(&self) -> &Atom<A, E, R> {
        &self.atom
    }

    /// Forced refresh bypasses TTL. Concurrent cache requests queue behind
    /// one gate; cancellation of a waiter is handled by the semaphore.
    pub fn refresh(
        &self,
        force: bool,
    ) -> Effect<bool, AtomCacheError<E, <R::Clock as WallClock>::Error>, R> {
        let cache = self.clone();
        self.refresh_gate
            .acquire(NonZeroUsize::MIN)
            .map_error(AtomCacheError::Admission)
            .flat_map(move |permit| {
                let cache = cache.clone();
                let action = Effect::wall_now()
                    .map_error(AtomCacheError::Clock)
                    .flat_map(move |now| {
                        let cache = cache.clone();
                        let cache_for_status = cache.clone();
                        cache
                            .atom
                            .read()
                            .map_error(AtomCacheError::Atom)
                            .flat_map(move |status| {
                                let cache = cache_for_status.clone();
                                let cached = *cache
                                    .successful_at
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                let fresh = cached.is_some_and(|(generation, at)| {
                                    status.generation() == Some(generation)
                                        && matches!(status.outcome(), Some(Exit::Success(_)))
                                        && at.checked_add(cache.ttl).is_ok_and(|until| now < until)
                                        && cache
                                            .atom
                                            .registry()
                                            .is_current(cache.atom.id(), generation)
                                });
                                if fresh && !force {
                                    return Effect::sync(|| false);
                                }
                                let cache_after = cache.clone();
                                let cache_for_settled = cache.clone();
                                cache
                                    .atom
                                    .refresh()
                                    .map_error(AtomCacheError::Atom)
                                    .flat_map(move |_| {
                                        cache_after.atom.read().map_error(AtomCacheError::Atom)
                                    })
                                    .flat_map(move |settled| {
                                        let cache = cache_for_settled.clone();
                                        let cache_for_update = cache.clone();
                                        Effect::wall_now().map_error(AtomCacheError::Clock).map(
                                            move |finished_at| {
                                                if let Some(generation) = settled.generation()
                                                    && matches!(
                                                        settled.outcome(),
                                                        Some(Exit::Success(_))
                                                    )
                                                    && cache_for_update.atom.registry().is_current(
                                                        cache_for_update.atom.id(),
                                                        generation,
                                                    )
                                                {
                                                    *cache_for_update
                                                        .successful_at
                                                        .lock()
                                                        .unwrap_or_else(
                                                            std::sync::PoisonError::into_inner,
                                                        ) = Some((generation, finished_at));
                                                }
                                                true
                                            },
                                        )
                                    })
                            })
                    });
                action.ensuring(Effect::sync_once(move || drop(permit)))
            })
    }
}
