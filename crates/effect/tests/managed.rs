extern crate effect as fx;
use effect::http::*;
use effect::websocket::*;
use effect::*;
use futures_executor::block_on;
use std::cell::Cell;
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
#[path = "support/drive.rs"]
mod drive;
#[path = "support/task_cases.rs"]
pub mod tasks;
#[derive(Default)]
struct Env {
    tasks: tasks::Threads,
    clock: TestClock,
    cancel: Cancellation,
}
impl HasTaskSpawner for Env {
    type Tasks = tasks::Threads;
    fn task_spawner(&self) -> &tasks::Threads {
        &self.tasks
    }
}
impl HasClock for Env {
    type Clock = TestClock;
    fn clock(&self) -> &TestClock {
        &self.clock
    }
}
impl HasCancellation for Env {
    fn cancellation(&self) -> &Cancellation {
        &self.cancel
    }
}
fn count() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}
fn nz(value: usize) -> NonZeroUsize {
    match NonZeroUsize::new(value) {
        Some(value) => value,
        None => NonZeroUsize::MIN,
    }
}
#[test]
fn cache_shares_owned_nonclone_values_then_invalidates_expires_and_evicts() {
    struct Value(Cell<usize>);
    let env = Arc::new(Env::default());
    let calls = count();
    let loads = Arc::clone(&calls);
    let program: Effect<(), SharedValue<()>, Env> = Scope::scoped(move |scope| {
        let loads = Arc::clone(&loads);
        effect! {
            let loads = Arc::clone(&loads);
            let cache = Cache::make(scope.clone(), CacheConfig { equal: Arc::new(|a: &usize,b| a==b), ttl: Some(Duration::from_secs(2)), capacity: CacheCapacity::Bounded(NonZeroUsize::MIN), cache_failure: Arc::new(|_| false) }, move |_| { let loads = Arc::clone(&loads); Effect::sync(move || Value(Cell::new(loads.fetch_add(1,Ordering::SeqCst)))) })?;
            let first = cache.get(1)?;
            let second = cache.get(1)?;
            assert_eq!(first.with(|value| std::ptr::from_ref(value) as usize), second.with(|value| std::ptr::from_ref(value) as usize));
            assert_eq!(first.with(|value| value.0.get()), Ok(0));
            cache.invalidate(1)?;
            let reloaded = cache.get(1)?;
            assert_eq!(reloaded.with(|value| value.0.get()), Ok(1));
            Effect::from_future_with_env(|env: Arc<Env>, _| env.clock.advance(Duration::from_secs(2)))?;
            let expired = cache.get(1)?;
            assert_eq!(expired.with(|value| value.0.get()), Ok(2));
            cache.get(2)?;
            let evicted = cache.get(1)?;
            assert_eq!(evicted.with(|value| value.0.get()), Ok(4));
        }
    });
    assert!(matches!(
        block_on(program.run(Arc::clone(&env))),
        Exit::Success(())
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    env.tasks.finish();
}
#[test]
fn concurrent_cache_misses_share_one_producer_and_observed_cleanup_is_not_retained() {
    let env = Arc::new(Env::default());
    let calls = count();
    let loads = Arc::clone(&calls);
    let program: Effect<(), SharedValue<()>, Env> = Scope::scoped(move |scope| {
        let loads = Arc::clone(&loads);
        effect! {
            let loads = Arc::clone(&loads);
            let cache = Cache::make(scope.clone(), CacheConfig { equal: Arc::new(|a: &usize,b| a==b), ttl: None, capacity: CacheCapacity::Unbounded, cache_failure: Arc::new(|_| false) }, move |_| { let loads = Arc::clone(&loads); Effect::sync(move || loads.fetch_add(1,Ordering::SeqCst)) })?;
            let results = Effect::all_concurrent(nz(4), (0..16).map(|_| cache.get(1)).collect())?;
            assert!(results.iter().all(|value| value.with(|value| *value) == Ok(0)));
        }
    });
    assert!(matches!(
        block_on(program.run(Arc::clone(&env))),
        Exit::Success(())
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let cleanup: Effect<(), SharedValue<()>, Env> = Scope::scoped(|scope| {
        effect! {
            let cache = Cache::make(scope.clone(), CacheConfig { equal: Arc::new(|a: &usize,b| a==b), ttl: None, capacity: CacheCapacity::NoRetention, cache_failure: Arc::new(|_| false) }, |_| Effect::sync(|| ()).ensuring(Effect::die(Defect::from(LibraryDefect::ProviderClosed))))?;
            let observed = Effect::from_future_with_env(move |env: Arc<Env>, _| { let cache = cache.clone(); async move { cache.get(1).run(env).await } })?;
            assert!(matches!(observed, Exit::FinalizationFailed(_)));
        }
    });
    assert!(matches!(
        block_on(cleanup.run(Arc::clone(&env))),
        Exit::Success(())
    ));
    env.tasks.finish();
}
#[test]
fn rejected_cache_submission_keeps_the_reported_provider_cause() {
    struct Reject;
    impl TaskSpawner for Reject {
        fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
            drop(task);
            Err(Defect::from(LibraryDefect::ProviderCapacityFull))
        }
    }
    struct Rejected(TestClock, Reject);
    impl HasClock for Rejected {
        type Clock = TestClock;
        fn clock(&self) -> &TestClock {
            &self.0
        }
    }
    impl HasTaskSpawner for Rejected {
        type Tasks = Reject;
        fn task_spawner(&self) -> &Reject {
            &self.1
        }
    }
    let program: Effect<(), SharedValue<()>, Rejected> = Scope::scoped(|scope| {
        Cache::make(
            scope.clone(),
            CacheConfig {
                equal: Arc::new(|a: &usize, b| a == b),
                ttl: None,
                capacity: CacheCapacity::Unbounded,
                cache_failure: Arc::new(|_| false),
            },
            |_| Effect::sync(|| ()),
        )
        .flat_map(|cache| cache.get(1).map(|_| ()))
    });
    assert!(
        matches!(block_on(program.run(Arc::new(Rejected(TestClock::default(),Reject)))), Exit::Die(defect) if defect.kind() == DefectKind::Library(LibraryDefect::ProviderCapacityFull))
    );
}
#[test]
fn weak_cache_never_retains_a_settled_value_to_key_cycle_and_partitions_are_separate() {
    let env = Arc::new(Env::default());
    let calls = count();
    let loads = Arc::clone(&calls);
    let program: Effect<(), SharedValue<()>, Env> = Scope::scoped(move |scope| {
        let loads = Arc::clone(&loads);
        effect! {
            let loads = Arc::clone(&loads);
            let key = Arc::new(String::from("key"));
            let weak = Arc::downgrade(&key);
            let cache = WeakCache::make(scope.clone())?;
            let source = Arc::clone(&key);
            let first = cache.get(&key, move || { let source = Arc::clone(&source); Effect::sync(move || Arc::clone(&source)) })?;
            let loads = Arc::clone(&loads);
            let partition = cache.get_partition(CachePartition::Named(Arc::from("other")), &key, move || { let loads = Arc::clone(&loads); Effect::sync(move || { loads.fetch_add(1,Ordering::SeqCst); Arc::new(String::from("separate")) }) })?;
            assert_eq!(partition.with(|value| value.as_str() == "separate"), Ok(true));
            drop(partition); drop(key); drop(first);
            assert!(weak.upgrade().is_none());
        }
    });
    assert!(matches!(
        block_on(program.run(Arc::clone(&env))),
        Exit::Success(())
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    env.tasks.finish();
}
#[test]
fn pool_population_is_independent_and_capacity_validation_ttl_and_disposal_own_resources() {
    struct Resource(usize);
    impl Drop for Resource {
        fn drop(&mut self) {}
    }
    let env = Arc::new(Env::default());
    let created = count();
    let disposed = count();
    let gate = Latch::new();
    let creating = Arc::clone(&created);
    let disposing = Arc::clone(&disposed);
    let wait = gate.clone();
    let program: Effect<(), (), Env> = Scope::scoped(move |scope| {
        let creating = Arc::clone(&creating);
        let disposing = Arc::clone(&disposing);
        let wait = wait.clone();
        let create = effect! { let creating = Arc::clone(&creating); let wait = wait.clone(); let resource = Effect::sync(move || Resource(creating.fetch_add(1,Ordering::SeqCst)))?; wait.await_open()?; resource };
        effect! {
            let disposing = Arc::clone(&disposing);
            let scope = scope.clone();
            let pool = Pool::make(scope.clone(), PoolCapacity::Bounded(nz(2)), Some(Duration::from_secs(2)), create.clone(), |resource: &Resource| { let valid = resource.0 != usize::MAX; Effect::sync(move || valid) }, move |resource| { let disposing = Arc::clone(&disposing); Effect::sync_once(move || { drop(resource); disposing.fetch_add(1,Ordering::SeqCst); }) })?;
            pool.ensure_size(2)?;
            let idle = pool.exists_idle(|_| true)?; assert!(idle);
            let lease = pool.acquire(scope.clone())?;
            assert!(lease.with(|resource| resource.0 < 2).unwrap_or(false));
            let status = lease.release()?; assert_eq!(status, PoolEndStatus::Clean);
            assert!(lease.with(|_| ()).is_err());
            Effect::from_future_with_env(|env: Arc<Env>, _| env.clock.advance(Duration::from_secs(2)))?;
            let replacement = pool.acquire(scope)?;
            assert!(replacement.with(|resource| resource.0 >= 2).unwrap_or(false));
            replacement.discard()?;
            pool.set_capacity(PoolCapacity::Bounded(NonZeroUsize::MIN))?;
        }
    });
    let mut run = Box::pin(program.run(Arc::clone(&env)));
    drive::until(run.as_mut(), || created.load(Ordering::SeqCst) == 2);
    assert_eq!(
        block_on(gate.open::<(), Env>().run(Arc::clone(&env))),
        Exit::Success(true)
    );
    assert_eq!(block_on(run), Exit::Success(()));
    assert_eq!(
        created.load(Ordering::SeqCst),
        disposed.load(Ordering::SeqCst)
    );
    env.tasks.finish();
}

struct Body {
    bytes: Mutex<Option<Vec<u8>>>,
    reads: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    fail_close: bool,
}
impl HttpBody for Body {
    fn read(&self, _: Cancellation) -> TransportFuture<Vec<u8>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let bytes = self
            .bytes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_default();
        Box::pin(async move { Ok(bytes) })
    }
    fn abort(&self) -> Result<(), Defect> {
        Ok(())
    }
    fn close(&self) -> TransportFuture<()> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        let failure = self.fail_close;
        Box::pin(async move {
            if failure {
                Err(Defect::from(LibraryDefect::ProviderClosed))
            } else {
                Ok(())
            }
        })
    }
}
struct HttpEnv {
    status: HttpStatus,
    bytes: &'static [u8],
    reads: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    fail_close: bool,
}
impl HttpTransport for HttpEnv {
    fn send(&self, _: HttpRequest, _: Cancellation) -> TransportFuture<HttpResponse> {
        let response = HttpResponse::new(
            self.status,
            HttpHeaders::new(),
            Arc::new(Body {
                bytes: Mutex::new(Some(self.bytes.to_vec())),
                reads: Arc::clone(&self.reads),
                closes: Arc::clone(&self.closes),
                fail_close: self.fail_close,
            }),
        );
        Box::pin(async move { Ok(response) })
    }
}
impl HasHttpTransport for HttpEnv {
    type Transport = Self;
    fn http_transport(&self) -> &Self {
        self
    }
}
#[test]
fn http_owns_rejected_and_invalid_responses_and_preserves_body_plus_cleanup_failure() {
    let request = HttpRequest {
        url: Arc::from("mock://input"),
        options: RequestOptions {
            method: HttpMethod::Get,
            headers: Arc::new(HttpHeaders::new()),
            body: None,
        },
    };
    for (status, bytes, fail_close) in [
        (HttpStatus(200), &b"body"[..], false),
        (HttpStatus(500), &b"ignored"[..], false),
        (HttpStatus(200), &b"\xff"[..], true),
    ] {
        let env = Arc::new(HttpEnv {
            status,
            bytes,
            reads: count(),
            closes: count(),
            fail_close,
        });
        let program = fetch_text(
            request.clone(),
            |status| status == HttpStatus(200),
            |error| error,
        );
        let outcome = block_on(program.run(Arc::clone(&env)));
        match (status, fail_close) {
            (HttpStatus(200), false) => {
                assert!(matches!(outcome, Exit::Success(text) if text=="body"))
            }
            (HttpStatus(200), true) => assert!(matches!(
                outcome,
                Exit::FinalizationFailed(Finalization {
                    primary: PrimaryOutcome::Failed(FetchFailure::Decode(_)),
                    first: CleanupFailure::ReleaseDefect(_),
                    ..
                })
            )),
            _ => assert!(matches!(
                outcome,
                Exit::Fail(FetchFailure::Http(HttpStatus(500)))
            )),
        }
        assert_eq!(env.closes.load(Ordering::SeqCst), 1);
        assert_eq!(
            env.reads.load(Ordering::SeqCst),
            usize::from(status == HttpStatus(200))
        );
    }
}
struct Socket {
    frames: Mutex<VecDeque<WebSocketFrame>>,
    sent: Arc<AtomicUsize>,
    disposed: Arc<AtomicUsize>,
    aborted: Arc<AtomicUsize>,
    abort: Cancellation,
}
impl WebSocket for Socket {
    fn send(&self, _: WebSocketMessage, _: Cancellation) -> TransportFuture<()> {
        self.sent.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
    fn receive(&self, _: NonZeroUsize, _: Cancellation) -> TransportFuture<WebSocketFrame> {
        let frame = self
            .frames
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        Box::pin(async move { frame.ok_or_else(|| Defect::from(LibraryDefect::ProviderClosed)) })
    }
    fn close(&self, _: Cancellation) -> TransportFuture<()> {
        let aborted = self.abort.clone();
        Box::pin(async move {
            aborted.cancelled().await;
            Ok(())
        })
    }
    fn abort(&self) -> Result<(), Defect> {
        self.aborted.fetch_add(1, Ordering::SeqCst);
        let _ = self.abort.request();
        Ok(())
    }
    fn dispose(&self) -> TransportFuture<()> {
        self.disposed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
    fn subprotocol(&self) -> Option<Arc<str>> {
        None
    }
}
#[test]
fn websocket_assembles_fragments_bounds_messages_and_disposes_after_close_timeout() {
    let env = Arc::new(Env::default());
    let disposed = count();
    let sent = count();
    let aborted = count();
    let socket: Arc<dyn WebSocket> = Arc::new(Socket {
        frames: Mutex::new(VecDeque::from([
            WebSocketFrame::Data {
                kind: WebSocketFragmentKind::Text,
                bytes: b"he".to_vec(),
                end: false,
            },
            WebSocketFrame::Data {
                kind: WebSocketFragmentKind::Continuation,
                bytes: b"llo".to_vec(),
                end: true,
            },
        ])),
        sent: Arc::clone(&sent),
        disposed: Arc::clone(&disposed),
        aborted: Arc::clone(&aborted),
        abort: Cancellation::new(),
    });
    let program = with_socket(
        WebSocketLimits {
            max_message_bytes: nz(5),
            receive_buffer_bytes: nz(2),
            close_timeout: Duration::from_secs(2),
        },
        Effect::sync(move || Arc::clone(&socket)),
        |connection| {
            let connection = connection
                .with(Clone::clone)
                .unwrap_or_else(|_| unreachable_socket());
            effect! {
                let connection = connection.clone();
                let received = connection.receive()?;
                assert!(matches!(received,WebSocketReceive::Message(WebSocketMessage::Text(text)) if text.as_ref()=="hello"));
                let oversized = Effect::from_future_with_env(move |env: Arc<Env>, _| { let connection = connection.clone(); async move { connection.send(WebSocketMessage::Text(Arc::from("too long"))).run(env).await } })?;
                assert!(matches!(oversized,Exit::Fail(WebSocketError::MessageTooLarge { .. })));
            }
        },
    );
    assert!(matches!(
        block_on(program.run(Arc::clone(&env))),
        Exit::Success(())
    ));
    assert_eq!(sent.load(Ordering::SeqCst), 0);
    assert_eq!(disposed.load(Ordering::SeqCst), 1);
    let socket: Arc<dyn WebSocket> = Arc::new(Socket {
        frames: Mutex::new(VecDeque::new()),
        sent,
        disposed: Arc::clone(&disposed),
        aborted: Arc::clone(&aborted),
        abort: Cancellation::new(),
    });
    let program = with_socket(
        WebSocketLimits {
            max_message_bytes: nz(5),
            receive_buffer_bytes: nz(2),
            close_timeout: Duration::from_secs(2),
        },
        Effect::sync(move || Arc::clone(&socket)),
        |connection| {
            connection
                .with(|connection| connection.close())
                .unwrap_or_else(|_| Effect::fail_with(|| WebSocketError::Closed))
        },
    );
    let mut run = Box::pin(program.run(Arc::clone(&env)));
    drive::until(run.as_mut(), || env.clock.pending() == 1);
    let (outcome, advanced) = block_on(futures_util::future::join(
        run,
        env.clock.advance(Duration::from_secs(2)),
    ));
    assert!(advanced.is_ok());
    assert!(matches!(outcome, Exit::Fail(WebSocketError::CloseTimeout)));
    assert_eq!(disposed.load(Ordering::SeqCst), 2);
    assert!(aborted.load(Ordering::SeqCst) >= 2);
}
#[allow(clippy::panic)]
fn unreachable_socket<T>() -> T {
    panic!("mock connection acquisition failed")
}
#[allow(clippy::panic)]
fn missing_scoped_handle<T>() -> T {
    panic!("scope did not expose its managed handle")
}

#[test]
fn cache_waiter_interruption_preserves_producer_and_root_cancellation_joins_cleanup() {
    let env = Arc::new(Env::default());
    let calls = count();
    let loaded = Arc::clone(&calls);
    let started = Latch::new();
    let released = Latch::new();
    let program: Effect<(), SharedValue<()>, Env> = Scope::scoped(move |scope| {
        let loaded = Arc::clone(&loaded);
        let started = started.clone();
        let released = released.clone();
        effect! {
            let scope=scope.clone(); let loaded=Arc::clone(&loaded); let started=started.clone(); let released=released.clone();
            let load_started=started.clone(); let load_released=released.clone();
            let cache=Cache::make(scope.clone(),CacheConfig { equal:Arc::new(|a:&usize,b|a==b), ttl:None,capacity:CacheCapacity::Unbounded,cache_failure:Arc::new(|_|false) },move |_| { let loaded=Arc::clone(&loaded); let started=load_started.clone(); let released=load_released.clone(); effect! { let loaded=Arc::clone(&loaded); let started=started.clone(); let released=released.clone(); Effect::sync(move|| { loaded.fetch_add(1,Ordering::SeqCst); })?; started.open()?; released.await_open()?; 7usize } })?;
            let first=scope.fork(cache.get(1))?;
            started.await_open()?;
            let cancelled=first.interrupt()?;
            assert!(matches!(cancelled,Exit::Interrupted));
            let second=scope.fork(cache.get(1))?;
            released.open()?;
            let value=second.join()?;
            assert_eq!(value.with(|value|*value),Ok(7));
        }
    });
    assert!(matches!(
        block_on(program.run(Arc::clone(&env))),
        Exit::Success(())
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    env.tasks.finish();
    let env = Arc::new(Env::default());
    let started = count();
    let entered = Arc::clone(&started);
    let cleaned = count();
    let cleanup = Arc::clone(&cleaned);
    let program: Effect<(), SharedValue<()>, Env> = Scope::scoped(move |scope| {
        let entered = Arc::clone(&entered);
        let cleanup = Arc::clone(&cleanup);
        Cache::make(
            scope,
            CacheConfig {
                equal: Arc::new(|a: &usize, b| a == b),
                ttl: None,
                capacity: CacheCapacity::Unbounded,
                cache_failure: Arc::new(|_| false),
            },
            move |_| {
                let entered = Arc::clone(&entered);
                let cleanup = Arc::clone(&cleanup);
                Effect::from_future(move |cancel| {
                    entered.fetch_add(1, Ordering::SeqCst);
                    async move {
                        cancel.cancelled().await;
                    }
                })
                .ensuring(Effect::sync(move || {
                    cleanup.fetch_add(1, Ordering::SeqCst);
                }))
            },
        )
        .flat_map(|cache| cache.get(1).map(|_| ()))
    });
    let mut run = Box::pin(program.run_cancellable(Arc::clone(&env)));
    drive::until(run.as_mut(), || started.load(Ordering::SeqCst) == 1);
    assert!(env.cancel.request());
    assert!(matches!(block_on(run), Exit::Interrupted));
    assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    env.tasks.finish();
}

#[test]
fn pool_invalid_idle_replacement_and_discard_keep_canonical_cleanup_errors() {
    let env = Arc::new(Env::default());
    let creates = count();
    let disposals = count();
    let created = Arc::clone(&creates);
    let disposed = Arc::clone(&disposals);
    let program: Effect<(), usize, Env> = Scope::scoped(move |scope| {
        let created = Arc::clone(&created);
        let disposed = Arc::clone(&disposed);
        effect! {
            let scope=scope.clone(); let created=Arc::clone(&created); let disposed=Arc::clone(&disposed);
            let pool=Pool::make(scope.clone(),PoolCapacity::Bounded(NonZeroUsize::MIN),None,Effect::sync(move||created.fetch_add(1,Ordering::SeqCst)),|value:&usize| { let valid=*value!=0; Effect::sync(move||valid) },move |_| { let disposed=Arc::clone(&disposed); Effect::sync(move|| { disposed.fetch_add(1,Ordering::SeqCst); }) })?;
            pool.ensure_size(1)?;
            let lease=pool.acquire(scope)?;
            assert_eq!(lease.with(|value|*value),Ok(1));
        }
    });
    assert_eq!(block_on(program.run(Arc::clone(&env))), Exit::Success(()));
    assert_eq!(creates.load(Ordering::SeqCst), 2);
    assert_eq!(disposals.load(Ordering::SeqCst), 2);
    let program: Effect<(), usize, Env> = Scope::scoped(|scope| {
        effect! {
            let scope=scope.clone();
            let pool=Pool::make(scope.clone(),PoolCapacity::Bounded(NonZeroUsize::MIN),None,Effect::sync(||()),|_|Effect::sync(||true),|_|Effect::fail_with(||7))?;
            let lease=pool.acquire(scope)?;
            let status=lease.discard()?;
            assert_eq!(status,PoolEndStatus::Failed);
        }
    });
    assert_eq!(
        block_on(program.run(Arc::clone(&env))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Succeeded,
            first: CleanupFailure::ReleaseFailure(7),
            rest: Vec::new()
        })
    );
    env.tasks.finish();
}
#[test]
fn late_http_acquisition_is_closed_before_cancellation_publication() {
    struct Late(Arc<AtomicUsize>);
    impl HasHttpTransport for Late {
        type Transport = Self;
        fn http_transport(&self) -> &Self {
            self
        }
    }
    impl HttpTransport for Late {
        fn send(&self, _: HttpRequest, cancel: Cancellation) -> TransportFuture<HttpResponse> {
            let response = HttpResponse::new(
                HttpStatus(200),
                HttpHeaders::new(),
                Arc::new(Body {
                    bytes: Mutex::new(Some(b"late".to_vec())),
                    reads: count(),
                    closes: Arc::clone(&self.0),
                    fail_close: false,
                }),
            );
            Box::pin(async move {
                let _ = cancel.request();
                Ok(response)
            })
        }
    }
    let closes = count();
    let request = HttpRequest {
        url: Arc::from("mock://late"),
        options: RequestOptions {
            method: HttpMethod::Get,
            headers: Arc::new(HttpHeaders::new()),
            body: None,
        },
    };
    let program = fetch_text(request, |_| true, |error| error);
    assert!(matches!(
        block_on(program.run(Arc::new(Late(Arc::clone(&closes))))),
        Exit::Interrupted
    ));
    assert_eq!(closes.load(Ordering::SeqCst), 1);
}

#[test]
fn abandoned_pool_scope_invalidates_lease_closes_admission_and_recovery_drains_resource() {
    let env = Arc::new(Env::default());
    let released = count();
    let disposed = Arc::clone(&released);
    let slot = Arc::new(Mutex::new(None::<(Pool<usize, ()>, PoolLease<usize, ()>)>));
    let exposed = Arc::clone(&slot);
    let program: Effect<(), (), Env> = Scope::scoped(move |scope| {
        let exposed = Arc::clone(&exposed);
        let disposed = Arc::clone(&disposed);
        effect! {
            let scope=scope.clone(); let exposed=Arc::clone(&exposed); let disposed=Arc::clone(&disposed);
            let pool=Pool::make(scope.clone(),PoolCapacity::Bounded(NonZeroUsize::MIN),None,Effect::sync(||7usize),|_|Effect::sync(||true),move |_| { let disposed=Arc::clone(&disposed); Effect::sync(move|| { disposed.fetch_add(1,Ordering::SeqCst); }) })?;
            let lease=pool.acquire(scope)?;
            *exposed.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=Some((pool,lease));
            Effect::from_future(|_|std::future::pending::<()>())?;
        }
    });
    let mut run = Box::pin(program.run(Arc::clone(&env)));
    drive::until(run.as_mut(), || {
        slot.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    });
    drop(run);
    let stored = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let (pool, lease) = stored.unwrap_or_else(missing_scoped_handle);
    assert!(lease.with(|value| *value).is_err());
    assert_eq!(released.load(Ordering::SeqCst), 0);
    assert_eq!(
        block_on(pool.shutdown::<Env>().run(Arc::clone(&env))),
        Exit::Success(PoolEndStatus::Failed)
    );
    assert_eq!(released.load(Ordering::SeqCst), 1);
    env.tasks.finish();
}
#[test]
fn abandoned_cache_and_live_weak_association_reject_new_work_and_can_be_drained() {
    let env = Arc::new(Env::default());
    let slot = Arc::new(Mutex::new(None::<Cache<usize, usize, ()>>));
    let exposed = Arc::clone(&slot);
    let program: Effect<(), SharedValue<()>, Env> = Scope::scoped(move |scope| {
        let exposed = Arc::clone(&exposed);
        Cache::make(
            scope,
            CacheConfig {
                equal: Arc::new(|a: &usize, b| a == b),
                ttl: None,
                capacity: CacheCapacity::Unbounded,
                cache_failure: Arc::new(|_| false),
            },
            |_| Effect::sync(|| 7usize),
        )
        .flat_map(move |cache| {
            *exposed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cache);
            Effect::from_future(|_| std::future::pending::<()>())
        })
    });
    let mut run = Box::pin(program.run(Arc::clone(&env)));
    drive::until(run.as_mut(), || {
        slot.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    });
    drop(run);
    let stored = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let cache = stored.unwrap_or_else(missing_scoped_handle);
    assert!(
        matches!(block_on(cache.get::<Env>(1).run(Arc::clone(&env))),Exit::Die(defect) if defect.kind()==DefectKind::Library(LibraryDefect::CacheClosed))
    );
    assert!(matches!(
        block_on(cache.shutdown::<Env>().run(Arc::clone(&env))),
        Exit::Success(PoolEndStatus::Failed)
    ));
    let slot = Arc::new(Mutex::new(
        None::<(
            WeakCache<String, usize, ()>,
            Arc<String>,
            WeakCacheValue<String, usize, ()>,
        )>,
    ));
    let exposed = Arc::clone(&slot);
    let program: Effect<(), SharedValue<()>, Env> = Scope::scoped(move |scope| {
        let exposed = Arc::clone(&exposed);
        effect! {
            let scope=scope.clone(); let exposed=Arc::clone(&exposed);
            let cache=WeakCache::make(scope)?;
            let key=Arc::new(String::from("live"));
            let value=cache.get(&key,||Effect::sync(||7usize))?;
            *exposed.lock().unwrap_or_else(std::sync::PoisonError::into_inner)=Some((cache,key,value));
            Effect::from_future(|_|std::future::pending::<()>())?;
        }
    });
    let mut run = Box::pin(program.run(Arc::clone(&env)));
    drive::until(run.as_mut(), || {
        slot.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    });
    drop(run);
    let stored = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let (cache, key, _value) = stored.unwrap_or_else(missing_scoped_handle);
    assert!(
        matches!(block_on(cache.get::<Env>(&key,||Effect::sync(||9usize)).run(Arc::clone(&env))),Exit::Die(defect) if defect.kind()==DefectKind::Library(LibraryDefect::CacheClosed))
    );
    assert!(matches!(
        block_on(cache.shutdown::<Env>().run(Arc::clone(&env))),
        Exit::Success(PoolEndStatus::Failed)
    ));
    env.tasks.finish();
}
#[test]
fn dependency_cache_observes_retention_sets_and_fences_dependencies() {
    let env = Arc::new(Env::default());
    let loads = count();
    let witness = Arc::clone(&loads);
    let program: Effect<(), SharedValue<DependencyCacheError<()>>, Env> = Scope::scoped(
        move |scope| {
            let loads = Arc::clone(&loads);
            effect! {
                let loads=Arc::clone(&loads);
                let cache=DependencyCache::make(scope.clone(),DependencyCachePolicy{retention:nz(2),maximum_active:nz(1),maximum_dependencies:nz(1),key_equal:Arc::new(|left:&usize,right|left==right),dependency_equal:Arc::new(|left:&usize,right|left==right),cache_failure:Arc::new(|_:&()|false),ttl:CacheTtl::Fixed(None)},move |key|{let key=*key;let loads=Arc::clone(&loads);Effect::sync(move ||{loads.fetch_add(1,Ordering::SeqCst);(key,vec![key])})})?;
                let observed=cache.get_observed(1)?;assert_eq!(observed.observation,CacheObservation::Loaded);
                let observed=cache.get_observed(1)?;assert_eq!(observed.observation,CacheObservation::Retained);
                cache.set(1,7,vec![2])?;let value=cache.try_get(1)?;assert_eq!(value.unwrap().with(|value|*value),Ok(7));
                cache.invalidate_dependency(2)?;let missing=cache.try_get(1)?;assert!(missing.is_none());
                let observed=cache.get_observed(1)?;assert_eq!(observed.observation,CacheObservation::Loaded);
                cache.clear()?;let missing=cache.try_get(1)?;assert!(missing.is_none());
            }
        },
    );
    assert!(matches!(block_on(program.run(env)), Exit::Success(())));
    assert_eq!(witness.load(Ordering::SeqCst), 2);
}
#[test]
fn dynamic_cache_ttl_failure_settles_and_preserves_cleanup() {
    let env = Arc::new(Env::default());
    let program: Effect<(), SharedValue<()>, Env> = Scope::scoped(|scope| {
        Cache::make_with_options(
            scope,
            CacheConfig {
                equal: Arc::new(|left: &usize, right| left == right),
                ttl: None,
                capacity: CacheCapacity::Bounded(NonZeroUsize::MIN),
                cache_failure: Arc::new(|_: &()| false),
            },
            CacheOptions {
                ttl: CacheTtl::Outcome(Arc::new(|_| {
                    Err(Defect::from(LibraryDefect::ClockDeadlineOverflow))
                })),
                admission: CacheAdmission::Unbounded,
            },
            |_| Effect::succeed(1usize),
        )
        .flat_map(|cache| cache.get(1).map(|_| ()))
    });
    assert!(matches!(
        block_on(program.run(env)),
        Exit::FinalizationFailed(_)
    ));
}

#[test]
fn bounded_cache_admission_rejects_before_dispatch_and_recovers_after_retirement() {
    let env = Arc::new(Env::default());
    let calls = count();
    let loaded = Arc::clone(&calls);
    let started = Latch::new();
    let released = Latch::new();
    let program: Effect<(), SharedValue<&'static str>, Env> = Scope::scoped(move |scope| {
        let loaded = Arc::clone(&loaded);
        let started = started.clone();
        let released = released.clone();
        effect! {
            let scope=scope.clone();let loaded=Arc::clone(&loaded);let started=started.clone();let released=released.clone();
            let load_started=started.clone();let load_released=released.clone();
            let cache=Cache::make_with_options(scope.clone(),CacheConfig{equal:Arc::new(|a:&usize,b|a==b),ttl:None,capacity:CacheCapacity::NoRetention,cache_failure:Arc::new(|_|false)},CacheOptions{ttl:CacheTtl::Fixed(None),admission:CacheAdmission::Bounded{maximum:nz(1),capacity_error:Arc::new(||"full")}},move |_|{let loaded=Arc::clone(&loaded);let started=load_started.clone();let released=load_released.clone();effect!{let loaded=Arc::clone(&loaded);let started=started.clone();let released=released.clone();Effect::sync(move||{loaded.fetch_add(1,Ordering::SeqCst);})?;started.open()?;released.await_open()?;7usize}})?;
            let first=scope.fork(cache.get_observed(1))?;started.await_open()?;
            let denied=cache.get_observed(2).either()?;assert!(matches!(denied,Err(error) if error.with(|error|*error)==Ok("full")));
            released.open()?;let observed=first.join()?;assert_eq!(observed.observation,CacheObservation::Loaded);
            let observed=cache.get_observed(2)?;assert_eq!(observed.observation,CacheObservation::Loaded);
        }
    });
    assert!(matches!(
        block_on(program.run(Arc::clone(&env))),
        Exit::Success(())
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    env.tasks.finish();
}

#[test]
fn cache_observation_distinguishes_shared_loading_and_set_fences_active_generation() {
    let env = Arc::new(Env::default());
    let slot = Arc::new(Mutex::new(None::<Cache<usize, usize, ()>>));
    let exposed = Arc::clone(&slot);
    let loads = count();
    let entered = Arc::clone(&loads);
    let comparisons = count();
    let matched = Arc::clone(&comparisons);
    let released = Latch::new();
    let load_release = released.clone();
    let blocked_release = Latch::new();
    let program: Effect<(), SharedValue<()>, Env> = Scope::scoped(move |scope| {
        let exposed = Arc::clone(&exposed);
        let entered = Arc::clone(&entered);
        let matched = Arc::clone(&matched);
        let load_release = load_release.clone();
        let blocked_release = blocked_release.clone();
        Cache::make(
            scope,
            CacheConfig {
                equal: Arc::new(move |left: &usize, right| {
                    matched.fetch_add(1, Ordering::SeqCst);
                    left == right
                }),
                ttl: None,
                capacity: CacheCapacity::Unbounded,
                cache_failure: Arc::new(|_| false),
            },
            move |key| {
                let key = *key;
                let entered = Arc::clone(&entered);
                let released = if key == 2 {
                    blocked_release.clone()
                } else {
                    load_release.clone()
                };
                Effect::sync(move || {
                    entered.fetch_add(1, Ordering::SeqCst);
                })
                .zip(released.await_open())
                .map(move |_| key)
            },
        )
        .flat_map(move |cache| {
            *exposed.lock().unwrap() = Some(cache);
            Effect::from_future(|_| std::future::pending::<()>())
        })
    });
    let mut program = Box::pin(program.run(Arc::clone(&env)));
    drive::until(program.as_mut(), || slot.lock().unwrap().is_some());
    let cache = slot.lock().unwrap().take().unwrap();
    let first = cache.get_observed(1);
    let mut first = Box::pin(first.run(Arc::clone(&env)));
    drive::until(first.as_mut(), || loads.load(Ordering::SeqCst) == 1);
    comparisons.store(0, Ordering::SeqCst);
    let second = cache.get_observed(1);
    let mut second = Box::pin(second.run(Arc::clone(&env)));
    drive::until(second.as_mut(), || comparisons.load(Ordering::SeqCst) > 0);
    assert!(matches!(
        block_on(released.open::<(), Env>().run(Arc::clone(&env))),
        Exit::Success(true)
    ));
    let first = match block_on(first) {
        Exit::Success(value) => value,
        _ => panic!("loaded"),
    };
    let second = match block_on(second) {
        Exit::Success(value) => value,
        _ => panic!("shared"),
    };
    assert_eq!(first.observation, CacheObservation::Loaded);
    assert_eq!(second.observation, CacheObservation::Shared);
    assert!(
        matches!(block_on(cache.get_observed(1).run(Arc::clone(&env))),Exit::Success(value) if value.observation==CacheObservation::Retained)
    );
    // A fresh blocked producer is replaced, joined, and cannot publish over set.
    let third = cache.get_observed(2);
    let mut third = Box::pin(third.run(Arc::clone(&env)));
    drive::until(third.as_mut(), || loads.load(Ordering::SeqCst) == 2);
    assert!(matches!(
        block_on(cache.set(2, 99).run(Arc::clone(&env))),
        Exit::Success(())
    ));
    assert!(!matches!(block_on(third), Exit::Success(_)));
    assert!(
        matches!(block_on(cache.get_observed(2).run(Arc::clone(&env))),Exit::Success(value) if value.value.with(|value|*value)==Ok(99))
    );
    drop(program);
    assert!(matches!(
        block_on(cache.shutdown::<Env>().run(Arc::clone(&env))),
        Exit::Success(_)
    ));
    env.tasks.finish();
}
