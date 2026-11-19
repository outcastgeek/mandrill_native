//! Prepared provider admission controls with separate settlement authorities.
use crate::AdmissionError;
use crate::memory::{
    Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool, prepared_notification,
};
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

struct Counts {
    closed: bool,
    active: usize,
    drivers: usize,
}
struct State {
    counts: Mutex<Counts>,
    capacity: NonZeroUsize,
    changed: prepared_notification::Event,
}

/// An admission refusal, distinguishing physical control admission from the provider limit.
#[derive(Debug)]
pub enum PreparedAdmissionError {
    Memory(MemoryError),
    Admission(AdmissionError),
}
/// Physically prepared provider controls and caller-sized drain listener storage.
pub struct PreparedProviderAdmission {
    state: SharedSlot<'static, State>,
}
/// The jointly admitted capacity and driver authorities.
pub struct PreparedAdmissionPermit {
    capacity: PreparedCapacityPermit,
    driver: PreparedDriverPermit,
}
/// Capacity remains owned until the actual accepted job settles.
pub struct PreparedCapacityPermit {
    state: Option<SharedSlot<'static, State>>,
}
/// Driver ownership remains until the actual driver wrapper retires.
pub struct PreparedDriverPermit {
    state: Option<SharedSlot<'static, State>>,
}
/// A physically admitted observer of driver retirement.
pub struct PreparedProviderDrain {
    listener: prepared_notification::Registration,
    state: SharedSlot<'static, State>,
}

impl PreparedProviderAdmission {
    /// Concrete payload/pool bytes, excluding allocator headers and padding.
    pub fn storage_bytes(listeners: usize) -> Result<Bytes, MemoryError> {
        SlotPool::<State>::storage_bytes(1)?
            .checked_add(prepared_notification::Event::storage_bytes(listeners)?)
    }
    /// Prepare the selected admission and concurrent drain-listener limits.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: NonZeroUsize,
        listeners: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(listeners)?;
        let mut changed = Some(prepared_notification::Event::prepare(
            memory, region, listeners,
        )?);
        let pool = memory.slots(region, 1, |_| {
            Ok(State {
                counts: Mutex::new(Counts {
                    closed: false,
                    active: 0,
                    drivers: 0,
                }),
                capacity,
                changed: changed.take().ok_or(MemoryError::NoSlot)?,
            })
        })?;
        Ok(Self {
            state: pool.checkout()?.share(),
        })
    }
    /// Fallibly retain the same controls.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            state: self.state.try_clone()?,
        })
    }
    /// Acquire both owners before committing either counter.
    pub fn admit(&self) -> Result<PreparedAdmissionPermit, PreparedAdmissionError> {
        let capacity = self
            .state
            .try_clone()
            .map_err(PreparedAdmissionError::Memory)?;
        let driver = self
            .state
            .try_clone()
            .map_err(PreparedAdmissionError::Memory)?;
        let mut counts = self
            .state
            .counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if counts.closed {
            return Err(PreparedAdmissionError::Admission(AdmissionError::Closed));
        }
        if counts.active == self.state.capacity.get() {
            return Err(PreparedAdmissionError::Admission(AdmissionError::Full));
        }
        let active = counts
            .active
            .checked_add(1)
            .ok_or(PreparedAdmissionError::Admission(AdmissionError::Overflow))?;
        let drivers = counts
            .drivers
            .checked_add(1)
            .ok_or(PreparedAdmissionError::Admission(AdmissionError::Overflow))?;
        counts.active = active;
        counts.drivers = drivers;
        drop(counts);
        Ok(PreparedAdmissionPermit {
            capacity: PreparedCapacityPermit {
                state: Some(capacity),
            },
            driver: PreparedDriverPermit {
                state: Some(driver),
            },
        })
    }
    /// Close admission independently of listener availability.
    pub fn stop_now(&self) {
        let first = {
            let mut counts = self
                .state
                .counts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let first = !counts.closed;
            counts.closed = true;
            first
        };
        if first {
            self.state.changed.notify_all();
        }
    }
    /// Reserve a drain observer before shutdown or business work requiring it.
    pub fn admit_drain(&self) -> Result<PreparedProviderDrain, MemoryError> {
        let listener = self.state.changed.listen()?;
        let state = self.state.try_clone()?;
        Ok(PreparedProviderDrain { listener, state })
    }
    /// Accepted operations whose capacity has not yet settled.
    #[must_use]
    pub fn active(&self) -> usize {
        self.state
            .counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active
    }
}
impl PreparedAdmissionPermit {
    /// Move the capacity and driver authorities to their actual distinct owners.
    #[must_use]
    pub fn split(self) -> (PreparedCapacityPermit, PreparedDriverPermit) {
        (self.capacity, self.driver)
    }
}
impl Drop for PreparedCapacityPermit {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            state
                .counts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .active -= 1;
        }
    }
}
impl Drop for PreparedDriverPermit {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let empty = {
                let mut counts = state
                    .counts
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                counts.drivers -= 1;
                counts.drivers == 0
            };
            if empty {
                state.changed.notify_all();
            }
        }
    }
}
impl Future for PreparedProviderDrain {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        this.listener.reset();
        let _ = this.listener.poll(cx);
        let counts = this
            .state
            .counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if counts.closed && counts.drivers == 0 {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
