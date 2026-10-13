//! Cache-backed atom refresh and invalidation.

use super::{Atom, AtomError};
use crate::{Effect, Exit, HasWallClock, Semaphore, SemaphoreError, UtcMillis, WallClock};
use crate::{effect, effect_map, effect_map_error, effect_resource, effect_sync};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Typed cache failures for reactive computations; atom errors retain shared non-Clone evaluator failures and require Send ownership.
#[derive(Debug)]
pub enum AtomCacheError<E: Send + 'static, ClockError> {
    /// The selected reactive atom could not be evaluated or updated. Retains the `AtomError<E>` detail.
    Atom(AtomError<E>),
    /// The selected clock could not schedule or observe the requested time. Retains the `ClockError` detail.
    Clock(ClockError),
    /// The provider rejected work before accepting ownership. Retains the `SemaphoreError` detail.
    Admission(SemaphoreError),
}

/// TTL is a policy of this handle, not a process-global timer. Expiry is
/// checked against the clock in R and never publishes by itself.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `R`: Environment type.
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
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Construct a `AtomCache` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `atom` (`Atom<A, E, R>`): The atom supplied to this operation.
    /// - `ttl` (`Duration`): The ttl supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(atom: Atom<A, E, R>, ttl: Duration) -> Self {
        Self {
            atom,
            ttl,
            successful_at: Arc::new(Mutex::new(None)),
            refresh_gate: Semaphore::full(NonZeroUsize::MIN),
        }
    }

    #[must_use]
    /// Select the registered atom identified by this value.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Returns
    /// Type: `&Atom<A, E, R>`.
    pub fn atom(&self) -> &Atom<A, E, R> {
        &self.atom
    }

    /// Forced refresh bypasses TTL. Concurrent cache requests queue behind
    /// one gate; cancellation of a waiter is handled by the semaphore.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    /// - `R`: Environment type.
    ///
    /// # Parameters
    /// - `force` (`bool`): The force supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<bool, AtomCacheError<E, <R::Clock as WallClock>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn refresh(
        &self,
        force: bool,
    ) -> Effect<bool, AtomCacheError<E, <R::Clock as WallClock>::Error>, R> {
        let cache = self.clone();
        {
            let __dsl_program = effect_map_error! { program: self.refresh_gate
            .acquire(NonZeroUsize::MIN), map: AtomCacheError::Admission, };
            effect! { let __dsl_program = __dsl_program.clone(); let cache = cache.clone(); let permit = __dsl_program.clone()?; {
                let cache = cache.clone();
                let action = { let __dsl_program = effect_map_error! { program: Effect::wall_now(), map: AtomCacheError::Clock, }; effect! { let __dsl_program = __dsl_program.clone(); let cache = cache.clone(); let now = __dsl_program.clone()?; {
                        let cache = cache.clone();
                        let cache_for_status = cache.clone();
                        { let __dsl_program = effect_map_error! { program: cache
                            .atom
                            .read(), map: AtomCacheError::Atom, }; effect! { let __dsl_program = __dsl_program.clone(); let cache_for_status = cache_for_status.clone(); let status = __dsl_program.clone()?; (move || {
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
                                    return effect_sync! { body: || false, };
                                }
                                let cache_after = cache.clone();
                                let cache_for_settled = cache.clone();
                                { let __dsl_program = { let __dsl_program = effect_map_error! { program: cache
                                    .atom
                                    .refresh(), map: AtomCacheError::Atom, }; effect! { let cache_after = cache_after.clone(); let _ = __dsl_program.clone()?; {
                                        effect_map_error! { program: cache_after.atom.read(), map: AtomCacheError::Atom, }
                                    }? } }; effect! { let cache_for_settled = cache_for_settled.clone(); let settled = __dsl_program.clone()?; {
                                        let cache = cache_for_settled.clone();
                                        let cache_for_update = cache.clone();
                                        effect_map! { program: effect_map_error! { program: Effect::wall_now(), map: AtomCacheError::Clock, }, map: move |finished_at| {
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
                                            }, }
                                    }? } }
                            })()? } }
                    }? } };
                { let __dsl_program = action; let __dsl_cleanup = effect_sync! { once; body: move || drop(permit), }; effect_resource! { acquire: effect_sync! { body: || (), }, release: move |(), _| __dsl_cleanup.clone(), body: move |_| __dsl_program.clone(), } }
            }? }
        }
    }
}
