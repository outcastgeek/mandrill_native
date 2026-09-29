use effect::persistence::{
    BatchReadItem, CorruptCacheRecord, HasKeyValueStore, KeyValueStore, MemoryKeyValueStore,
    MemoryStoreError, Persistable, PersistedCache, PersistedCacheError, PersistedCacheMemory,
    PersistedCachePolicy, RecordVersion, StoreCapacity, StoreEntry, StoreRead, StoreRevision,
    StoredValue,
};
use effect::{
    CacheCapacity, Defect, Effect, Exit, HasClock, HasTaskSpawner, HasWallClock, KeyDomain,
    KeyNamespace, KeySegment, ParseLimits, ScheduledTask, Scope, SharedValue, StoredOutcome,
    TaskSpawner, TestClock, TestWallClock, UtcMillis, json_i64,
};
use futures_executor::block_on;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

#[path = "support/suspended.rs"]
mod suspension;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "persisted-cache-invalidation";
}

struct TestTasks;
impl TaskSpawner for TestTasks {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        std::thread::Builder::new()
            .spawn(move || {
                block_on(task);
            })
            .map(|_| ())
            .map_err(|error| Defect::new(error.to_string()))
    }
}

struct Environment {
    tasks: TestTasks,
    clock: TestClock,
    wall: TestWallClock,
    store: Arc<MemoryKeyValueStore>,
}

impl HasTaskSpawner for Environment {
    type Tasks = TestTasks;
    fn task_spawner(&self) -> &Self::Tasks {
        &self.tasks
    }
}
impl HasClock for Environment {
    type Clock = TestClock;
    fn clock(&self) -> &Self::Clock {
        &self.clock
    }
}
impl HasWallClock for Environment {
    type Clock = TestWallClock;
    fn wall_clock(&self) -> &Self::Clock {
        &self.wall
    }
}
impl HasKeyValueStore for Environment {
    type Store = MemoryKeyValueStore;
    fn key_value_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.store)
    }
}

type CacheError = PersistedCacheError<i64, MemoryStoreError, Infallible>;

fn environment() -> Arc<Environment> {
    Arc::new(Environment {
        tasks: TestTasks,
        clock: TestClock::default(),
        wall: TestWallClock::new(UtcMillis::new(0)),
        store: Arc::new(MemoryKeyValueStore::new(StoreCapacity::Unbounded)),
    })
}

fn definition() -> Persistable<usize, i64, i64, Domain> {
    definition_with_segments(|key| Ok(vec![KeySegment::Text(Arc::from(key.to_string()))]))
}

fn definition_with_segments(
    segments: impl Fn(&usize) -> Result<Vec<KeySegment>, effect::KeyError> + Send + Sync + 'static,
) -> Persistable<usize, i64, i64, Domain> {
    let limits = ParseLimits {
        bytes: NonZeroUsize::MAX,
        depth: NonZeroUsize::MAX,
        nodes: NonZeroUsize::MAX,
        collection: NonZeroUsize::MAX,
        issues: NonZeroUsize::MAX,
    };
    Persistable::<usize, i64, i64, Domain>::new(
        KeyNamespace::new("cache").expect("valid namespace"),
        RecordVersion::new("cache-v1").expect("valid record version"),
        segments,
        json_i64(),
        json_i64(),
        limits,
    )
}

#[test]
fn memory_entries_follow_stable_identity() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let definition = definition_with_segments(|key| {
        Ok(vec![KeySegment::Text(Arc::from((key % 2).to_string()))])
    });
    let program: Effect<(i64, i64, i64), SharedValue<CacheError>, Environment> =
        Scope::scoped(move |scope| {
            PersistedCache::make(scope, cache_config(), definition.clone(), cache_policy(), {
                let counted = Arc::clone(&counted);
                move |key| {
                    let counted = Arc::clone(&counted);
                    let value = i64::try_from(*key).expect("fixture key fits i64");
                    Effect::<i64, i64, Environment>::sync(move || {
                        counted.fetch_add(1, Ordering::SeqCst);
                        value
                    })
                }
            })
            .flat_map(|cache: PersistedCache<_, _, _, _, _, Domain>| {
                cache.get(1).flat_map(move |first| {
                    let first = first.with(|value| *value).expect("read first");
                    let cache = cache.clone();
                    cache.get(3).flat_map(move |same_identity| {
                        let same_identity = same_identity.with(|value| *value).expect("read alias");
                        cache.get(2).map(move |different_identity| {
                            (
                                first,
                                same_identity,
                                different_identity
                                    .with(|value| *value)
                                    .expect("read distinct"),
                            )
                        })
                    })
                })
            })
        });
    assert!(matches!(
        block_on(program.run(environment())),
        Exit::Success((1, 1, 2))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

fn cache_config() -> PersistedCacheMemory<CacheError> {
    PersistedCacheMemory {
        ttl: None,
        capacity: CacheCapacity::Unbounded,
        cache_failure: Arc::new(|_| false),
    }
}

fn cache_policy() -> PersistedCachePolicy {
    PersistedCachePolicy {
        persistent_ttl: None,
        persist_failures: false,
        corrupt_record: CorruptCacheRecord::Reject,
    }
}

#[test]
fn invalidation_removes_both_scoped_and_persisted_values() {
    let environment = environment();
    let definition = definition();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&calls);
    let program: Effect<(i64, i64), SharedValue<CacheError>, Environment> =
        Scope::scoped(move |scope| {
            PersistedCache::make(scope, cache_config(), definition.clone(), cache_policy(), {
                let count = Arc::clone(&count);
                move |_| {
                    let count = Arc::clone(&count);
                    Effect::<i64, i64, Environment>::sync(move || {
                        i64::try_from(count.fetch_add(1, Ordering::SeqCst) + 1)
                            .expect("fixture load count fits i64")
                    })
                }
            })
            .flat_map(|cache: PersistedCache<_, _, _, _, _, Domain>| {
                cache.get(1).flat_map(move |first| {
                    let cache = cache.clone();
                    cache.invalidate(1).flat_map(move |_| {
                        let first = first
                            .with(|value| *value)
                            .expect("first value is inspectable");
                        cache.get(1).map(move |second| {
                            (
                                first,
                                second
                                    .with(|value| *value)
                                    .expect("second value is inspectable"),
                            )
                        })
                    })
                })
            })
        });
    assert!(matches!(
        block_on(program.run(environment)),
        Exit::Success((1, 2))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn domain_failure_releases_key_for_invalidation_and_retry() {
    let calls = Arc::new(AtomicUsize::new(0));
    let invoked = Arc::clone(&calls);
    let program: Effect<bool, SharedValue<CacheError>, Environment> = Scope::scoped(move |scope| {
        PersistedCache::make(scope, cache_config(), definition(), cache_policy(), {
            let invoked = Arc::clone(&invoked);
            move |_| {
                let invoked = Arc::clone(&invoked);
                Effect::<i64, i64, Environment>::fail_with(move || {
                    invoked.fetch_add(1, Ordering::SeqCst);
                    7
                })
            }
        })
        .flat_map(|cache: PersistedCache<_, _, _, _, _, Domain>| {
            cache.get(1).either().flat_map(move |first| {
                let first_failed = first.err().is_some_and(|error| {
                    error
                        .with(|value| matches!(value, CacheError::Domain(7)))
                        .unwrap_or(false)
                });
                let cache = cache.clone();
                cache.invalidate(1).flat_map(move |_| {
                    cache.get(1).either().map(move |second| {
                        first_failed
                            && second.err().is_some_and(|error| {
                                error
                                    .with(|value| matches!(value, CacheError::Domain(7)))
                                    .unwrap_or(false)
                            })
                    })
                })
            })
        })
    });
    assert!(matches!(
        block_on(program.run(environment())),
        Exit::Success(true)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[derive(Default)]
struct ManualTasks(Mutex<VecDeque<ScheduledTask>>);
impl TaskSpawner for ManualTasks {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(task);
        Ok(())
    }
}
impl ManualTasks {
    fn pop(&self) -> Option<ScheduledTask> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
    }
    fn is_empty(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }
}

struct RemovalGateStore {
    backing: Arc<MemoryKeyValueStore>,
    entered: mpsc::Sender<()>,
    release: Arc<Mutex<mpsc::Receiver<()>>>,
}
impl KeyValueStore for RemovalGateStore {
    type Error = MemoryStoreError;
    fn get(&self, key: Arc<[u8]>, now: UtcMillis) -> Effect<StoreRead, Self::Error> {
        self.backing.get(key, now)
    }
    fn get_many(
        &self,
        keys: Vec<Arc<[u8]>>,
        now: UtcMillis,
    ) -> Effect<Vec<BatchReadItem<Self::Error>>, Self::Error> {
        self.backing.get_many(keys, now)
    }
    fn put(&self, key: Arc<[u8]>, value: StoredValue) -> Effect<StoreRevision, Self::Error> {
        self.backing.put(key, value)
    }
    fn remove(&self, key: Arc<[u8]>) -> Effect<Option<StoreEntry>, Self::Error> {
        let entered = self.entered.clone();
        let release = Arc::clone(&self.release);
        let backing = Arc::clone(&self.backing);
        Effect::<(), Self::Error>::sync_once(move || {
            entered.send(()).expect("removal entered signal");
            release
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .recv()
                .expect("removal release signal");
        })
        .flat_map(move |_| backing.remove(Arc::clone(&key)))
    }
    fn cleanup_expired(&self, now: UtcMillis) -> Effect<usize, Self::Error> {
        self.backing.cleanup_expired(now)
    }
}

struct RaceEnvironment {
    tasks: ManualTasks,
    clock: TestClock,
    wall: TestWallClock,
    store: Arc<RemovalGateStore>,
}
impl HasTaskSpawner for RaceEnvironment {
    type Tasks = ManualTasks;
    fn task_spawner(&self) -> &Self::Tasks {
        &self.tasks
    }
}
impl HasClock for RaceEnvironment {
    type Clock = TestClock;
    fn clock(&self) -> &Self::Clock {
        &self.clock
    }
}
impl HasWallClock for RaceEnvironment {
    type Clock = TestWallClock;
    fn wall_clock(&self) -> &Self::Clock {
        &self.wall
    }
}
impl HasKeyValueStore for RaceEnvironment {
    type Store = RemovalGateStore;
    fn key_value_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.store)
    }
}

#[test]
fn lookup_started_during_backing_removal_cannot_install_stale_value() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let entered_rx = Arc::new(Mutex::new(entered_rx));
    let (release_tx, release_rx) = mpsc::channel();
    let backing = Arc::new(MemoryKeyValueStore::new(StoreCapacity::Unbounded));
    let store = Arc::new(RemovalGateStore {
        backing: Arc::clone(&backing),
        entered: entered_tx,
        release: Arc::new(Mutex::new(release_rx)),
    });
    let environment = Arc::new(RaceEnvironment {
        tasks: ManualTasks::default(),
        clock: TestClock::default(),
        wall: TestWallClock::new(UtcMillis::new(0)),
        store,
    });
    let key_calls = Arc::new(AtomicUsize::new(0));
    let identity_calls = Arc::clone(&key_calls);
    let definition = Persistable::<usize, i64, i64, Domain>::new(
        KeyNamespace::new("cache").expect("valid namespace"),
        RecordVersion::new("cache-v1").expect("valid record version"),
        move |key| {
            identity_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![KeySegment::Text(Arc::from(key.to_string()))])
        },
        json_i64(),
        json_i64(),
        ParseLimits {
            bytes: NonZeroUsize::MAX,
            depth: NonZeroUsize::MAX,
            nodes: NonZeroUsize::MAX,
            collection: NonZeroUsize::MAX,
            issues: NonZeroUsize::MAX,
        },
    );
    let stable_key = definition.key(&1).expect("valid record key").into_bytes();
    let stale = definition
        .encode(&StoredOutcome::Success(41))
        .expect("valid stale record");
    assert!(matches!(
        block_on(
            backing
                .put(
                    stable_key,
                    StoredValue {
                        bytes: stale,
                        expires_at: None,
                    }
                )
                .run(Arc::new(()))
        ),
        Exit::Success(_)
    ));
    let run_environment = Arc::clone(&environment);
    let program: Effect<i64, SharedValue<CacheError>, RaceEnvironment> =
        Scope::scoped(move |scope| {
            let entered_rx = Arc::clone(&entered_rx);
            let release_tx = release_tx.clone();
            PersistedCache::make(
                scope,
                cache_config(),
                definition.clone(),
                cache_policy(),
                |_| Effect::<i64, i64, RaceEnvironment>::sync(|| 99),
            )
            .flat_map({
                let environment = Arc::clone(&run_environment);
                let key_calls = Arc::clone(&key_calls);
                move |cache: PersistedCache<_, _, _, _, _, Domain>| {
                    let environment = Arc::clone(&environment);
                    let key_calls = Arc::clone(&key_calls);
                    let entered_rx = Arc::clone(&entered_rx);
                    let release_tx = release_tx.clone();
                    Effect::sync_once(move || {
                        std::thread::spawn(move || {
                            let invalidating = {
                                let cache = cache.clone();
                                let environment = Arc::clone(&environment);
                                std::thread::spawn(move || {
                                    block_on(cache.invalidate(1).run(environment))
                                })
                            };
                            entered_rx
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .recv()
                                .expect("invalidation reached backing removal");
                            let mut lookup = Box::pin(cache.get(1).run(Arc::clone(&environment)));
                            suspension::suspended(lookup.as_mut());
                            assert!(
                                key_calls.load(Ordering::SeqCst) >= 3,
                                "lookup reached its identity boundary"
                            );
                            assert!(
                                environment.tasks.is_empty(),
                                "lookup may not start a stale load during removal"
                            );
                            release_tx.send(()).expect("release backing removal");
                            assert!(matches!(
                                invalidating.join().expect("invalidation joined"),
                                Exit::Success(())
                            ));
                            suspension::suspended(lookup.as_mut());
                            let task = environment.tasks.pop().expect("fresh loader submitted");
                            block_on(task);
                            match block_on(lookup) {
                                Exit::Success(value) => {
                                    value.with(|value| *value).expect("fresh value inspectable")
                                }
                                _ => panic!("lookup after invalidation must succeed"),
                            }
                        })
                        .join()
                        .expect("race orchestration joined")
                    })
                }
            })
        });
    assert!(matches!(
        block_on(program.run(environment)),
        Exit::Success(99)
    ));
}
