extern crate effect as fx;

use effect::{
    Cancellation, CleanupFailure, Defect, DefectKind, Effect, Exit, Finalization, HasTaskSpawner,
    LibraryDefect, PrimaryOutcome, ScheduledTask, TaskSpawner,
};
use futures_executor::block_on;
use std::cell::Cell;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::time::Duration;

#[path = "support/drive.rs"]
mod drive;
#[path = "support/suspended.rs"]
mod suspension;
#[path = "support/task_cases.rs"]
pub mod tasks;
use tasks::Environment;

// Fixture deadlock detection, not a production scheduling/latency policy.
const DEADLOCK_DEADLINE: Duration = Duration::from_secs(5);
#[allow(clippy::panic)] // Fail the harness directly rather than fabricate an Effect outcome.
fn run<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    program: Effect<A, E, R>,
    env: Arc<R>,
) -> Exit<A, E> {
    let (send, receive) = mpsc::channel();
    let owner = std::thread::spawn(move || {
        assert!(send.send(block_on(program.run(env))).is_ok());
    });
    let outcome = receive
        .recv_timeout(DEADLOCK_DEADLINE)
        .unwrap_or_else(|error| panic!("parallel operation did not settle: {error}"));
    assert!(owner.join().is_ok());
    outcome
}
#[allow(clippy::panic)] // Explicit positive fixture policy; an invalid fixture is a test failure.
fn limit(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap_or_else(|| panic!("fixture concurrency must be positive"))
}

// Result and expected-error payloads deliberately implement neither Clone nor Sync.
#[derive(Debug, PartialEq, Eq)]
struct Value(Cell<usize>);
#[derive(Debug, PartialEq, Eq)]
enum Error {
    Body(Cell<usize>),
    Release(Cell<usize>),
}

#[test]
fn fixed_workers_run_synchronous_bodies_independently_and_keep_input_order() {
    const WORKERS: usize = 4; // Four actual barrier participants, as in the source regression.
    const WAVES: usize = 2; // Reuse the same worker set for two complete barrier rounds.
    let inputs: Vec<_> = (0..WORKERS * WAVES).collect();
    let barrier = Arc::new(Barrier::new(WORKERS));
    let env = Arc::new(Environment::default());
    let program: Effect<Vec<(Value, std::thread::ThreadId)>, Error, Environment> =
        Effect::for_each_concurrent(limit(WORKERS), inputs, move |input| {
            let input = *input;
            let barrier = Arc::clone(&barrier);
            Effect::sync(move || {
                let _ = barrier.wait();
                (Value(Cell::new(input)), std::thread::current().id())
            })
        });
    let outcome = run(program, Arc::clone(&env));
    assert!(matches!(outcome, Exit::Success(_)));
    if let Exit::Success(values) = outcome {
        assert_eq!(
            values
                .iter()
                .map(|(value, _)| value.0.get())
                .collect::<Vec<_>>(),
            (0..WORKERS * WAVES).collect::<Vec<_>>()
        );
        let workers: std::collections::HashSet<_> =
            values.iter().map(|(_, worker)| *worker).collect();
        assert_eq!(workers.len(), WORKERS);
    }
    assert_eq!(env.tasks.submitted(), WORKERS);
    env.tasks.finish();
}

#[test]
fn bounded_reservations_do_not_exceed_limit_and_late_results_keep_order() {
    const WORKERS: usize = 2; // Two concurrent jobs, each with its own explicit completion gate.
    let env = Arc::new(Environment::default());
    let entered: Arc<Vec<_>> = Arc::new((0..WORKERS).map(|_| Cancellation::new()).collect());
    let gates: Arc<Vec<_>> = Arc::new((0..WORKERS).map(|_| Cancellation::new()).collect());
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicUsize::new(0));
    let e = Arc::clone(&entered);
    let g = Arc::clone(&gates);
    let a = Arc::clone(&active);
    let m = Arc::clone(&maximum);
    let s = Arc::clone(&started);
    let program: Effect<Vec<Value>, Error, Environment> =
        Effect::for_each_concurrent(limit(WORKERS), vec![0, 1, 2, 3], move |input| {
            let input = *input;
            let e = Arc::clone(&e);
            let g = Arc::clone(&g);
            let a = Arc::clone(&a);
            let m = Arc::clone(&m);
            let s = Arc::clone(&s);
            Effect::from_future(move |_| {
                let _ = s.fetch_add(1, Ordering::SeqCst);
                let now = a.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = m.fetch_max(now, Ordering::SeqCst);
                let a = Arc::clone(&a);
                let gate = g.get(input).cloned();
                if let Some(entered) = e.get(input) {
                    let _ = entered.request();
                }
                async move {
                    if let Some(gate) = gate {
                        gate.cancelled().await;
                    }
                    let _ = a.fetch_sub(1, Ordering::SeqCst);
                    Value(Cell::new(input))
                }
            })
        });
    let (mut driver, observer) = program.owned_run(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || {
        entered.iter().all(Cancellation::is_requested)
    });
    assert_eq!(started.load(Ordering::SeqCst), WORKERS);
    assert_eq!(active.load(Ordering::SeqCst), WORKERS);
    if let Some(last) = gates.last() {
        assert!(last.request());
    }
    if let Some(first) = gates.first() {
        assert!(first.request());
    }
    block_on(driver);
    assert_eq!(
        block_on(observer),
        Exit::Success((0..4).map(|input| Value(Cell::new(input))).collect())
    );
    assert_eq!(maximum.load(Ordering::SeqCst), WORKERS);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    env.tasks.finish();
}

#[test]
fn shared_non_clone_inputs_are_borrowed_and_runs_create_fresh_owned_results() {
    struct Input(String); // No Clone: callbacks must borrow the shared collection.
    let env = Arc::new(Environment::default());
    let inputs = vec![Input(String::from("one")), Input(String::from("second"))];
    let expected: Vec<_> = inputs.iter().map(|input| input.0.len()).collect();
    let addresses = Arc::new(Mutex::new(Vec::new()));
    let witness = Arc::clone(&addresses);
    let program: Effect<Vec<Value>, Error, Environment> =
        Effect::for_each_concurrent(NonZeroUsize::MIN, inputs, move |input| {
            witness
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(std::ptr::from_ref(input) as usize);
            let length = input.0.len();
            Effect::sync(move || Value(Cell::new(length)))
        });
    for _ in 0..2 {
        // Two executions must refer to the same immutable inputs.
        assert_eq!(
            run(program.clone(), Arc::clone(&env)),
            Exit::Success(
                expected
                    .iter()
                    .map(|value| Value(Cell::new(*value)))
                    .collect()
            )
        );
    }
    let addresses = addresses
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut runs = addresses.chunks_exact(expected.len());
    assert_eq!(runs.next(), runs.next());
    env.tasks.finish();
}

#[test]
fn first_failure_stops_reservation_without_creating_later_callbacks() {
    let called = Arc::new(Mutex::new(Vec::new()));
    let witness = Arc::clone(&called);
    let env = Arc::new(Environment::default());
    let program: Effect<Vec<Value>, Error, Environment> =
        Effect::for_each_concurrent(NonZeroUsize::MIN, vec![0, 1, 2], move |input| {
            witness
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(*input);
            let input = *input;
            if input == 1 {
                Effect::fail_with(move || Error::Body(Cell::new(input)))
            } else {
                Effect::sync(move || Value(Cell::new(input)))
            }
        });
    assert_eq!(
        run(program, Arc::clone(&env)),
        Exit::Fail(Error::Body(Cell::new(1)))
    );
    assert_eq!(
        *called
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![0, 1]
    );
    env.tasks.finish();
}

#[test]
fn concurrent_failure_cancels_and_joins_started_sibling_release() {
    const WORKERS: usize = 2;
    let env = Arc::new(Environment::default());
    let entered = Cancellation::new();
    let releasing = Cancellation::new();
    let finish = Cancellation::new();
    let start = entered.clone();
    let release = releasing.clone();
    let gate = finish.clone();
    let sibling: Effect<Value, Error, Environment> = Effect::from_future(move |token| {
        let _ = start.request();
        async move {
            token.cancelled().await;
            Value(Cell::new(1))
        }
    })
    .ensuring(Effect::try_future(
        move |_| {
            let _ = release.request();
            let gate = gate.clone();
            async move {
                gate.cancelled().await;
                Err::<(), _>(Error::Release(Cell::new(2)))
            }
        },
        |error| error,
    ));
    let start = entered.clone();
    let failure: Effect<Value, Error, Environment> = Effect::try_future(
        move |_| {
            let start = start.clone();
            async move {
                start.cancelled().await;
                Err::<Value, _>(Error::Body(Cell::new(3)))
            }
        },
        |error| error,
    );
    let program = Effect::all_concurrent(limit(WORKERS), vec![sibling, failure]);
    let (mut driver, observer) = program.owned_run(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || releasing.is_requested());
    suspension::suspended(std::pin::Pin::new(&mut driver));
    assert!(finish.request());
    block_on(driver);
    assert_eq!(
        block_on(observer),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(Error::Body(Cell::new(3))),
            first: CleanupFailure::ReleaseFailure(Error::Release(Cell::new(2))),
            rest: Vec::new(),
        })
    );
    env.tasks.finish();
}

#[test]
fn parent_cancellation_joins_all_concurrent_resources() {
    const WORKERS: usize = 2;
    let env = Arc::new(Environment::default());
    let entered = Arc::new(AtomicUsize::new(0));
    let releasing = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicUsize::new(0));
    let finish = Cancellation::new();
    let start = Arc::clone(&entered);
    let release = Arc::clone(&releasing);
    let done = Arc::clone(&released);
    let gate = finish.clone();
    let program: Effect<Vec<Value>, Error, Environment> =
        Effect::for_each_concurrent(limit(WORKERS), (0..WORKERS).collect(), move |input| {
            let input = *input;
            let start = Arc::clone(&start);
            let release = Arc::clone(&release);
            let done = Arc::clone(&done);
            let gate = gate.clone();
            Effect::from_future(move |token| {
                let _ = start.fetch_add(1, Ordering::SeqCst);
                async move {
                    token.cancelled().await;
                    Value(Cell::new(input))
                }
            })
            .ensuring(Effect::from_future(move |_| {
                let _ = release.fetch_add(1, Ordering::SeqCst);
                let gate = gate.clone();
                let done = Arc::clone(&done);
                async move {
                    gate.cancelled().await;
                    let _ = done.fetch_add(1, Ordering::SeqCst);
                }
            }))
        });
    let (mut driver, observer) = program.owned_run_cancellable(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || {
        entered.load(Ordering::SeqCst) == WORKERS
    });
    assert!(env.cancellation.request());
    drive::until(std::pin::Pin::new(&mut driver), || {
        releasing.load(Ordering::SeqCst) == WORKERS
    });
    suspension::suspended(std::pin::Pin::new(&mut driver));
    assert!(finish.request());
    block_on(driver);
    assert_eq!(block_on(observer), Exit::Interrupted);
    assert_eq!(released.load(Ordering::SeqCst), WORKERS);
    env.tasks.finish();
}

#[test]
fn synchronous_race_competitors_progress_on_independent_workers() {
    const COMPETITORS: usize = 2;
    let env = Arc::new(Environment::default());
    let barrier = Arc::new(Barrier::new(COMPETITORS));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let contender = |input| {
        let barrier = Arc::clone(&barrier);
        let seen = Arc::clone(&seen);
        Effect::<Value, Error, Environment>::sync(move || {
            seen.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(std::thread::current().id());
            let _ = barrier.wait();
            Value(Cell::new(input))
        })
    };
    let outcome = run(contender(1).race(contender(2)), Arc::clone(&env));
    assert!(matches!(outcome, Exit::Success(ref value) if [1, 2].contains(&value.0.get())));
    let seen = seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(seen.len(), COMPETITORS);
    assert_ne!(seen.first(), seen.last());
    env.tasks.finish();
}

#[test]
fn race_selects_terminal_failure_then_joins_loser_before_returning() {
    let env = Arc::new(Environment::default());
    let entered = Cancellation::new();
    let releasing = Cancellation::new();
    let finish = Cancellation::new();
    let start = entered.clone();
    let release = releasing.clone();
    let gate = finish.clone();
    let loser: Effect<Value, Error, Environment> = Effect::from_future(move |token| {
        let _ = start.request();
        async move {
            token.cancelled().await;
            Value(Cell::new(1))
        }
    })
    .ensuring(Effect::try_future(
        move |_| {
            let _ = release.request();
            let gate = gate.clone();
            async move {
                gate.cancelled().await;
                Err::<(), _>(Error::Release(Cell::new(2)))
            }
        },
        |error| error,
    ));
    let start = entered.clone();
    let winner: Effect<Value, Error, Environment> = Effect::try_future(
        move |_| {
            let start = start.clone();
            async move {
                start.cancelled().await;
                Err::<Value, _>(Error::Body(Cell::new(3)))
            }
        },
        |error| error,
    );
    let program = loser.race(winner);
    let (mut driver, observer) = program.owned_run(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || releasing.is_requested());
    suspension::suspended(std::pin::Pin::new(&mut driver));
    assert!(finish.request());
    block_on(driver);
    assert_eq!(
        block_on(observer),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(Error::Body(Cell::new(3))),
            first: CleanupFailure::ReleaseFailure(Error::Release(Cell::new(2))),
            rest: Vec::new(),
        })
    );
    env.tasks.finish();
}

#[test]
fn race_discards_loser_ordinary_error_but_retains_its_cleanup_error() {
    let env = Arc::new(Environment::default());
    let entered = Cancellation::new();
    let gate = Cancellation::new();
    let finish = gate.clone();
    let child_token = Arc::new(Mutex::new(None::<Cancellation>));
    let token_witness = Arc::clone(&child_token);
    let start = entered.clone();
    let loser: Effect<Value, Error, Environment> = Effect::try_future(
        move |_| {
            let _ = start.request();
            let finish = finish.clone();
            async move {
                finish.cancelled().await;
                Err::<Value, _>(Error::Body(Cell::new(1)))
            }
        },
        |error| error,
    )
    .uninterruptible()
    .ensuring(Effect::fail_with(|| Error::Release(Cell::new(2))));
    let loser = Effect::<(), Error, Environment>::from_future(move |token| {
        *token_witness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(token);
        std::future::ready(())
    })
    .flat_map(move |()| loser.clone());
    let start = entered.clone();
    let winner = Effect::from_future(move |_| {
        let start = start.clone();
        async move {
            start.cancelled().await;
            Value(Cell::new(3))
        }
    });
    let program = winner.race(loser);
    let (mut driver, observer) = program.owned_run(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || {
        child_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(Cancellation::is_requested)
    });
    suspension::suspended(std::pin::Pin::new(&mut driver));
    assert!(gate.request());
    block_on(driver);
    assert_eq!(
        block_on(observer),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Succeeded,
            first: CleanupFailure::ReleaseFailure(Error::Release(Cell::new(2))),
            rest: Vec::new(),
        })
    );
    env.tasks.finish();
}

#[derive(Clone, Copy)]
enum Fault {
    Reject,
    Lose,
}
impl TaskSpawner for Fault {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        drop(task);
        match self {
            Self::Reject => Err(Defect::library(LibraryDefect::TaskSubmissionRejected)),
            Self::Lose => Ok(()),
        }
    }
}
struct Faulty(Fault);
impl HasTaskSpawner for Faulty {
    type Tasks = Fault;
    fn task_spawner(&self) -> &Fault {
        &self.0
    }
}

#[test]
fn accepted_worker_loss_settles_traversal_and_preserves_every_ownership_defect() {
    const WORKERS: usize = 2;
    let programs = vec![
        Effect::<Value, Error, Faulty>::sync(|| Value(Cell::new(1))),
        Effect::sync(|| Value(Cell::new(2))),
    ];
    assert_eq!(
        run(
            Effect::all_concurrent(limit(WORKERS), programs),
            Arc::new(Faulty(Fault::Lose))
        ),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Defected(Defect::library(LibraryDefect::ScheduledTaskDropped)),
            first: CleanupFailure::ReleaseDefect(Defect::library(
                LibraryDefect::ScheduledTaskDropped
            )),
            rest: Vec::new(),
        })
    );
}

#[test]
fn race_driver_loss_wakes_selection_and_retains_loser_ownership_failure() {
    let left = Effect::<Value, Error, Faulty>::sync(|| Value(Cell::new(1)));
    let right = Effect::sync(|| Value(Cell::new(2)));
    assert_eq!(
        run(left.race(right), Arc::new(Faulty(Fault::Lose))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Defected(Defect::library(LibraryDefect::ScheduledTaskDropped)),
            first: CleanupFailure::ReleaseDefect(Defect::library(
                LibraryDefect::ScheduledTaskDropped
            )),
            rest: Vec::new(),
        })
    );
}

#[test]
fn rejected_submission_reports_one_primary_failure_without_running_jobs() {
    let called = Arc::new(AtomicBool::new(false));
    let witness = Arc::clone(&called);
    let job: Effect<Value, Error, Faulty> = Effect::sync(move || {
        witness.store(true, Ordering::SeqCst);
        Value(Cell::new(1))
    });
    assert_eq!(
        run(
            Effect::all_concurrent(NonZeroUsize::MIN, vec![job.clone()]),
            Arc::new(Faulty(Fault::Reject))
        ),
        Exit::Die(Defect::library(LibraryDefect::TaskSubmissionRejected))
    );
    assert_eq!(
        run(job.clone().race(job), Arc::new(Faulty(Fault::Reject))),
        Exit::Die(Defect::library(LibraryDefect::TaskSubmissionRejected))
    );
    assert!(!called.load(Ordering::SeqCst));
}

#[test]
#[allow(clippy::panic)] // User transform panic is an ordinary defect, not successful traversal.
fn transform_panics_are_defects_and_skip_unreserved_inputs() {
    let env = Arc::new(Environment::default());
    let called = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&called);
    let program: Effect<Vec<Value>, Error, Environment> =
        Effect::for_each_concurrent(NonZeroUsize::MIN, vec![1, 2], move |_| {
            let _ = witness.fetch_add(1, Ordering::SeqCst);
            panic!("intentional transform panic")
        });
    assert!(
        matches!(run(program, Arc::clone(&env)), Exit::Die(ref defect) if defect.kind() == DefectKind::CallbackPanic)
    );
    assert_eq!(called.load(Ordering::SeqCst), 1);
    env.tasks.finish();
}

#[test]
fn empty_traversal_needs_no_task_submission_even_with_rejecting_provider() {
    let program = Effect::<Value, Error, Faulty>::all_concurrent(NonZeroUsize::MIN, Vec::new());
    assert_eq!(
        run(program, Arc::new(Faulty(Fault::Reject))),
        Exit::Success(Vec::new())
    );
}

#[path = "support/parallel_cases.rs"]
mod portable;

#[test]
fn external_provider_supports_shared_inputs_descriptions_and_races() {
    block_on(portable::parallel_cases());
}

#[derive(Clone, Copy)]
enum Boundary {
    Root,
    ScopedFiber,
}
fn cancel_after_winner(boundary: Boundary) -> Exit<(), Error> {
    let env = Arc::new(Environment::default());
    let entered = Cancellation::new();
    let releasing = Cancellation::new();
    let finish = Cancellation::new();
    let start = entered.clone();
    let release = releasing.clone();
    let gate = finish.clone();
    let loser: Effect<Value, Error, Environment> = Effect::from_future(move |token| {
        let _ = start.request();
        async move {
            token.cancelled().await;
            Value(Cell::new(1))
        }
    })
    .ensuring(Effect::from_future(move |_| {
        let _ = release.request();
        let gate = gate.clone();
        async move { gate.cancelled().await }
    }));
    let start = entered.clone();
    let winner: Effect<Value, Error, Environment> = Effect::try_future(
        move |_| {
            let start = start.clone();
            async move {
                start.cancelled().await;
                Err::<Value, _>(Error::Body(Cell::new(2)))
            }
        },
        |error| error,
    )
    .uninterruptible();
    let race = winner.race(loser);
    let program = match boundary {
        Boundary::Root => race.map(drop),
        Boundary::ScopedFiber => effect::Scope::scoped(move |scope| {
            scope
                .fork(race.clone())
                .flat_map(|_| Effect::from_future(|token| async move { token.cancelled().await }))
        }),
    };
    let (mut driver, observer) = program.owned_run_cancellable(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || releasing.is_requested());
    assert!(env.cancellation.request());
    suspension::suspended(std::pin::Pin::new(&mut driver));
    assert!(finish.request());
    block_on(driver);
    let outcome = block_on(observer);
    env.tasks.finish();
    outcome
}

#[test]
fn root_race_publishes_interruption_when_parent_cancels_during_loser_join() {
    assert_eq!(cancel_after_winner(Boundary::Root), Exit::Interrupted);
}

#[test]
fn nested_race_retains_protected_winner_failure_for_its_cancelled_fiber_owner() {
    assert_eq!(
        cancel_after_winner(Boundary::ScopedFiber),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::WasInterrupted,
            first: CleanupFailure::ReleaseFailure(Error::Body(Cell::new(2))),
            rest: Vec::new(),
        })
    );
}

#[test]
fn committed_right_winner_is_not_replaced_by_left_biased_status_polling() {
    // Admission remains owned while the fixture pauses driving. Resumed tasks
    // run on independent workers; the parent is deliberately not polled while
    // the right completion commits, followed by the left completion.
    #[derive(Default)]
    struct Paused(Mutex<Vec<ScheduledTask>>);
    impl TaskSpawner for Paused {
        fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(task);
            Ok(())
        }
    }
    impl HasTaskSpawner for Paused {
        type Tasks = Self;
        fn task_spawner(&self) -> &Self {
            self
        }
    }
    const COMPETITORS: usize = 2;
    let env = Arc::new(Paused::default());
    let left = Effect::<Value, Error, Paused>::sync(|| Value(Cell::new(1)));
    let right = Effect::sync(|| Value(Cell::new(2)));
    let program = left.race(right);
    let (mut driver, observer) = program.owned_run(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || {
        env.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
            == COMPETITORS
    });
    let pending = std::mem::take(
        &mut *env
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    let mut tasks = pending.into_iter();
    let left = tasks.next();
    let right = tasks.next();
    assert!(left.is_some() && right.is_some() && tasks.next().is_none());
    if let Some(right) = right {
        assert!(std::thread::spawn(move || block_on(right)).join().is_ok());
    }
    if let Some(left) = left {
        assert!(std::thread::spawn(move || block_on(left)).join().is_ok());
    }
    block_on(driver);
    assert_eq!(block_on(observer), Exit::Success(Value(Cell::new(2))));
}

#[test]
fn cleanup_groups_commit_in_order_before_discarded_primary_destructors() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Label {
        FirstBody,
        SecondBody,
        ThirdBody,
        SecondCleanup,
        ThirdCleanup,
    }
    struct DropGate {
        entered: Cancellation,
        allowed: Mutex<bool>,
        changed: std::sync::Condvar,
    }
    struct Failure {
        label: Label,
        gate: Option<Arc<DropGate>>,
    }
    impl std::fmt::Debug for Failure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.label.fmt(f)
        }
    }
    impl PartialEq for Failure {
        fn eq(&self, other: &Self) -> bool {
            self.label == other.label
        }
    }
    impl Eq for Failure {}
    impl Drop for Failure {
        fn drop(&mut self) {
            if let Some(gate) = &self.gate {
                let _ = gate.entered.request();
                let allowed = gate
                    .allowed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let result = gate
                    .changed
                    .wait_timeout_while(allowed, DEADLOCK_DEADLINE, |allowed| !*allowed)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                assert!(
                    *result.0,
                    "discarded primary destructor did not receive its explicit release"
                );
            }
        }
    }
    const WORKERS: usize = 3; // First terminal error, then two ordered cleanup groups.
    let env = Arc::new(Environment::default());
    let drop_gate = Arc::new(DropGate {
        entered: Cancellation::new(),
        allowed: Mutex::new(false),
        changed: std::sync::Condvar::new(),
    });
    let started = Arc::new(AtomicUsize::new(0));
    let all_entered = Cancellation::new();
    let second = Cancellation::new();
    let third = Cancellation::new();
    let group_token = Arc::new(Mutex::new(None::<Cancellation>));
    let token = Arc::clone(&group_token);
    let count = Arc::clone(&started);
    let ready = all_entered.clone();
    let second_gate = second.clone();
    let third_gate = third.clone();
    let blocked = Arc::clone(&drop_gate);
    let jobs: Effect<Vec<Value>, Failure, Environment> = Effect::for_each_concurrent(
        limit(WORKERS),
        vec![Label::FirstBody, Label::SecondBody, Label::ThirdBody],
        move |label| {
            let label = *label;
            let count = Arc::clone(&count);
            let ready = ready.clone();
            let second = second_gate.clone();
            let third = third_gate.clone();
            let blocked = Arc::clone(&blocked);
            let body: Effect<Value, Failure, Environment> = Effect::try_future(
                move |_| {
                    if count.fetch_add(1, Ordering::SeqCst) + 1 == WORKERS {
                        let _ = ready.request();
                    }
                    let ready = ready.clone();
                    let second = second.clone();
                    let third = third.clone();
                    let blocked = Arc::clone(&blocked);
                    async move {
                        match label {
                            Label::FirstBody => ready.cancelled().await,
                            Label::SecondBody => second.cancelled().await,
                            Label::ThirdBody => third.cancelled().await,
                            Label::SecondCleanup | Label::ThirdCleanup => {}
                        }
                        Err::<Value, _>(Failure {
                            label,
                            gate: if label == Label::SecondBody {
                                Some(blocked)
                            } else {
                                None
                            },
                        })
                    }
                },
                |error| error,
            )
            .uninterruptible();
            let body = match label {
                Label::FirstBody => body,
                Label::SecondBody | Label::ThirdBody => {
                    body.ensuring(Effect::fail_with(move || Failure {
                        label: if label == Label::SecondBody {
                            Label::SecondCleanup
                        } else {
                            Label::ThirdCleanup
                        },
                        gate: None,
                    }))
                }
                Label::SecondCleanup | Label::ThirdCleanup => body,
            };
            let token = Arc::clone(&token);
            Effect::from_future(move |cancellation| {
                *token
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cancellation);
                std::future::ready(())
            })
            .flat_map(move |()| body.clone())
        },
    );
    // Fiber publication preserves protected expected errors after group cancel.
    let program =
        effect::Scope::scoped(move |scope| scope.fork(jobs.clone()).flat_map(|fiber| fiber.join()));
    let (mut driver, observer) = program.owned_run(Arc::clone(&env));
    drive::until(std::pin::Pin::new(&mut driver), || {
        all_entered.is_requested()
            && group_token
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_some_and(Cancellation::is_requested)
    });
    assert!(second.request());
    drive::until(std::pin::Pin::new(&mut driver), || {
        drop_gate.entered.is_requested()
    });
    assert!(third.request());
    // First and third workers can finish while the second primary destructor is
    // blocked. This proves the destructor holds no batch bookkeeping gate.
    drive::until(std::pin::Pin::new(&mut driver), || {
        env.tasks.settled() >= WORKERS - 1
    });
    *drop_gate
        .allowed
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
    drop_gate.changed.notify_all();
    block_on(driver);
    assert_eq!(
        block_on(observer),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(Failure {
                label: Label::FirstBody,
                gate: None
            }),
            first: CleanupFailure::ReleaseFailure(Failure {
                label: Label::SecondCleanup,
                gate: None
            }),
            rest: vec![CleanupFailure::ReleaseFailure(Failure {
                label: Label::ThirdCleanup,
                gate: None
            })],
        })
    );
    env.tasks.finish();
}

#[test]
fn projected_task_provider_overrides_one_subtree_without_replacing_root_cancellation() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Tag {
        Parent,
        Projected,
    }
    struct Provider {
        threads: tasks::Threads,
        calls: AtomicUsize,
    }
    impl TaskSpawner for Provider {
        fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
            let _ = self.calls.fetch_add(1, Ordering::SeqCst);
            self.threads.spawn(task)
        }
    }
    struct Parent {
        tasks: Arc<Provider>,
        projected_tasks: Arc<Provider>,
        cancellation: Cancellation,
        projected_cancellation: Cancellation,
        root_reads: AtomicUsize,
        projected_reads: Arc<AtomicUsize>,
    }
    struct Projected {
        tasks: Arc<Provider>,
        cancellation: Cancellation,
        reads: Arc<AtomicUsize>,
    }
    impl HasTaskSpawner for Parent {
        type Tasks = Provider;
        fn task_spawner(&self) -> &Provider {
            &self.tasks
        }
    }
    impl HasTaskSpawner for Projected {
        type Tasks = Provider;
        fn task_spawner(&self) -> &Provider {
            &self.tasks
        }
    }
    impl effect::HasCancellation for Parent {
        fn cancellation(&self) -> &Cancellation {
            let _ = self.root_reads.fetch_add(1, Ordering::SeqCst);
            &self.cancellation
        }
    }
    impl effect::HasCancellation for Projected {
        fn cancellation(&self) -> &Cancellation {
            let _ = self.reads.fetch_add(1, Ordering::SeqCst);
            &self.cancellation
        }
    }
    let env = Arc::new(Parent {
        tasks: Arc::new(Provider {
            threads: tasks::Threads::default(),
            calls: AtomicUsize::new(0),
        }),
        projected_tasks: Arc::new(Provider {
            threads: tasks::Threads::default(),
            calls: AtomicUsize::new(0),
        }),
        cancellation: Cancellation::new(),
        projected_cancellation: Cancellation::new(),
        root_reads: AtomicUsize::new(0),
        projected_reads: Arc::new(AtomicUsize::new(0)),
    });
    assert!(env.projected_cancellation.request());
    let parent: Effect<Tag, Error, Parent> = Effect::service(|_| Tag::Parent);
    let projected: Effect<Tag, Error, Projected> = Effect::service(|_| Tag::Projected);
    let loser = Effect::from_future(|token| async move {
        token.cancelled().await;
        Tag::Projected
    });
    let projected = projected
        .race(loser)
        .provide_some(|parent: &Parent| Projected {
            tasks: Arc::clone(&parent.projected_tasks),
            cancellation: parent.projected_cancellation.clone(),
            reads: Arc::clone(&parent.projected_reads),
        });
    const BRANCHES: usize = 2; // One parent branch and one projected race subtree.
    let program = Effect::all_concurrent(limit(BRANCHES), vec![parent, projected]);
    assert_eq!(
        block_on(program.run_cancellable(Arc::clone(&env))),
        Exit::Success(vec![Tag::Parent, Tag::Projected])
    );
    assert_eq!(env.root_reads.load(Ordering::SeqCst), 1);
    assert_eq!(env.projected_reads.load(Ordering::SeqCst), 0);
    assert_eq!(env.tasks.calls.load(Ordering::SeqCst), BRANCHES);
    assert_eq!(env.projected_tasks.calls.load(Ordering::SeqCst), BRANCHES);
    env.tasks.threads.finish();
    env.projected_tasks.threads.finish();
}
