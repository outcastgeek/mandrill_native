use effect::{Cancellation, Clock, ClockError, ClockFuture, ClockTick};
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Poll, Waker};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

struct Signal {
    elapsed: AtomicBool,
    stopped: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

struct TimerWorker {
    signal: Arc<Signal>,
    thread: Thread,
    handle: Option<JoinHandle<()>>,
}

impl Drop for TimerWorker {
    fn drop(&mut self) {
        self.signal.stopped.store(true, Ordering::Release);
        self.thread.unpark();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub struct HostClock {
    origin: Instant,
}

impl HostClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for HostClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn sleep(&self, duration: Duration, cancellation: Cancellation) -> ClockFuture {
        Box::pin(async move {
            if cancellation.is_requested() {
                return Err(ClockError::Interrupted);
            }
            let deadline = Instant::now()
                .checked_add(duration)
                .ok_or(ClockError::Overflow)?;
            let signal = Arc::new(Signal {
                elapsed: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
                waker: Mutex::new(None),
            });
            let worker_signal = Arc::clone(&signal);
            let handle = thread::spawn(move || {
                while !worker_signal.stopped.load(Ordering::Acquire) {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        worker_signal.elapsed.store(true, Ordering::Release);
                        if let Some(waker) = worker_signal
                            .waker
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take()
                        {
                            waker.wake();
                        }
                        return;
                    }
                    thread::park_timeout(remaining);
                }
            });
            let worker = TimerWorker {
                signal: Arc::clone(&signal),
                thread: handle.thread().clone(),
                handle: Some(handle),
            };
            let cancelled = cancellation.cancelled();
            let mut cancelled = pin!(cancelled);
            let result = poll_fn(|context| {
                if cancelled.as_mut().poll(context).is_ready() {
                    return Poll::Ready(Err(ClockError::Interrupted));
                }
                if signal.elapsed.load(Ordering::Acquire) {
                    return Poll::Ready(Ok(ClockTick::elapsed()));
                }
                *signal
                    .waker
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(context.waker().clone());
                if signal.elapsed.load(Ordering::Acquire) {
                    Poll::Ready(Ok(ClockTick::elapsed()))
                } else {
                    Poll::Pending
                }
            })
            .await;
            drop(worker);
            result
        })
    }
}
