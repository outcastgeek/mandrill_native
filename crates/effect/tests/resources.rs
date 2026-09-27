use effect::LibraryDefect;
use effect::effect;
use effect::{
    Cancellation, CleanupFailure, Defect, Effect, Exit, Finalization, PrimaryOutcome,
    PrimaryOutcomeRef, Release, ResourceBinding, ResourceValue,
};
use futures_executor::block_on;
use futures_util::future::{join, poll_fn};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;

#[path = "support/drive.rs"]
mod drive;

#[derive(Debug, PartialEq, Eq)]
enum Error {
    Body,
    Release(usize),
}

struct Native {
    id: usize,
}

#[test]
fn abandoned_resource_owner_invalidates_handles_without_claiming_awaited_release() {
    struct Owned(Arc<AtomicUsize>);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let releases = Arc::new(AtomicUsize::new(0));
    let slot = Arc::new(Mutex::new(None::<ResourceValue<Owned>>));
    let owned = Arc::clone(&drops);
    let released = Arc::clone(&releases);
    let witness = Arc::clone(&slot);
    let program: Effect<(), Error> = ResourceBinding::new(
        Effect::sync(move || Owned(Arc::clone(&owned))),
        move |resource, _| {
            let released = Arc::clone(&released);
            Release::from_future(move |_| async move {
                drop(resource);
                let _ = released.fetch_add(1, Ordering::SeqCst);
            })
        },
    )
    .use_with(move |handle| {
        *witness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
        Effect::from_future(|_| std::future::pending())
    });
    let (mut driver, observer) = program.owned_run(Arc::new(()));
    drive::until(std::pin::Pin::new(&mut driver), || {
        slot.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    });
    drop(driver);
    assert!(matches!(block_on(observer), Exit::Die(_)));
    let handle = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    assert!(handle.is_some());
    if let Some(handle) = handle {
        assert_eq!(
            handle.with(|_| ()),
            Err(Defect::from(LibraryDefect::ResourceScopeEnded))
        );
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(releases.load(Ordering::SeqCst), 0);
}

fn binding(
    acquires: Arc<AtomicUsize>,
    releases: Arc<AtomicUsize>,
) -> ResourceBinding<Native, Error> {
    ResourceBinding::new(
        Effect::sync(move || Native {
            id: acquires.fetch_add(1, Ordering::SeqCst),
        }),
        move |_native, _reason| {
            let releases = Arc::clone(&releases);
            Release::sync(move || {
                let _ = releases.fetch_add(1, Ordering::SeqCst);
            })
        },
    )
}

#[test]
fn acquisition_is_lazy_fresh_and_releases_non_clone_values_once_per_run() {
    let acquires = Arc::new(AtomicUsize::new(0));
    let releases = Arc::new(AtomicUsize::new(0));
    let program = binding(Arc::clone(&acquires), Arc::clone(&releases))
        .use_with(|handle| handle.access(|native| native.id));
    assert_eq!(acquires.load(Ordering::SeqCst), 0);
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(0));
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(1));
    assert_eq!(acquires.load(Ordering::SeqCst), 2);
    assert_eq!(releases.load(Ordering::SeqCst), 2);
}

#[test]
fn escaped_handle_is_invalid_after_release_and_cannot_invoke_access() {
    let releases = Arc::new(AtomicUsize::new(0));
    let program =
        binding(Arc::new(AtomicUsize::new(0)), Arc::clone(&releases)).use_with(Effect::succeed);
    let handle = match block_on(program.run(Arc::new(()))) {
        Exit::Success(handle) => Some(handle),
        Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
    };
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    assert!(handle.is_some());
    if let Some(handle) = handle {
        assert_eq!(
            handle.with(|native| native.id),
            Err(Defect::from(LibraryDefect::ResourceScopeEnded))
        );
        let invalid: Effect<usize, Error> = handle.access(|_| 42);
        assert_eq!(
            block_on(invalid.run(Arc::new(()))),
            Exit::Die(Defect::from(LibraryDefect::ResourceScopeEnded))
        );
    }
}

#[test]
fn macro_resource_bindings_scope_the_remaining_workflow_and_release_in_reverse_order() {
    let released = Arc::new(Mutex::new(Vec::new()));
    let make = |id| {
        let released = Arc::clone(&released);
        ResourceBinding::<Native, Error>::new(
            Effect::sync(move || Native { id }),
            move |native, _| {
                let released = Arc::clone(&released);
                Release::sync(move || {
                    released
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(native.id);
                })
            },
        )
    };
    let first = make(1);
    let second = make(2);
    let program: Effect<usize, Error> = effect! {
        let first = first.clone();
        let second = second.clone();
        let first_handle = first?;
        let second_handle = second?;
        let first_id = first_handle.access(|native| native.id)?;
        let second_id = second_handle.access(|native| native.id)?;
        first_id + second_id
    };
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(3));
    assert_eq!(
        *released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![2, 1]
    );
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(3));
    assert_eq!(
        *released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![2, 1, 2, 1]
    );
}

#[test]
fn late_success_installs_ownership_before_pending_cancellation() {
    let cancellation = Arc::new(Cancellation::new());
    let request = Arc::clone(&cancellation);
    let entered = Cancellation::new();
    let entered_witness = entered.clone();
    let releases = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&releases);
    let bodies = Arc::new(AtomicUsize::new(0));
    let body_witness = Arc::clone(&bodies);
    let resource: ResourceBinding<Native, Error, Cancellation> = ResourceBinding::new(
        Effect::from_future(move |cancel| {
            assert!(entered_witness.request());
            async move {
                cancel.cancelled().await;
                Native { id: 42 }
            }
        }),
        move |native, reason| {
            assert_eq!(native.id, 42);
            assert!(matches!(reason, PrimaryOutcomeRef::WasInterrupted));
            let witness = Arc::clone(&witness);
            Release::sync(move || {
                let _ = witness.fetch_add(1, Ordering::SeqCst);
            })
        },
    );
    let program = resource.use_with(move |_| {
        let _ = body_witness.fetch_add(1, Ordering::SeqCst);
        Effect::succeed(())
    });
    let (driver, observer) = program.owned_run_cancellable(cancellation);
    let (((), outcome), ()) = block_on(join(join(driver, observer), async move {
        entered.cancelled().await;
        assert!(request.request());
    }));
    assert_eq!(outcome, Exit::Interrupted);
    assert_eq!(bodies.load(Ordering::SeqCst), 0);
    assert_eq!(releases.load(Ordering::SeqCst), 1);
}

#[test]
fn nested_release_failures_preserve_primary_error_and_reverse_order() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let make = |id| {
        let order = Arc::clone(&order);
        ResourceBinding::<Native, Error>::new(
            Effect::sync(move || Native { id }),
            move |native, reason| {
                assert!(matches!(reason, PrimaryOutcomeRef::Failed(Error::Body)));
                let order = Arc::clone(&order);
                Release::try_sync(move || {
                    order
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(native.id);
                    Err(Error::Release(native.id))
                })
            },
        )
    };
    let inner = make(2).use_with(|_| Effect::<(), Error>::fail_with(|| Error::Body));
    let program = make(1).use_with(move |_| inner.clone());
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(Error::Body),
            first: CleanupFailure::ReleaseFailure(Error::Release(2)),
            rest: vec![CleanupFailure::ReleaseFailure(Error::Release(1))],
        })
    );
    assert_eq!(
        *order
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![2, 1]
    );
}

#[test]
fn cancellation_during_awaited_release_is_masked_and_published_after_settlement() {
    let cancellation = Arc::new(Cancellation::new());
    let request = Arc::clone(&cancellation);
    let releases = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&releases);
    let binding: ResourceBinding<Native, Error, Cancellation> =
        ResourceBinding::new(Effect::sync(|| Native { id: 42 }), move |native, reason| {
            assert!(matches!(reason, PrimaryOutcomeRef::Succeeded));
            let request = Arc::clone(&request);
            let witness = Arc::clone(&witness);
            Release::try_future(move |masked| {
                assert!(!masked.is_requested());
                assert!(request.request());
                let mut suspended = false;
                poll_fn(move |cx| {
                    if suspended {
                        assert!(!masked.is_requested());
                        let _ = witness.fetch_add(1, Ordering::SeqCst);
                        Poll::Ready(Err(Error::Release(native.id)))
                    } else {
                        suspended = true;
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                })
            })
        });
    let program = binding.use_with(|handle| handle.access(|native| native.id));
    assert_eq!(
        block_on(program.run_cancellable(cancellation)),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::WasInterrupted,
            first: CleanupFailure::ReleaseFailure(Error::Release(42)),
            rest: Vec::new(),
        })
    );
    assert_eq!(releases.load(Ordering::SeqCst), 1);
}

#[test]
#[allow(clippy::panic)] // Both factories are user-code defect boundaries with an installed owner.
fn body_and_release_factory_panics_preserve_release_obligations_and_outcomes() {
    let releases = Arc::new(AtomicUsize::new(0));
    let program: Effect<(), Error> = binding(Arc::new(AtomicUsize::new(0)), Arc::clone(&releases))
        .use_with(|_| panic!("body factory"));
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("body factory")))
    );
    assert_eq!(releases.load(Ordering::SeqCst), 1);

    let resource: ResourceBinding<Native, Error> =
        ResourceBinding::new(Effect::sync(|| Native { id: 42 }), |_, _| {
            panic!("release factory")
        });
    let program = resource.use_with(|handle| handle.access(|native| native.id));
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Succeeded,
            first: CleanupFailure::ReleaseDefect(Defect::from_panic(Box::new("release factory"))),
            rest: Vec::new(),
        })
    );
}

#[test]
fn losing_observer_keeps_resource_release_owned_by_the_driver() {
    let cancellation = Arc::new(Cancellation::new());
    let releases = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&releases);
    let binding: ResourceBinding<Native, Error, Cancellation> =
        ResourceBinding::new(Effect::sync(|| Native { id: 42 }), move |_, _| {
            let witness = Arc::clone(&witness);
            Release::sync(move || {
                let _ = witness.fetch_add(1, Ordering::SeqCst);
            })
        });
    let entered = Cancellation::new();
    let entered_witness = entered.clone();
    let program = binding.use_with(move |_| {
        let entered_witness = entered_witness.clone();
        Effect::from_future(move |cancel| {
            assert!(entered_witness.request());
            async move {
                cancel.cancelled().await;
            }
        })
    });
    let (mut driver, observer) = program.owned_run_cancellable(Arc::clone(&cancellation));
    drive::until(std::pin::Pin::new(&mut driver), || entered.is_requested());
    drop(observer);
    assert!(cancellation.request());
    block_on(driver);
    assert_eq!(releases.load(Ordering::SeqCst), 1);
}
