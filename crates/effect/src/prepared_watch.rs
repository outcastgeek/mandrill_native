//! Bounded prepared watch with independently retained immutable versions.
use crate::memory::{
    Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotId, SlotLease, SlotPool,
    prepared_notification,
};
use std::sync::{Mutex, MutexGuard};
use std::task::Poll;

struct Version<T: Send + 'static> {
    slot: Option<SharedSlot<'static, Mutex<Option<T>>>>,
}
impl<T: Send + 'static> Version<T> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        match &self.slot {
            Some(slot) => Ok(Self {
                slot: Some(slot.try_clone()?),
            }),
            None => Err(MemoryError::Closed),
        }
    }
    fn id(&self) -> Option<SlotId> {
        self.slot.as_ref().map(SharedSlot::id)
    }
}
impl<T: Send + 'static> Drop for Version<T> {
    fn drop(&mut self) {
        if let Some(lease) = self.slot.take().and_then(SharedSlot::into_last) {
            let value = lease
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(value);
            drop(lease);
        }
    }
}
struct Value<T: Send + 'static> {
    current: Option<Version<T>>,
    senders: usize,
}
struct State<T: Send + 'static> {
    value: Mutex<Value<T>>,
    versions: SlotPool<'static, Mutex<Option<T>>>,
    changed: prepared_notification::Event,
}
struct StateOwner<T: Send + 'static>(Option<SharedSlot<'static, State<T>>>);
impl<T: Send + 'static> StateOwner<T> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        self.0
            .as_ref()
            .ok_or(MemoryError::Closed)?
            .try_clone()
            .map(|state| Self(Some(state)))
    }
}
impl<T: Send + 'static> std::ops::Deref for StateOwner<T> {
    type Target = State<T>;
    #[allow(clippy::expect_used)] // An endpoint always owns its state until its own drop.
    fn deref(&self) -> &Self::Target {
        self.0.as_deref().expect("live watch state")
    }
}
impl<T: Send + 'static> Drop for StateOwner<T> {
    fn drop(&mut self) {
        if let Some(lease) = self.0.take().and_then(SharedSlot::into_last) {
            let current = {
                let mut value = lease
                    .value
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                value.current.take()
            };
            drop(current);
            drop(lease);
        }
    }
}
/// Unique receiver registrations and fallibly shareable senders own this control.
pub struct PreparedWatchSender<T: Send + 'static> {
    state: StateOwner<T>,
}
/// A physical version and sender authority reserved before publication work.
pub struct PreparedWatchPublication<T: Send + 'static> {
    version: SlotLease<'static, Mutex<Option<T>>>,
    sender: PreparedWatchSender<T>,
}
impl<T: Send + 'static> PreparedWatchPublication<T> {
    /// Publish into the already reserved version without a capacity lookup.
    pub fn publish(self, next: T) {
        let Self { version, sender } = self;
        let stale = version
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(stale);
        *version
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(next);
        let old = sender
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current
            .replace(Version {
                slot: Some(version.share()),
            });
        let notify = std::mem::DropGuard::new(&sender.state.changed, |event| event.notify_all());
        drop(old);
        drop(notify);
    }
}
/// A receiver holds a physical listener until it is dropped.
pub struct PreparedWatchReceiver<T: Send + 'static> {
    listener: prepared_notification::Registration,
    state: StateOwner<T>,
    seen: SlotId,
}
/// An owned view retains the exact physical version until dropped.
pub struct PreparedWatchView<T: Send + 'static> {
    version: Version<T>,
}
/// Startup-provisioned reusable watch states and their physical backing.
/// A state returns after both endpoints retire. A retained view independently
/// owns its version; such views can exhaust the version pool during reuse.
pub struct PreparedWatchPool<T: Send + 'static> {
    states: SlotPool<'static, State<T>>,
}
impl<T: Send + 'static> PreparedWatchPool<T> {
    /// Pool payload and metadata bytes, excluding allocator headers and padding.
    pub fn storage_bytes(
        watches: usize,
        receivers: usize,
        versions: usize,
    ) -> Result<Bytes, MemoryError> {
        SlotPool::<State<T>>::storage_bytes(watches)?
            .checked_add(
                prepared_notification::Event::storage_bytes(receivers)?.checked_mul(watches)?,
            )?
            .checked_add(
                SlotPool::<Mutex<Option<T>>>::storage_bytes(versions)?.checked_mul(watches)?,
            )
    }
    /// Prepare every watch state, listener pool, and version pool at startup.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        watches: usize,
        receivers: usize,
        versions: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(watches, receivers, versions)?;
        let states = memory.slots(region, watches, |_| {
            Ok(State {
                value: Mutex::new(Value {
                    current: None,
                    senders: 0,
                }),
                versions: memory.slots(region, versions, |_| Ok(Mutex::new(None)))?,
                changed: prepared_notification::Event::prepare(memory, region, receivers)?,
            })
        })?;
        Ok(Self { states })
    }
    /// Retain the existing pool without a new allocation.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            states: self.states.try_clone()?,
        })
    }
    /// Admit one watch, returning the original initial value on refusal.
    pub fn try_watch(
        &self,
        initial: T,
    ) -> Result<(PreparedWatchSender<T>, PreparedWatchReceiver<T>), (MemoryError, T)> {
        let lease = match self.states.checkout() {
            Ok(lease) => lease,
            Err(error) => return Err((error, initial)),
        };
        let prior = {
            let mut value = lease
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            value.current.take()
        };
        drop(prior);
        let listener = match lease.changed.listen() {
            Ok(listener) => listener,
            Err(error) => return Err((error, initial)),
        };
        let version = match lease.versions.checkout() {
            Ok(version) => version,
            Err(error) => return Err((error, initial)),
        };
        let id = version.id();
        let state = StateOwner(Some(lease.share()));
        let receiver = match state.try_clone() {
            Ok(receiver) => receiver,
            Err(error) => return Err((error, initial)),
        };
        *version
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(initial);
        {
            let mut value = state
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            value.current = Some(Version {
                slot: Some(version.share()),
            });
            value.senders = 1;
        }
        Ok((
            PreparedWatchSender { state },
            PreparedWatchReceiver {
                listener,
                state: receiver,
                seen: id,
            },
        ))
    }
}
/// Borrow of the value in a retained version.
pub struct PreparedWatchBorrow<'a, T> {
    guard: MutexGuard<'a, Option<T>>,
}
impl<T> PreparedWatchBorrow<'_, T> {
    /// The value remains present while its owning view is retained.
    #[must_use]
    pub fn get(&self) -> Option<&T> {
        self.guard.as_ref()
    }
}
impl<T: Send + 'static> PreparedWatchView<T> {
    /// Borrow this version without cloning T.
    #[must_use]
    pub fn borrow(&self) -> Option<PreparedWatchBorrow<'_, T>> {
        self.version.slot.as_ref().map(|slot| PreparedWatchBorrow {
            guard: slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        })
    }
}
/// No senders remain and this receiver has observed the latest version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedWatchClosed;
/// Layout bytes for the control, listener and version pools, excluding allocator overhead.
pub fn prepared_watch_storage_bytes<T: Send + 'static>(
    receivers: usize,
    versions: usize,
) -> Result<Bytes, MemoryError> {
    SlotPool::<State<T>>::storage_bytes(1)?
        .checked_add(prepared_notification::Event::storage_bytes(receivers)?)?
        .checked_add(SlotPool::<Mutex<Option<T>>>::storage_bytes(versions)?)
}
/// Prepare a watch and initial receiver; on any refusal the original value is returned.
pub fn prepared_watch<T: Send + 'static>(
    memory: &PreparedMemory<'static>,
    region: RegionId,
    receivers: usize,
    versions: usize,
    initial: T,
) -> Result<(PreparedWatchSender<T>, PreparedWatchReceiver<T>), (MemoryError, T)> {
    #[allow(clippy::type_complexity)]
    fn setup<T: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        receivers: usize,
        versions: usize,
    ) -> Result<
        (
            StateOwner<T>,
            StateOwner<T>,
            prepared_notification::Registration,
            Version<T>,
            SlotId,
        ),
        MemoryError,
    > {
        let _ = prepared_watch_storage_bytes::<T>(receivers, versions)?;
        let changed = prepared_notification::Event::prepare(memory, region, receivers)?;
        let listener = changed.listen()?;
        let versions = memory.slots(region, versions, |_| Ok(Mutex::new(None)))?;
        let version = versions.checkout()?;
        let seen = version.id();
        let mut changed = Some(changed);
        let mut versions = Some(versions);
        let states = memory.slots(region, 1, |_| {
            Ok(State {
                value: Mutex::new(Value {
                    current: None,
                    senders: 1,
                }),
                versions: versions.take().ok_or(MemoryError::NoSlot)?,
                changed: changed.take().ok_or(MemoryError::NoSlot)?,
            })
        })?;
        let state = StateOwner(Some(states.checkout()?.share()));
        let receiver = state.try_clone()?;
        Ok((
            state,
            receiver,
            listener,
            Version {
                slot: Some(version.share()),
            },
            seen,
        ))
    }
    let (state, receiver, listener, version, seen) =
        match setup(memory, region, receivers, versions) {
            Ok(value) => value,
            Err(error) => return Err((error, initial)),
        };
    if let Some(slot) = &version.slot {
        *slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(initial);
    }
    state
        .value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .current = Some(version);
    Ok((
        PreparedWatchSender { state },
        PreparedWatchReceiver {
            listener,
            state: receiver,
            seen,
        },
    ))
}
impl<T: Send + 'static> PreparedWatchSender<T> {
    /// Reserve the actual publication version and sender before user work.
    pub fn reserve_publication(&self) -> Result<PreparedWatchPublication<T>, MemoryError> {
        let sender = self.try_clone()?;
        let version = sender.state.versions.checkout()?;
        Ok(PreparedWatchPublication { version, sender })
    }
    /// Retain another sender, with checked physical owner and sender count.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        let state = self.state.try_clone()?;
        let mut value = state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = value
            .senders
            .checked_add(1)
            .ok_or(MemoryError::OwnerLimit)?;
        value.senders = count;
        drop(value);
        Ok(Self { state })
    }
    /// Publish even when no receivers remain. A refusal returns the original T.
    pub fn publish_retained(&self, next: T) -> Result<(), (MemoryError, T)> {
        let lease = match self.state.versions.checkout() {
            Ok(lease) => lease,
            Err(error) => return Err((error, next)),
        };
        let stale = lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(stale);
        *lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(next);
        let version = Version {
            slot: Some(lease.share()),
        };
        let old = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current
            .replace(version);
        let notify = std::mem::DropGuard::new(&self.state.changed, |event| event.notify_all());
        drop(old);
        drop(notify);
        Ok(())
    }
}
impl<T: Send + 'static> Drop for PreparedWatchSender<T> {
    fn drop(&mut self) {
        let last = {
            let mut value = self
                .state
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            value.senders -= 1;
            value.senders == 0
        };
        if last {
            self.state.changed.notify_all();
        }
    }
}
impl<T: Send + 'static> PreparedWatchReceiver<T> {
    /// Reserve another listener and retain the same last-seen version identity.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        let state = self.state.try_clone()?;
        let listener = state.changed.listen()?;
        Ok(Self {
            listener,
            state,
            seen: self.seen,
        })
    }
    /// Retain the latest actual version and mark that version observed.
    pub fn borrow_and_update(&mut self) -> Result<Option<PreparedWatchView<T>>, MemoryError> {
        let value = self
            .state
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(current) = &value.current else {
            return Ok(None);
        };
        let version = current.try_clone()?;
        if let Some(id) = version.id() {
            self.seen = id;
        }
        Ok(Some(PreparedWatchView { version }))
    }
    /// Wait for an unseen version; a final unseen update precedes sender closure.
    pub async fn changed(&mut self) -> Result<(), PreparedWatchClosed> {
        std::future::poll_fn(|cx| {
            self.listener.reset();
            let _ = self.listener.poll(cx);
            let value = self
                .state
                .value
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(id) = value.current.as_ref().and_then(Version::id)
                && id != self.seen
            {
                self.seen = id;
                Poll::Ready(Ok(()))
            } else if value.senders == 0 {
                Poll::Ready(Err(PreparedWatchClosed))
            } else {
                Poll::Pending
            }
        })
        .await
    }
}
