use effect::{
    effect, effect_future, effect_map, effect_resource, effect_sandbox, effect_scope, effect_sync,
    effect_try_future, effect_uninterruptible,
};
extern crate effect as fx;

use effect::{
    Cancellation, CleanupFailure, Defect, DefectKind, Effect, Exit, FiberStatus, Finalization,
    HasTaskSpawner, LibraryDefect, PrimaryOutcome, ScheduledTask, TaskSpawner,
};
use futures_executor::block_on;
use futures_util::FutureExt;
use std::cell::Cell;
use std::sync::{Arc, Barrier, mpsc};
use std::time::Duration;

#[path = "support/drive.rs"]
mod drive;
#[path = "support/suspended.rs"]
mod suspension;
#[path = "support/task_cases.rs"]
mod tasks;
use tasks::{Environment, Marker, Payload};

// A fixture deadline detects deadlocks; it is not Effect scheduling policy or
// a production latency target. Signals/barriers determine execution order.
const DEADLOCK_DEADLINE: Duration = Duration::from_secs(5);

// Intentional harness failure; a deadlock must fail without inventing an Exit.

fn run<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    program: Effect<A, E, R>,
    env: Arc<R>,
) -> Result<Exit<A, E>, String> {
    let (send, receive) = mpsc::channel();
    let owner = std::thread::spawn(move || {
        assert!(send.send(block_on(program.run(env))).is_ok());
    });
    let outcome = receive
        .recv_timeout(DEADLOCK_DEADLINE)
        .map_err(|error| format!("child ownership/observation did not settle: {error}"))?;
    assert!(owner.join().is_ok());
    Ok(outcome)
}

#[test]
fn external_provider_transfers_and_shares_non_clone_non_sync_payloads() {
    block_on(tasks::task_cases());
}

#[test]
fn shared_views_reuse_payload_then_allow_owned_transfer_and_exact_destruction() -> Result<(), String>
{
    struct Owned(Arc<std::sync::atomic::AtomicUsize>, Cell<Marker>);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let destroyed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let witness = Arc::clone(&destroyed);
    let env = Arc::new(Environment::default());
    let program: Effect<Owned, Payload, Environment> = effect_scope! {
    body: move |scope| {
            let witness = Arc::clone(&witness);
            effect! {
     bind;
     program: scope
                .fork(effect_sync! { body: move || {
                    Owned(Arc::clone(&witness), Cell::new(Marker::Value))
                }, }),
     body: |fiber| {
                    effect! {
                        let fiber = fiber.clone();
                        let view = fiber.await_exit_shared()?;
                        let rejected = effect_sandbox! { program: fiber.join(), defect: |defect| {
                            assert_eq!(defect.kind(), DefectKind::Library(LibraryDefect::FiberOutcomeShared));
                            Payload(Cell::new(Marker::Error))
                        }, }.either()?;
                        assert!(rejected.is_err());
                        assert_eq!(view.with(|exit| matches!(exit, Exit::Success(value) if value.1.get() == Marker::Value)), Ok(true));
                        let repeated = view.clone(); // Shared ownership, no payload clone.
                        assert_eq!(view.with(|exit| match exit {
                            Exit::Success(value) => Some(std::ptr::from_ref(value)),
                            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
                        }), repeated.with(|exit| match exit {
                            Exit::Success(value) => Some(std::ptr::from_ref(value)),
                            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
                        }));
                        drop(view);
                        drop(repeated);
                        fiber.join()?
                    }
                },
     }
        },
    };
    let outcome = run(program, Arc::clone(&env))?;
    assert!(matches!(outcome, Exit::Success(ref value) if value.1.get() == Marker::Value));
    assert_eq!(destroyed.load(std::sync::atomic::Ordering::SeqCst), 0);
    drop(outcome);
    env.tasks.finish();
    assert_eq!(destroyed.load(std::sync::atomic::Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn shared_handle_release_restores_unique_owned_join() -> Result<(), String> {
    let env = Arc::new(Environment::default());
    let program: Effect<Payload, Payload, Environment> = effect_scope! {
    body: |scope| {
            effect! {
     bind;
     program: scope
                .fork(effect_sync! { body: || Payload(Cell::new(Marker::Value)), }),
     body: |fiber| {
                    effect! {
                        let fiber = fiber.clone();
                        let view = fiber.await_exit_shared()?;
                        assert_eq!(view.with(|exit| matches!(exit, Exit::Success(_))), Ok(true));
                        drop(view);
                        fiber.join()?
                    }
                },
     }
        },
    };
    assert_eq!(
        run(program, Arc::clone(&env))?,
        Exit::Success(Payload(Cell::new(Marker::Value)))
    );
    env.tasks.finish();
    Ok(())
}

#[test]
fn shared_inspection_uses_separate_gate_from_completion_status() -> Result<(), String> {
    let env = Arc::new(Environment::default());
    let program: Effect<(), Payload, Environment> = effect_scope! {
    body: |scope| {
            effect! {
     bind;
     program: scope
                .fork(effect_sync! { body: || Payload(Cell::new(Marker::Value)), }),
     body: |fiber| {
                    let witness = fiber.clone();
                    effect_map! { program: fiber.await_exit_shared(), map: move |view| {
                        assert_eq!(
                            view.with(|_| witness.settled().now_or_never()),
                            Ok(Some(FiberStatus::Succeeded))
                        );
                    }, }
                },
     }
        },
    };
    assert_eq!(run(program, Arc::clone(&env))?, Exit::Success(()));
    env.tasks.finish();
    Ok(())
}

#[test]
fn scoped_children_drive_synchronous_work_on_independent_workers() -> Result<(), String> {
    // Two children are the two actual barrier participants: on a serial worker
    // they cannot both enter this callback, even with cooperative Future polls.
    const PARTICIPANTS: usize = 2;
    let barrier = Arc::new(Barrier::new(PARTICIPANTS));
    let env = Arc::new(Environment::default());
    let program: Effect<Vec<std::thread::ThreadId>, Payload, Environment> = effect_scope! {
    body: move |scope| {
                let barrier = Arc::clone(&barrier);
                let child: Effect<std::thread::ThreadId, Payload, Environment> =
                    effect_sync! { body: move || {
                        let _ = barrier.wait();
                        std::thread::current().id()
                    }, };
                let second = scope.fork(child.clone());
                effect! {
     bind;
     program: scope.fork(child),
     body: move |first| {
                    let first = first.clone();
                    effect! {
     bind;
     program: second.clone(),
     body: move |second| {
                        effect! {
     bind;
     program: first
                            .join(),
     body: move |left| effect_map! { program: second.join(), map: move |right| vec![left, right], },
     }
                    },
     }
                },
     }
            },
    };
    let outcome = run(program, Arc::clone(&env))?;
    assert!(
        matches!(outcome, Exit::Success(ref workers) if workers.len() == PARTICIPANTS && workers.first() != workers.last())
    );
    env.tasks.finish();
    Ok(())
}

#[test]
fn repeated_interrupt_observations_share_the_same_interrupted_exit() -> Result<(), String> {
    let entered = Cancellation::new();
    let witness = entered.clone();
    let env = Arc::new(Environment::default());
    let child: Effect<(), Payload, Environment> = effect_future! { body: move |token| {
        let _ = witness.request();
        async move { token.cancelled().await }
    }, };
    let program: Effect<(), Payload, Environment> = effect_scope! {
    body: move |scope| {
            let child = child.clone();
            let entered = entered.clone();
            effect! {
     bind;
     program: scope.fork(child),
     body: move |fiber| {
                let entered = entered.clone();
                effect! {
                    let fiber = fiber.clone();
                    let entered = entered.clone();
                    effect_future! { body: move |_| {
                        let entered = entered.clone();
                        async move { entered.cancelled().await }
                    }, }?;
                    let first = fiber.interrupt_shared()?;
                    let second = fiber.interrupt_shared()?;
                    let last = fiber.await_exit_shared()?;
                    assert_eq!(first.with(|exit| matches!(exit, Exit::Interrupted)), Ok(true));
                    assert_eq!(second.status(), FiberStatus::Interrupted);
                    assert_eq!(last.status(), FiberStatus::Interrupted);
                }
            },
     }
        },
    };
    assert_eq!(run(program, Arc::clone(&env))?, Exit::Success(()));
    env.tasks.finish();
    Ok(())
}

#[test]
fn parent_cancellation_joins_protected_child_and_retains_unobserved_error() {
    let entered = Cancellation::new();
    let finish = Cancellation::new();
    let witness = entered.clone();
    let gate = finish.clone();
    let env = Arc::new(Environment::default());
    let child: Effect<(), Payload, Environment> = effect_uninterruptible! { program: effect_try_future! { body: move |_| {
        let _ = witness.request();
        let gate = gate.clone();
        async move {
            gate.cancelled().await;
            Err::<(), _>(Payload(Cell::new(Marker::Error)))
        }
    }, on_error: |error| error, }, };
    let program: Effect<(), Payload, Environment> = effect_scope! {
    body: move |scope| {
            effect! {
     bind;
     program: scope
                .fork(child.clone()),
     body: |_| effect_future! { body: |token| async move { token.cancelled().await }, },
     }
        },
    };
    let (mut driver, observer) = program.owned_run_cancellable(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || entered.is_requested());
    assert!(env.cancellation.request());
    suspension::suspended(std::pin::Pin::new(&mut driver));
    assert!(finish.request());
    block_on(driver);
    assert_eq!(
        block_on(observer),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::WasInterrupted,
            first: CleanupFailure::ReleaseFailure(Payload(Cell::new(Marker::Error))),
            rest: Vec::new(),
        })
    );
    env.tasks.finish();
}

#[derive(Clone, Copy)]
enum ProviderBehavior {
    Reject,
    Lose,
}
impl TaskSpawner for ProviderBehavior {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        drop(task);
        match self {
            Self::Reject => Err(Defect::from(LibraryDefect::TaskSubmissionRejected)),
            Self::Lose => Ok(()),
        }
    }
}
struct Controlled(ProviderBehavior);
impl HasTaskSpawner for Controlled {
    type Tasks = ProviderBehavior;
    fn task_spawner(&self) -> &ProviderBehavior {
        &self.0
    }
}

#[test]
fn provider_rejection_reports_primary_defect_without_duplicate_loss() -> Result<(), String> {
    let program: Effect<(), Payload, Controlled> = effect_scope! {
    body: |scope| effect_map! { program: scope.fork(effect_sync! { body: || (), }), map: |_| (), },
    };
    assert_eq!(
        run(program, Arc::new(Controlled(ProviderBehavior::Reject)))?,
        Exit::Die(Defect::from(LibraryDefect::TaskSubmissionRejected))
    );
    Ok(())
}

#[test]
fn accepted_driver_loss_remains_an_unobserved_cleanup_defect() -> Result<(), String> {
    let program: Effect<(), Payload, Controlled> = effect_scope! {
    body: |scope| effect_map! { program: scope.fork(effect_sync! { body: || (), }), map: |_| (), },
    };
    assert_eq!(
        run(program, Arc::new(Controlled(ProviderBehavior::Lose)))?,
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Succeeded,
            first: CleanupFailure::ReleaseDefect(Defect::from(LibraryDefect::ScheduledTaskDropped)),
            rest: Vec::new(),
        })
    );
    Ok(())
}

#[test]
fn observing_accepted_driver_loss_handles_it_once() -> Result<(), String> {
    let program: Effect<(), Payload, Controlled> = effect_scope! {
    body: |scope| {
            effect! {
     bind;
     program: scope.fork(effect_sync! { body: || (), }),
     body: |fiber| {
                effect_map! { program: fiber.await_exit(), map: |exit| {
                    assert_eq!(
                        exit,
                        Exit::Die(Defect::from(LibraryDefect::ScheduledTaskDropped))
                    );
                }, }
            },
     }
        },
    };
    assert_eq!(
        run(program, Arc::new(Controlled(ProviderBehavior::Lose)))?,
        Exit::Success(())
    );
    Ok(())
}

#[test]
fn late_fork_rejects_before_child_callbacks_or_provider_access() -> Result<(), String> {
    let env = Arc::new(Environment::default());
    let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let witness = Arc::clone(&started);
    let child: Effect<(), Payload, Environment> =
        effect_sync! { body: move || witness.store(true, std::sync::atomic::Ordering::SeqCst), };
    let program = effect_scope! {
    body: move |scope| {
            let child = child.clone();
            effect_map! { program: effect! {
     bind;
     program: scope
                .close(),
     body: move |_| scope.fork(child.clone()),
     }, map: |_| (), }
        },
    };
    assert_eq!(
        run(program, Arc::clone(&env))?,
        Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed))
    );
    assert!(!started.load(std::sync::atomic::Ordering::SeqCst));
    env.tasks.finish();
    Ok(())
}

#[test]
fn interruption_waits_for_child_release_before_observation() {
    let releasing = Cancellation::new();
    let finish = Cancellation::new();
    let entered = Cancellation::new();
    let env = Arc::new(Environment::default());
    let started = entered.clone();
    let witness = releasing.clone();
    let gate = finish.clone();
    let child: Effect<(), Payload, Environment> = effect_resource! {
    ensuring; program: effect_future! { body: move |token| {
            let _ = started.request();
            async move { token.cancelled().await }
        }, }, release: { let __dsl_cleanup = effect_future! { body: move |_| {
            let witness = witness.clone();
            let gate = gate.clone();
            async move {
                let _ = witness.request();
                gate.cancelled().await;
            }
        }, }; move |_| __dsl_cleanup.clone() },
    };
    let program = effect_scope! {
    body: move |scope| {
            let entered = entered.clone();
            effect! {
     bind;
     program: scope.fork(child.clone()),
     body: move |fiber| {
                let entered = entered.clone();
                effect! {
     bind;
     program: effect_future! { body: move |_| {
                    let entered = entered.clone();
                    async move { entered.cancelled().await }
                }, },
     body: move |()| fiber.interrupt(),
     }
            },
     }
        },
    };
    let (mut driver, observer) = program.owned_run(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || releasing.is_requested());
    suspension::suspended(std::pin::Pin::new(&mut driver));
    assert!(finish.request());
    block_on(driver);
    assert_eq!(block_on(observer), Exit::Success(Exit::Interrupted));
    env.tasks.finish();
}

#[test]
fn cancelled_external_waiter_leaves_protected_failure_for_the_scope_owner() {
    let entered = Cancellation::new();
    let finish_child = Cancellation::new();
    let finish_body = Cancellation::new();
    let slot = Arc::new(std::sync::Mutex::new(None));
    let published = Arc::clone(&slot);
    let witness = entered.clone();
    let child_gate = finish_child.clone();
    let body_gate = finish_body.clone();
    let child: Effect<(), Payload, Environment> = effect_uninterruptible! { program: effect_try_future! { body: move |_| {
        let _ = witness.request();
        let gate = child_gate.clone();
        async move {
            gate.cancelled().await;
            Err::<(), _>(Payload(Cell::new(Marker::Error)))
        }
    }, on_error: |error| error, }, };
    let env = Arc::new(Environment::default());
    let program: Effect<(), Payload, Environment> = effect_scope! {
    body: move |scope| {
            let published = Arc::clone(&published);
            let body_gate = body_gate.clone();
            effect! {
     bind;
     program: scope.fork(child.clone()),
     body: move |fiber| {
                *published
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(fiber);
                let gate = body_gate.clone();
                effect_future! { body: move |_| {
                    let gate = gate.clone();
                    async move { gate.cancelled().await }
                }, }
            },
     }
        },
    };
    let (mut driver, observer) = program.owned_run(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || {
        entered.is_requested()
            && slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some()
    });
    let fiber = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    assert!(fiber.is_some());
    if let Some(fiber) = fiber {
        let cancellation = Arc::new(Cancellation::new());
        let (mut waiter, waited) = fiber
            .await_exit()
            .owned_run_cancellable(Arc::clone(&cancellation));
        suspension::suspended(std::pin::Pin::new(&mut waiter));
        assert!(cancellation.request());
        block_on(waiter);
        assert_eq!(block_on(waited), Exit::Interrupted);
    }
    assert!(finish_child.request());
    assert!(finish_body.request());
    block_on(driver);
    assert_eq!(
        block_on(observer),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Succeeded,
            first: CleanupFailure::ReleaseFailure(Payload(Cell::new(Marker::Error))),
            rest: Vec::new(),
        })
    );
    env.tasks.finish();
}

#[test]
fn provider_getter_panic_and_pre_cancel_are_owned_without_starting_user_work() -> Result<(), String>
{
    struct Panicking {
        cancellation: Cancellation,
        called: std::sync::atomic::AtomicBool,
    }
    impl HasTaskSpawner for Panicking {
        type Tasks = tasks::Threads;
        fn task_spawner(&self) -> &tasks::Threads {
            self.called.store(true, std::sync::atomic::Ordering::SeqCst);
            std::panic::resume_unwind(Box::new("intentional provider access panic"))
        }
    }
    impl effect::HasCancellation for Panicking {
        fn cancellation(&self) -> &Cancellation {
            &self.cancellation
        }
    }
    let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let witness = Arc::clone(&started);
    let program: Effect<(), Payload, Panicking> = effect_scope! {
    body: move |scope| {
            let witness = Arc::clone(&witness);
            effect_map! { program: scope
                .fork(effect_sync! { body: move || {
                    witness.store(true, std::sync::atomic::Ordering::SeqCst)
                }, }), map: |_| (), }
        },
    };
    let env = Arc::new(Panicking {
        cancellation: Cancellation::new(),
        called: std::sync::atomic::AtomicBool::new(false),
    });
    assert!(
        matches!(run(program.clone(), Arc::clone(&env))?, Exit::Die(ref defect) if defect.kind() == DefectKind::CallbackPanic)
    );
    assert!(env.called.swap(false, std::sync::atomic::Ordering::SeqCst));
    assert!(!started.load(std::sync::atomic::Ordering::SeqCst));
    assert!(env.cancellation.request());
    assert_eq!(
        block_on(program.run_cancellable(Arc::clone(&env))),
        Exit::Interrupted
    );
    assert!(!env.called.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!started.load(std::sync::atomic::Ordering::SeqCst));
    Ok(())
}

#[test]
fn shared_inspection_panic_leaves_data_available_to_later_inspections() -> Result<(), String> {
    let env = Arc::new(Environment::default());
    let program: Effect<(), Payload, Environment> = effect_scope! {
    body: |scope| {
            effect! {
     bind;
     program: scope.fork(effect_sync! { body: || Payload(Cell::new(Marker::Value)), }),
     body: |fiber| {
                effect_map! { program: fiber.await_exit_shared(), map: |view| {
                    let failure = view.with::<()>(|_| std::panic::resume_unwind(Box::new("intentional inspection panic")));
                    assert!(matches!(failure, Err(ref defect) if defect.kind() == DefectKind::CallbackPanic));
                    assert_eq!(view.with(|exit| matches!(exit, Exit::Success(value) if value.0.get() == Marker::Value)), Ok(true));
                }, }
            },
     }
        },
    };
    assert_eq!(run(program, Arc::clone(&env))?, Exit::Success(()));
    env.tasks.finish();
    Ok(())
}
