//! Startup-allocated retention owned by the actual connection and child futures.
//! Rust future destruction is the boundary tracked here, not external settlement.
use crate::Reply;
use effect::{
    LeasedRunResult, PreparedPayloadLink, PreparedPayloadPool, PreparedPayloadReservation, PreparedPayloadView,
    memory::{Admission, AdmissionBundle, CapacityUnit, FixedVector, FramePool, FrameReservation, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool},
};
use std::{
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

type Guard<E, R> = LeasedRunResult<(), Reply<E, R>, E>;
type RequestSlot<E, R> = SharedSlot<'static, Mutex<Option<Guard<E, R>>>>;
struct WakeOnDrop(SharedSlot<'static, futures_util::task::AtomicWaker>);
impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        self.0.wake();
    }
}
// Slot field drops before WakeOnDrop, including on unwind.
struct Group<E: Send + 'static, R: Send + Sync + 'static> {
    slot: Option<SharedSlot<'static, ConnectionRecord<E, R>>>,
    wake: WakeOnDrop,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Group<E, R> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        let slot = self.slot.as_ref().ok_or(MemoryError::ConstructionUnbound)?.try_clone()?;
        let wake = self.wake.0.try_clone()?;
        Ok(Self { slot: Some(slot), wake: WakeOnDrop(wake) })
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for Group<E, R> {
    fn drop(&mut self) {
        if let Some(mut slot) = self.slot.take().and_then(SharedSlot::into_last) {
            let quota = slot.quota.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            drop(slot);
            drop(quota);
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Deref for Group<E, R> {
    type Target = ConnectionRecord<E, R>;
    #[expect(clippy::unreachable, reason = "a live group retains its slot until Drop")]
    fn deref(&self) -> &Self::Target {
        let Some(slot) = &self.slot else { unreachable!("live connection group"); };
        slot
    }
}
#[derive(Clone, Copy)]
pub(crate) enum ConnectionRefusal { Memory(MemoryError), Provider(effect::prepared_provider::ProviderRefusal) }
struct ConnectionState<E: Send + 'static, R: Send + Sync + 'static> {
    closed: bool,
    draining: bool,
    active: usize,
    pending: usize,
    refusal: Option<ConnectionRefusal>,
    accepted_at: Option<tokio::time::Instant>,
    last_activity: Option<tokio::time::Instant>,
    first_head: Option<tokio::time::Instant>,
    records: FixedVector<'static, RequestSlot<E, R>>,
}
struct ConnectionRecord<E: Send + 'static, R: Send + Sync + 'static> {
    quota: Mutex<Option<AdmissionBundle<'static, 1>>>,
    state: Mutex<ConnectionState<E, R>>,
    requests: SlotPool<'static, Mutex<Option<Guard<E, R>>>>,
    children: SlotPool<'static, ()>,
    refusal_notify: tokio::sync::Notify,
    retirement_notify: tokio::sync::Notify,
    #[cfg_attr(not(feature = "http2"), allow(dead_code))]
    http2_max_streams: u32,
}
pub(crate) struct ConnectionRetentionPool<E: Send + 'static, R: Send + Sync + 'static> {
    connections: SlotPool<'static, ConnectionRecord<E, R>>,
    available: SharedSlot<'static, futures_util::task::AtomicWaker>,
    child_guards: PreparedPayloadPool<NativeGuard<E, R>>,
    admission: Option<Admission<'static, 1>>,
}
/// Unique pre-admission of both references to one actual child guard.
pub(crate) struct PreparedChildReservation<E: Send + 'static, R: Send + Sync + 'static> {
    payload: PreparedPayloadReservation<NativeGuard<E, R>>,
    link: PreparedPayloadLink<NativeGuard<E, R>>,
    post: FrameReservation<'static, PreparedPayloadView<NativeGuard<E, R>>>,
}
/// Footer retained through the concrete future and its prepared descendants.
pub(crate) struct PreparedChildFooter<E: Send + 'static, R: Send + Sync + 'static>(PreparedPayloadView<NativeGuard<E, R>>);
pub(crate) struct PreparedChildOwners<E: Send + 'static, R: Send + Sync + 'static> {
    pub(crate) footer: PreparedChildFooter<E, R>,
    pub(crate) post: effect::prepared_provider::PreparedExecutionLease,
}
/// This owner is held only by an actual connection or child future wrapper.
struct NativeGuard<E: Send + 'static, R: Send + Sync + 'static> {
    group: Group<E, R>,
    connection: bool,
    _child: Option<SlotLease<'static, ()>>,
}
/// F is declared first so its destructor completes before the native guard retires.
pub(crate) struct TrackedNative<F, E: Send + 'static, R: Send + Sync + 'static> {
    future: F,
    guard: NativeGuard<E, R>,
}
pub(crate) struct ConnectionHandle<E: Send + 'static, R: Send + Sync + 'static> {
    group: Group<E, R>,
}
pub(crate) struct ConnectionTicket<E: Send + 'static, R: Send + Sync + 'static> {
    guard: NativeGuard<E, R>,
    handle: ConnectionHandle<E, R>,
}
pub(crate) struct RequestRetention<E: Send + 'static, R: Send + Sync + 'static> {
    // Return the request slot before the final group release can wake reuse.
    record: Option<RequestSlot<E, R>>,
    group: Group<E, R>,
    pending: bool,
}
impl<E: Send + 'static, R: Send + Sync + 'static> ConnectionRetentionPool<E, R> {
    pub(crate) fn storage_bytes(
        connections: usize,
        requests: usize,
        children: usize,
    ) -> Result<effect::memory::Bytes, MemoryError> {
        use effect::memory::Bytes;
        let request_list = std::alloc::Layout::array::<RequestSlot<E, R>>(requests)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let per_connection = SlotPool::<Mutex<Option<Guard<E, R>>>>::storage_bytes(requests)?
            .checked_add(SlotPool::<()>::storage_bytes(children)?)?
            .checked_add(Bytes::new(request_list.size()))?;
        SlotPool::<ConnectionRecord<E, R>>::storage_bytes(connections)?
            .checked_add(per_connection.checked_mul(connections)?)?
            .checked_add(Self::child_guard_storage_bytes(connections, children)?)?
            .checked_add(SlotPool::<futures_util::task::AtomicWaker>::storage_bytes(1)?)
    }

    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        connections: usize,
        requests: usize,
        children: usize,
    ) -> Result<Self, MemoryError> {
        // The native H2 SETTINGS value has a narrower representation than a
        // physical pool count. Reject rather than truncate before allocation.
        let http2_max_streams =
            u32::try_from(requests).map_err(|_| MemoryError::ArithmeticOverflow)?;
        let guard_capacity = connections.checked_mul(children).ok_or(MemoryError::ArithmeticOverflow)?;
        let child_guards = PreparedPayloadPool::prepare(memory, region, guard_capacity)?;
        let connections = memory.slots(region, connections, |_| {
            Ok(ConnectionRecord {
                quota: Mutex::new(None),
                state: Mutex::new(ConnectionState {
                    closed: true,
                    draining: false,
                    active: 0,
                    pending: 0,
                    refusal: None,
                    accepted_at: None,
                    last_activity: None,
                    first_head: None,
                    records: FixedVector::prepare(memory.allocator(region)?, requests)?,
                }),
                requests: memory.slots(region, requests, |_| Ok(Mutex::new(None)))?,
                children: memory.slots(region, children, |_| Ok(()))?,
                refusal_notify: tokio::sync::Notify::new(),
                retirement_notify: tokio::sync::Notify::new(),
                http2_max_streams,
            })
        })?;
        Ok(Self {
            connections,
            child_guards,
            admission: None,
            available: memory
                .slots(region, 1, |_| Ok(futures_util::task::AtomicWaker::new()))?
                .try_checkout()?
                .share(),
        })
    }
    pub(crate) fn install_admission(&mut self, admission: Admission<'static, 1>) -> Result<(), MemoryError> {
        if admission.capacities()[0].unit != CapacityUnit::Slots { return Err(MemoryError::UnsupportedRunContract); }
        self.admission = Some(admission);
        Ok(())
    }
    pub(crate) fn admit_quota(&self, ticket: &ConnectionTicket<E, R>) -> Result<(), MemoryError> {
        if let Some(admission) = &self.admission {
            let quota = admission.reserve([1], [0])?;
            *ticket.handle.group.quota.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(quota);
        }
        Ok(())
    }
    pub(crate) fn child_post_frame_layout() -> std::alloc::Layout {
        std::alloc::Layout::new::<PreparedPayloadView<NativeGuard<E, R>>>()
    }
    pub(crate) fn child_guard_storage_bytes(connections: usize, children: usize) -> Result<effect::memory::Bytes, MemoryError> {
        let count = connections.checked_mul(children).ok_or(MemoryError::ArithmeticOverflow)?;
        PreparedPayloadPool::<NativeGuard<E, R>>::storage_bytes(count)
    }
    /// Both actual payload owners and the post-retirement frame are admitted
    /// before constructing a NativeGuard or initializing its future.
    pub(crate) fn reserve_child_owners(&self, frames: &FramePool<'static>) -> Result<PreparedChildReservation<E, R>, MemoryError> {
        let payload = self.child_guards.reserve()?;
        let link = payload.try_link()?;
        let post = frames.reserve::<PreparedPayloadView<NativeGuard<E, R>>>()?;
        Ok(PreparedChildReservation { payload, link, post })
    }
    pub(crate) async fn reserve_when_available(
        &self,
    ) -> Result<ConnectionTicket<E, R>, MemoryError> {
        std::future::poll_fn(|cx| {
            self.available.register(cx.waker());
            match self.reserve_connection() {
                Err(MemoryError::NoSlot) => Poll::Pending,
                result => Poll::Ready(result),
            }
        })
        .await
    }
    /// Reserve the connection record before constructing its future.
    pub(crate) fn reserve_connection(&self) -> Result<ConnectionTicket<E, R>, MemoryError> {
        let group = Group {
            slot: Some(self.connections.checkout()?.share()),
            wake: WakeOnDrop(self.available.try_clone()?),
        };
        let handle = ConnectionHandle {
            group: group.try_clone()?,
        };
        {
            let mut state = group
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !state.records.is_empty()
                || state.active != 0
                || state.pending != 0
                || state.draining
            {
                return Err(MemoryError::Busy);
            }
            state.closed = false;
            state.refusal = None;
            state.accepted_at = None;
            state.last_activity = None;
            state.first_head = None;
            state.active = 1;
        }
        Ok(ConnectionTicket {
            guard: NativeGuard {
                group,
                connection: true,
                _child: None,
            },
            handle,
        })
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> PreparedChildReservation<E, R> {
    pub(crate) fn publish(self, connection: &ConnectionHandle<E, R>) -> Result<PreparedChildOwners<E, R>, MemoryError> {
        let Self { payload, link, post } = self;
        let guard = connection.reserve_child_guard()?;
        let first = payload.publish(guard).map_err(|(payload, guard)| { drop(guard); drop(payload); MemoryError::ConstructionUnbound })?;
        let second = link.into_view().map_err(|link| { drop(link); MemoryError::ConstructionUnbound })?;
        Ok(PreparedChildOwners { footer: PreparedChildFooter(first), post: effect::prepared_provider::PreparedExecutionLease::new(post, second) })
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> ConnectionTicket<E, R> {
    pub(crate) fn accepted(&self, now: tokio::time::Instant) {
        let mut state = self.handle.group.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.accepted_at = Some(now);
        state.last_activity = Some(now);
    }
    pub(crate) fn try_handle(&self) -> Result<ConnectionHandle<E, R>, MemoryError> {
        self.handle.try_clone()
    }
    /// The factory runs only after the physical connection reservation.
    pub(crate) fn start<F: Future>(
        self,
        factory: impl FnOnce(&ConnectionHandle<E, R>) -> F,
    ) -> (TrackedNative<F, E, R>, ConnectionHandle<E, R>) {
        let future = factory(&self.handle);
        (
            TrackedNative {
                future,
                guard: self.guard,
            },
            self.handle,
        )
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> ConnectionHandle<E, R> {
    pub(crate) fn same_connection(&self, other: &Self) -> bool { match (&self.group.slot, &other.group.slot) { (Some(left), Some(right)) => left.same_slot(right), _ => false } }
    pub(crate) fn activity(&self, idle: Option<std::time::Duration>) {
        let now = tokio::time::Instant::now();
        let mut state = self.group.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Once idle has expired, later I/O cannot restart that deadline.
        if let (Some(last), Some(timeout)) = (state.last_activity, idle)
            && last.checked_add(timeout).is_none_or(|deadline| now >= deadline) { return; }
        state.last_activity = Some(now);
    }
    pub(crate) fn first_head(&self, first: Option<std::time::Duration>, idle: Option<std::time::Duration>) -> bool {
        let now = tokio::time::Instant::now();
        let mut state = self.group.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let observed = *state.first_head.get_or_insert(now);
        let first_expired = match (state.accepted_at, first) { (Some(start), Some(timeout)) => start.checked_add(timeout).is_none_or(|deadline| observed >= deadline), (None, Some(_)) => true, _ => false };
        let idle_expired = match (state.last_activity, idle) { (Some(last), Some(timeout)) => last.checked_add(timeout).is_none_or(|deadline| now >= deadline), (None, Some(_)) => true, _ => false };
        first_expired || idle_expired
    }
    pub(crate) fn progress(&self) -> Option<(tokio::time::Instant, tokio::time::Instant, Option<tokio::time::Instant>)> {
        let state = self.group.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Some((state.accepted_at?, state.last_activity?, state.first_head))
    }
    #[cfg(feature = "http2")]
    pub(crate) fn http2_max_streams(&self) -> u32 {
        self.group.http2_max_streams
    }
    pub(crate) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            group: self.group.try_clone()?,
        })
    }
    /// Clone for Hyper only while open; record any true pre-close refusal atomically.
    fn try_tracked_clone(&self) -> Result<Self, MemoryError> {
        let mut state = self
            .group
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(MemoryError::Closed);
        }
        match self.group.try_clone() {
            Ok(group) => Ok(Self { group }),
            Err(error) => {
                state.closed = true;
                if state.refusal.is_none() {
                    state.refusal = Some(ConnectionRefusal::Memory(error));
                }
                drop(state);
                self.group.refusal_notify.notify_one();
                Err(error)
            }
        }
    }
    pub(crate) fn refuse(&self, error: ConnectionRefusal) {
        let mut state = self.group.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed { return; }
        state.closed = true;
        state.refusal.get_or_insert(error);
        drop(state);
        self.group.refusal_notify.notify_one();
    }
    pub(crate) fn tasks_changed(&self) -> impl Future<Output = ()> + '_ { self.group.retirement_notify.notified() }
    pub(crate) fn signal_tasks(&self) { self.group.retirement_notify.notify_one(); }
    pub(crate) fn is_closed(&self) -> bool {
        self.group.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).closed
    }
    pub(crate) fn close(&self) {
        self.group
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed = true;
    }
    pub(crate) fn refusal(&self) -> Option<ConnectionRefusal> {
        self.group
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .refusal
    }
    pub(crate) fn children_are_retired(&self) -> bool {
        let state = self.group.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active <= 1 && state.pending == 0
    }
    pub(crate) async fn children_retired(&self) {
        loop {
            let notified = self.group.retirement_notify.notified();
            let done = {
                let state = self
                    .group
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.active <= 1 && state.pending == 0
            };
            if done {
                return;
            }
            notified.await;
        }
    }
    pub(crate) async fn refused(&self) -> ConnectionRefusal {
        loop {
            let notified = self.group.refusal_notify.notified();
            if let Some(error) = self
                .group
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .refusal
            {
                return error;
            }
            notified.await;
        }
    }
    /// Reserve physical request record and list position before endpoint admission.
    pub(crate) fn reserve_request(&self) -> Result<RequestRetention<E, R>, MemoryError> {
        let group = &self.group;
        let mut state = crate::try_admission_lock(&group.state)?;
        if state.closed {
            return Err(MemoryError::Closed);
        }
        let pending = state
            .pending
            .checked_add(1)
            .ok_or(MemoryError::OwnerLimit)?;
        let owner = group.try_clone()?;
        let record = match group.requests.try_checkout() {
            Ok(record) => record.share(),
            Err(error) => {
                drop(state);
                drop(owner);
                return Err(error);
            }
        };
        let retained = match record.try_clone() {
            Ok(retained) => retained,
            Err(error) => {
                drop(state);
                drop(record);
                drop(owner);
                return Err(error);
            }
        };
        if let Err(retained) = state.records.push(retained) {
            drop(state);
            drop(retained);
            drop(record);
            drop(owner);
            return Err(MemoryError::NoSlot);
        }
        state.pending = pending;
        Ok(RequestRetention {
            group: owner,
            record: Some(record),
            pending: true,
        })
    }
    /// Reserve the actual child guard before native submission.
    fn reserve_child_guard(&self) -> Result<NativeGuard<E, R>, MemoryError> {
        let group = &self.group;
        let mut state = group.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed { return Err(MemoryError::Closed); }
        let Some(active) = state.active.checked_add(1) else {
            let error = MemoryError::OwnerLimit;
            state.closed = true; state.refusal.get_or_insert(ConnectionRefusal::Memory(error)); drop(state);
            group.refusal_notify.notify_one(); return Err(error);
        };
        let child = match group.children.try_checkout() {
            Ok(child) => child,
            Err(error) => { state.closed = true; state.refusal.get_or_insert(ConnectionRefusal::Memory(error)); drop(state); group.refusal_notify.notify_one(); return Err(error); }
        };
        let owner = match group.try_clone() {
            Ok(owner) => owner,
            Err(error) => { state.closed = true; state.refusal.get_or_insert(ConnectionRefusal::Memory(error)); drop(state); drop(child); group.refusal_notify.notify_one(); return Err(error); }
        };
        state.active = active;
        Ok(NativeGuard { group: owner, connection: false, _child: Some(child) })
    }
    /// Ordinary executor wrapper retains its native child guard until retirement.
    pub(crate) fn wrap_child<C: Future>(&self, future: C) -> Result<TrackedNative<C, E, R>, (MemoryError, C)> {
        match self.reserve_child_guard() { Ok(guard) => Ok(TrackedNative { future, guard }), Err(error) => Err((error, future)) }
    }

}
impl<F: Future, E: Send + 'static, R: Send + Sync + 'static> Future for TrackedNative<F, E, R> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: future is structurally pinned and never moved until wrapper drop.
        unsafe { self.map_unchecked_mut(|this| &mut this.future) }.poll(cx)
    }
}
struct Drain<'a, E: Send + 'static, R: Send + Sync + 'static> {
    group: &'a Group<E, R>,
    active: bool,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drain<'_, E, R> {
    fn run(&mut self) {
        loop {
            // Advance the list before user destruction, so Drop continues it on unwind.
            let record = self
                .group
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .records
                .pop();
            let Some(record) = record else {
                break;
            };
            let guard = record
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(guard);
            drop(record);
        }
        self.group
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .draining = false;
        self.active = false;
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for Drain<'_, E, R> {
    fn drop(&mut self) {
        if self.active {
            self.run();
        }
    }
}
fn drain_if_retired<E: Send + 'static, R: Send + Sync + 'static>(group: &Group<E, R>) {
    {
        let mut state = group
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.closed || state.active != 0 || state.pending != 0 || state.draining {
            return;
        }
        state.draining = true;
    }
    Drain {
        group,
        active: true,
    }
    .run();
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for NativeGuard<E, R> {
    fn drop(&mut self) {
        drop(self._child.take());
        {
            let mut state = self
                .group
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.connection {
                state.closed = true;
            }
            state.active -= 1;
        }
        self.group.retirement_notify.notify_one();
        drain_if_retired(&self.group);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> RequestRetention<E, R> {
    /// Retire the duplicate root and return its pre-admission record after the
    /// execution, body and terminal owners have been installed independently.
    pub(crate) fn handoff_duplicate(self, guard: Guard<E, R>) {
        drop(guard);
        drop(self);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for RequestRetention<E, R> {
    fn drop(&mut self) {
        if self.pending {
            let Some(record) = self.record.take() else { return };
            let removed = {
                let mut state = self
                    .group
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let index = state
                    .records
                    .as_slice()
                    .iter()
                    .position(|retained| retained.same_slot(&record));
                if let Some(index) = index {
                    let last = state.records.len() - 1;
                    state.records.as_mut_slice().swap(index, last);
                    state.records.pop()
                } else {
                    None
                }
            };
            drop(removed);
            drop(record);
            // Keep retirement conservative until both physical slot owners are gone.
            self.group
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pending -= 1;
            self.group.retirement_notify.notify_one();
            drain_if_retired(&self.group);
        }
    }
}

/// Hyper requires infallible Clone. Saturation returns a permanently refused
/// adapter after publishing the actual error on the original group.
pub(crate) struct TrackedExecutor<E: Send + 'static, R: Send + Sync + 'static> {
    handle: Option<ConnectionHandle<E, R>>,
    reactor: tokio::runtime::Handle,
    tasks: Option<Arc<dyn crate::prepared_native_task::NativeTaskDispatch<E, R>>>,
    retention: Arc<ConnectionRetentionPool<E, R>>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> TrackedExecutor<E, R> {
    pub(crate) fn first_head(&self, first: Option<std::time::Duration>, idle: Option<std::time::Duration>) -> bool { self.handle.as_ref().is_some_and(|handle| handle.first_head(first, idle)) }
    pub(crate) fn try_handle(&self) -> Result<ConnectionHandle<E, R>, MemoryError> {
        let Some(handle) = &self.handle else {
            return Err(MemoryError::Closed);
        };
        handle.try_tracked_clone()
    }
    pub(crate) fn new(handle: ConnectionHandle<E, R>, reactor: tokio::runtime::Handle, tasks: Option<Arc<dyn crate::prepared_native_task::NativeTaskDispatch<E, R>>>, retention: Arc<ConnectionRetentionPool<E, R>>) -> Self {
        Self { handle: Some(handle), reactor, tasks, retention }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Clone for TrackedExecutor<E, R> {
    fn clone(&self) -> Self {
        let handle = self
            .handle
            .as_ref()
            .and_then(|handle| handle.try_tracked_clone().ok());
        Self {
            handle,
            reactor: self.reactor.clone(),
            tasks: self.tasks.clone(),
            retention: self.retention.clone(),
        }
    }
}
impl<F, E, R> hyper::rt::Executor<F> for TrackedExecutor<E, R>
where
    F: Future<Output = ()> + Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn execute(&self, future: F) {
        let Some(handle) = &self.handle else {
            let _entered = self.reactor.enter();
            drop(future);
            return;
        };
        if let Some(tasks) = &self.tasks {
            let reservation = match effect::memory::PreparedScopedUnitFutureReservation::<F, crate::prepared_native_task::TokioNativeContext, PreparedChildFooter<E, R>>::reserve(tasks.frames()) {
                Ok(value) => value,
                Err(error) => { let _entered = self.reactor.enter(); drop(future); handle.refuse(ConnectionRefusal::Memory(error)); return; }
            };
            let child = match self.retention.reserve_child_owners(tasks.frames()) {
                Ok(value) => value,
                Err(error) => { let _entered = self.reactor.enter(); drop(future); handle.refuse(ConnectionRefusal::Memory(error)); return; }
            };
            let owner = match handle.try_clone() {
                Ok(value) => value,
                Err(error) => { let _entered = self.reactor.enter(); drop(future); handle.refuse(ConnectionRefusal::Memory(error)); return; }
            };
            let owners = match child.publish(handle) {
                Ok(value) => value,
                Err(error) => { let _entered = self.reactor.enter(); drop(future); handle.refuse(ConnectionRefusal::Memory(error)); return; }
            };
            let future = reservation.initialize(future, crate::prepared_native_task::TokioNativeContext(self.reactor.clone()), owners.footer);
            if let Err(error) = tasks.submit(owner, future, owners.post, self.reactor.clone()) { handle.refuse(ConnectionRefusal::Provider(error)); }
            return;
        }
        match handle.wrap_child(future) {
            Ok(child) => {
                drop(self.reactor.spawn(child));
            }
            Err((_error, future)) => {
                // Closed rejects late submissions without replacing the original outcome.
                drop(future);
            }
        }
    }
}
