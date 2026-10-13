use effect::{effect, effect_future, effect_map, effect_resource, effect_scope};
extern crate effect as fx;

use effect::{
    AsyncQueue, Cancellation, CleanupFailure, Defect, DefectKind, Effect, Exit, LibraryDefect,
    PrimaryOutcome, PrimaryOutcomeRef, QueueCapacity, QueueError, QueueShutdown, ScheduledTask,
    TaskSpawner,
};
use futures_executor::block_on;
use std::cell::Cell;
use std::collections::VecDeque;
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex, mpsc};
use std::task::{Context, Wake, Waker};
use std::time::{Duration, Instant};

#[path = "support/queue_cases.rs"]
mod portable;
#[path = "support/suspended.rs"]
mod suspension;
#[path = "support/task_cases.rs"]
pub mod tasks;
#[path = "../src/internal/waiting.rs"]
mod waiting;

#[test]
fn external_queue_workflows_move_payloads_and_compose_with_scoped_children() {
    block_on(portable::queue_cases());
}

// The same finite deadlock-detection deadline as existing lifecycle regressions.
const DEADLOCK_DEADLINE: Duration = Duration::from_secs(5);

fn bounded(capacity: usize) -> QueueCapacity {
    assert!(capacity > 0, "positive capacity required");
    QueueCapacity::Bounded(NonZeroUsize::MIN.saturating_add(capacity - 1))
}

fn success<A>(outcome: Exit<A, QueueError>) -> Result<A, String> {
    match outcome {
        Exit::Success(value) => Ok(value),
        Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
            Err("expected fixture success".to_owned())
        }
    }
}

fn run<A: Send + 'static, R: Send + Sync + 'static>(
    program: Effect<A, QueueError, R>,
    env: Arc<R>,
) -> Result<Exit<A, QueueError>, String> {
    let (send, receive) = mpsc::channel();
    let driver = std::thread::spawn(move || {
        assert!(send.send(block_on(program.run(env))).is_ok());
    });
    let outcome = receive
        .recv_timeout(DEADLOCK_DEADLINE)
        .map_err(|error| format!("queue operation did not settle: {error}"))?;
    assert!(driver.join().is_ok());
    Ok(outcome)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    First,
    Second,
    Third,
    Fourth,
}

#[derive(Debug, PartialEq, Eq)]
struct Payload(Cell<Marker>); // Neither Clone nor Sync.

fn payload(marker: Marker) -> Payload {
    Payload(Cell::new(marker))
}
fn offer(queue: &AsyncQueue<Payload>, marker: Marker) {
    assert_eq!(
        block_on(queue.offer(payload(marker)).run(Arc::new(()))),
        Exit::Success(())
    );
}
fn take(queue: &AsyncQueue<Payload>, marker: Marker) {
    assert_eq!(
        block_on(queue.take().run(Arc::new(()))),
        Exit::Success(payload(marker))
    );
}
fn size<A: Send + 'static>(queue: &AsyncQueue<A>) -> Result<usize, String> {
    success(block_on(queue.size().run(Arc::new(()))))
}

#[test]
fn unbounded_queue_moves_non_clone_values_in_fifo_order_and_reports_only_buffered_size()
-> Result<(), String> {
    let queue = AsyncQueue::new(QueueCapacity::Unbounded);
    let values = [Marker::First, Marker::Second, Marker::Third];
    for value in values {
        offer(&queue, value);
    }
    assert_eq!(size(&queue)?, values.len());
    for value in values {
        take(&queue, value);
    }
    assert_eq!(size(&queue)?, 0);
    Ok(())
}

#[test]
fn bounded_queue_applies_backpressure_and_refills_from_pending_offers_in_fifo_order()
-> Result<(), String> {
    let queue = AsyncQueue::new(bounded(1)); // One real buffer slot exercises every refill.
    offer(&queue, Marker::First);
    let mut second = Box::pin(queue.offer(payload(Marker::Second)).run(Arc::new(())));
    let mut third = Box::pin(queue.offer(payload(Marker::Third)).run(Arc::new(())));
    suspension::suspended(second.as_mut());
    suspension::suspended(third.as_mut());
    assert_eq!(size(&queue)?, 1);
    take(&queue, Marker::First);
    assert_eq!(block_on(second), Exit::Success(()));
    assert_eq!(size(&queue)?, 1);
    take(&queue, Marker::Second);
    assert_eq!(block_on(third), Exit::Success(()));
    take(&queue, Marker::Third);
    assert_eq!(size(&queue)?, 0);
    Ok(())
}

#[test]
fn rendezvous_requires_a_peer_and_preserves_waiting_taker_order() -> Result<(), String> {
    let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
    let mut first_offer = Box::pin(queue.offer(payload(Marker::First)).run(Arc::new(())));
    suspension::suspended(first_offer.as_mut());
    assert_eq!(size(&queue)?, 0);
    take(&queue, Marker::First);
    assert_eq!(block_on(first_offer), Exit::Success(()));
    let mut first = Box::pin(queue.take().run(Arc::new(())));
    let mut second = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(first.as_mut());
    suspension::suspended(second.as_mut());
    offer(&queue, Marker::Second);
    offer(&queue, Marker::Third);
    assert_eq!(block_on(first), Exit::Success(payload(Marker::Second)));
    assert_eq!(block_on(second), Exit::Success(payload(Marker::Third)));
    Ok(())
}

#[test]
fn requested_offer_is_removed_before_a_peer_can_match_it_even_when_its_driver_is_unpolled() {
    let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
    let cancellation = Arc::new(Cancellation::new());
    let mut cancelled = Box::pin(
        queue
            .offer(payload(Marker::First))
            .run_cancellable(Arc::clone(&cancellation)),
    );
    suspension::suspended(cancelled.as_mut());
    assert!(cancellation.request());
    let mut waiting = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(waiting.as_mut());
    assert_eq!(block_on(cancelled), Exit::Interrupted);
    offer(&queue, Marker::Second);
    assert_eq!(block_on(waiting), Exit::Success(payload(Marker::Second)));
}

#[test]
fn requested_taker_is_skipped_before_a_peer_can_commit_to_it() {
    let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
    let cancellation = Arc::new(Cancellation::new());
    let mut cancelled = Box::pin(queue.take().run_cancellable(Arc::clone(&cancellation)));
    let mut survivor = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(cancelled.as_mut());
    suspension::suspended(survivor.as_mut());
    assert!(cancellation.request());
    offer(&queue, Marker::Second);
    assert_eq!(block_on(cancelled), Exit::Interrupted);
    assert_eq!(block_on(survivor), Exit::Success(payload(Marker::Second)));
}

#[test]
fn cancelling_a_middle_offer_preserves_fifo_refill_and_other_waiters() {
    let queue = AsyncQueue::new(bounded(1));
    offer(&queue, Marker::First);
    let cancellation = Arc::new(Cancellation::new());
    let mut second = Box::pin(queue.offer(payload(Marker::Second)).run(Arc::new(())));
    let mut middle = Box::pin(
        queue
            .offer(payload(Marker::Third))
            .run_cancellable(Arc::clone(&cancellation)),
    );
    let mut fourth = Box::pin(queue.offer(payload(Marker::Fourth)).run(Arc::new(())));
    suspension::suspended(second.as_mut());
    suspension::suspended(middle.as_mut());
    suspension::suspended(fourth.as_mut());
    assert!(cancellation.request());
    assert_eq!(block_on(middle), Exit::Interrupted);
    take(&queue, Marker::First);
    assert_eq!(block_on(second), Exit::Success(()));
    take(&queue, Marker::Second);
    assert_eq!(block_on(fourth), Exit::Success(()));
    take(&queue, Marker::Fourth);
}

#[test]
fn dropping_pending_offers_and_takers_removes_only_those_waiting_nodes() {
    let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
    let mut abandoned = Box::pin(queue.offer(payload(Marker::First)).run(Arc::new(())));
    suspension::suspended(abandoned.as_mut());
    drop(abandoned);
    let mut waiting = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(waiting.as_mut());
    drop(waiting);
    let mut survivor = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(survivor.as_mut());
    offer(&queue, Marker::Second);
    assert_eq!(block_on(survivor), Exit::Success(payload(Marker::Second)));
}

#[test]
fn stale_committed_offer_guard_cannot_remove_a_new_waiter_reusing_its_slot() {
    let queue = AsyncQueue::new(bounded(1));
    offer(&queue, Marker::First);
    let mut committed = Box::pin(queue.offer(payload(Marker::Second)).run(Arc::new(())));
    suspension::suspended(committed.as_mut());
    take(&queue, Marker::First); // Second offer committed, but its driver is unpolled.
    let mut reused = Box::pin(queue.offer(payload(Marker::Third)).run(Arc::new(())));
    suspension::suspended(reused.as_mut());
    drop(committed); // Removing the old registration must not remove Third.
    take(&queue, Marker::Second);
    assert_eq!(block_on(reused), Exit::Success(()));
    take(&queue, Marker::Third);
}

#[test]
fn stale_committed_taker_guard_cannot_remove_a_new_waiter_reusing_its_slot() {
    let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
    let mut committed = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(committed.as_mut());
    offer(&queue, Marker::First);
    let mut reused = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(reused.as_mut());
    drop(committed); // First transfer is discarded, never requeued or undone.
    offer(&queue, Marker::Second);
    assert_eq!(block_on(reused), Exit::Success(payload(Marker::Second)));
}

#[test]
fn cancellation_after_commit_does_not_undo_buffer_or_rendezvous_transfers() {
    let queue = AsyncQueue::new(bounded(1));
    offer(&queue, Marker::First);
    let cancellation = Arc::new(Cancellation::new());
    let mut incoming = Box::pin(
        queue
            .offer(payload(Marker::Second))
            .run_cancellable(Arc::clone(&cancellation)),
    );
    suspension::suspended(incoming.as_mut());
    take(&queue, Marker::First);
    assert!(cancellation.request());
    assert_eq!(block_on(incoming), Exit::Interrupted); // Publication masks success.
    take(&queue, Marker::Second); // Its committed buffer value persists.

    let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
    let cancellation = Arc::new(Cancellation::new());
    let mut taker = Box::pin(queue.take().run_cancellable(Arc::clone(&cancellation)));
    suspension::suspended(taker.as_mut());
    offer(&queue, Marker::First);
    assert!(cancellation.request());
    assert_eq!(block_on(taker), Exit::Interrupted); // Committed value is discarded by publication.
    let mut next = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(next.as_mut());
    offer(&queue, Marker::Second);
    assert_eq!(block_on(next), Exit::Success(payload(Marker::Second)));
}

#[test]
fn shutdown_drain_retains_only_committed_buffer_and_rejects_pending_and_future_offers()
-> Result<(), String> {
    let queue = AsyncQueue::new(bounded(2)); // The exact two-buffer-value source drain case.
    offer(&queue, Marker::First);
    offer(&queue, Marker::Second);
    let mut pending = Box::pin(queue.offer(payload(Marker::Third)).run(Arc::new(())));
    suspension::suspended(pending.as_mut());
    assert_eq!(
        block_on(queue.shutdown(QueueShutdown::Drain).run(Arc::new(()))),
        Exit::Success(())
    );
    assert_eq!(block_on(pending), Exit::Fail(QueueError::Shutdown));
    assert_eq!(size(&queue)?, 2);
    assert_eq!(
        block_on(queue.offer(payload(Marker::Fourth)).run(Arc::new(()))),
        Exit::Fail(QueueError::Shutdown)
    );
    take(&queue, Marker::First);
    take(&queue, Marker::Second);
    assert_eq!(
        block_on(queue.take().run(Arc::new(()))),
        Exit::Fail(QueueError::Shutdown)
    );
    Ok(())
}

#[test]
fn shutdown_discards_buffer_wakes_all_takers_and_can_discard_after_an_earlier_drain()
-> Result<(), String> {
    let queue = AsyncQueue::new(bounded(2));
    offer(&queue, Marker::First);
    offer(&queue, Marker::Second);
    assert_eq!(
        block_on(queue.shutdown(QueueShutdown::Drain).run(Arc::new(()))),
        Exit::Success(())
    );
    assert_eq!(size(&queue)?, 2);
    assert_eq!(
        block_on(queue.shutdown(QueueShutdown::Discard).run(Arc::new(()))),
        Exit::Success(())
    );
    assert_eq!(size(&queue)?, 0);
    assert_eq!(
        block_on(queue.take().run(Arc::new(()))),
        Exit::Fail(QueueError::Shutdown)
    );
    assert_eq!(
        block_on(queue.shutdown(QueueShutdown::Drain).run(Arc::new(()))),
        Exit::Success(())
    );

    let queue: AsyncQueue<Payload> = AsyncQueue::new(QueueCapacity::Rendezvous);
    const WAITERS: usize = 32; // Explicit shutdown notification coverage, never a queue default.
    let mut waiters = Vec::new();
    for _ in 0..WAITERS {
        let mut waiter = Box::pin(queue.take().run(Arc::new(())));
        suspension::suspended(waiter.as_mut());
        waiters.push(waiter);
    }
    assert_eq!(
        block_on(queue.shutdown(QueueShutdown::Drain).run(Arc::new(()))),
        Exit::Success(())
    );
    for waiter in waiters {
        assert_eq!(block_on(waiter), Exit::Fail(QueueError::Shutdown));
    }
    Ok(())
}

#[test]
fn queue_construction_and_offer_factories_are_lazy_reusable_and_precancel_safe()
-> Result<(), String> {
    let make: Effect<AsyncQueue<Payload>, QueueError> = AsyncQueue::make(QueueCapacity::Unbounded);
    let first = success(block_on(make.clone().run(Arc::new(()))))?;
    let second = success(block_on(make.run(Arc::new(()))))?;
    let calls = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&calls);
    let offering = first.offer_with(move || {
        let _ = c.fetch_add(1, Ordering::SeqCst);
        payload(Marker::First)
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let cancelled = Arc::new(Cancellation::new());
    assert!(cancelled.request());
    assert_eq!(
        block_on(offering.clone().run_cancellable(cancelled)),
        Exit::Interrupted
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        block_on(offering.clone().run(Arc::new(Cancellation::new()))),
        Exit::Success(())
    );
    assert_eq!(
        block_on(offering.run(Arc::new(Cancellation::new()))),
        Exit::Success(())
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(size(&first)?, 2);
    assert_eq!(size(&second)?, 0);
    take(&first, Marker::First);
    take(&first, Marker::First);

    let candidate = first.offer(payload(Marker::Second));
    let cancelled = Arc::new(Cancellation::new());
    assert!(cancelled.request());
    assert_eq!(
        block_on(candidate.clone().run_cancellable(cancelled)),
        Exit::Interrupted
    );
    assert_eq!(
        block_on(candidate.clone().run(Arc::new(Cancellation::new()))),
        Exit::Success(())
    );
    assert!(
        matches!(block_on(candidate.run(Arc::new(Cancellation::new()))), Exit::Die(defect)
        if defect.kind() == DefectKind::Library(LibraryDefect::QueueOfferConsumed))
    );
    take(&first, Marker::Second);
    Ok(())
}

#[test]
fn panicking_offer_factory_leaves_queue_usable() -> Result<(), String> {
    let queue: AsyncQueue<Payload> = AsyncQueue::new(QueueCapacity::Unbounded);
    assert!(
        matches!(block_on(queue.offer_with(|| std::panic::resume_unwind(Box::new("factory fixture"))).run(Arc::new(()))), Exit::Die(defect)
        if defect.kind() == DefectKind::CallbackPanic)
    );
    assert_eq!(size(&queue)?, 0);
    offer(&queue, Marker::First);
    take(&queue, Marker::First);
    Ok(())
}

struct InspectWake {
    queue: AsyncQueue<Payload>,
    inspected: AtomicBool,
}
impl Wake for InspectWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        let mut size = Box::pin(self.queue.size().run(Arc::new(())));
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let std::task::Poll::Ready(outcome) = size.as_mut().poll(&mut context) {
                assert_eq!(outcome, Exit::Success(0));
                self.inspected.store(true, Ordering::SeqCst);
                return;
            }
        }
    }
}

#[test]
fn transfer_notifications_can_reenter_queue_inspection_without_holding_admission_gate()
-> Result<(), String> {
    let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
    let mut taker = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(taker.as_mut());
    let wake = Arc::new(InspectWake {
        queue: queue.clone(),
        inspected: AtomicBool::new(false),
    });
    let waker = Waker::from(Arc::clone(&wake));
    assert!(
        taker
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    assert_eq!(
        run(queue.offer(payload(Marker::First)), Arc::new(()))?,
        Exit::Success(())
    );
    assert!(wake.inspected.load(Ordering::SeqCst));
    assert_eq!(block_on(taker), Exit::Success(payload(Marker::First)));
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
        let mut release = gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !*release {
            release = changed
                .wait(release)
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
fn shutdown_notifies_waiters_and_releases_the_gate_before_discarded_payload_destruction()
-> Result<(), String> {
    struct Value {
        _gate: Option<DropGate>,
    }
    let entered = Cancellation::new();
    let released = Arc::new((Mutex::new(false), Condvar::new()));
    let release = ReleaseGate(Arc::clone(&released));
    let queue = AsyncQueue::new(bounded(1));
    assert_eq!(
        block_on(
            queue
                .offer(Value {
                    _gate: Some(DropGate {
                        entered: entered.clone(),
                        release: released
                    })
                })
                .run(Arc::new(()))
        ),
        Exit::Success(())
    );
    let mut pending = Box::pin(queue.offer(Value { _gate: None }).run(Arc::new(())));
    suspension::suspended(pending.as_mut());
    let shutdown = queue.shutdown(QueueShutdown::Discard);
    let (send, receive) = mpsc::channel();
    let driver = std::thread::spawn(move || {
        assert!(send.send(block_on(shutdown.run(Arc::new(())))).is_ok());
    });
    let entered_wait = effect_future! { body: move |_| {
        let entered = entered.clone();
        async move { entered.cancelled().await }
    }, };
    assert_eq!(run(entered_wait, Arc::new(()))?, Exit::Success(()));
    assert_eq!(run(queue.size(), Arc::new(()))?, Exit::Success(0));
    assert_eq!(block_on(pending), Exit::Fail(QueueError::Shutdown));
    assert!(receive.try_recv().is_err());
    drop(release);
    assert_eq!(
        receive.recv_timeout(DEADLOCK_DEADLINE),
        Ok(Exit::Success(()))
    );
    assert!(driver.join().is_ok());
    Ok(())
}

struct PanicDrop;
impl Drop for PanicDrop {
    fn drop(&mut self) {
        std::panic::resume_unwind(Box::new("discard fixture"));
    }
}

#[test]
fn shutdown_preserves_every_discard_defect_after_committing_closed_state_and_waiter_failures()
-> Result<(), String> {
    let queue = AsyncQueue::new(bounded(1));
    assert_eq!(
        block_on(queue.offer(PanicDrop).run(Arc::new(()))),
        Exit::Success(())
    );
    let mut pending = Box::pin(queue.offer(PanicDrop).run(Arc::new(())));
    suspension::suspended(pending.as_mut());
    let outcome = block_on(queue.shutdown(QueueShutdown::Discard).run(Arc::new(())));
    assert!(matches!(outcome, Exit::FinalizationFailed(_)));
    if let Exit::FinalizationFailed(details) = outcome {
        assert_eq!(details.primary, PrimaryOutcome::Succeeded);
        assert!(
            matches!(details.first, CleanupFailure::ReleaseDefect(defect) if defect.kind() == DefectKind::CallbackPanic)
        );
        assert_eq!(details.rest.len(), 1); // Buffered and pending payload each failed destruction.
        assert!(details.rest.into_iter().all(|failure| matches!(failure, CleanupFailure::ReleaseDefect(defect) if defect.kind() == DefectKind::CallbackPanic)));
    }
    assert_eq!(block_on(pending), Exit::Fail(QueueError::Shutdown));
    assert_eq!(size(&queue)?, 0);
    assert!(matches!(
        block_on(queue.take().run(Arc::new(()))),
        Exit::Fail(QueueError::Shutdown)
    ));
    Ok(())
}

#[test]
fn explicit_cloned_offers_are_reusable_and_capacity_conversion_is_const_evaluated() {
    const UNBOUNDED: QueueCapacity = QueueCapacity::from_limit(None);
    const RENDEZVOUS: QueueCapacity = QueueCapacity::from_limit(Some(0)); // Language-defined absence of capacity.
    const BOUNDED: QueueCapacity = QueueCapacity::from_limit(Some(1)); // One explicit fixture buffer slot.
    assert_eq!(UNBOUNDED, QueueCapacity::Unbounded);
    assert_eq!(RENDEZVOUS, QueueCapacity::Rendezvous);
    assert_eq!(BOUNDED, bounded(1));
    let queue = AsyncQueue::new(UNBOUNDED);
    let offering = queue.offer_cloned(String::from("payload"));
    assert_eq!(
        block_on(offering.clone().run(Arc::new(()))),
        Exit::Success(())
    );
    assert_eq!(block_on(offering.run(Arc::new(()))), Exit::Success(()));
    assert_eq!(
        block_on(queue.take().run(Arc::new(()))),
        Exit::Success(String::from("payload"))
    );
    assert_eq!(
        block_on(queue.take().run(Arc::new(()))),
        Exit::Success(String::from("payload"))
    );
}

struct FlagWake(AtomicBool);
impl Wake for FlagWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn shutdown_actually_wakes_every_registered_offer_and_take_driver() {
    const WAITERS: usize = 32; // Explicit targeted notification workload for both waiting lists.
    let offers = AsyncQueue::new(QueueCapacity::Rendezvous);
    let mut offering = Vec::new();
    for _ in 0..WAITERS {
        let mut future = Box::pin(offers.offer(payload(Marker::First)).run(Arc::new(())));
        suspension::suspended(future.as_mut());
        let wake = Arc::new(FlagWake(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&wake));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert!(!wake.0.load(Ordering::SeqCst));
        offering.push((future, wake));
    }
    assert_eq!(
        block_on(offers.shutdown(QueueShutdown::Drain).run(Arc::new(()))),
        Exit::Success(())
    );
    for (future, wake) in offering {
        assert!(wake.0.load(Ordering::SeqCst));
        assert_eq!(block_on(future), Exit::Fail(QueueError::Shutdown));
    }
    let takes: AsyncQueue<Payload> = AsyncQueue::new(QueueCapacity::Rendezvous);
    let mut taking = Vec::new();
    for _ in 0..WAITERS {
        let mut future = Box::pin(takes.take().run(Arc::new(())));
        suspension::suspended(future.as_mut());
        let wake = Arc::new(FlagWake(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&wake));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        assert!(!wake.0.load(Ordering::SeqCst));
        taking.push((future, wake));
    }
    assert_eq!(
        block_on(takes.shutdown(QueueShutdown::Discard).run(Arc::new(()))),
        Exit::Success(())
    );
    for (future, wake) in taking {
        assert!(wake.0.load(Ordering::SeqCst));
        assert_eq!(block_on(future), Exit::Fail(QueueError::Shutdown));
    }
}

#[test]
fn earlier_pending_cancellation_remains_interrupted_through_shutdown_and_raw_release_reason() {
    enum Waiting {
        Offer,
        Take,
    }
    for waiting in [Waiting::Offer, Waiting::Take] {
        let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
        let cancellation = Arc::new(Cancellation::new());
        let interrupted = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&interrupted);
        let pending: Effect<(), QueueError, Cancellation> = match waiting {
            Waiting::Offer => queue.offer(payload(Marker::First)),
            Waiting::Take => effect_map! { program: queue.take(), map: drop, },
        };
        let pending = effect_resource! { ensuring_release; program: pending, release: move |reason| {
            let was_interrupted = matches!(reason, PrimaryOutcomeRef::WasInterrupted);
            let observed = Arc::clone(&observed);
            effect_resource! { release_sync; body: move || observed.store(was_interrupted, Ordering::SeqCst), }
        }, };
        let mut pending = Box::pin(pending.run_cancellable(Arc::clone(&cancellation)));
        suspension::suspended(pending.as_mut());
        assert!(cancellation.request());
        // Leave this driver unpolled until after shutdown to exercise logical
        // cancellation removal rather than relying on host scheduling order.
        assert_eq!(
            block_on(queue.shutdown(QueueShutdown::Drain).run(Arc::new(()))),
            Exit::Success(())
        );
        assert_eq!(block_on(pending), Exit::Interrupted);
        assert!(interrupted.load(Ordering::SeqCst));
    }
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

fn advance_to_completion_or_native_wait<F: Future + ?Sized>(
    mut future: std::pin::Pin<&mut F>,
) -> Option<F::Output> {
    let wakes = Arc::new(CountWake(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wakes));
    let mut context = Context::from_waker(&waker);
    let deadline = Instant::now() + DEADLOCK_DEADLINE;
    loop {
        let before = wakes.0.load(Ordering::SeqCst);
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(outcome) => return Some(outcome),
            std::task::Poll::Pending if wakes.0.load(Ordering::SeqCst) == before => return None,
            std::task::Poll::Pending => {
                assert!(Instant::now() < deadline, "native queue wait not reached")
            }
        }
    }
}

#[test]
fn matching_racing_cancellation_either_commits_once_or_leaves_the_offer_for_a_new_taker() {
    const ATTEMPTS: usize = 32; // Finite coverage of both cancellation/commit gate orders.
    const PARTICIPANTS: usize = 2; // Actual cancellation thread and matching worker.
    for _ in 0..ATTEMPTS {
        let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
        let cancellation = Arc::new(Cancellation::new());
        let mut original = Box::pin(queue.take().run_cancellable(Arc::clone(&cancellation)));
        suspension::suspended(original.as_mut());
        let barrier = Arc::new(Barrier::new(PARTICIPANTS));
        let gate = Arc::clone(&barrier);
        let offering = queue.offer(payload(Marker::First));
        let worker = std::thread::spawn(move || {
            let mut future = Box::pin(offering.run(Arc::new(())));
            let _ = gate.wait();
            let outcome = advance_to_completion_or_native_wait(future.as_mut());
            (future, outcome)
        });
        let _ = barrier.wait();
        assert!(cancellation.request());
        let returned = worker.join();
        assert!(returned.is_ok());
        if let Ok((future, outcome)) = returned {
            match outcome {
                Some(outcome) => {
                    assert_eq!(outcome, Exit::Success(()));
                    drop(future);
                    // A committed transfer is consumed, even though cancellation
                    // masks the original taker's publication. Never redeliver it.
                    let mut next = Box::pin(queue.take().run(Arc::new(())));
                    suspension::suspended(next.as_mut());
                    offer(&queue, Marker::Second);
                    assert_eq!(block_on(next), Exit::Success(payload(Marker::Second)));
                }
                None => {
                    // Cancellation removed the original waiter first. The first
                    // value remains a pending offer and commits to this new take.
                    take(&queue, Marker::First);
                    assert_eq!(block_on(future), Exit::Success(()));
                }
            }
        }
        assert_eq!(block_on(original), Exit::Interrupted);
    }
}

#[test]
fn indexed_waiting_links_preserve_order_through_middle_removal_and_slot_reuse() {
    const ENTRIES: usize = 10_000; // A finite high-water/reuse workload, not a queue capacity default.
    let mut list = waiting::Waiting::default();
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
    let values: Vec<_> = (0..ENTRIES).map(Arc::new).collect();
    let positions: Vec<_> = values
        .iter()
        .map(|value| list.push_back(Arc::clone(value)))
        .collect();
    assert_eq!(list.len(), values.len());
    let mut expected = VecDeque::from(values.clone()); // Only fixture Arc handles are copied.
    // Remove alternate interior entries, then append the same number of fresh
    // entries. This exercises both neighbor directions and every reused hole.
    for (index, (position, value)) in positions.iter().zip(&values).enumerate() {
        if index % 2 != 0 {
            assert!(
                list.remove_where(*position, |stored| Arc::ptr_eq(stored, value))
                    .is_some()
            );
            expected.retain(|stored| !Arc::ptr_eq(stored, value));
        }
    }
    let replacements: Vec<_> = (ENTRIES..ENTRIES + ENTRIES / 2).map(Arc::new).collect();
    for value in &replacements {
        let _ = list.push_back(Arc::clone(value));
        expected.push_back(Arc::clone(value));
    }
    for (position, old) in positions.iter().zip(&values) {
        // Matching by actual retained allocation distinguishes live originals
        // from later entries in reused slots, without a guessed generation ID.
        let removed = list.remove_where(*position, |stored| Arc::ptr_eq(stored, old));
        if removed.is_some() {
            expected.retain(|stored| !Arc::ptr_eq(stored, old));
        }
    }
    while let Some(value) = expected.pop_front() {
        assert_eq!(list.pop_front(), Some(value));
        assert_eq!(list.len(), expected.len());
    }
    assert!(list.pop_front().is_none());
    assert!(list.is_empty());
}

struct SuspendedTasks {
    entered: Cancellation,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl TaskSpawner for SuspendedTasks {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        let entered = self.entered.clone();
        let worker = std::thread::Builder::new()
            .spawn(move || {
                let mut task = Box::pin(task);
                suspension::suspended(task.as_mut()); // Actual queue wait, not just callback entry.
                let _ = entered.request();
                block_on(task);
            })
            .map_err(|error| Defect::new(error.to_string()))?;
        *self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(worker);
        Ok(())
    }
}
impl effect::HasTaskSpawner for SuspendedTasks {
    type Tasks = Self;
    fn task_spawner(&self) -> &Self {
        self
    }
}

#[test]
fn scope_closure_cancels_and_joins_a_child_actually_blocked_on_queue_offer() -> Result<(), String> {
    let env = Arc::new(SuspendedTasks {
        entered: Cancellation::new(),
        worker: Mutex::new(None),
    });
    let entered = env.entered.clone();
    let queue = AsyncQueue::new(QueueCapacity::Rendezvous);
    let offered = queue.clone();
    let program = effect_scope! {
    body: move |scope| {
            let entered = entered.clone();
            effect! {
     bind;
     program: scope
                .fork(offered.offer(payload(Marker::First))),
     body: move |_fiber| {
                    let entered = entered.clone();
                    effect_future! { body: move |_| {
                        let entered = entered.clone();
                        async move { entered.cancelled().await }
                    }, }
                },
     }
        },
    };
    assert_eq!(run(program, Arc::clone(&env))?, Exit::Success(()));
    let worker = env
        .worker
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    assert!(matches!(
        worker.map(std::thread::JoinHandle::join),
        Some(Ok(()))
    ));
    let mut next = Box::pin(queue.take().run(Arc::new(())));
    suspension::suspended(next.as_mut());
    offer(&queue, Marker::Second);
    assert_eq!(block_on(next), Exit::Success(payload(Marker::Second)));
    Ok(())
}
