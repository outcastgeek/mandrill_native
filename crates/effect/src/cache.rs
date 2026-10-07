//! Scoped in-memory cache lookup, ownership, and invalidation.

use crate::execution::TaskEvent;
use crate::internal::notification::notify_all;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Step};
use crate::scope::Registered;
use crate::{
    Cancellation, Clock, Defect, Effect, Exit, HasClock, HasTaskSpawner, LibraryDefect,
    ScheduledTask, Scope, TaskSpawner,
};
use event_listener::Event;
use futures_util::{
    FutureExt,
    future::{Either, select},
};
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// Shared ownership of a payload without requiring `T: Clone`.
///
/// Cloning shares one protected value. [`Self::with`] borrows it for a synchronous
/// inspection; [`Self::try_into_owned`] transfers it only when this is the last
/// owner. Debug output is opaque and does not inspect the application payload.
pub struct SharedValue<T: Send + 'static> {
    value: Arc<Mutex<T>>,
}
impl<T: Send + 'static> std::fmt::Debug for SharedValue<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedValue")
            .finish_non_exhaustive()
    }
}
impl<T: Send + 'static> Clone for SharedValue<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
        }
    }
}
impl<T: Send + 'static> SharedValue<T> {
    #[must_use]
    /// Construct a `SharedValue` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`T`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(value: T) -> Self {
        Self {
            value: Arc::new(Mutex::new(value)),
        }
    }
    /// Borrow the contained value only for the duration of the supplied callback.
    ///
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `inspect` (`impl FnOnce(&T) -> A`): The inspect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<A, Defect>`.
    pub fn with<A>(&self, inspect: impl FnOnce(&T) -> A) -> Result<A, Defect> {
        let value = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        catch_unwind(AssertUnwindSafe(|| inspect(&value))).map_err(Defect::from_panic)
    }
    /// Take the guarded value when no other shared views retain it.
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Result<T, Self>`.
    pub fn try_into_owned(self) -> Result<T, Self> {
        match Arc::try_unwrap(self.value) {
            Ok(value) => Ok(value
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner)),
            Err(value) => Err(Self { value }),
        }
    }
}
#[derive(Clone, Copy)]
/// Explicit resource bounds for cache.
pub enum CacheCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
    /// No retention is available.
    NoRetention,
}
/// Type alias for equality in cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
type Equality<K> = Arc<dyn Fn(&K, &K) -> bool + Send + Sync>;
/// The equality, expiry, capacity, and failure-retention policies for a cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
/// - `E`: Domain-error type.
pub struct CacheConfig<K, E> {
    /// Key equality function used to identify cache entries. Type: `Equality<K>`.
    pub equal: Equality<K>,
    /// Time for which the stored value remains valid. Type: `Option<Duration>`.
    pub ttl: Option<Duration>,
    /// Settled-entry retention capacity; active producer admission is controlled separately.
    pub capacity: CacheCapacity,
    /// Predicate selecting expected errors that remain cached. Type: `Arc<dyn Fn(&E) -> bool + Send + Sync>`.
    pub cache_failure: Arc<dyn Fn(&E) -> bool + Send + Sync>,
}
impl<K, E> Clone for CacheConfig<K, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            equal: Arc::clone(&self.equal),
            ttl: self.ttl,
            capacity: self.capacity,
            cache_failure: Arc::clone(&self.cache_failure),
        }
    }
}
/// The lookup path selected atomically when a key is acquired by [`Cache::get_observed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheObservation {
    /// A fresh settled value was reused without dispatching a producer.
    Retained,
    /// The lookup joined already active producer work for an equivalent key.
    Shared,
    /// The lookup admitted new producer work for this key.
    Loaded,
}
/// A shared successful payload and its acquisition classification; no payload clone is required.
pub struct CacheLookup<A: Send + 'static> {
    /// Shared ownership of the successful cached payload.
    pub value: SharedValue<A>,
    /// How this acquisition found or started work, even if completion races with observation.
    pub observation: CacheObservation,
}
type OutcomeTtl<A, E> = Arc<dyn Fn(&Exit<A, E>) -> Result<Option<Duration>, Defect> + Send + Sync>;
/// Retention lifetime selected after producer settlement, using the environment clock.
///
/// Expiry affects retained reuse, not execution cancellation. None means no expiry;
/// failure retention still depends on [`CacheConfig::cache_failure`].
pub enum CacheTtl<A, E> {
    /// Use one optional duration for all retainable outcomes; None retains without time expiry.
    Fixed(Option<Duration>),
    /// Compute an optional duration from the full settled outcome outside the cache gate; callback errors remain defects.
    Outcome(OutcomeTtl<A, E>),
}
impl<A, E> Clone for CacheTtl<A, E> {
    fn clone(&self) -> Self {
        match self {
            Self::Fixed(ttl) => Self::Fixed(*ttl),
            Self::Outcome(callback) => Self::Outcome(Arc::clone(callback)),
        }
    }
}
/// Admission limit for independent active producers, checked before task-provider dispatch.
pub enum CacheAdmission<E> {
    /// Allow independent active producers without an explicit admission count bound.
    Unbounded,
    /// Reject new independent work at the supplied active bound; waiters joining existing work consume no additional slot.
    Bounded {
        /// Maximum admitted independent producer count.
        maximum: NonZeroUsize,
        /// Factory producing the typed rejection when active admission is exhausted.
        capacity_error: Arc<dyn Fn() -> E + Send + Sync>,
    },
}
impl<E> Clone for CacheAdmission<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Unbounded => Self::Unbounded,
            Self::Bounded {
                maximum,
                capacity_error,
            } => Self::Bounded {
                maximum: *maximum,
                capacity_error: Arc::clone(capacity_error),
            },
        }
    }
}
/// Outcome expiry and active admission settings used by [`Cache::make_with_options`].
pub struct CacheOptions<A, E> {
    /// Retention lifetime selection; overrides the legacy fixed TTL in CacheConfig.
    pub ttl: CacheTtl<A, E>,
    /// Active producer bound, independent of settled retention capacity.
    pub admission: CacheAdmission<E>,
}
impl<A, E> Clone for CacheOptions<A, E> {
    fn clone(&self) -> Self {
        Self {
            ttl: self.ttl.clone(),
            admission: self.admission.clone(),
        }
    }
}
pub(crate) enum CacheSelectionError<E: Send + 'static> {
    Defect(Defect),
    Capacity(SharedValue<E>),
}
impl<E: Send + 'static> CacheSelectionError<E> {
    pub(crate) fn exit<A>(self) -> Exit<A, SharedValue<E>> {
        match self {
            Self::Defect(defect) => Exit::Die(defect),
            Self::Capacity(error) => Exit::Fail(error),
        }
    }
}

type SharedOutcome<A, E> = Exit<SharedValue<A>, SharedValue<E>>;
/// Typed entry state used by cache.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct EntryState<A: Send + 'static, E: Send + 'static> {
    outcome: Option<SharedOutcome<A, E>>,
    expires: Option<Duration>,
    invalidated: bool,
    retiring: bool,
    observed: bool,
}
/// Typed entry used by cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub(crate) struct Entry<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    pub(crate) key: Arc<K>,
    state: Mutex<EntryState<A, E>>,
    changed: Event,
    cancellation: Cancellation,
}
/// Type alias for diagnostic in cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type Diagnostic<K, A, E> = (Arc<Entry<K, A, E>>, SharedOutcome<A, E>);
/// Variants of submission used by cache.
enum Submission {
    Pending(Option<Defect>),
    Accepted,
    Finished,
}
/// Type alias for start load in cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type StartLoad<K, A, E> =
    Arc<dyn Fn(Arc<Entry<K, A, E>>, Weak<Inner<K, A, E>>, Context) + Send + Sync>;
/// Typed cache state used by cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct CacheState<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    closed: bool,
    revision: Arc<()>,
    retained: VecDeque<Arc<Entry<K, A, E>>>,
    active: Vec<Arc<Entry<K, A, E>>>,
    diagnostics: Vec<Diagnostic<K, A, E>>,
    shutdown: CacheShutdown<E>,
    owner_loss: Option<Defect>,
}
/// Variants of cache shutdown used by cache.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
enum CacheShutdown<E: Send + 'static> {
    Open,
    Closing,
    Closed(Option<Exit<(), SharedValue<E>>>),
}
/// Typed inner used by cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Inner<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    state: Mutex<CacheState<K, A, E>>,
    changed: Event,
    parent: Cancellation,
    config: CacheConfig<K, E>,
    options: CacheOptions<A, E>,
    now: Arc<dyn Fn() -> Duration + Send + Sync>,
    start: StartLoad<K, A, E>,
}
/// A scoped cache that shares in-flight loads and retains their outcomes under an explicit policy.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct Cache<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    inner: Arc<Inner<K, A, E>>,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Clone for Cache<K, A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
///
/// # Type parameters
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
///
/// # Parameters
/// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
///
/// # Returns
/// Type: `SharedOutcome<A, E>`.
/// Returns `SharedOutcome<A, E>`.
fn share<A: Send + 'static, E: Send + 'static>(outcome: Exit<A, E>) -> SharedOutcome<A, E> {
    let outcome = outcome.map_error(SharedValue::new);
    match outcome {
        Exit::Success(value) => Exit::Success(SharedValue::new(value)),
        Exit::Fail(error) => Exit::Fail(error),
        Exit::Die(defect) => Exit::Die(defect),
        Exit::Interrupted => Exit::Interrupted,
        Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
    }
}
///
/// # Type parameters
/// - `K`: Bounds: `Send + Sync + 'static`.
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
///
/// # Parameters
/// - `entry` (`&Arc<Entry<K, A, E>>`): Stored record or event selected for this transition.
/// - `owner` (`&Weak<Inner<K, A, E>>`): Worker or scope identity used to fence stale operations.
/// - `original` (`Exit<A, E>`): The original supplied to this operation.
///
/// # Returns
/// Type: `()`.
fn settle<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static>(
    entry: &Arc<Entry<K, A, E>>,
    owner: &Weak<Inner<K, A, E>>,
    original: Exit<A, E>,
) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let keep = match &original {
            Exit::Success(_) => true,
            Exit::Fail(error) => owner
                .upgrade()
                .is_some_and(|owner| (owner.config.cache_failure)(error)),
            Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => false,
        };
        let expires = match owner.upgrade() {
            Some(owner) if keep => {
                let ttl = match &owner.options.ttl {
                    CacheTtl::Fixed(ttl) => *ttl,
                    CacheTtl::Outcome(callback) => callback(&original)?,
                };
                ttl.map(|ttl| {
                    (owner.now)()
                        .checked_add(ttl)
                        .ok_or_else(|| Defect::from(LibraryDefect::ClockDeadlineOverflow))
                })
                .transpose()?
            }
            Some(_) | None => None,
        };
        Ok::<_, Defect>((keep, expires))
    }));
    let (outcome, keep, expires) = match result {
        Ok(Ok((keep, expires))) => (share(original), keep, expires),
        Ok(Err(defect)) => (
            share(outcome::append_cleanup(original, Exit::Die(defect))),
            false,
            None,
        ),
        Err(panic) => (
            share(outcome::append_cleanup(
                original,
                Exit::Die(Defect::from_panic(panic)),
            )),
            false,
            None,
        ),
    };
    let mut entry_state = entry
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if entry_state.outcome.is_some() {
        return;
    }
    let diagnostic = matches!(outcome, Exit::FinalizationFailed(_)).then(|| outcome.clone());
    entry_state.outcome = Some(outcome);
    entry_state.expires = expires;
    let invalidated = entry_state.invalidated;
    let retiring = entry_state.retiring;
    drop(entry_state);
    if let Some(owner) = owner.upgrade() {
        let discarded = {
            let mut state = owner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state
                .active
                .retain(|candidate| !Arc::ptr_eq(candidate, entry));
            let mut discarded = Vec::new();
            if (!keep
                || invalidated
                || state.closed
                || matches!(owner.config.capacity, CacheCapacity::NoRetention))
                && let Some(position) = state
                    .retained
                    .iter()
                    .position(|candidate| Arc::ptr_eq(candidate, entry))
                && let Some(entry) = state.retained.remove(position)
            {
                discarded.push(entry);
            }
            if let Some(diagnostic) = diagnostic
                && !retiring
                && !entry
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .observed
            {
                state.diagnostics.push((Arc::clone(entry), diagnostic));
            }
            let capacity = match owner.config.capacity {
                CacheCapacity::Unbounded => None,
                CacheCapacity::Bounded(capacity) => Some(capacity.get()),
                CacheCapacity::NoRetention => Some(usize::default()),
            };
            if let Some(capacity) = capacity {
                let mut count = state
                    .retained
                    .iter()
                    .filter(|entry| {
                        entry
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .outcome
                            .is_some()
                    })
                    .count();
                while count > capacity {
                    let position = state.retained.iter().position(|entry| {
                        entry
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .outcome
                            .is_some()
                    });
                    if let Some(position) = position {
                        if let Some(entry) = state.retained.remove(position) {
                            discarded.push(entry);
                            count -= NonZeroUsize::MIN.get();
                        }
                    } else {
                        break;
                    }
                }
            }
            state.revision = Arc::new(());
            discarded
        };
        drop(discarded);
        notify_all(&owner.changed);
    }
    entry.cancellation.unlink();
    notify_all(&entry.changed);
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Cache<K, A, E> {
    /// Lazily construct a scoped cache with legacy fixed TTL and unbounded active admission.
    ///
    /// R supplies clock and task spawner. Retention still follows config.capacity and
    /// cache_failure. Use make_with_options for explicit active admission and outcome
    /// TTL. Scope closure cancels and joins owned producers.
    pub fn make<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        load: impl Fn(&K) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        let options = CacheOptions {
            ttl: CacheTtl::Fixed(config.ttl),
            admission: CacheAdmission::Unbounded,
        };
        Self::make_with_options(scope, config, options, load)
    }

    /// Lazily create a scoped cache with explicit retention, expiry and active admission.
    ///
    /// `R` supplies clock and task-spawner capabilities and is retained by the engine.
    /// Equivalent keys share one producer. Active capacity is checked before dispatch;
    /// outcome TTL and failure-retention callbacks run outside the state gate. The
    /// scope owns shutdown and joins producer cleanup, including abandonment.
    pub fn make_with_options<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
        config: CacheConfig<K, E>,
        options: CacheOptions<A, E>,
        load: impl Fn(&K) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<Self, SharedValue<E>, R> {
        let load = Arc::new(load);
        Effect::new(move |env: Arc<R>, context, finish| {
            let load_env = Arc::clone(&env);
            let now_env = env;
            let load = Arc::clone(&load);
            let cache = Self {
                inner: Arc::new(Inner {
                    state: Mutex::new(CacheState {
                        closed: false,
                        revision: Arc::new(()),
                        retained: VecDeque::new(),
                        active: Vec::new(),
                        diagnostics: Vec::new(),
                        shutdown: CacheShutdown::Open,
                        owner_loss: None,
                    }),
                    changed: Event::new(),
                    parent: context.cancellation.child(),
                    config: config.clone(),
                    options: options.clone(),
                    now: Arc::new(move || now_env.clock().now()),
                    start: Arc::new(move |entry, owner, context| {
                        let env = Arc::clone(&load_env);
                        let load = Arc::clone(&load);
                        let completed = Arc::clone(&entry);
                        let completed_owner = owner.clone();
                        let submission_owner = owner.clone();
                        let lost = Arc::clone(&entry);
                        let lost_owner = owner;
                        let submission = Arc::new(Mutex::new(Submission::Pending(None)));
                        let lost_submission = Arc::clone(&submission);
                        let task = ScheduledTask::new(
                            move |settlement| {
                                Box::pin(async move {
                                    let outcome = AssertUnwindSafe(async {
                                        let program = load(&completed.key);
                                        runtime::evaluate_context(program, env, &context).await
                                    })
                                    .catch_unwind()
                                    .await
                                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                                    settlement.release();
                                    settle(&completed, &completed_owner, outcome);
                                })
                            },
                            move |event| match event {
                                TaskEvent::Started => {}
                                TaskEvent::Lost(defect) => {
                                    let publish = {
                                        let mut admission = lost_submission
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                                        match &mut *admission {
                                            Submission::Pending(pending) => {
                                                *pending = Some(defect);
                                                None
                                            }
                                            Submission::Accepted => {
                                                *admission = Submission::Finished;
                                                Some(defect)
                                            }
                                            Submission::Finished => None,
                                        }
                                    };
                                    if let Some(defect) = publish {
                                        settle(&lost, &lost_owner, Exit::Die(defect));
                                    }
                                }
                            },
                        );
                        let submitted =
                            catch_unwind(AssertUnwindSafe(|| load_env.task_spawner().spawn(task)));
                        let failure = {
                            let mut admission = submission
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            match submitted {
                                Ok(Ok(())) => {
                                    match std::mem::replace(&mut *admission, Submission::Accepted) {
                                        Submission::Pending(pending) => pending,
                                        Submission::Accepted | Submission::Finished => None,
                                    }
                                }
                                Ok(Err(defect)) => {
                                    *admission = Submission::Finished;
                                    Some(defect)
                                }
                                Err(panic) => {
                                    *admission = Submission::Finished;
                                    Some(Defect::from_panic(panic))
                                }
                            }
                        };
                        if let Some(defect) = failure {
                            settle(&entry, &submission_owner, Exit::Die(defect));
                        }
                    }),
                }),
            };
            let close = cache.clone();
            let close_context = context.protected();
            let abandoned = cache.clone();
            let finalizer = Registered::with_abandon(
                move |finish| {
                    Step::Suspended(Box::pin(async move {
                        let cleanup = close.shutdown_owned(close_context).await;
                        runtime::finish(finish, cleanup)
                    }))
                },
                move || abandoned.abandon(),
            );
            scope.register_or_release(finalizer, cache, finish)
        })
    }
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) fn is_closed(&self) -> bool {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn abandon(&self) {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            state.owner_loss = Some(Defect::from(LibraryDefect::ScopeOwnerDropped));
            state.revision = Arc::new(());
            for entry in state.active.iter().chain(&state.retained) {
                entry
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .invalidated = true;
            }
        }
        let _ = self.inner.parent.request();
        notify_all(&self.inner.changed);
    }
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`Arc<K>`): Stable key used to locate or identify the selected value.
    /// - `context` (`&Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `Result<Option<Arc<Entry<K, A, E>>>, Defect>`.
    pub(crate) async fn select_entry(
        &self,
        key: Arc<K>,
        context: &Context,
    ) -> Result<Option<Arc<Entry<K, A, E>>>, CacheSelectionError<E>> {
        self.select_observed_entry(key, context)
            .await
            .map(|entry| entry.map(|(entry, _)| entry))
    }
    async fn select_observed_entry(
        &self,
        key: Arc<K>,
        context: &Context,
    ) -> Result<Option<(Arc<Entry<K, A, E>>, CacheObservation)>, CacheSelectionError<E>> {
        loop {
            if context.cancelled() {
                return Ok(None);
            }
            let (revision, entries) = {
                let state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    return Err(CacheSelectionError::Defect(Defect::from(
                        LibraryDefect::CacheClosed,
                    )));
                }
                (
                    Arc::clone(&state.revision),
                    state.retained.iter().map(Arc::clone).collect::<Vec<_>>(),
                )
            };
            let now = (self.inner.now)();
            let fresh = entries
                .into_iter()
                .filter(|entry| {
                    let state = entry
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    !state.invalidated && state.expires.is_none_or(|expires| now < expires)
                })
                .collect::<Vec<_>>();
            let found = fresh
                .iter()
                .find(|entry| (self.inner.config.equal)(&entry.key, &key))
                .map(Arc::clone);
            let admission_error = match &self.inner.options.admission {
                CacheAdmission::Unbounded => None,
                CacheAdmission::Bounded {
                    maximum,
                    capacity_error,
                } => Some((*maximum, Arc::clone(capacity_error))),
            };
            let selection = {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    return Err(CacheSelectionError::Defect(Defect::from(
                        LibraryDefect::CacheClosed,
                    )));
                }
                if !Arc::ptr_eq(&state.revision, &revision) {
                    None
                } else {
                    let old = std::mem::take(&mut state.retained);
                    state.retained = fresh.into();
                    let (entry, start) = match found {
                        Some(entry) => {
                            state
                                .retained
                                .retain(|candidate| !Arc::ptr_eq(candidate, &entry));
                            state.retained.push_back(Arc::clone(&entry));
                            (entry, false)
                        }
                        None if admission_error
                            .as_ref()
                            .is_some_and(|(maximum, _)| state.active.len() >= maximum.get()) =>
                        {
                            // Restore the snapshot; admission cannot evict entries.
                            state.retained = old;
                            drop(state);
                            if let Some((_, error)) = &admission_error {
                                return Err(CacheSelectionError::Capacity(SharedValue::new(
                                    error(),
                                )));
                            }
                            return Err(CacheSelectionError::Defect(Defect::from(
                                LibraryDefect::CacheClosed,
                            )));
                        }
                        None => {
                            let entry = Arc::new(Entry {
                                key: Arc::clone(&key),
                                state: Mutex::new(EntryState {
                                    outcome: None,
                                    expires: None,
                                    invalidated: false,
                                    retiring: false,
                                    observed: false,
                                }),
                                changed: Event::new(),
                                cancellation: self.inner.parent.child(),
                            });
                            state.active.push(Arc::clone(&entry));
                            state.retained.push_back(Arc::clone(&entry));
                            (entry, true)
                        }
                    };
                    state.revision = Arc::new(());
                    let observation = if start {
                        CacheObservation::Loaded
                    } else if entry
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .outcome
                        .is_some()
                    {
                        CacheObservation::Retained
                    } else {
                        CacheObservation::Shared
                    };
                    Some((entry, start, old, observation))
                }
            };
            if let Some((entry, start, old, observation)) = selection {
                drop(old);
                if start {
                    (self.inner.start)(
                        Arc::clone(&entry),
                        Arc::downgrade(&self.inner),
                        context.parallel(entry.cancellation.clone()),
                    );
                }
                return Ok(Some((entry, observation)));
            }
            cooperative_retry().await;
        }
    }
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `entry` (`&Arc<Entry<K, A, E>>`): Stored record or event selected for this transition.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `SharedOutcome<A, E>`.
    pub(crate) async fn observe(
        entry: &Arc<Entry<K, A, E>>,
        cancellation: Cancellation,
    ) -> SharedOutcome<A, E> {
        loop {
            let changed = entry.changed.listen();
            let result = {
                let state = entry
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if cancellation.is_requested() {
                    return Exit::Interrupted;
                }
                state.outcome.as_ref().map(|outcome| {
                    if state.invalidated {
                        outcome::append_failures(
                            Exit::Interrupted,
                            outcome::failures(outcome.clone()),
                        )
                    } else {
                        outcome.clone()
                    }
                })
            };
            if let Some(result) = result {
                return result;
            }
            match select(Box::pin(changed), Box::pin(cancellation.cancelled())).await {
                Either::Left(((), _)) => {}
                Either::Right(((), _)) => return Exit::Interrupted,
            }
        }
    }
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `entry` (`&Arc<Entry<K, A, E>>`): Stored record or event selected for this transition.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `SharedOutcome<A, E>`.
    pub(crate) async fn observe_value(
        &self,
        entry: &Arc<Entry<K, A, E>>,
        cancellation: Cancellation,
    ) -> SharedOutcome<A, E> {
        let result = Self::observe(entry, cancellation).await;
        if matches!(result, Exit::FinalizationFailed(_)) {
            entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .observed = true;
            let discarded = {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut discarded = Vec::new();
                state.diagnostics.retain(|(candidate, outcome)| {
                    if Arc::ptr_eq(candidate, entry) {
                        discarded.push(outcome.clone());
                        false
                    } else {
                        true
                    }
                });
                if !discarded.is_empty() {
                    state.revision = Arc::new(());
                }
                discarded
            };
            drop(discarded);
        }
        result
    }
    /// Lazily get or load one key, sharing producer work and successful/error ownership for equivalent keys; use get_observed for acquisition classification.
    pub fn get<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<SharedValue<A>, SharedValue<E>, R> {
        self.get_shared(Arc::new(key))
    }

    /// Lazily acquire a key and report Retained, Shared or Loaded with its shared success.
    ///
    /// Classification is selected with the cache entry; all equivalent waiters observe
    /// one producer outcome. Cancelling one observer does not cancel work still owned
    /// by other observers or retention. Typed loader errors are shared, not cloned.
    #[doc(alias = "getObserved")]
    pub fn get_observed<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<CacheLookup<A>, SharedValue<E>, R> {
        let key = Arc::new(key);
        let cache = self.clone();
        Effect::new(move |_, context, finish| {
            let cache = cache.clone();
            let key = Arc::clone(&key);
            Step::Suspended(Box::pin(async move {
                let work = async {
                    match cache.select_observed_entry(key, &context).await {
                        Ok(Some((entry, observation))) => match cache
                            .observe_value(&entry, context.cancellation.clone())
                            .await
                        {
                            Exit::Success(value) => {
                                Exit::Success(CacheLookup { value, observation })
                            }
                            Exit::Fail(error) => Exit::Fail(error),
                            Exit::Die(defect) => Exit::Die(defect),
                            Exit::Interrupted => Exit::Interrupted,
                            Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
                        },
                        Ok(None) => Exit::Interrupted,
                        Err(error) => error.exit(),
                    }
                };
                let result = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, result)
            }))
        })
    }

    /// Lazily inspect only a fresh retained success without registering or dispatching work.
    ///
    /// Returns None for absent, expired, active or failed entries. Payload handles share
    /// ownership; this call never joins an active producer.
    pub fn try_get<R: Send + Sync + 'static>(
        &self,
        key: K,
    ) -> Effect<Option<SharedValue<A>>, SharedValue<E>, R> {
        let cache = self.clone();
        let key = Arc::new(key);
        Effect::new(move |_, context, finish| {
            let cache = cache.clone();
            let key = Arc::clone(&key);
            Step::Suspended(Box::pin(async move {
                let work = async {
                    loop {
                        if context.cancelled() {
                            return Exit::Interrupted;
                        }
                        let (revision, entries, closed) = {
                            let state = cache
                                .inner
                                .state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            (
                                Arc::clone(&state.revision),
                                state.retained.iter().cloned().collect::<Vec<_>>(),
                                state.closed,
                            )
                        };
                        if closed {
                            return Exit::Die(Defect::from(LibraryDefect::CacheClosed));
                        }
                        let now = (cache.inner.now)();
                        let found = entries
                            .iter()
                            .find(|entry| (cache.inner.config.equal)(&entry.key, &key));
                        let value = found.and_then(|entry| {
                            let state = entry
                                .state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if state.invalidated
                                || state.expires.is_some_and(|expires| now >= expires)
                            {
                                None
                            } else {
                                match &state.outcome {
                                    Some(Exit::Success(value)) => Some(value.clone()),
                                    _ => None,
                                }
                            }
                        });
                        let current = {
                            let state = cache
                                .inner
                                .state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            Arc::ptr_eq(&revision, &state.revision)
                        };
                        if current {
                            return Exit::Success(value);
                        }
                        cooperative_retry().await;
                    }
                };
                let result = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, result)
            }))
        })
    }

    pub(crate) fn get_shared<R: Send + Sync + 'static>(
        &self,
        key: Arc<K>,
    ) -> Effect<SharedValue<A>, SharedValue<E>, R> {
        let cache = self.clone();
        Effect::new(move |_, context, finish| {
            let cache = cache.clone();
            let key = Arc::clone(&key);
            Step::Suspended(Box::pin(async move {
                let work = async {
                    match cache.select_entry(key, &context).await {
                        Ok(Some(entry)) => {
                            cache
                                .observe_value(&entry, context.cancellation.clone())
                                .await
                        }
                        Ok(None) => Exit::Interrupted,
                        Err(error) => error.exit(),
                    }
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `selected` (`Vec<Arc<Entry<K, A, E>>>`): The selected supplied to this operation.
    /// - `earlier` (`Vec<SharedOutcome<A, E>>`): The earlier supplied to this operation.
    ///
    /// # Returns
    /// Type: `Exit<(), SharedValue<E>>`.
    /// Returns `Exit<(), SharedValue<E>>`.
    async fn retire(
        &self,
        selected: Vec<Arc<Entry<K, A, E>>>,
        earlier: Vec<SharedOutcome<A, E>>,
    ) -> Exit<(), SharedValue<E>> {
        for entry in &selected {
            let _ = entry.cancellation.request();
        }
        let mut cleanup = Exit::Success(());
        for outcome in earlier {
            cleanup = outcome::append_failures(cleanup, outcome::failures(outcome));
        }
        for entry in selected {
            loop {
                let changed = entry.changed.listen();
                let outcome = entry
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .outcome
                    .clone();
                if let Some(outcome) = outcome {
                    cleanup = outcome::append_failures(cleanup, outcome::failures(outcome));
                    break;
                }
                changed.await;
            }
        }
        cleanup
    }
    /// Mark the selected cached or reactive state stale for its consumers.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`K`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<(), SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn invalidate<R: Send + Sync + 'static>(&self, key: K) -> Effect<(), SharedValue<E>, R> {
        self.invalidate_shared(Arc::new(key))
    }

    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`Arc<K>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<(), SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub(crate) fn invalidate_shared<R: Send + Sync + 'static>(
        &self,
        key: Arc<K>,
    ) -> Effect<(), SharedValue<E>, R> {
        let equal = Arc::clone(&self.inner.config.equal);
        self.invalidate_matching(move |candidate, _, _| equal(candidate, &key))
    }

    pub(crate) fn invalidate_matching<R: Send + Sync + 'static>(
        &self,
        predicate: impl Fn(&K, Option<SharedValue<A>>, bool) -> bool + Send + Sync + 'static,
    ) -> Effect<(), SharedValue<E>, R> {
        let predicate = Arc::new(predicate);
        let cache = self.clone();
        Effect::new(move |_, _, finish| {
            let cache = cache.clone();
            let predicate = Arc::clone(&predicate);
            Step::Suspended(Box::pin(async move {
                let work = async {
                    loop {
                        let (revision, entries, diagnostics) = {
                            let state = cache
                                .inner
                                .state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            (
                                Arc::clone(&state.revision),
                                state
                                    .retained
                                    .iter()
                                    .chain(&state.active)
                                    .map(Arc::clone)
                                    .collect::<Vec<_>>(),
                                state
                                    .diagnostics
                                    .iter()
                                    .map(|(entry, _)| Arc::clone(entry))
                                    .collect::<Vec<_>>(),
                            )
                        };
                        let mut selected = Vec::new();
                        for entry in entries.into_iter().chain(diagnostics) {
                            let (value, pending) = {
                                let state = entry
                                    .state
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                (
                                    match &state.outcome {
                                        Some(Exit::Success(value)) => Some(value.clone()),
                                        _ => None,
                                    },
                                    state.outcome.is_none(),
                                )
                            };
                            if predicate(&entry.key, value, pending)
                                && !selected
                                    .iter()
                                    .any(|candidate| Arc::ptr_eq(candidate, &entry))
                            {
                                selected.push(entry);
                            }
                        }
                        let earlier = {
                            let mut state = cache
                                .inner
                                .state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if !Arc::ptr_eq(&state.revision, &revision) {
                                None
                            } else {
                                for entry in &selected {
                                    let mut entry_state = entry
                                        .state
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                                    entry_state.invalidated = true;
                                    entry_state.retiring = true;
                                }
                                state.retained.retain(|entry| {
                                    !selected
                                        .iter()
                                        .any(|candidate| Arc::ptr_eq(candidate, entry))
                                });
                                let diagnostics = std::mem::take(&mut state.diagnostics);
                                let mut earlier = Vec::new();
                                for (entry, outcome) in diagnostics {
                                    if selected
                                        .iter()
                                        .any(|candidate| Arc::ptr_eq(candidate, &entry))
                                    { /* selected canonical outcomes are joined below */
                                    } else {
                                        state.diagnostics.push((entry, outcome));
                                    }
                                }
                                state.revision = Arc::new(());
                                Some(std::mem::take(&mut earlier))
                            }
                        };
                        if let Some(earlier) = earlier {
                            return cache.retire(selected, earlier).await;
                        }
                        cooperative_retry().await;
                    }
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
        .uninterruptible()
    }
    /// Lazily evict all entries, fence active generations, and join their owned cleanup.
    ///
    /// Eviction commits before cancellation/destruction outside the gate. Cleanup
    /// failures remain in the effect outcome; the cache remains open for later loads.
    pub fn clear<R: Send + Sync + 'static>(&self) -> Effect<(), SharedValue<E>, R> {
        self.invalidate_matching(|_, _, _| true)
    }

    /// Lazily replace a key with one owned success, fencing older producer generations.
    ///
    /// Publishes the replacement under the gate, then cancels and joins old work
    /// outside it. A late producer cannot overwrite the new value. Retention/TTL rules
    /// still apply, and the stored input is consumed once without cloning.
    pub fn set<R: Send + Sync + 'static>(&self, key: K, value: A) -> Effect<(), SharedValue<E>, R> {
        let cache = self.clone();
        let key = Arc::new(key);
        Effect::from_result_once(Ok(value)).flat_map(move |value| {
            let cache = cache.clone();
            let key = Arc::clone(&key);
            let value = Arc::new(Mutex::new(Some(value)));
            Effect::new(move |_, _, finish| {
                let cache = cache.clone();
                let key = Arc::clone(&key);
                let value = Arc::clone(&value);
                Step::Suspended(Box::pin(async move {
                    let work = async {
                        let value = value
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        let Some(value) = value else {
                            return Exit::Die(Defect::from(LibraryDefect::CacheClosed));
                        };
                        loop {
                            let (revision, entries, closed) = {
                                let state = cache
                                    .inner
                                    .state
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                (
                                    Arc::clone(&state.revision),
                                    state
                                        .retained
                                        .iter()
                                        .chain(&state.active)
                                        .chain(state.diagnostics.iter().map(|(entry, _)| entry))
                                        .cloned()
                                        .collect::<Vec<_>>(),
                                    state.closed,
                                )
                            };
                            if closed {
                                return Exit::Die(Defect::from(LibraryDefect::CacheClosed));
                            }
                            let mut selected = Vec::new();
                            for entry in entries {
                                if (cache.inner.config.equal)(&entry.key, &key)
                                    && !selected
                                        .iter()
                                        .any(|candidate| Arc::ptr_eq(candidate, &entry))
                                {
                                    selected.push(entry);
                                }
                            }
                            let installed = {
                                let mut state = cache
                                    .inner
                                    .state
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                if !Arc::ptr_eq(&revision, &state.revision) {
                                    None
                                } else {
                                    for entry in &selected {
                                        let mut state = entry
                                            .state
                                            .lock()
                                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                                        state.invalidated = true;
                                        state.retiring = true;
                                    }
                                    let mut diagnostics = Vec::new();
                                    let mut index = 0;
                                    while index < state.diagnostics.len() {
                                        if selected.iter().any(|entry| {
                                            state.diagnostics.get(index).is_some_and(
                                                |(candidate, _)| Arc::ptr_eq(entry, candidate),
                                            )
                                        }) {
                                            diagnostics.push(state.diagnostics.remove(index).1);
                                        } else {
                                            index += 1;
                                        }
                                    }
                                    state.retained.retain(|entry| {
                                        !selected
                                            .iter()
                                            .any(|candidate| Arc::ptr_eq(candidate, entry))
                                    });
                                    let entry = Arc::new(Entry {
                                        key: Arc::clone(&key),
                                        state: Mutex::new(EntryState {
                                            outcome: None,
                                            expires: None,
                                            invalidated: false,
                                            retiring: false,
                                            observed: true,
                                        }),
                                        changed: Event::new(),
                                        cancellation: cache.inner.parent.child(),
                                    });
                                    state.retained.push_back(Arc::clone(&entry));
                                    state.active.push(Arc::clone(&entry));
                                    state.revision = Arc::new(());
                                    Some((entry, diagnostics))
                                }
                            };
                            if let Some((entry, diagnostics)) = installed {
                                settle(&entry, &Arc::downgrade(&cache.inner), Exit::Success(value));
                                let published = entry
                                    .state
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .outcome
                                    .clone()
                                    .unwrap_or_else(|| {
                                        Exit::Die(Defect::from(
                                            LibraryDefect::MissingInterpreterOutcome,
                                        ))
                                    });
                                let retired = cache.retire(selected, diagnostics).await;
                                return outcome::append_cleanup(
                                    match published {
                                        Exit::Success(_) => Exit::Success(()),
                                        Exit::Fail(error) => Exit::Fail(error),
                                        Exit::Die(defect) => Exit::Die(defect),
                                        Exit::Interrupted => Exit::Interrupted,
                                        Exit::FinalizationFailed(details) => {
                                            Exit::FinalizationFailed(details)
                                        }
                                    },
                                    retired,
                                );
                            }
                            cooperative_retry().await;
                        }
                    };
                    let result = AssertUnwindSafe(work)
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    runtime::finish(finish, result)
                }))
            })
            .uninterruptible()
        })
    }

    async fn shutdown_start(&self) {
        let selected = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.shutdown {
                CacheShutdown::Open => {
                    state.closed = true;
                    state.shutdown = CacheShutdown::Closing;
                    state.revision = Arc::new(());
                    let selected = state.active.iter().map(Arc::clone).collect::<Vec<_>>();
                    for entry in &selected {
                        let mut entry_state = entry
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        entry_state.invalidated = true;
                        entry_state.retiring = true;
                    }
                    let earlier: Vec<_> = std::mem::take(&mut state.diagnostics)
                        .into_iter()
                        .filter(|(entry, _)| {
                            !selected
                                .iter()
                                .any(|candidate| Arc::ptr_eq(candidate, entry))
                        })
                        .map(|(_, outcome)| outcome)
                        .collect();
                    let retained = std::mem::take(&mut state.retained);
                    Some((selected, earlier, retained))
                }
                CacheShutdown::Closing | CacheShutdown::Closed(_) => None,
            }
        };
        if let Some((selected, earlier, retained)) = selected {
            let mut guard = CacheShutdownDriver {
                cache: self.clone(),
                settled: false,
            };
            for entry in retained {
                entry
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .invalidated = true;
            }
            let _ = self.inner.parent.request();
            let mut outcome = self.retire(selected, earlier).await;
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(defect) = state.owner_loss.take() {
                outcome = outcome::append_cleanup(outcome, Exit::Die(defect));
            }
            state.shutdown = CacheShutdown::Closed(Some(outcome));
            drop(state);
            guard.settled = true;
            self.inner.parent.unlink();
            notify_all(&self.inner.changed);
        } else {
            loop {
                let changed = self.inner.changed.listen();
                if matches!(
                    self.inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .shutdown,
                    CacheShutdown::Closed(_)
                ) {
                    break;
                }
                changed.await;
            }
        }
    }
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Exit<(), SharedValue<E>>`.
    /// Returns `Exit<(), SharedValue<E>>`.
    async fn shutdown_owned(&self, _: Context) -> Exit<(), SharedValue<E>> {
        self.shutdown_start().await;
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut state.shutdown {
            CacheShutdown::Closed(outcome) => outcome.take().unwrap_or(Exit::Success(())),
            CacheShutdown::Open | CacheShutdown::Closing => {
                Exit::Die(Defect::from(LibraryDefect::CacheShutdownDriverDropped))
            }
        }
    }
    /// Stop accepting new work and describe the settlement of owned work.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<crate::PoolEndStatus, SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn shutdown<R: Send + Sync + 'static>(
        &self,
    ) -> Effect<crate::PoolEndStatus, SharedValue<E>, R> {
        let cache = self.clone();
        Effect::new(move |_, _, finish| {
            let cache = cache.clone();
            Step::Suspended(Box::pin(async move {
                cache.shutdown_start().await;
                let status = {
                    let state = cache
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    match &state.shutdown {
                        CacheShutdown::Closed(Some(Exit::Success(())))
                        | CacheShutdown::Closed(None) => crate::PoolEndStatus::Clean,
                        CacheShutdown::Open
                        | CacheShutdown::Closing
                        | CacheShutdown::Closed(Some(
                            Exit::Fail(_)
                            | Exit::Die(_)
                            | Exit::Interrupted
                            | Exit::FinalizationFailed(_),
                        )) => crate::PoolEndStatus::Failed,
                    }
                };
                runtime::finish(finish, Exit::Success(status))
            }))
        })
        .uninterruptible()
    }
}
/// Typed cache shutdown driver used by cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct CacheShutdownDriver<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    cache: Cache<K, A, E>,
    settled: bool,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Drop
    for CacheShutdownDriver<K, A, E>
{
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if !self.settled {
            self.cache
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .shutdown = CacheShutdown::Closed(Some(Exit::Die(Defect::from(
                LibraryDefect::CacheShutdownDriverDropped,
            ))));
            notify_all(&self.cache.inner.changed);
        }
    }
}

impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Entry<K, A, E> {
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `()`.
    pub(crate) fn invalidate_association(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .invalidated = true;
    }
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) fn is_invalidated(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .invalidated
    }
}

// One handoff per failed optimistic selection; no arbitrary retry batch.
///
/// # Returns
/// Type: `()`.
async fn cooperative_retry() {
    let mut yielded = false;
    std::future::poll_fn(|cx| {
        if yielded {
            std::task::Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
    .await;
}
