//! Weakly held cached values and partitioned identities.

use crate::cache::Entry;
use crate::internal::runtime::{self, Step};
use crate::{
    Cache, CacheCapacity, CacheConfig, Defect, Effect, Exit, HasClock, HasTaskSpawner,
    LibraryDefect, Release, Scope, SharedValue,
};
use crate::{
    effect, effect_all, effect_die, effect_finalizer, effect_map, effect_provide_some,
    effect_uninterruptible,
};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, PartialEq, Eq)]
/// A key partition and its weakly associated cached values.
pub enum CachePartition {
    /// Use the unnamed default partition.
    Default,
    /// Use a caller-supplied named partition. Contains `Arc<str>`.
    Named(Arc<str>),
}
/// Stable identity used by weak cache operations.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Key<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    identity: Weak<K>,
    partition: CachePartition,
    load: Mutex<Option<Effect<A, E, ()>>>,
}
/// Type alias for associations in weak cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup.
/// - `A`: Application value type.
/// - `E`: Domain-error type.
type Associations<K, A, E> = Mutex<Vec<Weak<Entry<Key<K, A, E>, A, E>>>>;
/// Typed inner used by weak cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Inner<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    cache: Cache<Key<K, A, E>, A, E>,
    entries: Associations<K, A, E>,
    closed: AtomicBool,
}
/// A cache whose key and association ownership can disappear when consumers release them.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct WeakCache<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    inner: Arc<Inner<K, A, E>>,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Clone for WeakCache<K, A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
/// Consumer ownership keeps a settled weak-cache association alive. The cache
/// itself has weak key AND weak entry references. This is deliberately weaker
/// retention than a tracing-GC ephemeron: dropping the last consumer handle may
/// cause a miss even while the key lives. A value-to-key reference cannot make
/// a successful settled association keep itself alive through the cache.
///
/// # Type parameters
/// - `K`: Key type used for identity or lookup. Bound or type: `Send + Sync + 'static`.
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct WeakCacheValue<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> {
    entry: Arc<Entry<Key<K, A, E>, A, E>>,
    value: SharedValue<A>,
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> Clone
    for WeakCacheValue<K, A, E>
{
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            entry: Arc::clone(&self.entry),
            value: self.value.clone(),
        }
    }
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> WeakCacheValue<K, A, E> {
    /// Borrow the contained value only for the duration of the supplied callback.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `B`: Transformed value type.
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `inspect` (`impl FnOnce(&A) -> B`): The inspect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<B, Defect>`.
    pub fn with<B>(&self, inspect: impl FnOnce(&A) -> B) -> Result<B, Defect> {
        self.value.with(inspect)
    }
}
impl<K: Send + Sync + 'static, A: Send + 'static, E: Send + 'static> WeakCache<K, A, E> {
    /// Create a `WeakCache` through the declared Effect and environment capabilities.
    ///
    /// # Type parameters
    /// - `K`: Key type used for identity or lookup. Bound: `Send + Sync + 'static`.
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `HasClock + HasTaskSpawner + Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `scope` (`Scope<SharedValue<E>>`): Owner that joins child work and releases resources at close.
    ///
    /// # Returns
    /// Type: `Effect<Self, SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make<R: HasClock + HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<SharedValue<E>>,
    ) -> Effect<Self, SharedValue<E>, R> {
        let config = CacheConfig {
            equal: Arc::new(|left: &Key<K, A, E>, right: &Key<K, A, E>| {
                left.identity.ptr_eq(&right.identity) && left.partition == right.partition
            }),
            ttl: None,
            capacity: CacheCapacity::NoRetention,
            cache_failure: Arc::new(|_| false),
        };
        let build = Cache::make(scope.clone(), config, |key: &Key<K, A, E>| {
            let program = key
                .load
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match program {
                Some(program) => effect_provide_some! { program: program, project: |_: &R| (), },
                None => effect_die! { defect: Defect::from(LibraryDefect::WeakCacheLoadConsumed), },
            }
        });
        {
            let __dsl_program = build;
            effect! { let scope = scope.clone(); let cache = __dsl_program.clone()?; {
                let cache = Self {
                    inner: Arc::new(Inner {
                        cache,
                        entries: Mutex::new(Vec::new()),
                        closed: AtomicBool::new(false),
                    }),
                };
                let close = cache.clone();
                let registered = effect_finalizer! {
                    release;
                    scope: scope,
                    release: move || {
                    let close = close.clone();
                    Release::sync(move || close.close_index())
                    },
                };
                effect_map! { program: registered, map: move |()| cache.clone(), }
            }? }
        }
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn close_index(&self) {
        self.inner.closed.store(true, Ordering::Release);
        let entries = std::mem::take(
            &mut *self
                .inner
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for entry in entries.into_iter().filter_map(|entry| entry.upgrade()) {
            entry.invalidate_association();
        }
    }
    /// Read the current value through the declared storage or state capability.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    /// - `K`: Type supplied by the enclosing impl or trait.
    ///
    /// # Parameters
    /// - `key` (`&Arc<K>`): Stable key used to locate or identify the selected value.
    /// - `load` (`impl Fn() -> Effect<A, E, R> + Send + Sync + 'static`): The load supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<WeakCacheValue<K, A, E>, SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn get<R: Send + Sync + 'static>(
        &self,
        key: &Arc<K>,
        load: impl Fn() -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<WeakCacheValue<K, A, E>, SharedValue<E>, R> {
        self.get_partition(CachePartition::Default, key, load)
    }
    /// Lazily acquire a weakly indexed value in this partition using the existing cache engine.
    ///
    /// Live ownership reuses producer outcomes without copying payloads. Selection
    /// preserves typed capacity rejection, defects and interruption; weak indexing does
    /// not introduce a second retention or task-provider mechanism.
    pub fn get_partition<R: Send + Sync + 'static>(
        &self,
        partition: CachePartition,
        key: &Arc<K>,
        load: impl Fn() -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<WeakCacheValue<K, A, E>, SharedValue<E>, R> {
        let cache = self.clone();
        let identity = Arc::downgrade(key);
        let load = Arc::new(load);
        Effect::new(move |env, context, finish| {
            let cache = cache.clone();
            let identity = identity.clone();
            let partition = partition.clone();
            let load = Arc::clone(&load);
            Step::Suspended(Box::pin(async move {
                let work = async {
                    if cache.inner.closed.load(Ordering::Acquire) || cache.inner.cache.is_closed() {
                        return Exit::Die(Defect::from(LibraryDefect::CacheClosed));
                    }
                    if identity.upgrade().is_none() {
                        return Exit::Die(Defect::from(LibraryDefect::WeakCacheKeyDropped));
                    }
                    let existing = {
                        let mut entries = cache
                            .inner
                            .entries
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        entries.retain(|entry| entry.strong_count() != usize::default());
                        entries.iter().filter_map(Weak::upgrade).find(|entry| {
                            entry.key.identity.ptr_eq(&identity)
                                && entry.key.partition == partition
                                && !entry.is_invalidated()
                        })
                    };
                    let entry = match existing {
                        Some(entry) => entry,
                        None => {
                            let recipe = Effect::defer(move || load()).provide(env);
                            let key = Arc::new(Key {
                                identity,
                                partition,
                                load: Mutex::new(Some(recipe)),
                            });
                            match cache.inner.cache.select_entry(key, &context).await {
                                Ok(Some(entry)) => {
                                    cache
                                        .inner
                                        .entries
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                                        .push(Arc::downgrade(&entry));
                                    entry
                                }
                                Ok(None) => return Exit::Interrupted,
                                Err(error) => return error.exit(),
                            }
                        }
                    };

                    match cache
                        .inner
                        .cache
                        .observe_value(&entry, context.cancellation)
                        .await
                    {
                        Exit::Success(value) => Exit::Success(WeakCacheValue { entry, value }),
                        Exit::Fail(error) => Exit::Fail(error),
                        Exit::Die(defect) => Exit::Die(defect),
                        Exit::Interrupted => Exit::Interrupted,
                        Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
                    }
                };
                let outcome =
                    futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(work))
                        .await
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
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
    /// - `key` (`&Arc<K>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<(), SharedValue<E>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn invalidate<R: Send + Sync + 'static>(
        &self,
        key: &Arc<K>,
    ) -> Effect<(), SharedValue<E>, R> {
        let cache = self.clone();
        let identity = Arc::downgrade(key);
        effect_uninterruptible! { program: { let __dsl_factory = move || {
            let selected = {
                let mut entries = cache
                    .inner
                    .entries
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let selected = entries
                    .iter()
                    .filter_map(Weak::upgrade)
                    .filter(|entry| entry.key.identity.ptr_eq(&identity))
                    .collect::<Vec<_>>();
                entries.retain(|entry| {
                    entry
                        .upgrade()
                        .is_some_and(|entry| !entry.key.identity.ptr_eq(&identity))
                });
                selected
            };
            let mut retire = Vec::new();
            for entry in selected {
                entry.invalidate_association();
                retire.push(cache.inner.cache.invalidate(Key {
                    identity: identity.clone(),
                    partition: entry.key.partition.clone(),
                    load: Mutex::new(None),
                }));
            }
            effect_map! { program: effect_all! { items: retire, }, map: |_| (), }
        }; effect! { __dsl_factory()? } }, }
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
        effect_uninterruptible! { program: { let __dsl_factory = move || {
            cache.close_index();
            cache.inner.cache.shutdown()
        }; effect! { __dsl_factory()? } }, }
    }
}
