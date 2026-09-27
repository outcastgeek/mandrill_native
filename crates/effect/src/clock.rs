use crate::internal::runtime::{self, Step};
use crate::internal::waiting::{Position, Waiting};
use crate::{Cancellation, Defect, Effect, Exit, LibraryDefect};
use std::collections::BTreeMap;
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

#[derive(Debug)]
pub enum ClockError {
    Interrupted,
    Overflow,
    Defect(Defect),
}

pub type ClockFuture = Pin<Box<dyn Future<Output = Result<ClockTick, ClockError>> + Send>>;

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Duration;
    fn sleep(&self, duration: Duration, cancellation: Cancellation) -> ClockFuture;
}
pub trait HasClock {
    type Clock: Clock + ?Sized;
    fn clock(&self) -> &Self::Clock;
}

#[derive(Default)]
struct Acknowledgement {
    done: AtomicBool,
    waiter: Mutex<Option<Waker>>,
}
impl Acknowledgement {
    fn finish(&self) {
        self.done.store(true, Ordering::Release);
        let waiter = self
            .waiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(waiter) = waiter {
            let _ = catch_unwind(AssertUnwindSafe(|| waiter.wake()));
        }
    }
    fn poll(&self, cx: &mut Context<'_>) -> Poll<()> {
        let mut waiter = self
            .waiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.done.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        if !waiter
            .as_ref()
            .is_some_and(|prior| prior.will_wake(cx.waker()))
        {
            *waiter = Some(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// A timer's elapsed completion. Virtual clocks may retain an acknowledgement
/// until the clock-driven interpreter reaches its next native wait or outcome.
pub struct ClockTick {
    acknowledgement: Option<Arc<Acknowledgement>>,
}
impl ClockTick {
    #[must_use]
    pub const fn elapsed() -> Self {
        Self {
            acknowledgement: None,
        }
    }
}
impl Drop for ClockTick {
    fn drop(&mut self) {
        if let Some(acknowledgement) = self.acknowledgement.take() {
            acknowledgement.finish();
        }
    }
}

#[derive(Default)]
pub(crate) struct Frontier {
    present: AtomicBool,
    ticks: Mutex<Vec<ClockTick>>,
}
impl Frontier {
    pub(crate) const fn new() -> Self {
        Self {
            present: AtomicBool::new(false),
            ticks: Mutex::new(Vec::new()),
        }
    }
    pub(crate) fn retain(&self, tick: ClockTick) {
        if tick.acknowledgement.is_some() {
            self.ticks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(tick);
            self.present.store(true, Ordering::Release);
        }
    }
    pub(crate) fn acknowledge(&self) {
        if self.present.swap(false, Ordering::AcqRel) {
            let ticks = std::mem::take(
                &mut *self
                    .ticks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            drop(ticks);
        }
    }
}

impl<E: Send + 'static, R: HasClock + Send + Sync + 'static> Effect<Duration, E, R> {
    pub fn clock_now() -> Self {
        Self::ready(|env, _| Exit::Success(env.clock().now()))
    }
}
impl<E: Send + 'static, R: HasClock + Send + Sync + 'static> Effect<(), E, R> {
    pub fn sleep(duration: Duration) -> Self {
        Self::new(move |env, context, finish| {
            let future = match catch_unwind(AssertUnwindSafe(|| {
                env.clock().sleep(duration, context.cancellation.clone())
            })) {
                Ok(future) => future,
                Err(panic) => return runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            };
            Step::Suspended(Box::pin(async move {
                let result = futures_util::FutureExt::catch_unwind(AssertUnwindSafe(future)).await;
                let outcome = match result {
                    Ok(Ok(tick)) => {
                        context.clock_tick(tick);
                        Exit::Success(())
                    }
                    Ok(Err(ClockError::Interrupted)) => Exit::Interrupted,
                    Ok(Err(ClockError::Overflow)) => {
                        Exit::Die(Defect::from(LibraryDefect::ClockDeadlineOverflow))
                    }
                    Ok(Err(ClockError::Defect(defect))) => Exit::Die(defect),
                    Err(panic) => Exit::Die(Defect::from_panic(panic)),
                };
                drop(env);
                runtime::finish(finish, outcome)
            }))
        })
    }
}

struct SleepState {
    outcome: Option<Result<ClockTick, ClockError>>,
    wake: Option<Waker>,
}
struct Sleeper {
    state: Mutex<SleepState>,
    cancellation: Cancellation,
}
impl Sleeper {
    fn complete(&self, outcome: Result<ClockTick, ClockError>) -> Result<(), ClockError> {
        let wake = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.outcome = Some(outcome);
            state.wake.take()
        };
        match wake {
            Some(wake) => catch_unwind(AssertUnwindSafe(|| wake.wake()))
                .map_err(|panic| ClockError::Defect(Defect::from_panic(panic))),
            None => Ok(()),
        }
    }
}
#[derive(Default)]
struct AdvanceEntry {
    wake: Mutex<Option<Waker>>,
}
impl AdvanceEntry {
    fn wake(&self) {
        let wake = self
            .wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(wake) = wake {
            let _ = catch_unwind(AssertUnwindSafe(|| wake.wake()));
        }
    }
}
struct VirtualState {
    now: Duration,
    sleepers: BTreeMap<Duration, Waiting<Arc<Sleeper>>>,
    advances: Waiting<Arc<AdvanceEntry>>,
    active: Option<Arc<AdvanceEntry>>,
}
#[derive(Clone)]
pub struct TestClock {
    state: Arc<Mutex<VirtualState>>,
}
impl Default for TestClock {
    fn default() -> Self {
        Self::new(Duration::ZERO)
    }
}
impl TestClock {
    #[must_use]
    pub fn new(initial: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(VirtualState {
                now: initial,
                sleepers: BTreeMap::new(),
                advances: Waiting::default(),
                active: None,
            })),
        }
    }
    #[must_use]
    pub fn pending(&self) -> usize {
        // The retained entries are actual registrations, not generated sequence IDs.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sleepers
            .values()
            .map(Waiting::len)
            .sum()
    }
    pub fn advance(&self, duration: Duration) -> TestClockAdvance {
        let entry = Arc::new(AdvanceEntry::default());
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let position = state.advances.push_back(Arc::clone(&entry));
        if state.active.is_none() {
            state.active = state.advances.pop_front();
        }
        TestClockAdvance {
            clock: self.clone(),
            entry,
            position,
            duration,
            target: None,
            awaiting: None,
            finished: false,
        }
    }
    fn remove_sleep(&self, deadline: Duration, position: Position, sleeper: &Arc<Sleeper>) {
        let removed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let removed = state.sleepers.get_mut(&deadline).and_then(|waits| {
                waits.remove_where(position, |actual| Arc::ptr_eq(actual, sleeper))
            });
            if state.sleepers.get(&deadline).is_some_and(Waiting::is_empty) {
                let _ = state.sleepers.remove(&deadline);
            }
            removed
        };
        drop(removed);
    }
}
impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .now
    }
    fn sleep(&self, duration: Duration, cancellation: Cancellation) -> ClockFuture {
        let clock = self.clone();
        Box::pin(async move {
            if cancellation.is_requested() {
                return Err(ClockError::Interrupted);
            }
            if duration.is_zero() {
                return Ok(ClockTick::elapsed());
            }
            let sleeper = Arc::new(Sleeper {
                cancellation: cancellation.clone(),
                state: Mutex::new(SleepState {
                    outcome: None,
                    wake: None,
                }),
            });
            let (deadline, position) = {
                let mut state = clock
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let Some(deadline) = state.now.checked_add(duration) else {
                    return Err(ClockError::Overflow);
                };
                let position = state
                    .sleepers
                    .entry(deadline)
                    .or_default()
                    .push_back(Arc::clone(&sleeper));
                (deadline, position)
            };
            let guard = SleepRegistration {
                clock,
                sleeper,
                deadline,
                position,
            };
            let requested = cancellation.clone();
            let mut cancelled = Box::pin(async move {
                requested.cancelled().await;
            });
            poll_fn(|cx| {
                if cancellation.is_requested() {
                    guard.clock.remove_sleep(deadline, position, &guard.sleeper);
                    return Poll::Ready(Err(ClockError::Interrupted));
                }
                {
                    let mut state = guard
                        .sleeper
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(outcome) = state.outcome.take() {
                        return Poll::Ready(outcome);
                    }
                    if !state
                        .wake
                        .as_ref()
                        .is_some_and(|wake| wake.will_wake(cx.waker()))
                    {
                        state.wake = Some(cx.waker().clone());
                    }
                }
                if cancelled.as_mut().poll(cx).is_ready() {
                    cx.waker().wake_by_ref();
                }
                Poll::Pending
            })
            .await
        })
    }
}
impl HasClock for TestClock {
    type Clock = Self;
    fn clock(&self) -> &Self {
        self
    }
}
struct SleepRegistration {
    clock: TestClock,
    sleeper: Arc<Sleeper>,
    deadline: Duration,
    position: Position,
}
impl Drop for SleepRegistration {
    fn drop(&mut self) {
        self.clock
            .remove_sleep(self.deadline, self.position, &self.sleeper);
    }
}

#[must_use = "drive virtual advancement through clock-driven continuation settlement"]
pub struct TestClockAdvance {
    clock: TestClock,
    entry: Arc<AdvanceEntry>,
    position: Position,
    duration: Duration,
    target: Option<Duration>,
    awaiting: Option<Arc<Acknowledgement>>,
    finished: bool,
}
impl TestClockAdvance {
    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        let next = {
            let mut state = self
                .clock
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state
                .active
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &self.entry))
            {
                state.active = state.advances.pop_front();
                state.active.as_ref().map(Arc::clone)
            } else {
                let _ = state
                    .advances
                    .remove_where(self.position, |entry| Arc::ptr_eq(entry, &self.entry));
                None
            }
        };
        if let Some(next) = next {
            next.wake();
        }
    }
}
impl Future for TestClockAdvance {
    type Output = Result<(), ClockError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(Ok(()));
        }
        loop {
            if let Some(awaiting) = &this.awaiting {
                if awaiting.poll(cx).is_pending() {
                    return Poll::Pending;
                }
                this.awaiting = None;
            }
            *this
                .entry
                .wake
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cx.waker().clone());
            let ready = {
                let mut state = this
                    .clock
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !state
                    .active
                    .as_ref()
                    .is_some_and(|entry| Arc::ptr_eq(entry, &this.entry))
                {
                    return Poll::Pending;
                }
                let target = match this.target {
                    Some(target) => target,
                    None => match state.now.checked_add(this.duration) {
                        Some(target) => {
                            this.target = Some(target);
                            target
                        }
                        None => {
                            drop(state);
                            this.finish();
                            return Poll::Ready(Err(ClockError::Overflow));
                        }
                    },
                };
                match state.sleepers.first_entry() {
                    Some(mut earliest) if *earliest.key() <= target => {
                        let deadline = *earliest.key();
                        let sleeper = earliest.get_mut().pop_front();
                        if earliest.get().is_empty() {
                            let _ = earliest.remove_entry();
                        }
                        state.now = deadline;
                        sleeper
                    }
                    Some(_) | None => {
                        state.now = target;
                        None
                    }
                }
            };
            let Some(sleeper) = ready else {
                this.finish();
                return Poll::Ready(Ok(()));
            };
            if sleeper.cancellation.is_requested() {
                if let Err(error) = sleeper.complete(Err(ClockError::Interrupted)) {
                    this.finish();
                    return Poll::Ready(Err(error));
                }
            } else {
                let acknowledgement = Arc::new(Acknowledgement::default());
                this.awaiting = Some(Arc::clone(&acknowledgement));
                let tick = ClockTick {
                    acknowledgement: Some(acknowledgement),
                };
                if let Err(error) = sleeper.complete(Ok(tick)) {
                    this.finish();
                    return Poll::Ready(Err(error));
                }
            }
        }
    }
}
impl Drop for TestClockAdvance {
    fn drop(&mut self) {
        self.finish();
    }
}

impl<
    A: Send + 'static,
    E: Send + 'static,
    R: HasClock + crate::HasTaskSpawner + Send + Sync + 'static,
> Effect<A, E, R>
{
    /// The timeout owns cancellation and joining of its loser, including cleanup.
    /// It does not drop a native future or a running worker callback to detach it.
    pub fn timeout(
        self,
        duration: Duration,
        on_timeout: impl Fn() -> E + Send + Sync + 'static,
    ) -> Self {
        let on_timeout = Arc::new(on_timeout);
        self.race(Effect::<(), E, R>::sleep(duration).flat_map(move |()| {
            let on_timeout = Arc::clone(&on_timeout);
            Effect::fail_with(move || on_timeout())
        }))
    }
}
