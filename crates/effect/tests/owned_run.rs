use effect::LibraryDefect;
use effect::{Cancellation, Defect, Effect, Exit};
use effect::{effect_future, effect_on_interrupt, effect_sync};
use futures_executor::block_on;
use futures_util::future::join;
use std::cell::Cell;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "support/suspended.rs"]
mod drive;

fn send_future<F: Future + Send>(_: &F) {}

#[test]
fn observation_does_not_require_clone_or_sync_payloads() {
    let program: Effect<Cell<usize>, Cell<usize>> = effect_sync! { body: || Cell::new(42), };
    let (driver, observer) = program.owned_run(Arc::new(()));
    send_future(&driver);
    send_future(&observer);
    let ((), outcome) = block_on(join(driver, observer));
    assert!(matches!(outcome, Exit::Success(value) if value.get() == 42));
}

#[test]
fn completion_before_observer_registration_is_retained() {
    let program: Effect<usize, ()> =
        effect_sync! { body: { let __dsl_value = 42; move || __dsl_value }, };
    let (driver, observer) = program.owned_run(Arc::new(()));
    block_on(driver);
    assert_eq!(block_on(observer), Exit::Success(42));
}

#[test]
fn losing_observer_does_not_drop_execution_or_its_cleanup() {
    let cancellation = Arc::new(Cancellation::new());
    let settled = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&settled);
    let program: Effect<(), (), Cancellation> = effect_on_interrupt! { program: effect_future! { body: |token| async move {
        token.cancelled().await;
    }, }, finalizer: effect_sync! { body: move || {
        let _ = witness.fetch_add(1, Ordering::SeqCst);
    }, }, };
    let (mut driver, observer) = program.owned_run_cancellable(Arc::clone(&cancellation));
    drive::suspended(std::pin::Pin::new(&mut driver));
    drop(observer);
    assert!(cancellation.request());
    block_on(driver);
    assert_eq!(settled.load(Ordering::SeqCst), 1);
}

#[test]
fn losing_owner_is_reported_as_failure_instead_of_success() {
    let program: Effect<(), ()> = effect_future! { body: |_| std::future::pending(), };
    let (driver, observer) = program.owned_run(Arc::new(()));
    drop(driver);
    assert_eq!(
        block_on(observer),
        Exit::Die(Defect::from(LibraryDefect::RunDriverDropped))
    );
}
