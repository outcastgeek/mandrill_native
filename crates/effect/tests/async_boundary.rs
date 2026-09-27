use effect::{Cancellation, Defect, Effect, Exit};
use futures_executor::block_on;
use futures_util::future::{join_all, poll_fn};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;

#[test]
fn fresh_futures_per_run_and_inline_execution() {
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let program: Effect<usize, ()> = Effect::from_future(move |_| {
        let value = witness.fetch_add(1, Ordering::SeqCst);
        async move { value }
    });
    let pending = program.run(Arc::new(()));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(block_on(pending), Exit::Success(0));
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(1));
    let thread = std::thread::current().id();
    let inline: Effect<std::thread::ThreadId, ()> = Effect::sync(|| std::thread::current().id());
    assert_eq!(block_on(inline.run(Arc::new(()))), Exit::Success(thread));
}

#[test]
fn every_registered_waiter_and_late_waiter_observes_cancellation() {
    let token = Cancellation::new();
    let witnesses = (0..32).map(|_| token.clone()).collect::<Vec<_>>();
    let waiters = witnesses.iter().map(Cancellation::cancelled);
    let requester = token.clone();
    let (results, ()) = block_on(futures_util::future::join(join_all(waiters), async move {
        assert!(requester.request());
    }));
    assert_eq!(results.len(), 32);
    assert!(!token.request());
    block_on(token.cancelled());
}

#[test]
fn concurrent_request_and_registration_cannot_lose_a_wakeup() {
    use std::future::Future;
    use std::sync::Barrier;
    use std::task::{Context, Wake, Waker};

    struct Witness(AtomicUsize);
    impl Wake for Witness {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            let _ = self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    for _ in 0..256 {
        let token = Cancellation::new();
        let requester = token.clone();
        let start = Arc::new(Barrier::new(2));
        let other = Arc::clone(&start);
        let worker = std::thread::spawn(move || {
            other.wait();
            assert!(requester.request());
        });
        let witness = Arc::new(Witness(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&witness));
        let mut context = Context::from_waker(&waker);
        let mut wait = std::pin::pin!(token.cancelled());
        start.wait();
        let first = wait.as_mut().poll(&mut context);
        assert!(worker.join().is_ok());
        if first.is_pending() {
            assert!(witness.0.load(Ordering::SeqCst) > 0);
            assert!(wait.as_mut().poll(&mut context).is_ready());
        }
    }
}

#[test]
fn cooperative_async_cancellation_waits_for_settlement() {
    let token = Cancellation::new();
    let requested = token.clone();
    let settled = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&settled);
    let entered = Cancellation::new();
    let entered_witness = entered.clone();
    let program: Effect<(), ()> = Effect::from_future(move |token| {
        let _ = entered_witness.request();
        let witness = Arc::clone(&witness);
        async move {
            token.cancelled().await;
            let _ = witness.fetch_add(1, Ordering::SeqCst);
        }
    });
    let (outcome, ()) = block_on(futures_util::future::join(
        program
            .provide_some(|_: &Cancellation| ())
            .run_cancellable(Arc::new(token)),
        async move {
            entered.cancelled().await;
            assert!(requested.request());
        },
    ));
    assert_eq!(outcome, Exit::Interrupted);
    assert_eq!(settled.load(Ordering::SeqCst), 1);
}

#[test]
#[allow(clippy::panic)] // Deliberate user-code defects exercise creation and polling boundaries.
fn unwinding_panics_are_defects_at_each_user_boundary() {
    let factory: Effect<(), ()> = Effect::from_future(|_| {
        panic!("factory");
        #[allow(unreachable_code)]
        async {}
    });
    assert_eq!(
        block_on(factory.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("factory")))
    );
    let polling: Effect<(), ()> =
        Effect::from_future(|_| poll_fn(|_| -> Poll<()> { panic!("poll") }));
    assert_eq!(
        block_on(polling.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("poll")))
    );
    let mapping: Effect<(), ()> = Effect::succeed(()).flat_map(|()| panic!("next"));
    assert_eq!(
        block_on(mapping.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("next")))
    );
    let token = Cancellation::new();
    let requester = token.clone();
    let panicking: Effect<(), ()> = Effect::sync(move || {
        let _ = requester.request();
        panic!("cancel and panic")
    });
    assert_eq!(
        block_on(
            panicking
                .provide_some(|_: &Cancellation| ())
                .run_cancellable(Arc::new(token))
        ),
        Exit::Die(Defect::from_panic(Box::new("cancel and panic")))
    );
}

#[test]
#[allow(clippy::panic)] // Recovery and mapping callbacks are separate user-code boundaries.
fn mapped_native_result_and_recovery_mapper_defects() {
    let program: Effect<usize, String> = Effect::try_future(
        |_| async { Err::<usize, _>(7) },
        |value| format!("error-{value}"),
    );
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::Fail(String::from("error-7"))
    );
    let sync: Effect<usize, String> =
        Effect::try_sync(|| Err::<usize, _>(8), |value| format!("error-{value}"));
    assert_eq!(
        block_on(sync.run(Arc::new(()))),
        Exit::Fail(String::from("error-8"))
    );
    let recover: Effect<(), ()> = Effect::fail(()).catch_all(|_| panic!("recover"));
    assert_eq!(
        block_on(recover.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("recover")))
    );
    let mapped: Effect<(), ()> = Effect::<(), ()>::fail(()).map_error(|_| panic!("map error"));
    assert_eq!(
        block_on(mapped.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("map error")))
    );
    let sandbox: Effect<(), ()> =
        Effect::die(Defect::new("original")).sandbox(|_| panic!("sandbox"));
    assert_eq!(
        block_on(sandbox.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("sandbox")))
    );
}
