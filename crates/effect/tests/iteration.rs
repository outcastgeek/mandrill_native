use effect::{Cancellation, Defect, Effect, Exit};
use futures_executor::block_on;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn guards_and_body_factories_are_lazy_and_short_circuit() {
    let steps = Arc::new(AtomicUsize::new(0));
    let guards = Arc::new(AtomicUsize::new(0));
    let factories = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&steps);
    let checked = Arc::clone(&guards);
    let made = Arc::clone(&factories);
    let program: Effect<(), usize> = Effect::while_loop(
        move || {
            let _ = checked.fetch_add(1, Ordering::SeqCst);
            count.load(Ordering::SeqCst) < 3
        },
        move || {
            let _ = made.fetch_add(1, Ordering::SeqCst);
            let steps = Arc::clone(&steps);
            Effect::from_future(move |_| {
                let steps = Arc::clone(&steps);
                async move {
                    let _ = steps.fetch_add(1, Ordering::SeqCst);
                }
            })
        },
    );
    assert_eq!(guards.load(Ordering::SeqCst), 0);
    assert_eq!(factories.load(Ordering::SeqCst), 0);
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(()));
    assert_eq!(guards.load(Ordering::SeqCst), 4);
    assert_eq!(factories.load(Ordering::SeqCst), 3);
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(()));
    assert_eq!(guards.load(Ordering::SeqCst), 5);
    assert_eq!(factories.load(Ordering::SeqCst), 3);
}

#[test]
fn expected_failure_and_cancellation_stop_guards_and_factories() {
    let guards = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&guards);
    let program: Effect<(), usize> = Effect::while_loop(
        move || {
            let _ = witness.fetch_add(1, Ordering::SeqCst);
            true
        },
        || Effect::fail_with(|| 42),
    );
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Fail(42));
    assert_eq!(guards.load(Ordering::SeqCst), 1);

    let token = Arc::new(Cancellation::new());
    let request = Arc::clone(&token);
    let witness = Arc::clone(&guards);
    let program: Effect<(), usize, Cancellation> = Effect::while_loop(
        move || {
            let _ = witness.fetch_add(1, Ordering::SeqCst);
            true
        },
        move || {
            let request = Arc::clone(&request);
            Effect::from_future(move |_| {
                let request = Arc::clone(&request);
                async move {
                    assert!(request.request());
                }
            })
        },
    );
    assert_eq!(
        block_on(program.run_cancellable(Arc::clone(&token))),
        Exit::Interrupted
    );
    assert_eq!(guards.load(Ordering::SeqCst), 2);
    assert_eq!(block_on(program.run_cancellable(token)), Exit::Interrupted);
    assert_eq!(guards.load(Ordering::SeqCst), 2);
}

#[test]
#[allow(clippy::panic)] // Guards and body factories are user-code boundaries.
fn guard_and_body_factory_panics_are_defects() {
    let guard: Effect<(), ()> = Effect::while_loop(|| panic!("guard"), || Effect::succeed(()));
    assert_eq!(
        block_on(guard.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("guard")))
    );
    let body: Effect<(), ()> = Effect::while_loop(|| true, || panic!("body factory"));
    assert_eq!(
        block_on(body.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("body factory")))
    );
}
