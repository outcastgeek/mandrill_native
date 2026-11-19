//! One startup-provisioned, single-use channel with physical listener backing.

use crate::memory::{
    Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool, prepared_notification,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

struct Value<T> {
    value: Option<T>,
    sender_alive: bool,
    receiver_alive: bool,
}
struct State<T> {
    value: Mutex<Value<T>>,
    changed: prepared_notification::Event,
}
/// Unique sending authority for one prepared channel.
pub struct PreparedSender<T: Send + 'static> {
    state: SharedSlot<'static, State<T>>,
}
/// Unique receiver and its pre-admitted physical listener.
pub struct PreparedReceiver<T: Send + 'static> {
    listener: prepared_notification::Registration,
    state: SharedSlot<'static, State<T>>,
}
// The receiver owns handles to stable storage and never structurally pins T.
impl<T: Send + 'static> Unpin for PreparedReceiver<T> {}
/// The channel's sender was dropped before delivering a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedOneshotClosed;
/// Startup-owned finite pool of single-use channels.
pub struct PreparedOneshotPool<T: Send + 'static> {
    states: SlotPool<'static, State<T>>,
}
impl<T: Send + 'static> PreparedOneshotPool<T> {
    /// Layout bytes for all channel and listener pools; allocator headers and
    /// padding are separately charged by the selected memory region.
    pub fn storage_bytes(capacity: usize) -> Result<Bytes, MemoryError> {
        SlotPool::<State<T>>::storage_bytes(capacity)?
            .checked_add(prepared_notification::Event::storage_bytes(1)?.checked_mul(capacity)?)
    }
    /// Prepare every channel control and its listener pool before execution.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        let states = memory.slots(region, capacity, |_| {
            Ok(State {
                value: Mutex::new(Value {
                    value: None,
                    sender_alive: false,
                    receiver_alive: false,
                }),
                changed: prepared_notification::Event::prepare(memory, region, 1)?,
            })
        })?;
        Ok(Self { states })
    }
    /// Retain the prepared pool without allocating a control block.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            states: self.states.try_clone()?,
        })
    }
    /// Admit one channel. The physical slot returns only after both endpoints retire.
    pub fn try_channel(&self) -> Result<(PreparedSender<T>, PreparedReceiver<T>), MemoryError> {
        let lease = self.states.checkout()?;
        let listener = lease.changed.listen()?;
        let previous = {
            let mut state = lease
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.value.take()
        };
        drop(previous);
        {
            let mut state = lease
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.sender_alive = true;
            state.receiver_alive = true;
        }
        let state = lease.share();
        let receiver = match state.try_clone() {
            Ok(receiver) => receiver,
            Err(error) => {
                drop(listener);
                drop(state);
                return Err(error);
            }
        };
        Ok((
            PreparedSender { state },
            PreparedReceiver {
                listener,
                state: receiver,
            },
        ))
    }
}
/// Pool layout bytes for one channel, excluding partition allocator headers and padding.
pub fn prepared_oneshot_storage_bytes<T: Send + 'static>() -> Result<Bytes, MemoryError> {
    PreparedOneshotPool::<T>::storage_bytes(1)
}
/// Prepare one single-use channel and its receiver's actual listener at startup.
pub fn prepared_oneshot<T: Send + 'static>(
    memory: &PreparedMemory<'static>,
    region: RegionId,
) -> Result<(PreparedSender<T>, PreparedReceiver<T>), MemoryError> {
    PreparedOneshotPool::<T>::prepare(memory, region, 1)?.try_channel()
}
impl<T: Send + 'static> PreparedSender<T> {
    /// Deliver the value once, or return it if the receiver has retired.
    pub fn send(self, value: T) -> Result<(), T> {
        let mut state = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.sender_alive = false;
        let result = if state.receiver_alive && state.value.is_none() {
            state.value = Some(value);
            Ok(())
        } else {
            Err(value)
        };
        drop(state);
        if result.is_ok() {
            self.state.changed.notify_all();
        }
        result
    }
}
impl<T: Send + 'static> Drop for PreparedSender<T> {
    fn drop(&mut self) {
        let mut state = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = state.sender_alive;
        state.sender_alive = false;
        drop(state);
        if changed {
            self.state.changed.notify_all();
        }
    }
}
impl<T: Send + 'static> Future for PreparedReceiver<T> {
    type Output = Result<T, PreparedOneshotClosed>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_ref().get_ref();
        let _notification = this.listener.poll(cx);
        let mut state = this
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(value) = state.value.take() {
            Poll::Ready(Ok(value))
        } else if state.sender_alive {
            Poll::Pending
        } else {
            Poll::Ready(Err(PreparedOneshotClosed))
        }
    }
}
impl<T: Send + 'static> Drop for PreparedReceiver<T> {
    fn drop(&mut self) {
        let value = {
            let mut state = self
                .state
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.receiver_alive = false;
            state.value.take()
        };
        drop(value);
    }
}
