//! Fixed timed registrations and clock-driving records.

use super::ClockError;
use crate::internal::runtime::prepared_graph::FutureContext;
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotId, SlotPool,
};
use crate::{Cancellation, Defect};
use futures_util::future::{Either, select};
use std::alloc::Layout;
use std::future::Future;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

/// Startup limits for a selected prepared virtual clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedClockCapacity {
    /// Registered or elapsed timers whose wait owner has yet to consume them.
    pub timers: NonZeroUsize,
    /// Advancements, including withdrawn advancements whose tick is still held.
    pub advances: NonZeroUsize,
}

impl PreparedClockCapacity {
    /// Layout demand for timer and advance slots and their fixed indexes.
    /// Allocator headers and alignment padding belong to the enclosing plan.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let timers = Layout::array::<Scheduled>(self.timers.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let advances = Layout::array::<AdvanceRef>(self.advances.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Timer>::storage_bytes(self.timers.get())?
            .checked_add(SlotPool::<Advance>::storage_bytes(self.advances.get())?)?
            .checked_add(Bytes::new(timers.size()))?
            .checked_add(Bytes::new(advances.size()))
    }
}

/// Clock failure or refusal from the selected physical timer owners.
#[derive(Debug)]
pub enum PreparedClockError {
    /// The selected physical storage could not admit the operation.
    Memory(MemoryError),
    /// Time, interruption or a wake failure.
    Clock(ClockError),
}

impl From<MemoryError> for PreparedClockError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<ClockError> for PreparedClockError {
    fn from(error: ClockError) -> Self {
        Self::Clock(error)
    }
}

type TimerRef = SharedSlot<'static, Timer>;
type AdvanceRef = SharedSlot<'static, Advance>;

struct Scheduled {
    deadline: Duration,
    timer: TimerRef,
}

enum TimerPhase {
    Waiting,
    Elapsed(PreparedClockTick),
    Closed,
}

struct TimerState {
    phase: TimerPhase,
    wake: Option<Waker>,
}

struct Timer(Mutex<TimerState>);

impl Timer {
    fn new() -> Self {
        Self(Mutex::new(TimerState {
            phase: TimerPhase::Waiting,
            wake: None,
        }))
    }

    fn poll_tick(
        &self,
        wake: &mut Option<Waker>,
    ) -> (
        Poll<Result<PreparedClockTick, PreparedClockError>>,
        Option<Waker>,
    ) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match std::mem::replace(&mut state.phase, TimerPhase::Closed) {
            TimerPhase::Waiting => {
                state.phase = TimerPhase::Waiting;
                (
                    Poll::Pending,
                    std::mem::replace(&mut state.wake, wake.take()),
                )
            }
            TimerPhase::Elapsed(tick) => (Poll::Ready(Ok(tick)), state.wake.take()),
            TimerPhase::Closed => (
                Poll::Ready(Err(ClockError::Interrupted.into())),
                state.wake.take(),
            ),
        }
    }

    fn fire(&self, tick: PreparedClockTick) -> (Option<Waker>, Option<PreparedClockTick>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(state.phase, TimerPhase::Waiting) {
            state.phase = TimerPhase::Elapsed(tick);
            (state.wake.take(), None)
        } else {
            (None, Some(tick))
        }
    }

    fn close(&self) -> (TimerPhase, Option<Waker>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        (
            std::mem::replace(&mut state.phase, TimerPhase::Closed),
            state.wake.take(),
        )
    }
}

#[derive(Default)]
struct AdvanceState {
    settled: bool,
    wake: Option<Waker>,
}

#[derive(Default)]
struct Advance(Mutex<AdvanceState>);

impl Advance {
    fn register(&self, cx: &Context<'_>) -> bool {
        let replacement = cx.waker().clone();
        let (settled, retired) = {
            let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            (state.settled, state.wake.replace(replacement))
        };
        drop(retired);
        settled
    }

    fn arm(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .settled = false;
    }

    fn take_wake(&self) -> Option<Waker> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .wake
            .take()
    }

    fn acknowledge(&self) -> Option<Waker> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.settled = true;
        state.wake.take()
    }
}

fn invoke_wake(wake: Option<Waker>) -> Result<(), ClockError> {
    if let Some(wake) = wake {
        catch_unwind(AssertUnwindSafe(|| wake.wake()))
            .map_err(|panic| ClockError::Defect(Defect::from_panic(panic)))?;
    }
    Ok(())
}

struct State {
    now: Duration,
    timers: FixedVector<'static, Scheduled>,
    advances: FixedVector<'static, AdvanceRef>,
}

struct Core {
    state: Mutex<State>,
    timers: SlotPool<'static, Timer>,
    advances: SlotPool<'static, Advance>,
}

/// A selected reusable virtual clock with bounded native waits and advancement.
#[derive(Clone)]
pub struct PreparedTestClock(Arc<Core>);

impl PreparedTestClock {
    /// Provision indexes and timer/advance records before sealing the region.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        initial: Duration,
        capacity: PreparedClockCapacity,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        Ok(Self(Arc::new(Core {
            state: Mutex::new(State {
                now: initial,
                timers: FixedVector::prepare(memory.allocator(region)?, capacity.timers.get())?,
                advances: FixedVector::prepare(memory.allocator(region)?, capacity.advances.get())?,
            }),
            timers: memory.slots(region, capacity.timers.get(), |_| Ok(Timer::new()))?,
            advances: memory.slots(region, capacity.advances.get(), |_| Ok(Advance::default()))?,
        })))
    }

    /// Observe the current virtual time.
    #[must_use]
    pub fn now(&self) -> Duration {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .now
    }

    /// Count registrations still waiting to become due.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .timers
            .len()
    }

    /// Register a native timer immediately. Dropping its future unlinks it.
    /// Equal deadlines settle in registration order.
    pub fn sleep(&self, duration: Duration) -> Result<PreparedClockWait, PreparedClockError> {
        if duration.is_zero() {
            return Ok(PreparedClockWait {
                state: Some(Wait::Immediate),
            });
        }
        let mut timer = self.0.timers.try_checkout()?;
        *timer = Timer::new();
        let timer = timer.share();
        let queue = timer.try_clone()?;
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let deadline = state
            .now
            .checked_add(duration)
            .ok_or(ClockError::Overflow)?;
        let index = state
            .timers
            .as_slice()
            .partition_point(|timer| timer.deadline <= deadline);
        state
            .timers
            .push(Scheduled {
                deadline,
                timer: queue,
            })
            .map_err(|_| MemoryError::NoSlot)?;
        state
            .timers
            .as_mut_slice()
            .split_at_mut(index)
            .1
            .rotate_right(1);
        Ok(PreparedClockWait {
            state: Some(Wait::Timed {
                clock: self.clone(),
                timer,
            }),
        })
    }

    /// Queue a finite clock advancement. Its cutoff is relative to the time
    /// when it becomes active and is first polled.
    pub fn advance(&self, duration: Duration) -> Result<PreparedClockAdvance, PreparedClockError> {
        let mut entry = self.0.advances.try_checkout()?;
        *entry = Advance::default();
        let entry = entry.share();
        let queue = entry.try_clone()?;
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .advances
            .push(queue)
            .map_err(|_| MemoryError::NoSlot)?;
        Ok(PreparedClockAdvance {
            clock: self.clone(),
            entry: Some(entry),
            duration,
            target: None,
            awaiting: false,
        })
    }

    fn unlink(&self, id: SlotId) {
        let removed = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            state
                .timers
                .as_slice()
                .iter()
                .position(|scheduled| scheduled.timer.id() == id)
                .and_then(|index| state.timers.remove_at(index))
        };
        drop(removed);
    }

    fn remove_advance(&self, id: SlotId) -> Option<Waker> {
        let (removed, wake) = {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let position = state
                .advances
                .as_slice()
                .iter()
                .position(|entry| entry.id() == id)?;
            let removed = state.advances.remove_at(position);
            let wake = if position == 0 {
                state
                    .advances
                    .as_slice()
                    .first()
                    .and_then(|entry| entry.take_wake())
            } else {
                None
            };
            (removed, wake)
        };
        drop(removed);
        wake
    }

    fn select_due(
        &self,
        active: &AdvanceRef,
        target: &mut Option<Duration>,
        duration: Duration,
    ) -> Result<Selection, PreparedClockError> {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state
            .advances
            .as_slice()
            .first()
            .is_none_or(|entry| entry.id() != active.id())
        {
            return Ok(Selection::Queued);
        }
        let cutoff = match *target {
            Some(target) => target,
            None => {
                let cutoff = state
                    .now
                    .checked_add(duration)
                    .ok_or(ClockError::Overflow)?;
                *target = Some(cutoff);
                cutoff
            }
        };
        if state
            .timers
            .as_slice()
            .first()
            .is_some_and(|scheduled| scheduled.deadline <= cutoff)
        {
            let settlement = active.try_clone()?;
            let scheduled = state
                .timers
                .remove_first()
                .ok_or(MemoryError::InvalidRange)?;
            active.arm();
            state.now = scheduled.deadline;
            let (wake, abandoned) = scheduled.timer.fire(PreparedClockTick(Some(settlement)));
            // Waiters observe the published phase under this same clock gate.
            // Retire our timed-record alias before they can consume the tick.
            drop(scheduled);
            Ok(Selection::Due { wake, abandoned })
        } else {
            state.now = cutoff;
            Ok(Selection::Complete)
        }
    }
}

enum Selection {
    Queued,
    Complete,
    Due {
        wake: Option<Waker>,
        abandoned: Option<PreparedClockTick>,
    },
}

/// A settled wait's acknowledgement. Keep it through the continuation to its
/// next native suspension or terminal outcome, then drop it.
#[must_use = "retain the tick until the clock-driven continuation settles"]
pub struct PreparedClockTick(Option<AdvanceRef>);

impl Drop for PreparedClockTick {
    fn drop(&mut self) {
        if let Some(settlement) = self.0.take() {
            let wake = settlement.acknowledge();
            drop(settlement);
            let _ = invoke_wake(wake);
        }
    }
}

enum Wait {
    Immediate,
    Timed {
        clock: PreparedTestClock,
        timer: TimerRef,
    },
}

/// A native wait with one actual timed registration until it settles or drops.
#[must_use = "drive or retire the prepared timer registration"]
pub struct PreparedClockWait {
    state: Option<Wait>,
}

impl PreparedClockWait {
    /// Use the enclosing prepared graph's already-reserved cancellation listener.
    pub async fn with_context<E: Send + 'static>(
        self,
        context: &mut FutureContext<E>,
    ) -> Result<PreparedClockTick, PreparedClockError> {
        if context.is_requested() {
            return Err(ClockError::Interrupted.into());
        }
        if matches!(self.state, Some(Wait::Immediate)) {
            return self.await;
        }
        let result = {
            let cancelled = pin!(context.cancelled());
            match select(cancelled, self).await {
                Either::Left((_, wait)) => {
                    drop(wait);
                    return Err(ClockError::Interrupted.into());
                }
                Either::Right((result, _)) => result,
            }
        };
        if context.is_requested() {
            Err(ClockError::Interrupted.into())
        } else {
            result
        }
    }

    /// Use the supplied cancellation token, admitting its listener before polling.
    pub async fn with_cancellation(
        self,
        cancellation: Cancellation,
    ) -> Result<PreparedClockTick, PreparedClockError> {
        if cancellation.is_requested() {
            return Err(ClockError::Interrupted.into());
        }
        if matches!(self.state, Some(Wait::Immediate)) {
            return self.await;
        }
        let notified = cancellation.reserve_wait_owned()?;
        let notified = pin!(notified.wait());
        match select(notified, self).await {
            Either::Left(((), wait)) => {
                drop(wait);
                Err(ClockError::Interrupted.into())
            }
            Either::Right((result, _)) if !cancellation.is_requested() => result,
            Either::Right(_) => Err(ClockError::Interrupted.into()),
        }
    }
}

impl Future for PreparedClockWait {
    type Output = Result<PreparedClockTick, PreparedClockError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let poll = match this.state.as_ref() {
            Some(Wait::Immediate) => Poll::Ready(Ok(PreparedClockTick(None))),
            Some(Wait::Timed { clock, timer }) => {
                let mut wake = Some(cx.waker().clone());
                let (poll, retired) = {
                    let _gate = clock.0.state.lock().unwrap_or_else(PoisonError::into_inner);
                    timer.poll_tick(&mut wake)
                };
                drop(retired);
                poll
            }
            None => Poll::Ready(Err(MemoryError::UnsupportedRunContract.into())),
        };
        if poll.is_ready() {
            // The tick owns advance settlement and can outlive this timer slot.
            this.state = None;
        }
        poll
    }
}

impl Drop for PreparedClockWait {
    fn drop(&mut self) {
        if let Some(Wait::Timed { clock, timer }) = self.state.take() {
            clock.unlink(timer.id());
            let (phase, wake) = timer.close();
            drop(timer);
            drop(phase);
            drop(wake);
        }
    }
}

/// A queued native advancement which waits for each elapsed continuation.
#[must_use = "poll the prepared advancement to move virtual time"]
pub struct PreparedClockAdvance {
    clock: PreparedTestClock,
    entry: Option<AdvanceRef>,
    duration: Duration,
    target: Option<Duration>,
    awaiting: bool,
}

impl PreparedClockAdvance {
    fn finish(&mut self) {
        if let Some(entry) = self.entry.take() {
            let next = self.clock.remove_advance(entry.id());
            let retired = entry.take_wake();
            drop(entry);
            let _ = invoke_wake(next);
            drop(retired);
        }
    }
}

impl Future for PreparedClockAdvance {
    type Output = Result<(), PreparedClockError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            let Some(entry) = this.entry.as_ref() else {
                return Poll::Ready(Ok(()));
            };
            if !entry.register(cx) && this.awaiting {
                return Poll::Pending;
            }
            this.awaiting = false;
            match this
                .clock
                .select_due(entry, &mut this.target, this.duration)
            {
                Ok(Selection::Queued) => return Poll::Pending,
                Ok(Selection::Complete) => {
                    this.finish();
                    return Poll::Ready(Ok(()));
                }
                Ok(Selection::Due { wake, abandoned }) => {
                    this.awaiting = true;
                    drop(abandoned);
                    if let Err(error) = invoke_wake(wake) {
                        this.finish();
                        return Poll::Ready(Err(error.into()));
                    }
                }
                Err(error) => {
                    this.finish();
                    return Poll::Ready(Err(error));
                }
            }
        }
    }
}

impl Drop for PreparedClockAdvance {
    fn drop(&mut self) {
        self.finish();
    }
}
