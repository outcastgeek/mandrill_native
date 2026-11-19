//! Startup-backed publication and subscriber ownership.
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
    prepared_notification,
};
use crate::{
    PreparedPayloadLink, PreparedPayloadPool, PreparedPayloadReservation, PreparedPayloadView,
    PubSubError, PubSubOptions, ReplayPolicy,
};
use std::sync::{Arc, Mutex};
type SubRef<T> = Arc<SlotLease<'static, Subscriber<T>>>;

/// Caller-selected physical capacities. Logical buffering remains in `PubSubOptions`.
#[derive(Clone, Copy, Debug)]
pub struct PreparedPubSubCapacity {
    /// Actual payload cells, including payloads retained by escaped views.
    pub payloads: usize,
    /// Simultaneously admitted publication operations.
    pub publishers: usize,
    /// Simultaneously retained subscriber records.
    pub subscribers: usize,
    /// Simultaneously admitted take listeners for each subscriber.
    pub takers_per_subscriber: usize,
}
/// Preparation failure keeps physical admission distinct from policy validation.
#[derive(Debug)]
pub enum PreparedPubSubError {
    Memory(MemoryError),
    Policy(PubSubError),
}
impl From<MemoryError> for PreparedPubSubError {
    fn from(value: MemoryError) -> Self {
        Self::Memory(value)
    }
}
struct Subscriber<T: Send + Sync + 'static> {
    buffer: Mutex<FixedVector<'static, PreparedPayloadView<T>>>,
    changed: prepared_notification::Event,
}
struct Indexed<T: Send + Sync + 'static> {
    id: u64,
    dropped: usize,
    subscriber: SubRef<T>,
}
struct HubState<T: Send + Sync + 'static> {
    closed: bool,
    next_id: Option<u64>,
    subscribers: FixedVector<'static, Indexed<T>>,
    history: FixedVector<'static, PreparedPayloadView<T>>,
    current: Option<PreparedPayloadView<T>>,
}
struct PublishScratch<T: Send + Sync + 'static> {
    links: FixedVector<'static, PreparedPayloadLink<T>>,
    notifications: FixedVector<'static, SubRef<T>>,
    discarded: FixedVector<'static, PreparedPayloadView<T>>,
}
struct CloseScratch<T: Send + Sync + 'static> {
    discarded: FixedVector<'static, PreparedPayloadView<T>>,
}
struct Hub<T: Send + Sync + 'static> {
    state: Mutex<HubState<T>>,
    space: prepared_notification::Event,
    payloads: PreparedPayloadPool<T>,
    subscribers: Mutex<FixedVector<'static, SubRef<T>>>,
    publishers: SlotPool<'static, PublishScratch<T>>,
    closes: SlotPool<'static, CloseScratch<T>>,
    shutdown: Mutex<Option<SlotLease<'static, PublishScratch<T>>>>,
    options: PubSubOptions,
    capacity: PreparedPubSubCapacity,
    retain_current: bool,
    panics: crate::PreparedPanicPool,
}
/// A bounded hub with physical payload, publisher and subscriber backing.
pub struct PreparedPubSub<T: Send + Sync + 'static> {
    hub: SharedSlot<'static, Hub<T>>,
}
/// Physical region demand; cleanup and diagnostic pools are separate dependencies.
#[derive(Clone, Copy, Debug)]
pub struct PreparedPubSubDemand {
    /// Region payload and pool metadata, excluding allocator headers and `T` pointees.
    pub region_bytes: Bytes,
    /// Actual preclaimed links per publication, including history/current.
    pub links_per_publication: usize,
    /// Notification records held independently for shutdown.
    pub shutdown_notifications: usize,
    /// Retired-view slots held independently for shutdown.
    pub shutdown_retired_views: usize,
    /// Startup System Arc controls; one per physical subscriber record.
    /// Private aliases are limited by admitted subscribers, take listeners,
    /// publishers and shutdown owners. Each alias clones an existing Arc; a
    /// record can be reused only after every alias retires and Arc::get_mut
    /// confirms the registry is the unique owner.
    pub startup_system_arc_controls: usize,
    /// Bytes in the controls' SlotLease payloads (count times its layout size).
    /// Arc headers, padding and System allocator rounding are additional; this
    /// is not an exact heap or RSS measurement.
    pub startup_system_arc_payload_bytes: Bytes,
}
fn array<T>(n: usize) -> Result<Bytes, MemoryError> {
    Ok(Bytes::new(
        std::alloc::Layout::array::<T>(n)
            .map_err(|_| MemoryError::InvalidLayout)?
            .size(),
    ))
}
fn checked_add(a: usize, b: usize) -> Result<usize, MemoryError> {
    a.checked_add(b).ok_or(MemoryError::ArithmeticOverflow)
}
impl<T: Send + Sync + 'static> PreparedPubSub<T> {
    /// Report actual preallocated storage; cleanup and diagnostic authorities are separate.
    pub fn demand(
        options: PubSubOptions,
        capacity: PreparedPubSubCapacity,
        retain_current: bool,
    ) -> Result<PreparedPubSubDemand, PreparedPubSubError> {
        options.validate().map_err(PreparedPubSubError::Policy)?;
        let s = capacity.subscribers;
        let replay = match options.replay {
            ReplayPolicy::None => 0,
            ReplayPolicy::Last(n) => n.get(),
        };
        let links = checked_add(
            checked_add(s, usize::from(replay != 0))?,
            usize::from(retain_current),
        )?;
        let retired = checked_add(
            checked_add(
                s.checked_mul(options.capacity.get())
                    .ok_or(MemoryError::ArithmeticOverflow)?,
                replay,
            )?,
            usize::from(retain_current),
        )?;
        let publish_retired = checked_add(
            checked_add(s, usize::from(replay != 0))?,
            usize::from(retain_current),
        )?;
        let pub_scratch = array::<PreparedPayloadLink<T>>(links)?
            .checked_add(array::<SubRef<T>>(s)?)?
            .checked_add(array::<PreparedPayloadView<T>>(publish_retired)?)?;
        let shutdown_arrays =
            array::<SubRef<T>>(s)?.checked_add(array::<PreparedPayloadView<T>>(retired)?)?;
        let bytes = SlotPool::<Hub<T>>::storage_bytes(1)?
            .checked_add(PreparedPayloadPool::<T>::storage_bytes(capacity.payloads)?)?
            .checked_add(SlotPool::<Subscriber<T>>::storage_bytes(s)?)?
            .checked_add(array::<SubRef<T>>(s)?)?
            .checked_add(array::<PreparedPayloadView<T>>(options.capacity.get())?.checked_mul(s)?)?
            .checked_add(
                prepared_notification::Event::storage_bytes(capacity.takers_per_subscriber)?
                    .checked_mul(s)?,
            )?
            .checked_add(prepared_notification::Event::storage_bytes(
                capacity.publishers,
            )?)?
            .checked_add(array::<Indexed<T>>(s)?)?
            .checked_add(array::<PreparedPayloadView<T>>(replay)?)?
            .checked_add(SlotPool::<PublishScratch<T>>::storage_bytes(
                capacity.publishers,
            )?)?
            .checked_add(SlotPool::<PublishScratch<T>>::storage_bytes(1)?)?
            .checked_add(pub_scratch.checked_mul(capacity.publishers)?)?
            .checked_add(shutdown_arrays)?
            .checked_add(SlotPool::<CloseScratch<T>>::storage_bytes(s)?)?
            .checked_add(
                array::<PreparedPayloadView<T>>(options.capacity.get())?.checked_mul(s)?,
            )?;
        Ok(PreparedPubSubDemand {
            region_bytes: bytes,
            links_per_publication: links,
            shutdown_notifications: s,
            shutdown_retired_views: retired,
            startup_system_arc_controls: s,
            startup_system_arc_payload_bytes: array::<SlotLease<'static, Subscriber<T>>>(s)?,
        })
    }
}
impl<T: Send + Sync + 'static> PreparedPubSub<T> {
    // The hub is not exposed until its fresh reserved current cell is filled.
    #[allow(clippy::result_large_err)]
    pub(crate) fn prepare_current(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        options: PubSubOptions,
        capacity: PreparedPubSubCapacity,
        panics: &crate::PreparedPanicPool,
        initial: T,
    ) -> Result<Self, (PreparedPubSubError, T)> {
        let hub = match Self::prepare(memory, region, options, capacity, true, panics) {
            Ok(hub) => hub,
            Err(error) => return Err((error, initial)),
        };
        let writer = match hub.hub.payloads.reserve() {
            Ok(writer) => writer,
            Err(error) => return Err((PreparedPubSubError::Memory(error), initial)),
        };
        let view = match writer.publish(initial) {
            Ok(view) => view,
            Err((_writer, initial)) => {
                return Err((
                    PreparedPubSubError::Memory(MemoryError::ConstructionAlreadyBound),
                    initial,
                ));
            }
        };
        {
            let mut state = hub
                .hub
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // This private constructor owns a fresh, unpublished hub.
            debug_assert!(state.current.is_none());
            state.current = Some(view);
        }
        Ok(hub)
    }
    /// Provision all hub storage before publication or subscriber callbacks.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        options: PubSubOptions,
        capacity: PreparedPubSubCapacity,
        retain_current: bool,
        panics: &crate::PreparedPanicPool,
    ) -> Result<Self, PreparedPubSubError> {
        let demand = Self::demand(options, capacity, retain_current)?;
        let s = capacity.subscribers;
        let replay = match options.replay {
            ReplayPolicy::None => 0,
            ReplayPolicy::Last(n) => n.get(),
        };
        let publish_retired = demand.links_per_publication;
        let scratch = memory.slots(region, capacity.publishers, |_| {
            Ok(PublishScratch {
                links: FixedVector::prepare(
                    memory.allocator(region)?,
                    demand.links_per_publication,
                )?,
                notifications: FixedVector::prepare(memory.allocator(region)?, s)?,
                discarded: FixedVector::prepare(memory.allocator(region)?, publish_retired)?,
            })
        })?;
        let shutdown_pool = memory.slots(region, 1, |_| {
            Ok(PublishScratch {
                links: FixedVector::prepare(memory.allocator(region)?, 0)?,
                notifications: FixedVector::prepare(memory.allocator(region)?, s)?,
                discarded: FixedVector::prepare(
                    memory.allocator(region)?,
                    demand.shutdown_retired_views,
                )?,
            })
        })?;
        let shutdown = shutdown_pool.checkout()?;
        let payloads = PreparedPayloadPool::prepare(memory, region, capacity.payloads)?;
        let subscribers = memory.slots(region, s, |_| {
            Ok(Subscriber {
                buffer: Mutex::new(FixedVector::prepare(
                    memory.allocator(region)?,
                    options.capacity.get(),
                )?),
                changed: prepared_notification::Event::prepare(
                    memory,
                    region,
                    capacity.takers_per_subscriber,
                )?,
            })
        })?;
        let mut registry = FixedVector::prepare(memory.allocator(region)?, s)?;
        for _ in 0..s {
            registry.push_reserved(Arc::new(subscribers.checkout()?));
        }
        let subscribers = Mutex::new(registry);
        let closes = memory.slots(region, s, |_| {
            Ok(CloseScratch {
                discarded: FixedVector::prepare(memory.allocator(region)?, options.capacity.get())?,
            })
        })?;
        let state = HubState {
            closed: false,
            next_id: Some(0),
            subscribers: FixedVector::prepare(memory.allocator(region)?, s)?,
            history: FixedVector::prepare(memory.allocator(region)?, replay)?,
            current: None,
        };
        let hub = Hub {
            state: Mutex::new(state),
            space: prepared_notification::Event::prepare(memory, region, capacity.publishers)?,
            payloads,
            subscribers,
            publishers: scratch,
            closes,
            shutdown: Mutex::new(Some(shutdown)),
            options,
            capacity,
            retain_current,
            panics: panics.try_clone()?,
        };
        let mut pending = Some(hub);
        let roots = memory.slots(region, 1, |_| pending.take().ok_or(MemoryError::NoSlot))?;
        Ok(Self {
            hub: roots.checkout()?.share(),
        })
    }
    /// Fallibly retain the same physical hub.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            hub: self.hub.try_clone()?,
        })
    }
}

/// Unique unsubscribe authority. Take futures retain the physical subscriber
/// record but cannot delay this authority's removal from the hub.
pub struct PreparedSubscription<T: Send + Sync + 'static> {
    hub: PreparedPubSub<T>,
    subscriber: SubRef<T>,
    id: u64,
    close: Option<CloseOwner<T>>,
}
struct CloseOwner<T: Send + Sync + 'static> {
    slot: Option<SlotLease<'static, CloseScratch<T>>>,
}
struct ContinueClose<'a, T: Send + Sync + 'static> {
    owner: &'a mut CloseOwner<T>,
    armed: bool,
}
impl<T: Send + Sync + 'static> Drop for ContinueClose<'_, T> {
    fn drop(&mut self) {
        if self.armed {
            self.owner.clear();
        }
    }
}
impl<T: Send + Sync + 'static> CloseOwner<T> {
    fn clear(&mut self) {
        while let Some(value) = self
            .slot
            .as_mut()
            .and_then(|slot| slot.discarded.remove_first())
        {
            let mut continuation = ContinueClose {
                owner: self,
                armed: true,
            };
            drop(value);
            continuation.armed = false;
        }
        drop(self.slot.take());
    }
}
impl<T: Send + Sync + 'static> Drop for CloseOwner<T> {
    fn drop(&mut self) {
        self.clear();
    }
}
impl<T: Send + Sync + 'static> PreparedPubSub<T> {
    /// Reserve a physical subscriber and its independent close authority before publication.
    pub fn subscribe(&self) -> Result<PreparedSubscription<T>, PreparedPubSubError> {
        self.subscribe_inner(false)
            .map(|(_, subscription)| subscription)
    }
    /// Atomically retain the current value and register without replaying history.
    #[allow(clippy::unreachable)] // The same gated admission requires and retains current.
    pub fn subscribe_current(
        &self,
    ) -> Result<(PreparedPayloadView<T>, PreparedSubscription<T>), PreparedPubSubError> {
        let (current, subscription) = self.subscribe_inner(true)?;
        match current {
            Some(value) => Ok((value, subscription)),
            None => unreachable!("current registration validates current under the hub gate"),
        }
    }
    /// Retain the committed current value under the publication gate.
    pub fn current(&self) -> Result<PreparedPayloadView<T>, PreparedPubSubError> {
        let state = self
            .hub
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(PreparedPubSubError::Policy(PubSubError::Shutdown));
        }
        let value = state
            .current
            .as_ref()
            .ok_or(PreparedPubSubError::Policy(PubSubError::Shutdown))?;
        Ok(value.try_clone()?)
    }
    fn subscribe_inner(
        &self,
        current_only: bool,
    ) -> Result<(Option<PreparedPayloadView<T>>, PreparedSubscription<T>), PreparedPubSubError>
    {
        let hub = self.try_clone()?;
        let subscriber = {
            let mut registry = self
                .hub
                .subscribers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut selected = None;
            for cell in registry.as_mut_slice() {
                if Arc::get_mut(cell).is_some() {
                    selected = Some(Arc::clone(cell));
                    break;
                }
            }
            selected.ok_or(PreparedPubSubError::Memory(MemoryError::NoSlot))?
        };
        let index_owner = Arc::clone(&subscriber);
        let mut close = CloseOwner {
            slot: Some(self.hub.closes.checkout()?),
        };
        let mut state = self
            .hub
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(PreparedPubSubError::Policy(PubSubError::Shutdown));
        }
        let current = if current_only {
            Some(
                state
                    .current
                    .as_ref()
                    .ok_or(PreparedPubSubError::Policy(PubSubError::Shutdown))?
                    .try_clone()?,
            )
        } else {
            None
        };
        let id = state.next_id.ok_or(PreparedPubSubError::Policy(
            PubSubError::SubscriptionIdExhausted,
        ))?;
        // History views are copied into pre-reserved close scratch first. A failed
        // retain cannot change the subscriber/index or destroy history under the gate.
        #[allow(clippy::expect_used)]
        let scratch = close.slot.as_mut().expect("admitted close scratch");
        if !current_only {
            for value in state.history.as_slice() {
                scratch.discarded.push_reserved(value.try_clone()?);
            }
        }
        {
            let mut buffer = subscriber
                .buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while let Some(value) = scratch.discarded.remove_first() {
                buffer.push_reserved(value);
            }
        }
        state.next_id = id.checked_add(1);
        state.subscribers.push_reserved(Indexed {
            id,
            dropped: 0,
            subscriber: index_owner,
        });
        drop(state);
        Ok((
            current,
            PreparedSubscription {
                hub,
                subscriber,
                id,
                close: Some(close),
            },
        ))
    }
}
impl<T: Send + Sync + 'static> PreparedSubscription<T> {
    /// Unregister immediately; retained takes subsequently observe End.
    pub fn close_now(&mut self) -> bool {
        let Some(mut close) = self.close.take() else {
            return false;
        };
        let mut state = self
            .hub
            .hub
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(index) = state.subscribers.as_slice().iter().position(|entry| {
            entry.id == self.id && Arc::ptr_eq(&entry.subscriber, &self.subscriber)
        }) else {
            return false;
        };
        #[allow(clippy::expect_used)]
        let removed = state
            .subscribers
            .remove_at(index)
            .expect("matched subscription index");
        #[allow(clippy::expect_used)]
        let scratch = close.slot.as_mut().expect("admitted close scratch");
        {
            let mut buffer = self
                .subscriber
                .buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while let Some(value) = buffer.remove_first() {
                scratch.discarded.push_reserved(value);
            }
        }
        drop(state);
        self.subscriber.changed.notify_all();
        self.hub.hub.space.notify_all();
        drop(removed);
        drop(close);
        true
    }
}
impl<T: Send + Sync + 'static> Drop for PreparedSubscription<T> {
    fn drop(&mut self) {
        let _ = self.close_now();
    }
}

/// One bounded subscription observation. Item views retain physical backing.
pub enum PreparedSubscriptionEvent<T: Send + Sync + 'static> {
    Item(PreparedPayloadView<T>),
    Lagged(std::num::NonZeroUsize),
    End,
}
enum Cancellation {
    Native(crate::internal::runtime::prepared_graph::PreparedCancellationControl),
    Standalone {
        wait: crate::PreparedCancellationSignalWait,
        token: crate::PreparedCancellation,
    },
}
impl Cancellation {
    fn requested(&self) -> bool {
        match self {
            Self::Native(control) => control.is_requested(),
            Self::Standalone { token, .. } => token.is_requested(),
        }
    }
    fn register(&mut self, cx: &mut std::task::Context<'_>) {
        match self {
            Self::Native(control) => {
                let _ = control.poll_cancelled(cx);
            }
            Self::Standalone { wait, .. } => {
                let _ = std::future::Future::poll(std::pin::Pin::new(wait), cx);
            }
        }
    }
}
/// An admitted subscription take, retaining the actual subscriber slot.
pub struct PreparedSubscriptionTake<T: Send + Sync + 'static> {
    listener: prepared_notification::Registration,
    subscriber: SubRef<T>,
    hub: PreparedPubSub<T>,
    id: u64,
    cancellation: Cancellation,
}
impl<T: Send + Sync + 'static> Unpin for PreparedSubscriptionTake<T> {}
impl<T: Send + Sync + 'static> PreparedSubscription<T> {
    /// Admit one bounded listener before waiting with a prepared cancellation token.
    pub fn take(
        &self,
        token: &crate::PreparedCancellation,
    ) -> Result<PreparedSubscriptionTake<T>, MemoryError> {
        let listener = self.subscriber.changed.listen()?;
        let subscriber = Arc::clone(&self.subscriber);
        let hub = self.hub.try_clone()?;
        let wait = token.cancelled()?;
        let token = token.try_clone()?;
        Ok(PreparedSubscriptionTake {
            listener,
            subscriber,
            hub,
            id: self.id,
            cancellation: Cancellation::Standalone { wait, token },
        })
    }
    /// Admit a listener while retaining the existing prepared future context.
    #[allow(clippy::result_large_err)]
    pub fn take_with_context<E: Send + 'static>(
        &self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
    ) -> Result<
        PreparedSubscriptionTake<T>,
        (
            MemoryError,
            crate::internal::runtime::prepared_graph::FutureContext<E>,
        ),
    > {
        let result = (|| {
            Ok::<_, MemoryError>((
                self.subscriber.changed.listen()?,
                Arc::clone(&self.subscriber),
                self.hub.try_clone()?,
            ))
        })();
        match result {
            Ok((listener, subscriber, hub)) => Ok(PreparedSubscriptionTake {
                listener,
                subscriber,
                hub,
                id: self.id,
                cancellation: Cancellation::Native(context.into_cancellation_control()),
            }),
            Err(error) => Err((error, context)),
        }
    }
}
impl<T: Send + Sync + 'static> std::future::Future for PreparedSubscriptionTake<T> {
    type Output = crate::Exit<PreparedSubscriptionEvent<T>, PubSubError>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = &mut *self;
        this.listener.reset();
        let _ = this.listener.poll(cx);
        this.cancellation.register(cx);
        let mut state = this
            .hub
            .hub
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if this.cancellation.requested() {
            return std::task::Poll::Ready(crate::Exit::Interrupted);
        }
        let closed = state.closed;
        let found =
            state.subscribers.as_mut_slice().iter_mut().find(|entry| {
                entry.id == this.id && Arc::ptr_eq(&entry.subscriber, &this.subscriber)
            });
        let (outcome, freed) = match found {
            None => (Some(PreparedSubscriptionEvent::End), false),
            Some(entry) => {
                if let Some(lag) = std::num::NonZeroUsize::new(std::mem::take(&mut entry.dropped)) {
                    (Some(PreparedSubscriptionEvent::Lagged(lag)), false)
                } else if let Some(value) = entry
                    .subscriber
                    .buffer
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove_first()
                {
                    (Some(PreparedSubscriptionEvent::Item(value)), true)
                } else if closed {
                    (Some(PreparedSubscriptionEvent::End), false)
                } else {
                    (None, false)
                }
            }
        };
        drop(state);
        if freed {
            this.hub.hub.space.notify_all();
        }
        match outcome {
            Some(value) => std::task::Poll::Ready(crate::Exit::Success(value)),
            None => std::task::Poll::Pending,
        }
    }
}

struct PublishOwner<T: Send + Sync + 'static> {
    slot: Option<SlotLease<'static, PublishScratch<T>>>,
}
struct ContinuePublish<'a, T: Send + Sync + 'static> {
    owner: &'a mut PublishOwner<T>,
    armed: bool,
}
impl<T: Send + Sync + 'static> Drop for ContinuePublish<'_, T> {
    fn drop(&mut self) {
        if self.armed {
            self.owner.clear();
        }
    }
}
enum PublishDrop<T: Send + Sync + 'static> {
    Link(PreparedPayloadLink<T>),
    Notification(SubRef<T>),
    Value(PreparedPayloadView<T>),
}
impl<T: Send + Sync + 'static> PublishOwner<T> {
    #[allow(clippy::expect_used)]
    fn get(&mut self) -> &mut PublishScratch<T> {
        self.slot
            .as_deref_mut()
            .expect("admitted publication scratch")
    }
    fn clear(&mut self) {
        loop {
            let next = self.slot.as_mut().and_then(|slot| {
                slot.notifications
                    .remove_first()
                    .map(PublishDrop::Notification)
                    .or_else(|| slot.discarded.remove_first().map(PublishDrop::Value))
                    .or_else(|| slot.links.remove_first().map(PublishDrop::Link))
            });
            let Some(value) = next else { break };
            let mut continuation = ContinuePublish {
                owner: self,
                armed: true,
            };
            match value {
                PublishDrop::Link(value) => drop(value),
                PublishDrop::Notification(value) => drop(value),
                PublishDrop::Value(value) => drop(value),
            }
            continuation.armed = false;
        }
        drop(self.slot.take());
    }
}
impl<T: Send + Sync + 'static> Drop for PublishOwner<T> {
    fn drop(&mut self) {
        self.clear();
    }
}
/// Preclaimed physical publication cell and fanout links.
pub struct PreparedPublication<T: Send + Sync + 'static> {
    listener: prepared_notification::Registration,
    scratch: PublishOwner<T>,
    writer: Option<PreparedPayloadReservation<T>>,
    hub: PreparedPubSub<T>,
    diagnostic: Option<crate::PreparedPanicReservation>,
}
impl<T: Send + Sync + 'static> PreparedPubSub<T> {
    /// Reserve every physical publication owner before the candidate factory.
    pub fn admit_publication(&self) -> Result<PreparedPublication<T>, MemoryError> {
        self.admit_publication_mode(true)
    }
    pub(crate) fn admit_publication_mode(
        &self,
        diagnostic_required: bool,
    ) -> Result<PreparedPublication<T>, MemoryError> {
        let listener = self.hub.space.listen()?;
        let writer = self.hub.payloads.reserve()?;
        let mut scratch = PublishOwner {
            slot: Some(self.hub.publishers.checkout()?),
        };
        let count = Self::demand(self.hub.options, self.hub.capacity, self.hub.retain_current)
            .map_err(|error| match error {
                PreparedPubSubError::Memory(error) => error,
                PreparedPubSubError::Policy(_) => MemoryError::InvalidLayout,
            })?
            .links_per_publication;
        for _ in 0..count {
            scratch.get().links.push_reserved(writer.try_link()?);
        }
        let diagnostic = if diagnostic_required {
            Some(self.hub.panics.try_reserve()?)
        } else {
            None
        };
        let hub = self.try_clone()?;
        Ok(PreparedPublication {
            listener,
            scratch,
            writer: Some(writer),
            hub,
            diagnostic,
        })
    }
}
enum Attempt {
    Wait,
    Complete(crate::PublishReport),
    Skipped,
    Interrupted,
    Failed(PubSubError),
}
impl<T: Send + Sync + 'static> PreparedPublication<T> {
    // The caller supplies a non-backpressured hub and an infallible metadata
    // commit. All publication checks and links precede it; wakes follow it.
    #[allow(clippy::expect_used, clippy::unreachable)]
    pub(crate) fn publish_committed(
        mut self,
        candidate: T,
        commit: impl FnOnce(&T),
    ) -> Result<crate::PublishReport, PubSubError> {
        let writer = self.writer.take().expect("fresh publication");
        let value = match writer.publish(candidate) {
            Ok(value) => value,
            Err(_) => unreachable!("fresh candidate"),
        };
        let commit = std::cell::Cell::new(Some(commit));
        let guard = |candidate: &T| {
            commit.take().expect("single publication attempt")(candidate);
            true
        };
        match self.attempt(&value, Some(&guard), None) {
            Attempt::Complete(report) => {
                self.notify_and_retire();
                Ok(report)
            }
            Attempt::Failed(error) => Err(error),
            Attempt::Wait | Attempt::Skipped | Attempt::Interrupted => {
                unreachable!("synchronous commit requires a dropping overflow policy")
            }
        }
    }

    fn attempt(
        &mut self,
        value: &PreparedPayloadView<T>,
        guard: Option<&dyn Fn(&T) -> bool>,
        cancel: Option<&Cancellation>,
    ) -> Attempt {
        let mut state = self
            .hub
            .hub
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        attempt_locked(
            &mut state,
            self.hub.hub.options,
            self.hub.hub.retain_current,
            &mut self.scratch,
            value,
            guard,
            cancel,
        )
    }
    fn notify_and_retire(&mut self) {
        while let Some(owner) = self.scratch.get().notifications.remove_first() {
            owner.changed.notify_all();
            drop(owner);
        }
        while let Some(value) = self.scratch.get().discarded.remove_first() {
            drop(value);
        }
    }
}

#[allow(clippy::expect_used, clippy::unreachable)]
fn attempt_locked<T: Send + Sync + 'static>(
    state: &mut HubState<T>,
    options: PubSubOptions,
    retain_current: bool,
    scratch: &mut PublishOwner<T>,
    value: &PreparedPayloadView<T>,
    guard: Option<&dyn Fn(&T) -> bool>,
    cancel: Option<&Cancellation>,
) -> Attempt {
    if cancel.is_some_and(Cancellation::requested) {
        return Attempt::Interrupted;
    }
    if state.closed {
        return Attempt::Failed(PubSubError::Shutdown);
    }
    let limit = options.capacity.get();
    let full = state
        .subscribers
        .as_slice()
        .iter()
        .filter(|entry| {
            entry
                .subscriber
                .buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
                == limit
        })
        .count();
    if full != 0 && matches!(options.overflow, crate::PubSubOverflow::Backpressure) {
        return Attempt::Wait;
    }
    if state.subscribers.as_slice().iter().any(|entry| {
        entry.dropped == usize::MAX
            && entry
                .subscriber
                .buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
                == limit
    }) {
        return Attempt::Failed(PubSubError::LagCountOverflow);
    }
    // Acquire each actual notification owner before the trusted commit hook.
    for entry in state.subscribers.as_slice() {
        scratch
            .get()
            .notifications
            .push_reserved(Arc::clone(&entry.subscriber))
    }
    if guard.is_some_and(|guard| !guard(value)) {
        return Attempt::Skipped;
    }
    let mut delivered = 0usize;
    for entry in state.subscribers.as_mut_slice() {
        let mut buffer = entry
            .subscriber
            .buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if buffer.len() == limit {
            entry.dropped += 1;
            match options.overflow {
                crate::PubSubOverflow::DropNewest => continue,
                crate::PubSubOverflow::DropOldest => {
                    if let Some(old) = buffer.remove_first() {
                        scratch.get().discarded.push_reserved(old);
                    }
                }
                crate::PubSubOverflow::Backpressure => {
                    unreachable!("capacity checked under the hub gate")
                }
            }
        }
        let link = scratch
            .get()
            .links
            .remove_first()
            .expect("preclaimed subscriber link");
        let view = match link.into_view() {
            Ok(view) => view,
            Err(_) => unreachable!("candidate is published before commit"),
        };
        buffer.push_reserved(view);
        delivered += 1;
    }
    if let ReplayPolicy::Last(limit) = options.replay {
        if state.history.len() == limit.get()
            && let Some(old) = state.history.remove_first()
        {
            scratch.get().discarded.push_reserved(old);
        }
        let link = scratch
            .get()
            .links
            .remove_first()
            .expect("preclaimed replay link");
        let view = match link.into_view() {
            Ok(view) => view,
            Err(_) => unreachable!("published candidate"),
        };
        state.history.push_reserved(view);
    }
    if retain_current {
        let link = scratch
            .get()
            .links
            .remove_first()
            .expect("preclaimed current link");
        let view = match link.into_view() {
            Ok(view) => view,
            Err(_) => unreachable!("published candidate"),
        };
        if let Some(old) = state.current.replace(view) {
            scratch.get().discarded.push_reserved(old);
        }
    }
    Attempt::Complete(crate::PublishReport {
        delivered,
        dropped: full,
    })
}

/// A candidate can fail in its policy channel or propagate an existing defect.
pub enum PreparedCandidateFailure {
    Policy(PubSubError),
    Defect(crate::Defect),
}

/// A single admitted publication. Its factory runs on first poll.
pub struct PreparedPublish<
    T: Send + Sync + 'static,
    F: Send + 'static,
    G: Fn(&T) -> bool + Send + 'static = fn(&T) -> bool,
> {
    publication: Option<PreparedPublication<T>>,
    factory: Option<F>,
    apply: fn(F) -> Result<T, PreparedCandidateFailure>,
    value: Option<PreparedPayloadView<T>>,
    cancellation: Cancellation,
    guard: Option<G>,
}
impl<T: Send + Sync + 'static, F: Send + 'static, G: Fn(&T) -> bool + Send + 'static> Unpin
    for PreparedPublish<T, F, G>
{
}
impl<T: Send + Sync + 'static> PreparedPublication<T> {
    /// Attach a standalone cancellation owner before the candidate factory starts.
    #[allow(clippy::result_large_err)]
    pub fn run<F: FnOnce() -> Result<T, PubSubError> + Send + 'static>(
        self,
        token: &crate::PreparedCancellation,
        factory: F,
    ) -> Result<PreparedPublish<T, F>, (MemoryError, Self, F)> {
        let result = (|| Ok::<_, MemoryError>((token.cancelled()?, token.try_clone()?)))();
        match result {
            Ok((wait, token)) => Ok(PreparedPublish {
                publication: Some(self),
                factory: Some(factory),
                apply: |factory: F| factory().map_err(PreparedCandidateFailure::Policy),
                value: None,
                cancellation: Cancellation::Standalone { wait, token },
                guard: None,
            }),
            Err(error) => Err((error, self, factory)),
        }
    }
    /// Attach the already admitted native cancellation context.
    pub fn run_with_context<
        E: Send + 'static,
        F: FnOnce() -> Result<T, PubSubError> + Send + 'static,
    >(
        self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
        factory: F,
    ) -> PreparedPublish<T, F> {
        PreparedPublish {
            publication: Some(self),
            factory: Some(factory),
            apply: |factory: F| factory().map_err(PreparedCandidateFailure::Policy),
            value: None,
            cancellation: Cancellation::Native(context.into_cancellation_control()),
            guard: None,
        }
    }
    /// Attach a context to a candidate that can return an existing defect.
    pub fn run_candidate_with_context<
        E: Send + 'static,
        F: FnOnce() -> Result<T, PreparedCandidateFailure> + Send + 'static,
    >(
        self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
        factory: F,
    ) -> PreparedPublish<T, F> {
        PreparedPublish {
            publication: Some(self),
            factory: Some(factory),
            apply: |factory| factory(),
            value: None,
            cancellation: Cancellation::Native(context.into_cancellation_control()),
            guard: None,
        }
    }
    // The caller is responsible for the existing trusted metadata-only guard contract.
    pub(crate) fn run_guarded_with_context<
        E: Send + 'static,
        F: FnOnce() -> Result<T, PubSubError> + Send + 'static,
        G: Fn(&T) -> bool + Send + 'static,
    >(
        self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
        factory: F,
        guard: G,
    ) -> PreparedPublish<T, F, G> {
        PreparedPublish {
            publication: Some(self),
            factory: Some(factory),
            apply: |factory: F| factory().map_err(PreparedCandidateFailure::Policy),
            value: None,
            cancellation: Cancellation::Native(context.into_cancellation_control()),
            guard: Some(guard),
        }
    }
}
impl<T: Send + Sync + 'static, F: Send + 'static, G: Fn(&T) -> bool + Send + 'static>
    PreparedPublish<T, F, G>
{
    #[allow(clippy::expect_used, clippy::unreachable)]
    fn step(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<crate::Exit<crate::PublishReport, PreparedPubSubError>> {
        if self.cancellation.requested() {
            self.retire();
            return std::task::Poll::Ready(crate::Exit::Interrupted);
        }
        if let Some(factory) = self.factory.take() {
            let value = match (self.apply)(factory) {
                Ok(value) => value,
                Err(PreparedCandidateFailure::Policy(error)) => {
                    self.retire();
                    return std::task::Poll::Ready(crate::Exit::Fail(PreparedPubSubError::Policy(
                        error,
                    )));
                }
                Err(PreparedCandidateFailure::Defect(defect)) => {
                    self.retire();
                    return std::task::Poll::Ready(crate::Exit::Die(defect));
                }
            };
            let writer = self
                .publication
                .as_mut()
                .expect("live publication")
                .writer
                .take()
                .expect("admitted candidate");
            self.value = Some(match writer.publish(value) {
                Ok(view) => view,
                Err(_) => unreachable!("fresh candidate cell"),
            });
        }
        let publication = self.publication.as_mut().expect("live publication");
        publication.listener.reset();
        let _ = publication.listener.poll(cx);
        self.cancellation.register(cx);
        if self.cancellation.requested() {
            self.retire();
            return std::task::Poll::Ready(crate::Exit::Interrupted);
        }
        let attempt = publication.attempt(
            self.value.as_ref().expect("published candidate"),
            self.guard
                .as_ref()
                .map(|guard| guard as &dyn Fn(&T) -> bool),
            Some(&self.cancellation),
        );
        let outcome = match attempt {
            Attempt::Wait => return std::task::Poll::Pending,
            Attempt::Complete(report) => {
                publication.notify_and_retire();
                crate::Exit::Success(report)
            }
            Attempt::Skipped => crate::Exit::Success(crate::PublishReport {
                delivered: 0,
                dropped: 0,
            }),
            Attempt::Interrupted => crate::Exit::Interrupted,
            Attempt::Failed(error) => crate::Exit::Fail(PreparedPubSubError::Policy(error)),
        };
        self.retire();
        std::task::Poll::Ready(outcome)
    }
    fn retire(&mut self) {
        drop(self.value.take());
        drop(self.publication.take());
    }
}
struct PublicationUnwind<
    'a,
    T: Send + Sync + 'static,
    F: Send + 'static,
    G: Fn(&T) -> bool + Send + 'static,
> {
    future: &'a mut PreparedPublish<T, F, G>,
    armed: bool,
}
impl<T: Send + Sync + 'static, F: Send + 'static, G: Fn(&T) -> bool + Send + 'static> Drop
    for PublicationUnwind<'_, T, F, G>
{
    fn drop(&mut self) {
        if self.armed {
            self.future.retire();
        }
    }
}
impl<T: Send + Sync + 'static, F: Send + 'static, G: Fn(&T) -> bool + Send + 'static>
    std::future::Future for PreparedPublish<T, F, G>
{
    type Output = crate::Exit<crate::PublishReport, PreparedPubSubError>;
    #[allow(clippy::expect_used)]
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = &mut *self;
        let diagnostic = this
            .publication
            .as_mut()
            .expect("live publication")
            .diagnostic
            .take()
            .expect("preadmitted diagnostic");
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut guard = PublicationUnwind {
                future: this,
                armed: true,
            };
            let result = guard.future.step(cx);
            guard.armed = false;
            result
        })) {
            Ok(std::task::Poll::Pending) => {
                this.publication
                    .as_mut()
                    .expect("pending publication")
                    .diagnostic = Some(diagnostic);
                std::task::Poll::Pending
            }
            Ok(std::task::Poll::Ready(outcome)) => std::task::Poll::Ready(outcome),
            Err(payload) => std::task::Poll::Ready(crate::Exit::Die(diagnostic.capture(payload))),
        }
    }
}
impl<T: Send + Sync + 'static> PreparedPubSub<T> {
    /// First shutdown policy wins. Shutdown has independent preheld scratch.
    pub fn shutdown_now(&self, mode: crate::PubSubShutdown) -> Result<bool, MemoryError> {
        if self
            .hub
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed
        {
            return Ok(false);
        }
        let reserved = self
            .hub
            .shutdown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(slot) = reserved else {
            if self
                .hub
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .closed
            {
                return Ok(false);
            }
            return Err(MemoryError::Busy);
        };
        let mut scratch = PublishOwner { slot: Some(slot) };
        let mut state = self
            .hub
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Ok(false);
        }
        for entry in state.subscribers.as_slice() {
            scratch
                .get()
                .notifications
                .push_reserved(Arc::clone(&entry.subscriber));
        }
        state.closed = true;
        while let Some(value) = state.history.remove_first() {
            scratch.get().discarded.push_reserved(value);
        }
        if let Some(value) = state.current.take() {
            scratch.get().discarded.push_reserved(value);
        }
        if matches!(mode, crate::PubSubShutdown::Discard) {
            for entry in state.subscribers.as_slice() {
                let mut buffer = entry
                    .subscriber
                    .buffer
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                while let Some(value) = buffer.remove_first() {
                    scratch.get().discarded.push_reserved(value);
                }
            }
        }
        drop(state);
        self.hub.space.notify_all();
        while let Some(owner) = scratch.get().notifications.remove_first() {
            owner.changed.notify_all();
            drop(owner);
        }
        while let Some(value) = scratch.get().discarded.remove_first() {
            drop(value);
        }
        Ok(true)
    }
}

/// Synchronous callback facade with caller-selected contention behavior.
pub struct PreparedCallbackPubSub<T: Send + Sync + 'static> {
    hub: PreparedPubSub<T>,
    contention: crate::CallbackContention,
}
impl<T: Send + Sync + 'static> PreparedCallbackPubSub<T> {
    /// Fallibly retain the existing prepared hub without allocating a new control.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            hub: self.hub.try_clone()?,
            contention: self.contention,
        })
    }
    /// Prepare the same finite hub with synchronous overflow policies.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        options: crate::CallbackPubSubOptions,
        capacity: PreparedPubSubCapacity,
        panics: &crate::PreparedPanicPool,
    ) -> Result<Self, PreparedPubSubError> {
        let hub = PreparedPubSub::prepare(
            memory,
            region,
            PubSubOptions {
                capacity: options.capacity,
                overflow: match options.overflow {
                    crate::CallbackOverflow::DropNewest => crate::PubSubOverflow::DropNewest,
                    crate::CallbackOverflow::DropOldest => crate::PubSubOverflow::DropOldest,
                },
                replay: options.replay,
            },
            capacity,
            false,
            panics,
        )?;
        Ok(Self {
            hub,
            contention: options.contention,
        })
    }
    /// Access the underlying subscription and shutdown methods.
    #[must_use]
    pub fn hub(&self) -> &PreparedPubSub<T> {
        &self.hub
    }
    /// Admit resources before invoking the candidate factory; this path does not catch panics.
    #[allow(clippy::result_large_err, clippy::unreachable)]
    pub fn try_publish_with<F: FnOnce() -> T>(
        &self,
        factory: F,
    ) -> Result<Result<crate::CallbackPublish, PubSubError>, (MemoryError, F)> {
        let publication = match self.hub.admit_publication_mode(false) {
            Ok(value) => value,
            Err(error) => return Err((error, factory)),
        };
        Ok(self.publish_admitted(publication, factory()))
    }
    /// Physical refusal returns the original value without invoking publication.
    #[allow(clippy::result_large_err)]
    pub fn try_publish(
        &self,
        value: T,
    ) -> Result<Result<crate::CallbackPublish, PubSubError>, (MemoryError, T)> {
        let publication = match self.hub.admit_publication_mode(false) {
            Ok(publication) => publication,
            Err(error) => return Err((error, value)),
        };
        Ok(self.publish_admitted(publication, value))
    }
    #[allow(clippy::unreachable)]
    fn publish_admitted(
        &self,
        mut publication: PreparedPublication<T>,
        value: T,
    ) -> Result<crate::CallbackPublish, PubSubError> {
        let Some(writer) = publication.writer.take() else {
            unreachable!("admitted candidate")
        };
        let value = match writer.publish(value) {
            Ok(value) => value,
            Err(_) => unreachable!("fresh candidate"),
        };
        let mut state = match self.hub.hub.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return match self.contention {
                    crate::CallbackContention::DropNewest => Ok(crate::CallbackPublish::Dropped(
                        crate::CallbackDrop::Contended,
                    )),
                    crate::CallbackContention::Reject => Err(PubSubError::Contended),
                };
            }
        };
        let attempt = attempt_locked(
            &mut state,
            self.hub.hub.options,
            false,
            &mut publication.scratch,
            &value,
            None,
            None,
        );
        drop(state);
        match attempt {
            Attempt::Complete(report) => {
                publication.notify_and_retire();
                if report.delivered == 0 && report.dropped > 0 {
                    Ok(crate::CallbackPublish::Dropped(crate::CallbackDrop::Full(
                        report,
                    )))
                } else {
                    Ok(crate::CallbackPublish::Accepted(report))
                }
            }
            Attempt::Failed(PubSubError::Shutdown) => Ok(crate::CallbackPublish::Closed),
            Attempt::Failed(error) => Err(error),
            Attempt::Wait | Attempt::Interrupted | Attempt::Skipped => {
                unreachable!("callback policy has no backpressure, cancellation, or guard")
            }
        }
    }
}
