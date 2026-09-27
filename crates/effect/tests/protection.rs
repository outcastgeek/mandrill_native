use effect::{
    Cancellation, CleanupFailure, Defect, Effect, Exit, Finalization, PrimaryOutcome,
    PrimaryOutcomeRef, Release,
};
use futures_executor::block_on;
use futures_util::future::{join, poll_fn};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;

#[path = "support/drive.rs"]
mod drive;

#[derive(Debug, PartialEq, Eq)]
struct Error(&'static str);

#[test]
fn ensuring_runs_for_each_started_outcome_and_borrows_non_clone_primary_error() {
    let cleanups = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&cleanups);
    let program = Effect::<(), Error>::fail_with(|| Error("body")).ensuring_with(move |reason| {
        assert!(matches!(reason, PrimaryOutcomeRef::Failed(Error("body"))));
        let witness = Arc::clone(&witness);
        Release::try_sync(move || {
            let _ = witness.fetch_add(1, Ordering::SeqCst);
            Err(Error("cleanup"))
        })
    });
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(Error("body")),
            first: CleanupFailure::ReleaseFailure(Error("cleanup")),
            rest: Vec::new(),
        })
    );
    assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    let witness = Arc::clone(&cleanups);
    let finalizer = Effect::<(), Error>::sync(move || {
        let _ = witness.fetch_add(1, Ordering::SeqCst);
    });
    for body in [Effect::succeed(()), Effect::die(Defect::new("defect"))] {
        let outcome = block_on(body.ensuring(finalizer.clone()).run(Arc::new(())));
        assert!(matches!(outcome, Exit::Success(()) | Exit::Die(_)));
    }
    assert_eq!(cleanups.load(Ordering::SeqCst), 3);
}

#[test]
fn ensuring_awaits_masked_cleanup_after_cancellation_but_skips_pre_cancelled_start() {
    let token = Arc::new(Cancellation::new());
    let count = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&count);
    let finalizer = Effect::<(), Error, Cancellation>::from_future(move |masked| {
        let witness = Arc::clone(&witness);
        async move {
            assert!(!masked.is_requested());
            let _ = witness.fetch_add(1, Ordering::SeqCst);
        }
    });
    let entered = Cancellation::new();
    let entered_witness = entered.clone();
    let program = Effect::from_future(move |cancel| {
        assert!(entered_witness.request());
        async move {
            cancel.cancelled().await;
        }
    })
    .ensuring(finalizer);
    let mut pending = Box::pin(program.run_cancellable(Arc::clone(&token)));
    drive::until(pending.as_mut(), || entered.is_requested());
    assert!(token.request());
    assert_eq!(block_on(pending), Exit::Interrupted);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(block_on(program.run_cancellable(token)), Exit::Interrupted);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn pre_cancelled_run_does_not_start_protected_work() {
    let cancellation = Arc::new(Cancellation::new());
    assert!(cancellation.request());
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let program: Effect<(), (), Cancellation> = Effect::sync(move || {
        let _ = witness.fetch_add(1, Ordering::SeqCst);
    })
    .uninterruptible();
    assert_eq!(
        block_on(program.run_cancellable(cancellation)),
        Exit::Interrupted
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn masked_body_completes_before_parent_publishes_interruption() {
    let cancellation = Arc::new(Cancellation::new());
    let request = Arc::clone(&cancellation);
    let continued = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&continued);
    let program: Effect<bool, (), Cancellation> = Effect::from_future(move |masked| {
        assert!(request.request());
        async move { masked.is_requested() }
    })
    .map(move |requested| {
        let _ = witness.fetch_add(1, Ordering::SeqCst);
        assert!(!requested);
        requested
    })
    .uninterruptible();
    assert_eq!(
        block_on(program.run_cancellable(cancellation)),
        Exit::Interrupted
    );
    assert_eq!(continued.load(Ordering::SeqCst), 1);
}

fn wait_for_cancel(entered: Cancellation) -> Effect<(), Error, Cancellation> {
    Effect::from_future(move |cancel| {
        let _ = entered.request();
        async move {
            cancel.cancelled().await;
        }
    })
}

#[test]
fn interruption_cleanup_is_masked_awaited_and_runs_once() {
    let cancellation = Arc::new(Cancellation::new());
    let request = Arc::clone(&cancellation);
    let settled = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&settled);
    let finalizer = Effect::from_future(move |masked| {
        assert!(!masked.is_requested());
        let witness = Arc::clone(&witness);
        let mut suspended = false;
        poll_fn(move |cx| {
            if suspended {
                let _ = witness.fetch_add(1, Ordering::SeqCst);
                Poll::Ready(())
            } else {
                suspended = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        })
    });
    let entered = Cancellation::new();
    let program = wait_for_cancel(entered.clone()).on_interrupt(finalizer);
    let (outcome, ()) = block_on(join(program.run_cancellable(cancellation), async move {
        entered.cancelled().await;
        assert!(request.request());
    }));
    assert_eq!(outcome, Exit::Interrupted);
    assert_eq!(settled.load(Ordering::SeqCst), 1);
}

#[test]
fn nested_cleanup_failures_retain_interruption_and_release_order_without_clone() {
    let cancellation = Arc::new(Cancellation::new());
    let request = Arc::clone(&cancellation);
    let entered = Cancellation::new();
    let program = wait_for_cancel(entered.clone())
        .on_interrupt(Effect::fail_with(|| Error("inner")))
        .on_interrupt(Effect::fail_with(|| Error("outer")));
    let (outcome, ()) = block_on(join(program.run_cancellable(cancellation), async move {
        entered.cancelled().await;
        assert!(request.request());
    }));
    assert_eq!(
        outcome,
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::WasInterrupted,
            first: CleanupFailure::ReleaseFailure(Error("inner")),
            rest: vec![CleanupFailure::ReleaseFailure(Error("outer"))],
        })
    );
}

#[test]
#[allow(clippy::panic)] // Intentional finalizer defect; the primary interruption must survive.
fn finalizer_panic_is_retained_as_a_release_defect() {
    let cancellation = Arc::new(Cancellation::new());
    let request = Arc::clone(&cancellation);
    let entered = Cancellation::new();
    let program = wait_for_cancel(entered.clone()).on_interrupt(Effect::sync(|| panic!("cleanup")));
    let (outcome, ()) = block_on(join(program.run_cancellable(cancellation), async move {
        entered.cancelled().await;
        assert!(request.request());
    }));
    assert_eq!(
        outcome,
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::WasInterrupted,
            first: CleanupFailure::ReleaseDefect(Defect::from_panic(Box::new("cleanup"))),
            rest: Vec::new(),
        })
    );
}

#[test]
fn other_outcomes_do_not_run_interruption_cleanup() {
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let cleanup: Effect<(), Error> = Effect::sync(move || {
        let _ = witness.fetch_add(1, Ordering::SeqCst);
    });
    for program in [
        Effect::succeed(()),
        Effect::fail_with(|| Error("expected")),
        Effect::die(Defect::new("defect")),
    ] {
        let _outcome = block_on(program.on_interrupt(cleanup.clone()).run(Arc::new(())));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
