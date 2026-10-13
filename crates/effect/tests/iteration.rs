use effect::{Cancellation, Defect, Effect, Exit};
use effect::{effect, effect_future, effect_sync, effect_try_sync};
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
    let program: Effect<(), usize> = effect! { while_loop;
        guard: move || {
            let _ = checked.fetch_add(1, Ordering::SeqCst);
            count.load(Ordering::SeqCst) < 3
        },
        body: move || {
            let _ = made.fetch_add(1, Ordering::SeqCst);
            let steps = Arc::clone(&steps);
            effect_future! { body: move |_| {
                let steps = Arc::clone(&steps);
                async move {
                    let _ = steps.fetch_add(1, Ordering::SeqCst);
                }
            }, }
        },
    };
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
    let program: Effect<(), usize> = effect! { while_loop;
        guard: move || {
            let _ = witness.fetch_add(1, Ordering::SeqCst);
            true
        },
        body: || effect_try_sync! { body: { let __dsl_error = || 42; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, },
    };
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Fail(42));
    assert_eq!(guards.load(Ordering::SeqCst), 1);

    let token = Arc::new(Cancellation::new());
    let request = Arc::clone(&token);
    let witness = Arc::clone(&guards);
    let program: Effect<(), usize, Cancellation> = effect! { while_loop;
        guard: move || {
            let _ = witness.fetch_add(1, Ordering::SeqCst);
            true
        },
        body: move || {
            let request = Arc::clone(&request);
            effect_future! { body: move |_| {
                let request = Arc::clone(&request);
                async move {
                    assert!(request.request());
                }
            }, }
        },
    };
    assert_eq!(
        block_on(program.run_cancellable(Arc::clone(&token))),
        Exit::Interrupted
    );
    assert_eq!(guards.load(Ordering::SeqCst), 2);
    assert_eq!(block_on(program.run_cancellable(token)), Exit::Interrupted);
    assert_eq!(guards.load(Ordering::SeqCst), 2);
}

#[test]
fn guard_and_body_factory_panics_are_defects() {
    let guard: Effect<(), ()> = effect! { while_loop;
        guard: || std::panic::resume_unwind(Box::new("guard")),
        body: || effect_sync! { body: { let __dsl_value = (); move || __dsl_value }, },
    };
    assert_eq!(
        block_on(guard.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("guard")))
    );
    let body: Effect<(), ()> = effect! { while_loop;
        guard: || true,
        body: || std::panic::resume_unwind(Box::new("body factory")),
    };
    assert_eq!(
        block_on(body.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("body factory")))
    );
}
