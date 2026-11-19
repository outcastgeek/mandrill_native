//! Typed region storage for the selected Tokio timer payload.

use effect::__private::prepared_graph::FutureContext;
use effect::memory::{
    Bytes, FramePool, FrameReservation, MemoryError, PinnedValue, PreparedMemory, RegionId,
};
use effect::{ClockError, ClockTick};
use futures_util::future::{Either, select};
use std::alloc::Layout;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::time::Sleep;

/// Refusal at the physical frame or clock-provider boundary.
#[derive(Debug)]
pub enum PreparedTokioClockError {
    /// The selected timer backing could not admit another wait.
    Memory(MemoryError),
    /// Cancellation or a checked deadline overflow.
    Clock(ClockError),
}

impl From<MemoryError> for PreparedTokioClockError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<ClockError> for PreparedTokioClockError {
    fn from(error: ClockError) -> Self {
        Self::Clock(error)
    }
}

/// Tokio timer adapter providing real fixed storage for native Sleep values.
/// The supplied runtime owns its time driver and other Tokio resources.
/// Keep the runtime alive until admitted waits settle, and enable its time driver
/// as for the ordinary Tokio clock.
pub struct PreparedTokioClock {
    timers: FramePool<'static>,
    runtime: Arc<Runtime>,
    origin: Instant,
}

impl PreparedTokioClock {
    /// Physical timer frame demand excluding region allocator overhead and the
    /// enclosing graph's independent cancellation listener storage.
    pub fn storage_bytes(timers: NonZeroUsize) -> Result<Bytes, MemoryError> {
        FramePool::storage_bytes(timers.get(), Layout::new::<Sleep>())
    }

    pub(super) fn prepare(
        runtime: Arc<Runtime>,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        timers: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(timers)?;
        Ok(Self {
            timers: memory.frames(region, timers.get(), Layout::new::<Sleep>())?,
            runtime,
            origin: Instant::now(),
        })
    }

    /// Observe elapsed monotonic time since this adapter was prepared.
    #[must_use]
    pub fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    /// Reserve the actual native timer frame. Polling establishes the deadline
    /// and constructs the timer under the supplied runtime.
    pub fn reserve_sleep(
        &self,
        duration: Duration,
    ) -> Result<PreparedTokioSleep, PreparedTokioClockError> {
        let frame = self.timers.reserve::<Sleep>()?;
        Ok(PreparedTokioSleep {
            timer: Timer::Reserved(frame),
            runtime: Some(Arc::clone(&self.runtime)),
            duration,
        })
    }
}

enum Timer {
    Reserved(FrameReservation<'static, Sleep>),
    Running(PinnedValue<'static, Sleep>),
    Complete,
}

/// An affine native timer. The real Tokio Sleep remains pinned in its region
/// until completion or withdrawal. Dropping it deregisters an active timer.
#[must_use = "drive or retire the prepared Tokio wait"]
pub struct PreparedTokioSleep {
    // Any native timer is destroyed before its owning runtime.
    timer: Timer,
    runtime: Option<Arc<Runtime>>,
    duration: Duration,
}

// Moving the wait moves only an empty reservation or a pinned storage handle.
impl Unpin for PreparedTokioSleep {}

impl PreparedTokioSleep {
    /// Wait using the enclosing prepared graph's admitted cancellation listener.
    /// Cancellation wins both before attempting native work and if it races
    /// with a ready timer. The graph lends the listener for this wait.
    pub async fn with_context<E: Send + 'static>(
        self,
        context: &mut FutureContext<E>,
    ) -> Result<ClockTick, PreparedTokioClockError> {
        if context.is_requested() {
            return Err(ClockError::Interrupted.into());
        }
        let outcome = {
            let cancelled = pin!(context.cancelled());
            match select(cancelled, self).await {
                Either::Left((_, timer)) => {
                    drop(timer);
                    return Err(ClockError::Interrupted.into());
                }
                Either::Right((outcome, _)) => outcome,
            }
        };
        if outcome.is_ok() && context.is_requested() {
            Err(ClockError::Interrupted.into())
        } else {
            outcome
        }
    }

    fn start(&mut self) -> Result<(), PreparedTokioClockError> {
        let Timer::Reserved(frame) = std::mem::replace(&mut self.timer, Timer::Complete) else {
            return Err(MemoryError::UnsupportedRunContract.into());
        };
        let runtime = self
            .runtime
            .as_ref()
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let timer = {
            let _entered = runtime.enter();
            let deadline = tokio::time::Instant::now()
                .checked_add(self.duration)
                .ok_or(ClockError::Overflow)?;
            tokio::time::sleep_until(deadline)
        };
        self.timer = Timer::Running(frame.initialize(timer));
        Ok(())
    }

    fn retire(&mut self) {
        self.timer = Timer::Complete;
        drop(self.runtime.take());
    }
}

impl Future for PreparedTokioSleep {
    type Output = Result<ClockTick, PreparedTokioClockError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if matches!(this.timer, Timer::Reserved(_))
            && let Err(error) = this.start()
        {
            this.retire();
            return Poll::Ready(Err(error));
        }
        let Timer::Running(timer) = &mut this.timer else {
            return Poll::Ready(Err(MemoryError::UnsupportedRunContract.into()));
        };
        match timer.as_pin_mut().poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(()) => {
                this.retire();
                Poll::Ready(Ok(ClockTick::elapsed()))
            }
        }
    }
}
