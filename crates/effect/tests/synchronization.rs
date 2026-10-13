use effect::{effect_for_each_concurrent, effect_future, effect_ref};
extern crate effect as fx;

use effect::{
    AtomicRef, Cancellation, CleanupFailure, Defect, DefectKind, Deferred, Effect, Exit,
    Finalization, Latch, LibraryDefect, PrimaryOutcome,
};
use futures_executor::block_on;
use std::cell::Cell;
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex, mpsc};
use std::task::{Context, Wake, Waker};
use std::time::Duration;

#[path = "support/synchronization_cases.rs"]
mod portable;
#[path = "support/suspended.rs"]
mod suspension;
#[path = "support/task_cases.rs"]
pub mod tasks;

#[test]
fn external_synchronization_uses_only_public_apis_and_non_clone_payloads() {
    block_on(portable::synchronization_cases());
}

// Existing finite regression deadline; never a library scheduling policy.
const DEADLOCK_DEADLINE: Duration = Duration::from_secs(5);

fn success<A, E>(outcome: Exit<A, E>) -> Result<A, String> {
    match outcome {
        Exit::Success(value) => Ok(value),
        Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
            Err("fixture expected success".to_owned())
        }
    }
}

fn run<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    program: Effect<A, E, R>,
    env: Arc<R>,
) -> Result<Exit<A, E>, String> {
    let (send, receive) = mpsc::channel();
    let driver = std::thread::spawn(move || {
        assert!(send.send(block_on(program.run(env))).is_ok());
    });
    let outcome = receive
        .recv_timeout(DEADLOCK_DEADLINE)
        .map_err(|error| format!("synchronization operation did not settle: {error}"))?;
    assert!(driver.join().is_ok());
    Ok(outcome)
}

fn limit(value: usize) -> NonZeroUsize {
    assert!(value > 0, "fixture concurrency must be positive");
    NonZeroUsize::MIN.saturating_add(value - 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Value,
    Error,
    Cleanup,
    Later,
}

// Payloads deliberately implement neither Clone nor Sync.
#[derive(Debug, PartialEq, Eq)]
struct Payload(Cell<Marker>);

fn success_outcome() -> Exit<Payload, Payload> {
    Exit::Success(Payload(Cell::new(Marker::Value)))
}
fn failure_outcome() -> Exit<Payload, Payload> {
    Exit::Fail(Payload(Cell::new(Marker::Error)))
}
fn defect_outcome() -> Exit<Payload, Payload> {
    Exit::Die(Defect::library(LibraryDefect::TaskSubmissionRejected))
}
fn interrupted_outcome() -> Exit<Payload, Payload> {
    Exit::Interrupted
}
fn cleanup_outcome() -> Exit<Payload, Payload> {
    Exit::FinalizationFailed(Finalization {
        primary: PrimaryOutcome::Failed(Payload(Cell::new(Marker::Error))),
        first: CleanupFailure::ReleaseFailure(Payload(Cell::new(Marker::Cleanup))),
        rest: vec![CleanupFailure::ReleaseFailure(Payload(Cell::new(
            Marker::Later,
        )))],
    })
}

#[test]
fn deferred_first_completion_preserves_every_outcome_and_ordered_cleanup() -> Result<(), String> {
    // Each terminal category from Facilities.fs, with additional ordered cleanup.
    for outcome in [
        success_outcome,
        failure_outcome,
        defect_outcome,
        interrupted_outcome,
        cleanup_outcome,
    ] {
        let deferred: Deferred<Payload, Payload> = Deferred::new();
        assert!(matches!(
            block_on(deferred.poll_shared().run(Arc::new(()))),
            Exit::Success(None)
        ));
        assert_eq!(
            block_on(deferred.complete(outcome()).run(Arc::new(()))),
            Exit::Success(true)
        );
        assert_eq!(
            block_on(deferred.complete(success_outcome()).run(Arc::new(()))),
            Exit::Success(false)
        );
        let polled = success(block_on(deferred.poll_shared().run(Arc::new(()))))?;
        assert!(polled.is_some());
        if let Some(polled) = polled {
            assert_eq!(polled.with(|stored| assert_eq!(*stored, outcome())), Ok(()));
            drop(polled);
        }
        assert_eq!(
            block_on(deferred.await_value().run(Arc::new(()))),
            outcome()
        );
        assert!(matches!(
            block_on(deferred.await_value().run(Arc::new(()))),
            Exit::Die(defect)
                if defect.kind() == DefectKind::Library(LibraryDefect::DeferredOutcomeConsumed)
        ));
        assert!(matches!(
            block_on(deferred.poll_shared().run(Arc::new(()))),
            Exit::Die(defect)
                if defect.kind() == DefectKind::Library(LibraryDefect::DeferredOutcomeConsumed)
        ));
        assert_eq!(
            block_on(deferred.complete(outcome()).run(Arc::new(()))),
            Exit::Success(false)
        );
    }
    Ok(())
}

#[test]
fn deferred_completion_recipe_moves_once_and_precancel_does_not_consume_it() {
    let deferred: Deferred<Payload, Payload> = Deferred::new();
    let completion = deferred.succeed(Payload(Cell::new(Marker::Value)));
    let cancelled = Arc::new(tasks::Environment::default());
    assert!(cancelled.cancellation.request());
    assert_eq!(
        block_on(completion.clone().run_cancellable(cancelled)),
        Exit::Interrupted
    );
    let active = Arc::new(tasks::Environment::default());
    assert_eq!(
        block_on(completion.clone().run(Arc::clone(&active))),
        Exit::Success(true)
    );
    assert_eq!(block_on(completion.run(active)), Exit::Success(false));
    assert_eq!(
        block_on(deferred.await_value().run(Arc::new(()))),
        success_outcome()
    );
}

#[test]
fn cancelling_or_dropping_deferred_waiters_leaves_completion_and_other_waiters_intact()
-> Result<(), String> {
    let deferred: Deferred<Payload, Payload> = Deferred::new();
    let env = Arc::new(tasks::Environment::default());
    let mut cancelled = Box::pin(deferred.await_value().run_cancellable(Arc::clone(&env)));
    let mut surviving = Box::pin(deferred.await_shared().run(Arc::new(())));
    let mut abandoned = Box::pin(deferred.await_shared().run(Arc::new(())));
    suspension::suspended(cancelled.as_mut());
    suspension::suspended(surviving.as_mut());
    suspension::suspended(abandoned.as_mut());
    drop(abandoned);
    assert!(env.cancellation.request());
    assert_eq!(block_on(cancelled), Exit::Interrupted);
    assert!(matches!(
        block_on(deferred.poll_shared().run(Arc::new(()))),
        Exit::Success(None)
    ));
    assert_eq!(
        block_on(
            deferred
                .fail(Payload(Cell::new(Marker::Error)))
                .run(Arc::new(()))
        ),
        Exit::Success(true)
    );
    let observed = success(block_on(surviving))?;
    assert_eq!(
        observed.with(|exit| assert_eq!(*exit, failure_outcome())),
        Ok(())
    );
    let later = success(block_on(deferred.await_shared().run(Arc::new(()))))?;
    assert_eq!(
        later.with(|exit| assert_eq!(*exit, failure_outcome())),
        Ok(())
    );
    drop((observed, later));
    assert_eq!(
        block_on(deferred.await_value().run(Arc::new(()))),
        failure_outcome()
    );
    Ok(())
}

struct Tracked {
    value: Cell<Marker>,
    drops: Arc<AtomicUsize>,
}
impl Drop for Tracked {
    fn drop(&mut self) {
        let _ = self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn shared_deferred_views_retain_identical_data_and_allow_later_unique_owned_transfer()
-> Result<(), String> {
    let drops = Arc::new(AtomicUsize::new(0));
    let deferred: Deferred<Tracked, Payload> = Deferred::new();
    assert_eq!(
        block_on(
            deferred
                .succeed(Tracked {
                    value: Cell::new(Marker::Value),
                    drops: Arc::clone(&drops),
                })
                .run(Arc::new(()))
        ),
        Exit::Success(true)
    );
    let first = success(block_on(deferred.await_shared().run(Arc::new(()))))?;
    let second = success(block_on(deferred.await_shared().run(Arc::new(()))))?;
    let address = |exit: &Exit<Tracked, Payload>| match exit {
        Exit::Success(value) => Some(std::ptr::from_ref(value)),
        Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
    };
    assert!(matches!(first.with(address), Ok(Some(_))));
    assert_eq!(first.with(address), second.with(address));
    assert!(matches!(
        block_on(deferred.await_value().run(Arc::new(()))),
        Exit::Die(defect)
            if defect.kind() == DefectKind::Library(LibraryDefect::DeferredOutcomeShared)
    ));
    assert_eq!(
        first.with(|exit| match exit {
            Exit::Success(value) => Some(value.value.get()),
            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
        }),
        Ok(Some(Marker::Value))
    );
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop((first, second));
    let owned = success(block_on(deferred.await_value().run(Arc::new(()))))?;
    assert_eq!(owned.value.get(), Marker::Value);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(owned);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn shared_deferred_data_outlives_its_handle_without_retaining_an_environment() -> Result<(), String>
{
    let drops = Arc::new(AtomicUsize::new(0));
    let deferred: Deferred<Tracked, Payload> = Deferred::new();
    assert_eq!(
        block_on(
            deferred
                .succeed(Tracked {
                    value: Cell::new(Marker::Value),
                    drops: Arc::clone(&drops),
                })
                .run(Arc::new(()))
        ),
        Exit::Success(true)
    );
    let environment = Arc::new(tasks::Environment::default());
    let weak_environment = Arc::downgrade(&environment);
    let shared = success(block_on(deferred.await_shared().run(environment)))?;
    assert!(weak_environment.upgrade().is_none());
    drop(deferred);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(matches!(
        shared.with(|exit| matches!(exit, Exit::Success(_))),
        Ok(true)
    ));
    drop(shared);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn deferred_factory_and_shared_inspection_panics_preserve_state() -> Result<(), String> {
    let deferred: Deferred<Payload, Payload> = Deferred::new();
    assert!(matches!(
        block_on(deferred.complete_with(|| std::panic::resume_unwind(Box::new("factory fixture"))).run(Arc::new(()))),
        Exit::Die(defect) if defect.kind() == DefectKind::CallbackPanic
    ));
    assert_eq!(
        block_on(deferred.complete(failure_outcome()).run(Arc::new(()))),
        Exit::Success(true)
    );
    let shared = success(block_on(deferred.await_shared().run(Arc::new(()))))?;
    assert!(
        matches!(shared.with::<()>(|_| std::panic::resume_unwind(Box::new("inspection fixture"))), Err(defect)
        if defect.kind() == DefectKind::CallbackPanic)
    );
    assert_eq!(
        shared.with(|exit| assert_eq!(*exit, failure_outcome())),
        Ok(())
    );
    drop(shared);
    assert_eq!(
        block_on(deferred.await_value().run(Arc::new(()))),
        failure_outcome()
    );
    Ok(())
}

struct CountWake(AtomicUsize);
impl Wake for CountWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn completion_broadcast_wakes_every_registered_deferred_waiter() -> Result<(), String> {
    const WAITERS: usize = 32; // Explicit broadcast coverage, not queue capacity.
    let deferred: Deferred<Payload, Payload> = Deferred::new();
    let mut waiters = Vec::new();
    for _ in 0..WAITERS {
        let mut waiter = Box::pin(deferred.await_shared().run(Arc::new(())));
        suspension::suspended(waiter.as_mut());
        let wake = Arc::new(CountWake(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wake));
        assert!(
            waiter
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert_eq!(wake.0.load(Ordering::SeqCst), 0);
        waiters.push((waiter, wake));
    }
    assert_eq!(
        block_on(deferred.complete(success_outcome()).run(Arc::new(()))),
        Exit::Success(true)
    );
    for (waiter, wake) in waiters {
        assert!(wake.0.load(Ordering::SeqCst) > 0);
        let shared = success(block_on(waiter))?;
        assert_eq!(
            shared.with(|exit| assert_eq!(*exit, success_outcome())),
            Ok(())
        );
    }
    Ok(())
}

struct InspectWake {
    deferred: Deferred<Payload, Payload>,
    inspected: AtomicBool,
}
impl Wake for InspectWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        // Poll an immediately runnable Effect directly: no nested executor.
        let mut poll = Box::pin(self.deferred.poll_shared().run(Arc::new(())));
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let std::task::Poll::Ready(outcome) = poll.as_mut().poll(&mut context) {
                assert!(matches!(outcome, Exit::Success(Some(_))));
                self.inspected.store(true, Ordering::SeqCst);
                return;
            }
        }
    }
}

#[test]
fn deferred_completion_invokes_wakers_after_releasing_its_gate() -> Result<(), String> {
    let deferred: Deferred<Payload, Payload> = Deferred::new();
    let mut waiter = Box::pin(deferred.await_shared().run(Arc::new(())));
    suspension::suspended(waiter.as_mut());
    let wake = Arc::new(InspectWake {
        deferred: deferred.clone(),
        inspected: AtomicBool::new(false),
    });
    let waker = Waker::from(Arc::clone(&wake));
    assert!(
        waiter
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert_eq!(
        run(deferred.complete(success_outcome()), Arc::new(()))?,
        Exit::Success(true)
    );
    assert!(wake.inspected.load(Ordering::SeqCst));
    let _ = success(block_on(waiter))?;
    Ok(())
}

#[test]
fn competing_completions_commit_exactly_one_candidate() -> Result<(), String> {
    const PARTICIPANTS: usize = 2; // Two actual completion callbacks meet at the barrier.
    const ATTEMPTS: usize = 32; // Finite race coverage of the source first-write contract.
    for _ in 0..ATTEMPTS {
        let deferred: Deferred<Payload, Payload> = Deferred::new();
        let barrier = Arc::new(Barrier::new(PARTICIPANTS));
        let env = Arc::new(tasks::Environment::default());
        let completing = deferred.clone();
        let operations = effect_for_each_concurrent! { concurrency: limit(PARTICIPANTS), values: vec![Marker::Value, Marker::Later], body: move |marker| {
            let marker = *marker;
            let barrier = Arc::clone(&barrier);
            completing.complete_with(move || {
                let _ = barrier.wait();
                Exit::Success(Payload(Cell::new(marker)))
            })
        }, };
        let accepted = success(run(operations, Arc::clone(&env))?)?;
        assert_eq!(accepted.iter().filter(|accepted| **accepted).count(), 1);
        let stored = success(block_on(deferred.await_value().run(Arc::new(()))))?;
        assert!(matches!(stored.0.get(), Marker::Value | Marker::Later));
        env.tasks.finish();
    }
    Ok(())
}

#[test]
fn synchronization_factories_are_lazy_and_produce_independent_state_per_run() -> Result<(), String>
{
    let factory_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&factory_calls);
    let references: Effect<AtomicRef<Payload>, Payload> = AtomicRef::make_with(move || {
        let _ = calls.fetch_add(1, Ordering::SeqCst);
        Payload(Cell::new(Marker::Value))
    });
    assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
    let first = success(block_on(references.clone().run(Arc::new(()))))?;
    let second = success(block_on(references.run(Arc::new(()))))?;
    assert_eq!(factory_calls.load(Ordering::SeqCst), 2);
    let update: Effect<(), Payload> = first.set_with(|| Payload(Cell::new(Marker::Later)));
    assert_eq!(block_on(update.run(Arc::new(()))), Exit::Success(()));
    let inspect: Effect<Marker, Payload> = second.with(|value| value.0.get());
    assert_eq!(
        block_on(inspect.run(Arc::new(()))),
        Exit::Success(Marker::Value)
    );

    let deferreds: Effect<Deferred<Payload, Payload>, Payload> = Deferred::make();
    let first = success(block_on(deferreds.clone().run(Arc::new(()))))?;
    let second = success(block_on(deferreds.run(Arc::new(()))))?;
    assert_eq!(
        block_on(first.complete(success_outcome()).run(Arc::new(()))),
        Exit::Success(true)
    );
    assert!(matches!(
        block_on(second.poll_shared().run(Arc::new(()))),
        Exit::Success(None)
    ));

    let latches: Effect<Latch, Payload> = Latch::make();
    let first = success(block_on(latches.clone().run(Arc::new(()))))?;
    let second = success(block_on(latches.run(Arc::new(()))))?;
    let opening: Effect<bool, Payload> = first.open();
    assert_eq!(block_on(opening.run(Arc::new(()))), Exit::Success(true));
    let waiting: Effect<(), Payload> = second.await_open();
    let mut waiting = Box::pin(waiting.run(Arc::new(())));
    suspension::suspended(waiting.as_mut());
    Ok(())
}

#[test]
fn latch_cancelled_and_abandoned_waiters_do_not_open_it_or_disturb_survivors() {
    let latch = Latch::new();
    let env = Arc::new(tasks::Environment::default());
    let cancelled: Effect<(), Payload, tasks::Environment> = latch.await_open();
    let waiting: Effect<(), Payload> = latch.await_open();
    let abandoned: Effect<(), Payload> = latch.await_open();
    let mut cancelled = Box::pin(cancelled.run_cancellable(Arc::clone(&env)));
    let mut waiting = Box::pin(waiting.run(Arc::new(())));
    let mut abandoned = Box::pin(abandoned.run(Arc::new(())));
    suspension::suspended(cancelled.as_mut());
    suspension::suspended(waiting.as_mut());
    suspension::suspended(abandoned.as_mut());
    drop(abandoned);
    assert!(env.cancellation.request());
    assert_eq!(block_on(cancelled), Exit::Interrupted);
    suspension::suspended(waiting.as_mut());
    let opening: Effect<bool, Payload> = latch.open();
    assert_eq!(
        block_on(opening.clone().run(Arc::new(()))),
        Exit::Success(true)
    );
    assert_eq!(block_on(waiting), Exit::Success(()));
    assert_eq!(block_on(opening.run(Arc::new(()))), Exit::Success(false));
    let late: Effect<(), Payload> = latch.await_open();
    assert_eq!(block_on(late.run(Arc::new(()))), Exit::Success(()));
}

#[test]
fn opening_broadcast_wakes_every_registered_latch_waiter() {
    const WAITERS: usize = 32; // Same explicit broadcast workload as the Deferred case.
    let latch = Latch::new();
    let mut waiters = Vec::new();
    for _ in 0..WAITERS {
        let waiting: Effect<(), Payload> = latch.await_open();
        let mut waiter = Box::pin(waiting.run(Arc::new(())));
        suspension::suspended(waiter.as_mut());
        let wake = Arc::new(CountWake(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wake));
        assert!(
            waiter
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        waiters.push((waiter, wake));
    }
    let opening: Effect<bool, Payload> = latch.open();
    assert_eq!(block_on(opening.run(Arc::new(()))), Exit::Success(true));
    for (waiter, wake) in waiters {
        assert!(wake.0.load(Ordering::SeqCst) > 0);
        assert_eq!(block_on(waiter), Exit::Success(()));
    }
}

#[test]
fn atomic_reference_preserves_non_clone_storage_during_borrowed_and_in_place_operations()
-> Result<(), String> {
    struct Buffer(Vec<Marker>);
    let reference = AtomicRef::new(Buffer(vec![Marker::Value, Marker::Later]));
    let inspect: Effect<(usize, Option<Marker>, Option<Marker>), Payload> =
        reference.with(|buffer| {
            (
                buffer.0.as_ptr() as usize,
                buffer.0.first().copied(),
                buffer.0.last().copied(),
            )
        });
    let before = success(block_on(inspect.clone().run(Arc::new(()))))?;
    let mutation: Effect<(), Payload> =
        effect_ref! { in_place; reference: reference, modify: |buffer| buffer.0.reverse(), };
    assert_eq!(block_on(mutation.run(Arc::new(()))), Exit::Success(()));
    let after = success(block_on(inspect.run(Arc::new(()))))?;
    assert_eq!(before.0, after.0);
    assert_eq!(
        (before.1, before.2),
        (Some(Marker::Value), Some(Marker::Later))
    );
    assert_eq!(
        (after.1, after.2),
        (Some(Marker::Later), Some(Marker::Value))
    );

    let modify: Effect<Payload, Payload> = effect_ref! { modify; reference: reference, modify: |buffer| {
        (
            Payload(Cell::new(Marker::Value)),
            Buffer(buffer.0.iter().rev().copied().collect()),
        )
    }, };
    assert_eq!(block_on(modify.run(Arc::new(()))), success_outcome());
    let replacement: Effect<(), Payload> =
        effect_ref! { update; reference: reference, update: |_| Buffer(Vec::new()), };
    assert_eq!(block_on(replacement.run(Arc::new(()))), Exit::Success(()));
    let empty: Effect<bool, Payload> = reference.with(|buffer| buffer.0.is_empty());
    assert_eq!(block_on(empty.run(Arc::new(()))), Exit::Success(true));
    Ok(())
}

#[test]
fn atomic_reference_callback_panics_release_the_gate_and_preserve_replacement_boundaries() {
    let reference = AtomicRef::new(Payload(Cell::new(Marker::Value)));
    let inspection: Effect<(), Payload> =
        reference.with(|_| std::panic::resume_unwind(Box::new("read fixture")));
    assert!(
        matches!(block_on(inspection.run(Arc::new(()))), Exit::Die(defect)
        if defect.kind() == DefectKind::CallbackPanic)
    );
    let replacement: Effect<(), Payload> = effect_ref! { update; reference: reference, update: |_| std::panic::resume_unwind(Box::new("replacement fixture")), };
    assert!(
        matches!(block_on(replacement.run(Arc::new(()))), Exit::Die(defect)
        if defect.kind() == DefectKind::CallbackPanic)
    );
    let factory: Effect<(), Payload> =
        reference.set_with(|| std::panic::resume_unwind(Box::new("set factory fixture")));
    assert!(
        matches!(block_on(factory.run(Arc::new(()))), Exit::Die(defect)
        if defect.kind() == DefectKind::CallbackPanic)
    );
    let current: Effect<Marker, Payload> = reference.with(|value| value.0.get());
    assert_eq!(
        block_on(current.clone().run(Arc::new(()))),
        Exit::Success(Marker::Value)
    );
    let in_place: Effect<(), Payload> = effect_ref! { in_place; reference: reference, modify: |value| {
        value.0.set(Marker::Later);
        std::panic::resume_unwind(Box::new("in-place fixture"))
    }, };
    assert!(
        matches!(block_on(in_place.run(Arc::new(()))), Exit::Die(defect)
        if defect.kind() == DefectKind::CallbackPanic)
    );
    assert_eq!(
        block_on(current.run(Arc::new(()))),
        Exit::Success(Marker::Later)
    );
}

#[test]
fn atomic_reference_updates_are_atomic_across_independent_workers() -> Result<(), String> {
    // Exact source Concurrency.fs workload, explicitly supplied fixture policy.
    const UPDATES: usize = 10_000;
    const WORKERS: usize = 16;
    struct Counter(Cell<usize>);
    let reference = AtomicRef::new(Counter(Cell::new(0)));
    let updating = reference.clone();
    let workers = effect_for_each_concurrent! { concurrency: limit(WORKERS), values: (0..UPDATES).collect(), body: move |_| {
        effect_ref! { update; reference: updating, update: |counter| Counter(Cell::new(counter.0.get() + 1)), }
    }, };
    let env = Arc::new(tasks::Environment::default());
    let result: Exit<Vec<()>, Payload> = run(workers, Arc::clone(&env))?;
    assert_eq!(success(result)?.len(), UPDATES);
    let current: Effect<usize, Payload> = reference.with(|counter| counter.0.get());
    assert_eq!(block_on(current.run(Arc::new(()))), Exit::Success(UPDATES));
    assert_eq!(env.tasks.submitted(), WORKERS);
    env.tasks.finish();
    Ok(())
}

struct DropGate {
    entered: Cancellation,
    release: Arc<(Mutex<bool>, Condvar)>,
}
impl Drop for DropGate {
    fn drop(&mut self) {
        let _ = self.entered.request();
        let (gate, changed) = &*self.release;
        let mut released = gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !*released {
            released = changed
                .wait(released)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}
struct ReleaseGate(Arc<(Mutex<bool>, Condvar)>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        let (gate, changed) = &*self.0;
        *gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        changed.notify_all();
    }
}

#[test]
fn replaced_reference_payloads_are_destroyed_outside_the_value_gate() -> Result<(), String> {
    struct State {
        marker: Marker,
        _drop: Option<DropGate>,
    }
    let entered = Cancellation::new();
    let released = Arc::new((Mutex::new(false), Condvar::new()));
    let release = ReleaseGate(Arc::clone(&released));
    let reference = AtomicRef::new(State {
        marker: Marker::Value,
        _drop: Some(DropGate {
            entered: entered.clone(),
            release: released,
        }),
    });
    let replacement: Effect<(), Payload> = reference.set_with(|| State {
        marker: Marker::Later,
        _drop: None,
    });
    let (send, receive) = mpsc::channel();
    let driver = std::thread::spawn(move || {
        assert!(send.send(block_on(replacement.run(Arc::new(())))).is_ok());
    });
    // An explicit destructor entry signal, with an external harness deadline.
    let entered_wait: Effect<(), Payload> = effect_future! { body: move |_| {
        let entered = entered.clone();
        async move { entered.cancelled().await }
    }, };
    assert_eq!(run(entered_wait, Arc::new(()))?, Exit::Success(()));
    let inspect: Effect<Marker, Payload> = reference.with(|state| state.marker);
    assert_eq!(run(inspect, Arc::new(()))?, Exit::Success(Marker::Later));
    assert!(receive.try_recv().is_err());
    drop(release);
    assert_eq!(
        receive.recv_timeout(DEADLOCK_DEADLINE),
        Ok(Exit::Success(()))
    );
    assert!(driver.join().is_ok());
    Ok(())
}

#[test]
fn rejected_deferred_candidates_are_destroyed_outside_the_completion_gate() -> Result<(), String> {
    struct State {
        marker: Marker,
        _drop: Option<DropGate>,
    }
    let deferred: Deferred<State, Payload> = Deferred::new();
    assert_eq!(
        block_on(
            deferred
                .succeed(State {
                    marker: Marker::Value,
                    _drop: None
                })
                .run(Arc::new(()))
        ),
        Exit::Success(true)
    );
    let entered = Cancellation::new();
    let released = Arc::new((Mutex::new(false), Condvar::new()));
    let release = ReleaseGate(Arc::clone(&released));
    let rejected = deferred.succeed(State {
        marker: Marker::Later,
        _drop: Some(DropGate {
            entered: entered.clone(),
            release: released,
        }),
    });
    let (send, receive) = mpsc::channel();
    let driver = std::thread::spawn(move || {
        assert!(send.send(block_on(rejected.run(Arc::new(())))).is_ok());
    });
    let entered_wait: Effect<(), Payload> = effect_future! { body: move |_| {
        let entered = entered.clone();
        async move { entered.cancelled().await }
    }, };
    assert_eq!(run(entered_wait, Arc::new(()))?, Exit::Success(()));
    let shared = success(run(deferred.await_shared(), Arc::new(()))?)?;
    assert_eq!(
        shared.with(|exit| match exit {
            Exit::Success(state) => Some(state.marker),
            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
        }),
        Ok(Some(Marker::Value))
    );
    assert!(receive.try_recv().is_err());
    drop(release);
    assert_eq!(
        receive.recv_timeout(DEADLOCK_DEADLINE),
        Ok(Exit::Success(false))
    );
    assert!(driver.join().is_ok());
    Ok(())
}

#[test]
fn explicit_reference_copying_constructors_and_snapshots_remain_reusable() -> Result<(), String> {
    let factory: Effect<AtomicRef<String>, Payload> = AtomicRef::make(String::from("initial"));
    let first = success(block_on(factory.clone().run(Arc::new(()))))?;
    let second = success(block_on(factory.run(Arc::new(()))))?;
    let set: Effect<(), Payload> = first.set(String::from("next"));
    assert_eq!(block_on(set.clone().run(Arc::new(()))), Exit::Success(()));
    assert_eq!(block_on(set.run(Arc::new(()))), Exit::Success(()));
    let get: Effect<String, Payload> = first.get();
    assert_eq!(
        block_on(get.clone().run(Arc::new(()))),
        Exit::Success(String::from("next"))
    );
    assert_eq!(
        block_on(get.run(Arc::new(()))),
        Exit::Success(String::from("next"))
    );
    let untouched: Effect<String, Payload> = second.get();
    assert_eq!(
        block_on(untouched.run(Arc::new(()))),
        Exit::Success(String::from("initial"))
    );
    Ok(())
}

#[test]
fn competing_owned_deferred_waiters_transfer_once_and_report_consumption() {
    let deferred: Deferred<Payload, Payload> = Deferred::new();
    let mut first = Box::pin(deferred.await_value().run(Arc::new(())));
    let mut second = Box::pin(deferred.await_value().run(Arc::new(())));
    suspension::suspended(first.as_mut());
    suspension::suspended(second.as_mut());
    assert_eq!(
        block_on(deferred.complete(cleanup_outcome()).run(Arc::new(()))),
        Exit::Success(true)
    );
    assert_eq!(block_on(first), cleanup_outcome());
    assert!(matches!(block_on(second), Exit::Die(defect)
        if defect.kind() == DefectKind::Library(LibraryDefect::DeferredOutcomeConsumed)));
}

#[test]
fn precancelled_synchronization_operations_start_no_callbacks_or_state_changes() {
    let cancelled = Arc::new(Cancellation::new());
    assert!(cancelled.request());
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let reference = AtomicRef::new(Payload(Cell::new(Marker::Value)));
    let mutation: Effect<(), Payload, Cancellation> = effect_ref! { update; reference: reference, update: move |_| {
        let _ = witness.fetch_add(1, Ordering::SeqCst);
        Payload(Cell::new(Marker::Later))
    }, };
    assert_eq!(
        block_on(mutation.run_cancellable(Arc::clone(&cancelled))),
        Exit::Interrupted
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let current: Effect<Marker, Payload> = reference.with(|value| value.0.get());
    assert_eq!(
        block_on(current.run(Arc::new(()))),
        Exit::Success(Marker::Value)
    );
    let latch = Latch::new();
    let opening: Effect<bool, Payload, Cancellation> = latch.open();
    assert_eq!(
        block_on(opening.clone().run_cancellable(cancelled)),
        Exit::Interrupted
    );
    assert_eq!(
        block_on(opening.run(Arc::new(Cancellation::new()))),
        Exit::Success(true)
    );
}

struct PanickingDrop {
    marker: Marker,
    panic_on_drop: bool,
}
impl Drop for PanickingDrop {
    fn drop(&mut self) {
        if self.panic_on_drop {
            std::panic::resume_unwind(Box::new("destruction fixture"));
        }
    }
}

#[test]
fn destruction_panics_report_defects_without_reverting_committed_state() -> Result<(), String> {
    let reference = AtomicRef::new(PanickingDrop {
        marker: Marker::Value,
        panic_on_drop: true,
    });
    let replacement: Effect<(), Payload> = effect_ref! { modify; reference: reference, modify: |_| {
        (
            (),
            PanickingDrop {
                marker: Marker::Later,
                panic_on_drop: false,
            },
        )
    }, };
    assert!(
        matches!(block_on(replacement.run(Arc::new(()))), Exit::Die(defect)
        if defect.kind() == DefectKind::CallbackPanic)
    );
    let current: Effect<Marker, Payload> = reference.with(|value| value.marker);
    assert_eq!(
        block_on(current.run(Arc::new(()))),
        Exit::Success(Marker::Later)
    );

    let deferred: Deferred<PanickingDrop, Payload> = Deferred::new();
    assert_eq!(
        block_on(
            deferred
                .succeed(PanickingDrop {
                    marker: Marker::Value,
                    panic_on_drop: false,
                })
                .run(Arc::new(()))
        ),
        Exit::Success(true)
    );
    assert!(matches!(block_on(deferred.succeed(PanickingDrop {
        marker: Marker::Later,
        panic_on_drop: true,
    }).run(Arc::new(()))), Exit::Die(defect)
        if defect.kind() == DefectKind::CallbackPanic));
    let retained = success(block_on(deferred.await_value().run(Arc::new(()))))?;
    assert_eq!(retained.marker, Marker::Value);
    Ok(())
}
