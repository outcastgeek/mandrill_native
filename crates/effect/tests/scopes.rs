use effect::LibraryDefect;
use effect::{
    Cancellation, CleanupFailure, Defect, Effect, Exit, Finalization, PrimaryOutcome, Release,
    ResourceValue, Scope, ScopeCloseStatus, effect,
};
use futures_executor::block_on;
use futures_util::future::join;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::task::{Context, Wake, Waker};

#[path = "support/suspended.rs"]
mod drive;

// Intentionally neither Clone nor Sync: shared close cannot copy these errors.
#[derive(Debug, PartialEq, Eq)]
enum Error {
    Body(std::cell::Cell<usize>),
    Release(std::cell::Cell<usize>),
}

fn count_release(counter: Arc<AtomicUsize>) -> Release<Error> {
    Release::sync(move || {
        let _ = counter.fetch_add(1, Ordering::SeqCst);
    })
}

#[test]
fn abandoned_scope_invalidates_owned_resources_before_waking_closed_observers() {
    struct Witness {
        resource: ResourceValue<String>,
        invalid_when_woken: AtomicBool,
    }
    impl Wake for Witness {
        fn wake(self: Arc<Self>) {
            self.invalid_when_woken
                .store(self.resource.with(String::len).is_err(), Ordering::SeqCst);
        }
    }
    let slot = Arc::new(Mutex::new(None::<(Scope<Error>, ResourceValue<String>)>));
    let witness = Arc::clone(&slot);
    let count = Arc::new(AtomicUsize::new(0));
    let released = Arc::clone(&count);
    let program: Effect<(), Error> = Scope::scoped(move |scope| {
        let released = Arc::clone(&released);
        let witness = Arc::clone(&witness);
        scope
            .acquire(Effect::sync(|| String::from("owned")), move |resource| {
                let released = Arc::clone(&released);
                Release::sync(move || {
                    drop(resource);
                    let _ = released.fetch_add(1, Ordering::SeqCst);
                })
            })
            .flat_map(move |handle| {
                *witness
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some((scope.clone(), handle));
                Effect::from_future(|_| std::future::pending())
            })
    });
    let (mut driver, observer) = program.owned_run(Arc::new(()));
    drive::suspended(Pin::new(&mut driver));
    let handles = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    assert!(handles.is_some());
    if let Some((scope, resource)) = handles {
        let witness = Arc::new(Witness {
            resource: resource.clone(),
            invalid_when_woken: AtomicBool::new(false),
        });
        let wake = Waker::from(Arc::clone(&witness));
        let mut context = Context::from_waker(&wake);
        let mut closed = Box::pin(scope.closed());
        assert!(closed.as_mut().poll(&mut context).is_pending());
        drop(driver);
        assert!(witness.invalid_when_woken.load(Ordering::SeqCst));
        assert!(matches!(block_on(observer), Exit::Die(_)));
        assert_eq!(block_on(closed), ScopeCloseStatus::Failed);
        assert_eq!(
            resource.with(String::len),
            Err(Defect::from(LibraryDefect::ResourceScopeEnded))
        );
    }
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn scopes_are_lazy_fresh_and_drain_reverse_order_after_primary_failure() {
    let started = Arc::new(AtomicUsize::new(0));
    let order = Arc::new(Mutex::new(Vec::new()));
    let started_witness = Arc::clone(&started);
    let order_witness = Arc::clone(&order);
    let program = Scope::scoped(move |scope| {
        let _ = started_witness.fetch_add(1, Ordering::SeqCst);
        let order = Arc::clone(&order_witness);
        let make = |id| {
            let order = Arc::clone(&order);
            scope.add_release(move || {
                let order = Arc::clone(&order);
                Release::try_sync(move || {
                    order
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(id);
                    Err(Error::Release(std::cell::Cell::new(id)))
                })
            })
        };
        let first = make(1);
        let second = make(2);
        effect! {
            let first = first.clone();
            let second = second.clone();
            first?;
            second?;
            Effect::<(), Error>::fail_with(|| Error::Body(std::cell::Cell::new(42)))?
        }
    });
    assert_eq!(started.load(Ordering::SeqCst), 0);
    for _ in 0..2 {
        assert_eq!(
            block_on(program.run(Arc::new(()))),
            Exit::FinalizationFailed(Finalization {
                primary: PrimaryOutcome::Failed(Error::Body(std::cell::Cell::new(42))),
                first: CleanupFailure::ReleaseFailure(Error::Release(std::cell::Cell::new(2))),
                rest: vec![CleanupFailure::ReleaseFailure(Error::Release(
                    std::cell::Cell::new(1)
                ))],
            })
        );
    }
    assert_eq!(started.load(Ordering::SeqCst), 2);
    assert_eq!(
        *order
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![2, 1, 2, 1]
    );
}

#[test]
fn concurrent_closers_join_one_drain_and_owner_retains_non_clone_errors() {
    let releases = Arc::new(AtomicUsize::new(0));
    let entered = Cancellation::new();
    let release_gate = Cancellation::new();
    let body_gate = Cancellation::new();
    let slot = Arc::new(Mutex::new(None::<Scope<Error>>));
    let slot_witness = Arc::clone(&slot);
    let released = Arc::clone(&releases);
    let entered_witness = entered.clone();
    let release_witness = release_gate.clone();
    let body_witness = body_gate.clone();
    let program: Effect<(), Error> = Scope::scoped(move |scope| {
        *slot_witness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(scope.clone());
        let released = Arc::clone(&released);
        let entered = entered_witness.clone();
        let release_gate = release_witness.clone();
        let registration = scope.add_release(move || {
            let released = Arc::clone(&released);
            let entered = entered.clone();
            let release_gate = release_gate.clone();
            Release::try_future(move |masked| async move {
                assert!(!masked.is_requested());
                let _ = released.fetch_add(1, Ordering::SeqCst);
                assert!(entered.request());
                release_gate.cancelled().await;
                assert!(!masked.is_requested());
                Err(Error::Release(std::cell::Cell::new(7)))
            })
        });
        let body_gate = body_witness.clone();
        registration.flat_map(move |()| {
            let body_gate = body_gate.clone();
            Effect::try_future(
                move |_| {
                    let body_gate = body_gate.clone();
                    async move {
                        body_gate.cancelled().await;
                        Err::<(), _>(Error::Body(std::cell::Cell::new(42)))
                    }
                },
                |error| error,
            )
        })
    });
    let (mut driver, observer) = program.owned_run(Arc::new(()));
    drive::suspended(Pin::new(&mut driver));
    let scope = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    assert!(scope.is_some());
    if let Some(scope) = scope {
        let mut first = Box::pin(scope.close::<()>().run(Arc::new(())));
        drive::suspended(first.as_mut());
        assert!(entered.is_requested());
        let second_program = scope.close::<()>();
        let mut second = Box::pin(second_program.run(Arc::new(())));
        drive::suspended(second.as_mut());
        assert_eq!(releases.load(Ordering::SeqCst), 1);
        assert!(release_gate.request());
        assert_eq!(
            block_on(join(first, second)),
            (
                Exit::Success(ScopeCloseStatus::Failed),
                Exit::Success(ScopeCloseStatus::Failed)
            )
        );
        assert_eq!(block_on(scope.closed()), ScopeCloseStatus::Failed);
        assert!(body_gate.request());
        let ((), outcome) = block_on(join(driver, observer));
        assert_eq!(
            outcome,
            Exit::FinalizationFailed(Finalization {
                primary: PrimaryOutcome::Failed(Error::Body(std::cell::Cell::new(42))),
                first: CleanupFailure::ReleaseFailure(Error::Release(std::cell::Cell::new(7))),
                rest: Vec::new(),
            })
        );
        // The acknowledgement remains available after the owner takes E.
        assert_eq!(
            block_on(scope.close::<()>().run(Arc::new(()))),
            Exit::Success(ScopeCloseStatus::Failed)
        );
    }
}

#[test]
fn scoped_handles_are_invalidated_and_failed_acquisition_installs_no_release() {
    let count = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&count);
    let program: Effect<_, Error> = Scope::scoped(move |scope| {
        let witness = Arc::clone(&witness);
        scope.acquire(Effect::sync(|| String::from("scoped")), move |value| {
            let witness = Arc::clone(&witness);
            Release::sync(move || {
                assert_eq!(value, "scoped");
                let _ = witness.fetch_add(1, Ordering::SeqCst);
            })
        })
    });
    let handle = match block_on(program.run(Arc::new(()))) {
        Exit::Success(handle) => Some(handle),
        Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
    };
    assert!(handle.is_some());
    if let Some(handle) = handle {
        assert_eq!(
            handle.with(String::len),
            Err(Defect::from(LibraryDefect::ResourceScopeEnded))
        );
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let witness = Arc::clone(&count);
    let failed: Effect<_, Error> = Scope::scoped(move |scope| {
        let witness = Arc::clone(&witness);
        scope.acquire(
            Effect::<String, Error>::fail_with(|| Error::Body(std::cell::Cell::new(1))),
            move |_| count_release(Arc::clone(&witness)),
        )
    });
    assert!(matches!(block_on(failed.run(Arc::new(()))), Exit::Fail(_)));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn acquisition_into_closed_scope_awaits_masked_release_before_rejection() {
    let count = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&count);
    let program: Effect<(), Error> = Scope::scoped(move |scope| {
        let witness = Arc::clone(&witness);
        let acquisition = scope
            .acquire(Effect::sync(|| String::from("late")), move |value| {
                let witness = Arc::clone(&witness);
                Release::try_future(move |masked| async move {
                    assert!(!masked.is_requested());
                    assert_eq!(value, "late");
                    let mut yielded = false;
                    std::future::poll_fn(move |cx| {
                        if yielded {
                            std::task::Poll::Ready(())
                        } else {
                            yielded = true;
                            cx.waker().wake_by_ref();
                            std::task::Poll::Pending
                        }
                    })
                    .await;
                    let _ = witness.fetch_add(1, Ordering::SeqCst);
                    Err(Error::Release(std::cell::Cell::new(1)))
                })
            })
            .map(|_| ());
        scope.close::<()>().flat_map(move |_| acquisition.clone())
    });
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Defected(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
            first: CleanupFailure::ReleaseFailure(Error::Release(std::cell::Cell::new(1))),
            rest: Vec::new(),
        })
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn registered_finalizers_keep_their_environment_across_scoped_provision() {
    struct Environment(&'static str);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let witness = Arc::clone(&seen);
    let program: Effect<(), Error> = Scope::scoped(move |scope| {
        let make = |name| {
            let witness = Arc::clone(&witness);
            let finalizer: Effect<(), Error, Environment> =
                Effect::service(move |env: &Environment| {
                    witness
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(env.0);
                });
            scope
                .add_finalizer(finalizer)
                .provide(Arc::new(Environment(name)))
        };
        make("first").zip(make("second")).map(|_| ())
    });
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(()));
    assert_eq!(
        *seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec!["second", "first"]
    );
}

#[test]
#[allow(clippy::panic)] // The fresh release factory is a user-code boundary.
fn release_factory_defect_does_not_skip_remaining_finalizers() {
    let count = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&count);
    let program: Effect<(), Error> = Scope::scoped(move |scope| {
        let witness = Arc::clone(&witness);
        let first = scope.add_release(move || count_release(Arc::clone(&witness)));
        let last = scope.add_release::<()>(|| panic!("scope release factory"));
        first.zip(last).map(|_| ())
    });
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Succeeded,
            first: CleanupFailure::ReleaseDefect(Defect::from_panic(Box::new(
                "scope release factory"
            ))),
            rest: Vec::new(),
        })
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn abandoned_close_driver_wakes_joiners_and_owner_publishes_cleanup_defect() {
    let slot = Arc::new(Mutex::new(None::<Scope<Error>>));
    let witness = Arc::clone(&slot);
    let body_gate = Cancellation::new();
    let body_witness = body_gate.clone();
    let program: Effect<(), Error> = Scope::scoped(move |scope| {
        *witness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(scope.clone());
        let body_gate = body_witness.clone();
        scope
            .add_release(|| Release::<Error>::from_future(|_| std::future::pending()))
            .flat_map(move |()| {
                let body_gate = body_gate.clone();
                Effect::from_future(move |_| {
                    let body_gate = body_gate.clone();
                    async move { body_gate.cancelled().await }
                })
            })
    });
    let (mut driver, observer) = program.owned_run(Arc::new(()));
    drive::suspended(Pin::new(&mut driver));
    let scope = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    assert!(scope.is_some());
    if let Some(scope) = scope {
        let mut closer = Box::pin(scope.close::<()>().run(Arc::new(())));
        drive::suspended(closer.as_mut());
        let mut joined = Box::pin(scope.closed());
        let mut context = Context::from_waker(Waker::noop());
        assert!(joined.as_mut().poll(&mut context).is_pending());
        drop(closer);
        assert_eq!(block_on(joined), ScopeCloseStatus::Failed);
        assert!(body_gate.request());
        assert_eq!(
            block_on(join(driver, observer)),
            (
                (),
                Exit::FinalizationFailed(Finalization {
                    primary: PrimaryOutcome::Succeeded,
                    first: CleanupFailure::ReleaseDefect(Defect::from(
                        LibraryDefect::ScopeCloseDriverDropped
                    )),
                    rest: Vec::new(),
                })
            )
        );
    }
}

#[test]
fn late_registration_releases_immediately_and_retains_its_failure() {
    let count = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&count);
    let program: Effect<(), Error> = Scope::scoped(move |scope| {
        let late = scope.add_release({
            let witness = Arc::clone(&witness);
            move || {
                let witness = Arc::clone(&witness);
                Release::try_sync(move || {
                    let _ = witness.fetch_add(1, Ordering::SeqCst);
                    Err(Error::Release(std::cell::Cell::new(9)))
                })
            }
        });
        let close = scope.close::<()>();
        close.flat_map(move |_| late.clone())
    });
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Defected(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
            first: CleanupFailure::ReleaseFailure(Error::Release(std::cell::Cell::new(9))),
            rest: Vec::new(),
        })
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn scoped_acquisition_owns_late_success_before_cancellation() {
    let releases = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&releases);
    let bodies = Arc::new(AtomicUsize::new(0));
    let body_witness = Arc::clone(&bodies);
    let token = Arc::new(Cancellation::new());
    let program: Effect<(), Error, Cancellation> = Scope::scoped(move |scope| {
        let witness = Arc::clone(&witness);
        let body_witness = Arc::clone(&body_witness);
        scope
            .acquire(
                Effect::from_future(|cancel| async move {
                    cancel.cancelled().await;
                    String::from("owned")
                }),
                move |value| {
                    let witness = Arc::clone(&witness);
                    Release::sync(move || {
                        assert_eq!(value, "owned");
                        let _ = witness.fetch_add(1, Ordering::SeqCst);
                    })
                },
            )
            .flat_map(move |_| {
                let _ = body_witness.fetch_add(1, Ordering::SeqCst);
                Effect::succeed(())
            })
    });
    let (mut driver, observer) = program.owned_run_cancellable(Arc::clone(&token));
    drive::suspended(Pin::new(&mut driver));
    assert!(token.request());
    let ((), outcome) = block_on(join(driver, observer));
    assert_eq!(outcome, Exit::Interrupted);
    assert_eq!(bodies.load(Ordering::SeqCst), 0);
    assert_eq!(releases.load(Ordering::SeqCst), 1);
}

#[test]
fn registration_close_races_release_each_obligation_once() -> Result<(), Box<dyn std::error::Error>>
{
    let body_gate = Cancellation::new();
    let witness = body_gate.clone();
    let slot = Arc::new(Mutex::new(None::<Scope<Error>>));
    let slot_witness = Arc::clone(&slot);
    let program: Effect<(), Error> = Scope::scoped(move |scope| {
        *slot_witness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(scope);
        let witness = witness.clone();
        Effect::from_future(move |_| {
            let witness = witness.clone();
            async move { witness.cancelled().await }
        })
    });
    let (mut driver, observer) = program.owned_run(Arc::new(()));
    drive::suspended(Pin::new(&mut driver));
    let scope = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .ok_or("missing scope")?;
    let barrier = Arc::new(Barrier::new(33));
    let count = Arc::new(AtomicUsize::new(0));
    let (sent, received) = mpsc::channel();
    let mut workers = Vec::new();
    for _ in 0..32 {
        let scope = scope.clone();
        let barrier = Arc::clone(&barrier);
        let count = Arc::clone(&count);
        let sent = sent.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            let result = block_on(
                scope
                    .add_release(move || count_release(Arc::clone(&count)))
                    .run(Arc::new(())),
            );
            let _ = sent.send(matches!(result, Exit::Success(()) | Exit::Die(_)));
        }));
    }
    let closer = scope.clone();
    let sent = sent.clone();
    workers.push(std::thread::spawn(move || {
        barrier.wait();
        let result = block_on(closer.close::<()>().run(Arc::new(())));
        let _ = sent.send(result == Exit::Success(ScopeCloseStatus::Clean));
    }));
    for _ in 0..33 {
        assert!(received.recv_timeout(std::time::Duration::from_secs(5))?);
    }
    for worker in workers {
        assert!(worker.join().is_ok());
    }
    assert_eq!(count.load(Ordering::SeqCst), 32);
    assert!(body_gate.request());
    assert_eq!(block_on(join(driver, observer)), ((), Exit::Success(())));
    Ok(())
}

#[test]
fn abandoned_owner_wakes_closed_observers_with_failure() {
    let slot = Arc::new(Mutex::new(None::<Scope<Error>>));
    let witness = Arc::clone(&slot);
    let program: Effect<(), Error> = Scope::scoped(move |scope| {
        *witness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(scope);
        Effect::from_future(|_| std::future::pending())
    });
    let (mut driver, observer) = program.owned_run(Arc::new(()));
    drive::suspended(Pin::new(&mut driver));
    let scope = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    drop(driver);
    assert_eq!(
        block_on(observer),
        Exit::Die(Defect::from(LibraryDefect::RunDriverDropped))
    );
    assert!(scope.is_some());
    if let Some(scope) = scope {
        assert_eq!(block_on(scope.closed()), ScopeCloseStatus::Failed);
    }
}
