//! One regional state cell and an admitted native future per retry/repeat series.

use super::{Repeat, Retry, Selection};
use crate::clock::{PreparedClockError, PreparedTestClock};
use crate::internal::outcome;
use crate::internal::runtime::prepared_graph::FutureContext;
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool};
use crate::{CleanupFailure, ClockError, Defect, Exit, LibraryDefect};
use std::future::{Future, poll_fn};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::pin;
use std::time::Duration;

/// One output and either the next delay and state, or the end of the series.
pub type PreparedScheduleTransition<State, Output> =
    Result<(Output, Option<(Duration, State)>), Defect>;

/// One reusable native attempt, sharing the series' preadmitted cancellation
/// authority. It settles its own cleanup before returning an outcome.
pub trait PreparedScheduleAttempt<A, E: Send + 'static>: Send {
    fn attempt(
        &mut self,
        context: &mut FutureContext<E>,
    ) -> impl Future<Output = Exit<A, E>> + Send;
}

impl<A, E, F, P> PreparedScheduleAttempt<A, E> for F
where
    E: Send + 'static,
    F: FnMut() -> P + Send,
    P: Future<Output = Exit<A, E>> + Send,
{
    fn attempt(&mut self, _: &mut FutureContext<E>) -> impl Future<Output = Exit<A, E>> + Send {
        self()
    }
}

/// A startup-selected state pool and reusable synchronous transition.
/// Each admitted series holds an exclusive cell throughout its attempts and
/// waits. The calling native frame retains previous outcomes during waits.
pub struct PreparedSchedule<State: Send + 'static, Transition> {
    slots: SlotPool<'static, Option<State>>,
    transition: Transition,
}

impl<State: Send + 'static, Transition> PreparedSchedule<State, Transition> {
    /// Regional state cells and slot bookkeeping, excluding allocator
    /// overhead and separate pointees retained by state or captures.
    pub fn storage_bytes(series: NonZeroUsize) -> Result<Bytes, MemoryError> {
        SlotPool::<Option<State>>::storage_bytes(series.get())
    }

    /// Provision all concurrent series before running the schedule.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        series: NonZeroUsize,
        transition: Transition,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            slots: memory.slots(region, series.get(), |_| Ok(None))?,
            transition,
        })
    }

    /// Retry only typed expected failures. Use inside a prepared native
    /// invocation with a reusable attempt and an explicitly prepared clock.
    /// During an attempt, the supplied operation observes cancellation via its
    /// context and completes its own cleanup. The transition may draw a sample
    /// from a captured random provider.
    pub async fn retry<A, E, O>(
        &self,
        clock: &PreparedTestClock,
        context: &mut FutureContext<E>,
        initial: impl FnOnce() -> State + Send,
        attempt: impl PreparedScheduleAttempt<A, E>,
        on_capacity: impl FnOnce(MemoryError) -> E + Send,
    ) -> Exit<A, E>
    where
        A: Send,
        E: Send + 'static,
        O: 'static,
        Transition: Fn(&State, &E, Duration) -> PreparedScheduleTransition<State, O> + Sync,
    {
        self.run::<A, E, O, Retry>(clock, context, initial, attempt, on_capacity)
            .await
    }

    /// Repeat only successful attempts under the same prepared storage policy
    /// as [Self::retry]. Terminal defects and cleanup outcomes pass through.
    pub async fn repeat<A, E, O>(
        &self,
        clock: &PreparedTestClock,
        context: &mut FutureContext<E>,
        initial: impl FnOnce() -> State + Send,
        attempt: impl PreparedScheduleAttempt<A, E>,
        on_capacity: impl FnOnce(MemoryError) -> E + Send,
    ) -> Exit<A, E>
    where
        A: Send,
        E: Send + 'static,
        O: 'static,
        Transition: Fn(&State, &A, Duration) -> PreparedScheduleTransition<State, O> + Sync,
    {
        self.run::<A, E, O, Repeat>(clock, context, initial, attempt, on_capacity)
            .await
    }

    async fn run<A, E, O, Mode>(
        &self,
        clock: &PreparedTestClock,
        context: &mut FutureContext<E>,
        initial: impl FnOnce() -> State + Send,
        mut attempt: impl PreparedScheduleAttempt<A, E>,
        on_capacity: impl FnOnce(MemoryError) -> E + Send,
    ) -> Exit<A, E>
    where
        A: Send,
        E: Send + 'static,
        O: 'static,
        Mode: Selection<A, E>,
        Transition:
            Fn(&State, &Mode::Input, Duration) -> PreparedScheduleTransition<State, O> + Sync,
    {
        if context.is_requested() {
            return Exit::Interrupted;
        }
        let slot = match self.slots.try_checkout() {
            Ok(slot) => slot,
            Err(error) => return Exit::Fail(on_capacity(error)),
        };
        let mut frontier = None;
        let mut state = SeriesState(slot);
        let started = clock.now();
        *state.0 = Some(initial());
        loop {
            if context.is_requested() {
                return Exit::Interrupted;
            }
            let previous = {
                let mut running = pin!(attempt.attempt(context));
                poll_fn(|task| {
                    let polled = running.as_mut().poll(task);
                    if polled.is_pending() {
                        drop(frontier.take());
                    }
                    polled
                })
                .await
            };
            if context.is_requested() {
                // The enclosing prepared invocation publishes the settled
                // attempt with its originating cancellation/protection policy.
                return previous;
            }
            let Some(input) = Mode::selected(&previous) else {
                return previous;
            };
            let decision = catch_unwind(AssertUnwindSafe(|| {
                let state = state
                    .0
                    .as_ref()
                    .ok_or_else(|| Defect::from(LibraryDefect::MissingScheduleRecipe))?;
                let elapsed = clock
                    .now()
                    .checked_sub(started)
                    .ok_or_else(|| Defect::from(LibraryDefect::ClockDeadlineOverflow))?;
                (self.transition)(state, input, elapsed)
            }));
            let (output, next) = match decision {
                Ok(Ok(decision)) => decision,
                Ok(Err(defect)) => return append_defect(previous, defect),
                Err(panic) => return append_defect(previous, Defect::from_panic(panic)),
            };
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(output))) {
                return append_defect(previous, Defect::from_panic(panic));
            }
            let Some((delay, next)) = next else {
                return previous;
            };
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(state.0.replace(next)))) {
                return append_defect(previous, Defect::from_panic(panic));
            }

            let wait = if context.is_requested() {
                Err(PreparedClockError::Clock(ClockError::Interrupted))
            } else {
                clock.sleep(delay)
            };
            let result = match wait {
                Ok(wait) => {
                    // Immediate waits do not establish a new timer frontier.
                    if !delay.is_zero() {
                        drop(frontier.take());
                    }
                    wait.with_context(context).await
                }
                Err(error) => Err(error),
            };
            let discarded = catch_unwind(AssertUnwindSafe(|| drop(previous)));
            match result {
                Ok(tick) => match discarded {
                    Ok(()) => {
                        if !delay.is_zero() {
                            frontier = Some(tick);
                        }
                    }
                    Err(panic) => return Exit::Die(Defect::from_panic(panic)),
                },
                Err(error) => {
                    let failure = match error {
                        PreparedClockError::Memory(error) => Exit::Fail(on_capacity(error)),
                        PreparedClockError::Clock(ClockError::Interrupted) => Exit::Interrupted,
                        PreparedClockError::Clock(ClockError::Overflow) => {
                            Exit::Die(Defect::from(LibraryDefect::ClockDeadlineOverflow))
                        }
                        PreparedClockError::Clock(ClockError::Defect(defect)) => Exit::Die(defect),
                    };
                    return match discarded {
                        Ok(()) => failure,
                        Err(panic) => append_defect(failure, Defect::from_panic(panic)),
                    };
                }
            }
        }
    }
}

struct SeriesState<State: Send + 'static>(SlotLease<'static, Option<State>>);

impl<State: Send + 'static> Drop for SeriesState<State> {
    fn drop(&mut self) {
        drop(self.0.take());
    }
}

fn append_defect<A, E>(previous: Exit<A, E>, defect: Defect) -> Exit<A, E> {
    // Every series failure appends one cause; an existing compound outcome
    // never enters the schedule transition.
    outcome::append_failures(
        previous,
        std::iter::once(CleanupFailure::ReleaseDefect(defect)),
    )
}
