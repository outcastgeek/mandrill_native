//! Finite terminal-origin ownership for prepared host outcomes.
use crate::Reply;
use effect::{
    LeasedRunResult,
    memory::{
        Admission, AdmissionBundle, CapacityUnit, FixedVector, FramePool, MemoryError, PreparedMemory, RegionId, RetainedValue,
        RetainedValueReservation, SharedSlot, SlotPool,
    },
};
use std::{
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

enum Origin<E: Send + 'static, R: Send + Sync + 'static> {
    Endpoint(LeasedRunResult<(), Reply<E, R>, E>),
    Connection(LeasedRunResult<(), (), E>),
}
struct EntryPayload<E: Send + 'static, R: Send + Sync + 'static> {
    origin: Mutex<Option<Origin<E, R>>>,
    quota: Mutex<Option<AdmissionBundle<'static, 1>>>,
}
struct Entry<E: Send + 'static, R: Send + Sync + 'static>(
    Option<SharedSlot<'static, EntryPayload<E, R>>>,
);
impl<E: Send + 'static, R: Send + Sync + 'static> Entry<E, R> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self(Some(self.0.as_ref().ok_or(MemoryError::ConstructionUnbound)?.try_clone()?)))
    }
    fn same_slot(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => a.same_slot(b),
            _ => false,
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Deref for Entry<E, R> {
    type Target = EntryPayload<E, R>;
    #[expect(clippy::unreachable, reason = "a live terminal entry retains its slot until Drop")]
    fn deref(&self) -> &Self::Target {
        let Some(slot) = &self.0 else { unreachable!("live terminal entry"); };
        slot
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for Entry<E, R> {
    fn drop(&mut self) {
        // Quota outlives the unique slot and any remaining origin on unwind.
        let quota;
        let Some(mut slot) = self.0.take().and_then(SharedSlot::into_last) else {
            return;
        };
        quota = slot.quota.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        let origin = slot.origin.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        drop(origin);
        drop(slot);
        drop(quota);
    }
}
type Group<E, R> = SharedSlot<'static, State<E, R>>;
// Field order is deliberate: cancellation destroys the actual in-flight host
// future and any outcomes it owns before releasing terminal-origin authority.
struct OrderedHost<F, O> {
    future: Option<F>,
    owner: Option<O>,
}
impl<F: Future, O> Future for OrderedHost<F, O> {
    type Output = (F::Output, O);
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: `future` is never moved after this wrapper is pinned; `owner`
        // is not structurally pinned and may be taken only after readiness.
        let this = unsafe { self.get_unchecked_mut() };
        if this.owner.is_none() {
            return Poll::Pending;
        }
        // SAFETY: the field remains at the same address for the pinned lifetime.
        let mut future = unsafe { Pin::new_unchecked(&mut this.future) };
        let Some(pinned) = future.as_mut().as_pin_mut() else {
            return Poll::Pending;
        };
        match pinned.poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(output) => {
                // Destroy the completed pinned future in place before transferring
                // the owner alongside its still-guarded outcome.
                future.set(None);
                match this.owner.take() {
                    Some(owner) => Poll::Ready((output, owner)),
                    None => Poll::Pending,
                }
            }
        }
    }
}
struct State<E: Send + 'static, R: Send + Sync + 'static> {
    records: SlotPool<'static, EntryPayload<E, R>>,
    inner: Mutex<Inner<E, R>>,
}
struct Inner<E: Send + 'static, R: Send + Sync + 'static> {
    entries: FixedVector<'static, Entry<E, R>>,
    pending: usize,
    sealed: bool,
    released: bool,
    draining: bool,
}
/// All record/list storage is prepared in the selected region before serving.
pub(crate) struct TerminalPool<E: Send + 'static, R: Send + Sync + 'static> {
    hosts: SlotPool<'static, State<E, R>>,
    admission: Option<Admission<'static, 1>>,
}
pub(crate) struct TerminalOwner<E: Send + 'static, R: Send + Sync + 'static> {
    origins: TerminalOrigins<E, R>,
}
pub(crate) struct TerminalHandle<E: Send + 'static, R: Send + Sync + 'static> {
    group: Group<E, R>,
    admission: Option<Admission<'static, 1>>,
}
pub(crate) struct TerminalRequest<E: Send + 'static, R: Send + Sync + 'static> {
    // Return the entry before its owning host group can be checked out again.
    entry: Entry<E, R>,
    group: Group<E, R>,
    installed: bool,
    pending: bool,
}
pub(crate) struct InstalledTerminalRequest<E: Send + 'static, R: Send + Sync + 'static> {
    request: TerminalRequest<E, R>,
}
/// Footer authority retained until the host outcome and its prepared descendants retire.
pub(crate) struct TerminalOrigins<E: Send + 'static, R: Send + Sync + 'static> {
    group: Group<E, R>,
}
/// Host outcome with borrow-only access and an origin footer. No raw Exit is detached.
pub struct PreparedHostResult<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    outcome: RetainedValue<effect::Exit<A, E>, TerminalOrigins<E, R>>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Deref
    for PreparedHostResult<A, E, R>
{
    type Target = effect::Exit<A, E>;
    fn deref(&self) -> &Self::Target {
        &self.outcome
    }
}
/// A result frame is reserved before host work, then receives the actual terminal outcome.
pub(crate) struct TerminalResultReservation<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    frame: RetainedValueReservation<effect::Exit<A, E>, TerminalOrigins<E, R>>,
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>
    TerminalResultReservation<A, E, R>
{
    pub(crate) fn layout() -> std::alloc::Layout {
        RetainedValueReservation::<effect::Exit<A, E>, TerminalOrigins<E, R>>::layout()
    }
    pub(crate) fn reserve(frames: &FramePool<'static>) -> Result<Self, MemoryError> {
        Ok(Self {
            frame: RetainedValueReservation::reserve(frames)?,
        })
    }
    pub(crate) fn publish(
        self,
        exit: effect::Exit<A, E>,
        origins: TerminalOrigins<E, R>,
    ) -> PreparedHostResult<A, E, R> {
        PreparedHostResult {
            outcome: self.frame.publish(exit, origins),
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> TerminalPool<E, R> {
    fn storage_bytes(hosts: usize, records: usize) -> Result<effect::memory::Bytes, MemoryError> {
        use effect::memory::Bytes;
        let list = std::alloc::Layout::array::<Entry<E, R>>(records).map_err(|_| MemoryError::InvalidLayout)?;
        let per_host = SlotPool::<EntryPayload<E, R>>::storage_bytes(records)?
            .checked_add(Bytes::new(list.size()))?;
        SlotPool::<State<E, R>>::storage_bytes(hosts)?.checked_add(per_host.checked_mul(hosts)?)
    }

    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        hosts: usize,
        records: usize,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            hosts: memory.slots(region, hosts, |_| {
                Ok(State {
                    records: memory.slots(region, records, |_| Ok(EntryPayload {
                        origin: Mutex::new(None),
                        quota: Mutex::new(None),
                    }))?,
                    inner: Mutex::new(Inner {
                        entries: FixedVector::prepare(memory.allocator(region)?, records)?,
                        pending: 0,
                        sealed: true,
                        released: false,
                        draining: false,
                    }),
                })
            })?,
            admission: None,
        })
    }
    fn install_admission(&mut self, admission: Admission<'static, 1>) -> Result<(), MemoryError> {
        if admission.capacities()[0].unit != CapacityUnit::Slots {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.admission = Some(admission);
        Ok(())
    }
    #[allow(clippy::type_complexity)]
    pub(crate) fn reserve(
        &self,
    ) -> Result<(TerminalOwner<E, R>, TerminalHandle<E, R>), MemoryError> {
        let admission = self.admission.as_ref().map(Admission::try_clone).transpose()?;
        let group = self.hosts.try_checkout()?.share();
        let handle = group.try_clone()?;
        {
            let mut inner = group
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if inner.pending != 0 || !inner.entries.is_empty() || inner.draining {
                return Err(MemoryError::Busy);
            }
            inner.sealed = false;
            inner.released = false;
        }
        Ok((
            TerminalOwner {
                origins: TerminalOrigins { group },
            },
            TerminalHandle { group: handle, admission },
        ))
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> TerminalHandle<E, R> {
    pub(crate) fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            group: self.group.try_clone()?,
            admission: self.admission.as_ref().map(Admission::try_clone).transpose()?,
        })
    }
    /// Reserve the physical record and list position before any request work.
    pub(crate) fn reserve_request(&self) -> Result<TerminalRequest<E, R>, MemoryError> {
        let mut inner = crate::try_admission_lock(&self.group.inner)?;
        if inner.sealed {
            return Err(MemoryError::Closed);
        }
        let pending = inner
            .pending
            .checked_add(1)
            .ok_or(MemoryError::OwnerLimit)?;
        let group = self.group.try_clone()?;
        let entry = match self.group.records.try_checkout() {
            Ok(slot) => Entry(Some(slot.share())),
            Err(error) => {
                drop(inner);
                drop(group);
                return Err(error);
            }
        };
        let retained = match entry.try_clone() {
            Ok(retained) => retained,
            Err(error) => {
                drop(inner);
                drop(entry);
                drop(group);
                return Err(error);
            }
        };
        if let Err(retained) = inner.entries.push(retained) {
            drop(inner);
            drop(retained);
            drop(entry);
            drop(group);
            return Err(MemoryError::NoSlot);
        }
        inner.pending = pending;
        let request = TerminalRequest {
            group,
            entry,
            installed: false,
            pending: true,
        };
        drop(inner);
        if let Some(admission) = &self.admission {
            let quota = admission.reserve([1], [0])?;
            *request.entry.quota.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(quota);
        }
        Ok(request)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> TerminalOwner<E, R> {
    /// Stop admissions and transfer terminal origin authority to the host result.
    pub(crate) fn finish(self) -> TerminalOrigins<E, R> {
        self.origins
            .group
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sealed = true;
        self.origins
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> TerminalRequest<E, R> {
    /// Consume the unique vacant ticket; the private record cannot be installed twice.
    pub(crate) fn install(
        self,
        origin: LeasedRunResult<(), Reply<E, R>, E>,
    ) -> InstalledTerminalRequest<E, R> {
        self.install_origin(Origin::Endpoint(origin))
    }
    pub(crate) fn install_connection(
        self,
        origin: LeasedRunResult<(), (), E>,
    ) -> InstalledTerminalRequest<E, R> {
        self.install_origin(Origin::Connection(origin))
    }
    fn install_origin(mut self, origin: Origin<E, R>) -> InstalledTerminalRequest<E, R> {
        *self
            .entry.origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(origin);
        self.installed = true;
        InstalledTerminalRequest { request: self }
    }
    fn retire(&mut self, release: bool) {
        let removed = {
            let mut inner = self
                .group
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.pending {
                inner.pending -= 1;
                self.pending = false;
            }
            if release || !self.installed {
                let index = inner
                    .entries
                    .as_slice()
                    .iter()
                    .position(|entry| entry.same_slot(&self.entry));
                if let Some(index) = index {
                    let last = inner.entries.len() - 1;
                    inner.entries.as_mut_slice().swap(index, last);
                    inner.entries.pop()
                } else {
                    None
                }
            } else {
                None
            }
        };
        // If releasing this origin unwinds, the last pending request must still
        // resume any terminal drain that an abandoned host already requested.
        let continuation = DrainAfterRetire { group: &self.group };
        if let Some(entry) = removed {
            let origin = entry.origin
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(origin);
            drop(entry);
        }
        drop(continuation);
    }
}
struct DrainAfterRetire<'a, E: Send + 'static, R: Send + Sync + 'static> {
    group: &'a Group<E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for DrainAfterRetire<'_, E, R> {
    fn drop(&mut self) {
        drain_if_released(self.group);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> InstalledTerminalRequest<E, R> {
    pub(crate) fn retain_terminal(mut self) {
        self.request.retire(false);
    }
    /// # Safety
    /// The moved outcome and all its descendants must already have retired or
    /// independently retained their actual origins before this call.
    pub(crate) unsafe fn release_handled(mut self) {
        self.request.retire(true);
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for TerminalRequest<E, R> {
    fn drop(&mut self) {
        if self.pending {
            self.retire(false);
        }
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drop for TerminalOrigins<E, R> {
    fn drop(&mut self) {
        {
            let mut inner = self
                .group
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            inner.sealed = true;
            inner.released = true;
        }
        drain_if_released(&self.group);
    }
}
struct Drain<'a, E: Send + 'static, R: Send + Sync + 'static> {
    group: &'a Group<E, R>,
    active: bool,
}
impl<E: Send + 'static, R: Send + Sync + 'static> Drain<'_, E, R> {
    fn run(&mut self) {
        loop {
            let entry = self
                .group
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entries
                .pop();
            let Some(entry) = entry else {
                break;
            };
            let origin = entry.origin
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(origin);
            drop(entry);
        }
        self.group
            .inner
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
fn drain_if_released<E: Send + 'static, R: Send + Sync + 'static>(group: &Group<E, R>) {
    {
        let mut inner = group
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !inner.released || inner.pending != 0 || inner.draining {
            return;
        }
        inner.draining = true;
    }
    Drain {
        group,
        active: true,
    }
    .run();
}

/// Explicit startup capacities for one host's endpoint and lifecycle records.
/// Hyper parser/storage and Tokio task allocation remain ordinary native allocations.
#[derive(Clone, Copy, Debug)]
pub struct PreparedHostCapacity {
    /// Simultaneously admitted native connections.
    pub connections: usize,
    /// Simultaneously reserved request-admission records per connection. For prepared HTTP/2
    /// hosting this also selects the maximum concurrent peer-initiated streams;
    /// zero permits no new HTTP/2 streams. The admission record is returned after
    /// independent execution, body and terminal origin owners are installed.
    pub requests_per_connection: usize,
    /// Simultaneously tracked native executor children per connection.
    pub children_per_connection: usize,
    /// Concurrent requests plus terminal outcomes retained by the host result.
    pub terminal_origins: usize,
}
/// Explicit per-request native control capacities; no values are selected by default.
#[derive(Clone, Copy, Debug)]
pub struct PreparedRequestControlCapacity {
    /// Simultaneous inner response channels for reentrant dispatch.
    pub inner_responses: usize,
    /// Classification watch listener capacity per request.
    pub classification_receivers: usize,
    /// Classification watch retained version capacity per request.
    pub classification_versions: usize,
    /// Selection watch listener capacity per request.
    pub selection_receivers: usize,
    /// Selection watch retained version capacity per request.
    pub selection_versions: usize,
    /// Total independently owned body completion records for this host.
    /// Front admission reserves the outer body and two host error-render sites;
    /// each inner dispatch reserves its endpoint and recovery body sites.
    pub body_records: usize,
    /// Physical completion waiters per body record. The outer body admits one
    /// host waiter and an additional waiter for each concurrent inner dispatch.
    pub body_listeners: usize,
}
/// Prepared graph and fixed driver backing for selected connection tasks.
pub struct PreparedConnectionTaskDemand {
    /// Graph frame, root, provider and operation demand; provider backing is selected by R.
    pub graph: effect::__private::prepared_graph::Demand,
    /// Selected-region fixed driver vector payload, excluding allocator overhead.
    pub driver_bytes: effect::memory::Bytes,
    /// Startup System Arc controls for the selected plan and selection owner.
    pub startup_arc_controls: usize,
    /// Arc payload sizes only; standard-library headers and allocator overhead are additional.
    pub startup_arc_payload_bytes: effect::memory::Bytes,
}
/// One-shot companion for prepared endpoints and finite lifecycle records.
/// This does not bound Hyper parser storage or Tokio task allocations. Arbitrary
/// detached work hidden in opaque native responses is outside tracked ownership.
pub struct PreparedServer<E: Send + 'static, R: Send + Sync + 'static> {
    server: crate::Server<E, R>,
    terminal: TerminalPool<E, R>,
    result_frames: FramePool<'static>,
    cleanup: Arc<effect::PreparedCleanupConfig<E>>,
    native_tasks: Option<Arc<dyn crate::prepared_native_task::NativeTaskDispatch<E, R>>>,
    connection_tasks: Option<Arc<crate::host::PreparedConnectionSelection<E, R>>>,
    terminal_capacity: usize,
    needs_connection_capacity: bool,
    needs_request_capacity: bool,
}
impl<E, R> PreparedServer<E, R>
where
    E: Send + 'static,
    R: crate::HasTokio + effect::HasCancellation + effect::HasTaskSpawner + Send + Sync + 'static,
{
    /// Exact post-retirement frame layout for each simultaneously admitted child.
    #[must_use]
    pub fn native_child_post_frame_layout() -> std::alloc::Layout {
        crate::prepared_connection_retention::ConnectionRetentionPool::<E, R>::child_post_frame_layout()
    }
    /// Region payload for shared actual native-child guard owners. Footer
    /// frame backing is accounted separately in the caller's selected frame pool.
    pub fn native_child_guard_region_bytes(
        capacity: PreparedHostCapacity,
    ) -> Result<effect::memory::Bytes, MemoryError> {
        crate::prepared_connection_retention::ConnectionRetentionPool::<E, R>::child_guard_storage_bytes(capacity.connections, capacity.children_per_connection)
    }
    /// Region payload for startup native extension loan controls. Allocator
    /// headers and padding are additional; standard Arc controls use System.
    pub fn native_slot_region_bytes(
        capacity: PreparedHostCapacity,
    ) -> Result<effect::memory::Bytes, MemoryError> {
        let count = capacity
            .connections
            .checked_mul(capacity.requests_per_connection)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        crate::prepared_native_slot::NativeSlotRegistry::<E, R>::storage_bytes(count)
    }
    /// Startup System Arc control count for the native slot registry and records.
    pub fn native_slot_startup_arc_controls(
        capacity: PreparedHostCapacity,
    ) -> Result<usize, MemoryError> {
        capacity
            .connections
            .checked_mul(capacity.requests_per_connection)
            .and_then(|count| count.checked_add(1))
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    /// Region payload for outer response and context-gate request controls.
    /// Allocator headers/padding and System Arc controls are additional.
    pub fn request_control_region_bytes(
        capacity: PreparedHostCapacity,
    ) -> Result<effect::memory::Bytes, MemoryError> {
        let count = capacity
            .connections
            .checked_mul(capacity.requests_per_connection)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        crate::prepared_request_control::RequestControlPool::<E, R>::storage_bytes(count)
    }
    /// Region payload including selected inner, middleware run and watch controls.
    pub fn request_control_region_bytes_with(
        capacity: PreparedHostCapacity,
        controls: PreparedRequestControlCapacity,
    ) -> Result<effect::memory::Bytes, MemoryError> {
        let count = capacity
            .connections
            .checked_mul(capacity.requests_per_connection)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        crate::prepared_request_control::RequestControlPool::<E, R>::storage_bytes_with(
            count,
            Some(controls),
        )
    }
    /// Startup System Arc controls including inner responses and two controls
    /// per prepared middleware run cell.
    pub fn request_control_startup_arc_controls_with(
        capacity: PreparedHostCapacity,
        controls: PreparedRequestControlCapacity,
    ) -> Result<usize, MemoryError> {
        capacity
            .connections
            .checked_mul(capacity.requests_per_connection)
            .and_then(|count| {
                controls
                    .inner_responses
                    .checked_mul(3)
                    .and_then(|inner| inner.checked_add(2))
                    .and_then(|per_request| count.checked_mul(per_request))
            })
            .and_then(|count| count.checked_add(1))
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    /// Region payload for the host's independently owned body controls.
    pub fn body_control_region_bytes(
        controls: PreparedRequestControlCapacity,
    ) -> Result<effect::memory::Bytes, MemoryError> {
        crate::prepared_body_completion::BodyPool::storage_bytes(
            controls.body_records,
            controls.body_listeners,
        )
    }
    /// Startup System Arc controls for the body pool and its records.
    pub fn body_control_startup_arc_controls(
        controls: PreparedRequestControlCapacity,
    ) -> Result<usize, MemoryError> {
        controls
            .body_records
            .checked_mul(2)
            .and_then(|count| count.checked_add(1))
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    /// Payload layout of each independent request/body loan gate Arc.
    #[must_use]
    pub fn control_loan_gate_arc_payload_layout() -> std::alloc::Layout {
        std::alloc::Layout::new::<std::sync::Mutex<usize>>()
    }
    /// Per-record Arc payload layout, excluding the Arc header and allocator rounding.
    #[must_use]
    pub fn body_control_arc_payload_layout() -> std::alloc::Layout {
        crate::prepared_body_completion::BodyPool::control_payload_layout()
    }
    /// Host-pool Arc payload layout, excluding the Arc header and allocator rounding.
    #[must_use]
    pub fn body_pool_arc_payload_layout() -> std::alloc::Layout {
        crate::prepared_body_completion::BodyPool::pool_payload_layout()
    }
    /// Region payload for one batch per configured simultaneous request.
    pub fn observer_batch_region_bytes(
        capacity: PreparedHostCapacity,
        server: &crate::Server<E, R>,
    ) -> Result<effect::memory::Bytes, MemoryError> {
        let count = capacity
            .connections
            .checked_mul(capacity.requests_per_connection)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        crate::prepared_observers::ObserverBatchPool::<E, R>::storage_bytes(
            count,
            server.observer_count(),
        )
    }
    /// One startup System Arc is used for the host observer-batch pool.
    #[must_use]
    pub fn observer_batch_startup_arc_controls() -> usize {
        1
    }
    /// Pool Arc payload layout, excluding its Arc header and allocator rounding.
    #[must_use]
    pub fn observer_batch_arc_payload_layout() -> std::alloc::Layout {
        crate::prepared_observers::ObserverBatchPool::<E, R>::arc_payload_layout()
    }
    /// Startup System Arc controls for the request-control registry and records.
    pub fn request_control_startup_arc_controls(
        capacity: PreparedHostCapacity,
    ) -> Result<usize, MemoryError> {
        capacity
            .connections
            .checked_mul(capacity.requests_per_connection)
            .and_then(|count| count.checked_mul(2))
            .and_then(|count| count.checked_add(1))
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    /// Exact result-frame layout for this host's chosen successful value.
    #[must_use]
    pub fn result_frame_layout<A: Send + 'static>() -> std::alloc::Layout {
        TerminalResultReservation::<A, E, R>::layout()
    }
    /// Region payload for the host tables created by `prepare_with_request_controls`.
    /// Additional request storage, body buffers, result/cleanup frames and allocator
    /// metadata have their own demand.
    pub fn host_region_bytes_with_request_controls(
        capacity: PreparedHostCapacity,
        controls: PreparedRequestControlCapacity,
        observers: usize,
    ) -> Result<effect::memory::Bytes, MemoryError> {
        let requests = capacity.connections.checked_mul(capacity.requests_per_connection)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        Self::native_slot_region_bytes(capacity)?
            .checked_add(Self::request_control_region_bytes_with(capacity, controls)?)?
            .checked_add(Self::body_control_region_bytes(controls)?)?
            .checked_add(crate::prepared_observers::ObserverBatchPool::<E, R>::storage_bytes(requests, observers)?)?
            .checked_add(crate::prepared_connection_retention::ConnectionRetentionPool::<E, R>::storage_bytes(
                capacity.connections, capacity.requests_per_connection, capacity.children_per_connection,
            )?)?
            .checked_add(TerminalPool::<E, R>::storage_bytes(1, capacity.terminal_origins)?)
    }
    /// Prepare finite lifecycle and terminal storage before running the host.
    /// `cleanup` supplies physical buffers with caller-selected capacity; request,
    /// connection, serving, and host admission reserve their actual buffers before work.
    /// This compatibility entrypoint retains ordinary inner-response and watch
    /// controls. Use `prepare_with_request_controls` to select their capacities.
    pub fn prepare(
        server: crate::Server<E, R>,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedHostCapacity,
        result_frames: FramePool<'static>,
        cleanup: effect::PreparedCleanupConfig<E>,
    ) -> Result<Self, MemoryError> {
        Self::prepare_inner(
            server,
            memory,
            region,
            capacity,
            result_frames,
            cleanup,
            None,
        )
    }
    /// Prepare request controls with explicit inner-channel and watch capacities.
    pub fn prepare_with_request_controls(
        server: crate::Server<E, R>,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedHostCapacity,
        result_frames: FramePool<'static>,
        cleanup: effect::PreparedCleanupConfig<E>,
        controls: PreparedRequestControlCapacity,
    ) -> Result<Self, MemoryError> {
        Self::prepare_inner(
            server,
            memory,
            region,
            capacity,
            result_frames,
            cleanup,
            Some(controls),
        )
    }
    fn prepare_inner(
        server: crate::Server<E, R>,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedHostCapacity,
        result_frames: FramePool<'static>,
        cleanup: effect::PreparedCleanupConfig<E>,
        controls: Option<PreparedRequestControlCapacity>,
    ) -> Result<Self, MemoryError> {
        let server = server.with_prepared_endpoint_hosting(
            memory,
            region,
            capacity.connections,
            capacity.requests_per_connection,
            capacity.children_per_connection,
            controls,
        )?;
        let terminal = TerminalPool::prepare(memory, region, 1, capacity.terminal_origins)?;
        Ok(Self {
            server,
            terminal,
            result_frames,
            cleanup: Arc::new(cleanup),
            native_tasks: None,
            connection_tasks: None,
            terminal_capacity: capacity.terminal_origins,
            needs_connection_capacity: capacity.connections != 0,
            needs_request_capacity: capacity.connections != 0
                && capacity.requests_per_connection != 0
                && capacity.terminal_origins != 0,
        })
    }
    /// Select accept-to-first-head and network idle deadlines for tracked HTTP.
    /// Native Tokio timer backing and separately upgraded sessions are not bounded here.
    #[must_use]
    pub fn with_http_timeouts(mut self, timeouts: crate::PreparedHttpTimeouts) -> Self {
        self.server.select_http_timeouts(timeouts);
        self
    }
    /// Park parsed request heads in the caller's finite FIFO while all permits
    /// are in use. A queued request does not poll its incoming body or reserve
    /// a native request record until it receives a permit. The permit remains
    /// held through cleanup and transport data retirement.
    ///
    /// Select a positive wait. Expiry and queue saturation use the existing
    /// overload reply. Native head, runtime and protocol buffering remain
    /// subject to the selected transport limits.
    #[must_use]
    pub fn with_request_queue(
        mut self,
        gate: Arc<effect::PreparedSemaphore>,
        wait: std::time::Duration,
    ) -> Self {
        self.server.select_request_queue(gate, wait);
        self
    }
    /// Select per-head, per-frame and per-trailer native response ingestion limits.
    /// Header byte totals exclude protocol framing and preexisting backing allocations.
    #[must_use]
    pub fn with_response_limits(mut self, limits: crate::PreparedResponseLimits) -> Self {
        self.server.select_response_limits(limits);
        self
    }
    /// Select native HTTP parser and flow-control limits for prepared serving.
    /// Adaptive HTTP/2 windows are disabled for the selected finite windows.
    pub fn with_http_limits(
        mut self,
        limits: crate::PreparedHttpLimits,
    ) -> Result<Self, crate::ServerError> {
        self.server.select_http_limits(limits)?;
        Ok(self)
    }
    /// Select HTTP/2 reset-retention counts without changing the ordinary host defaults.
    pub fn with_http2_reset_limits(mut self, limits: crate::PreparedHttp2ResetLimits) -> Result<Self, crate::ServerError> {
        self.server.select_http2_reset_limits(limits)?;
        Ok(self)
    }
    /// Calculate channel backing using this server's selected body policy.
    pub fn body_channel_demand(
        &self,
        capacity: crate::PreparedBodyChannelCapacity,
    ) -> Result<crate::PreparedBodyChannelDemand, MemoryError> {
        self.server.body_channel_demand(capacity)
    }
    /// Install independently prepared streaming channels before host execution.
    pub fn with_body_channels(
        mut self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: crate::PreparedBodyChannelCapacity,
        cleanup: &effect::PreparedCleanupConfig<effect::QueueError>,
        panics: &effect::PreparedPanicPool,
    ) -> Result<Self, MemoryError> {
        self.server
            .install_body_channels(memory, region, capacity, cleanup, panics)?;
        Ok(self)
    }
    /// Select finite startup-native backing for prepared blocking codec copies.
    /// Configure body channels first; unselected copies retain ordinary native behavior.
    pub fn with_native_bytes_pool(mut self, pool: Arc<crate::PreparedNativeBytesPool>) -> Result<Self, MemoryError> {
        self.server.install_native_bytes_pool(pool)?;
        Ok(self)
    }
    /// Select actual prepared JSON writer and sink-frame backing for body codecs.
    /// Install prepared body channels first; writer capacity and sink frames are caller-owned.
    /// Use [`Self::json_writer_sink_layout`] to size the supplied writer frame pool.
    pub fn with_json_writer_pool(mut self, pool: Arc<effect::PreparedJsonWriterPool>) -> Result<Self, MemoryError> {
        self.server.install_json_writers(pool)?;
        Ok(self)
    }
    /// Exact concrete frame layout needed for one prepared native JSON sink.
    #[must_use]
    pub fn json_writer_sink_layout() -> std::alloc::Layout {
        effect::PreparedJsonWriterPool::sink_layout::<crate::body::NativeJsonSink>()
    }
    /// Select a caller-prepared native task pool. Native executor integration
    /// uses the caller's prepared provider and working-set budget.
    #[must_use]
    pub fn with_native_tasks<D: effect::prepared_provider::ProviderWorkingSet>(
        mut self,
        pool: Arc<crate::PreparedNativeTaskPool<E, R, D>>,
    ) -> Self {
        self.native_tasks = Some(pool);
        self
    }
    /// Calculate the graph and driver demand for an explicit connection capacity.
    pub fn connection_task_demand(capacity: usize, contract: effect::DynamicRunContract) -> Result<PreparedConnectionTaskDemand, MemoryError>
    where R: effect::prepared_provider::HasPreparedProvider<(), E, effect::prepared_provider::TaskProvider>,
    {
        crate::host::connection_task_demand::<E, R>(capacity, contract)
    }
    /// Select independently prepared request-base and metadata snapshot backing.
    pub fn with_request_storage(mut self, pool: Arc<crate::PreparedRequestStoragePool>) -> Result<Self, MemoryError> {
        let capacity = pool.capacity();
        if self.needs_request_capacity && (capacity.requests == 0 || capacity.snapshots < 4) {
            return Err(MemoryError::NoSlot);
        }
        self.server.install_request_storage(pool);
        Ok(self)
    }
    /// Check whether the selected host uses prepared request and routing snapshots.
    #[must_use]
    pub fn has_prepared_request_storage(&self) -> bool {
        self.server.has_prepared_request_storage()
    }
    /// Admit shared body, decoded or output byte backing before routing.
    pub fn with_request_byte_pools(
        mut self,
        pools: Arc<crate::PreparedEndpointBytePools>,
    ) -> Result<Self, MemoryError> {
        if !self.has_prepared_request_storage() {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.server.install_request_byte_pools(pools);
        Ok(self)
    }
    /// Couple each physically admitted request-control slot to one work unit.
    /// This accounts for that slot only, not headers, transport or application owners.
    pub fn with_request_control_admission(
        mut self,
        admission: effect::memory::Admission<'static, 1>,
    ) -> Result<Self, MemoryError> {
        self.server.install_request_control_admission(admission)?;
        Ok(self)
    }
    /// Couple each actual body completion record to one work slot. This does
    /// not account for channels, native bytes or network settlement.
    pub fn with_body_completion_admission(
        mut self,
        admission: effect::memory::Admission<'static, 1>,
    ) -> Result<Self, MemoryError> {
        self.server.install_body_completion_admission(admission)?;
        Ok(self)
    }
    /// Couple each admitted connection record to one work slot.
    pub fn with_connection_admission(
        mut self,
        admission: effect::memory::Admission<'static, 1>,
    ) -> Result<Self, MemoryError> {
        self.server.install_connection_admission(admission)?;
        Ok(self)
    }
    /// Couple each prepared observer batch vector to one Slots work unit.
    /// Observer captures and their allocations remain separate.
    pub fn with_observer_batch_admission(
        mut self,
        admission: effect::memory::Admission<'static, 1>,
    ) -> Result<Self, MemoryError> {
        self.server.install_observer_batch_admission(admission)?;
        Ok(self)
    }
    /// Couple each actual terminal-origin record to one Slots work unit.
    pub fn with_terminal_admission(mut self, admission: Admission<'static, 1>) -> Result<Self, MemoryError> {
        self.terminal.install_admission(admission)?;
        Ok(self)
    }
    /// Select the opt-in startup response-header controls for request admission.
    #[must_use]
    pub fn with_response_headers_pool(mut self, pool: Arc<crate::PreparedResponseHeadersPool>) -> Self {
        self.server.install_response_headers(pool);
        self
    }
    /// Select caller-backed prepared provider execution for connection tasks.
    #[allow(clippy::too_many_arguments)] // Each caller-selected owner is independent.
    pub fn with_connection_tasks(
        mut self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: FramePool<'static>,
        contract: effect::DynamicRunContract,
        cleanup: effect::PreparedCleanupConfig<E>,
        panics: effect::PreparedPanicPool,
    ) -> Result<Self, MemoryError>
    where R: effect::prepared_provider::HasPreparedProvider<(), E, effect::prepared_provider::TaskProvider>,
    {
        if self.needs_connection_capacity && (capacity == 0 || self.terminal_capacity == 0) {
            return Err(MemoryError::NoSlot);
        }
        self.connection_tasks = Some(Arc::new(crate::host::PreparedConnectionSelection::prepare(memory, region, capacity, frames, contract, cleanup, panics)?));
        Ok(self)
    }
    /// The outer Effect succeeds with the complete guarded Exit, including E.
    #[allow(clippy::type_complexity)]
    pub fn with_host<A: Send + 'static>(
        self,
        use_host: impl Fn(crate::ServerHost) -> effect::Effect<A, E, R> + Send + Sync + 'static,
    ) -> Result<effect::Effect<PreparedHostResult<A, E, R>, std::convert::Infallible, R>, MemoryError>
    {
        let result = TerminalResultReservation::reserve(&self.result_frames)?;
        let (owner, handle) = self.terminal.reserve()?;
        let serving_cleanup = [
            self.cleanup.reserve_failures()?,
            self.cleanup.reserve_failures()?,
        ];
        let host_cleanup = self.cleanup.reserve_failures()?;
        let cleanup_waiter = self.cleanup.subscribe_returns()?;
        if self.needs_connection_capacity {
            // Host controls are already held. Check the configured connection
            // transition, and the request transition only when it is enabled.
            let connection = cleanup_waiter.try_reserve_pair()?;
            if self.needs_request_capacity {
                let request = cleanup_waiter.try_reserve_pair()?;
                drop(request);
            }
            drop(connection);
        }
        let host = self.server.with_host_terminal(
            use_host,
            Some(handle),
            Some(self.cleanup),
            Some(serving_cleanup),
            Some(host_cleanup),
            Some(cleanup_waiter),
            self.native_tasks,
            self.connection_tasks,
        );
        Ok(crate::endpoint::native_once(
            move |environment, context| async move {
                let (exit, owner) = OrderedHost {
                    future: Some(effect::effect! {
                        run_in_context;
                        context: context,
                        program: host,
                        environment: environment,
                    }),
                    owner: Some(owner),
                }
                .await;
                effect::Exit::Success(result.publish(exit, owner.finish()))
            },
        ))
    }
}
