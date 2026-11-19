//! Fixed context/listener records owned by one retained root generation. Node
//! references, generations and lifecycle—not bare indices—govern reuse.
use super::bridge::{Dispatch, Remote, View};
use crate::ClockTick;
use crate::memory::{FixedVector, MemoryError, PreparedMemory, RegionId};
use std::cell::UnsafeCell;
use std::sync::Mutex;
use std::task::Waker;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Id {
    index: usize,
    generation: usize,
}
#[derive(Clone, Copy)]
pub(super) enum Kind {
    Root,
    Child,
    Protected,
    Parallel,
}
// Selection aliases the supplied cancellation origin; inheritance owns a fresh
// local request flag while observing its parent. Failure/clock identity is separate.
#[derive(Clone, Copy)]
enum CancellationLink {
    Fresh,
    Inherit(Id),
    Selected(Id),
    RemoteInherit,
    RemoteSelected,
}
impl CancellationLink {
    fn parent(self) -> Option<Id> {
        match self {
            Self::Fresh | Self::RemoteInherit | Self::RemoteSelected => None,
            Self::Inherit(id) | Self::Selected(id) => Some(id),
        }
    }
}
struct Meta {
    generation: usize,
    references: usize,
    releasing: bool,
    retired: bool,
    cancelled: bool,
    protected: bool,
    fiber: bool,
    closed: bool,
    cancellation: CancellationLink,
    clock_parent: Option<Id>,
    remote_clock: bool,
    clock_borrowed: bool,
}
struct Batches {
    active: FixedVector<'static, ClockTick>,
    spare: Option<FixedVector<'static, ClockTick>>,
    pending_ack: bool,
}
struct ClockQueue {
    values: Mutex<Batches>,
}
impl ClockQueue {
    fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        ticks: usize,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            values: Mutex::new(Batches {
                active: FixedVector::prepare(memory.allocator(region)?, ticks)?,
                spare: Some(FixedVector::prepare(memory.allocator(region)?, ticks)?),
                pending_ack: false,
            }),
        })
    }
    fn acknowledge(&self) {
        let batch = {
            let mut values = self
                .values
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // An existing drainer must observe this request before restoring
            // spare. Publishing under the SAME gate closes the busy/restore race.
            values.pending_ack = true;
            let Some(mut spare) = values.spare.take() else {
                return;
            };
            std::mem::swap(&mut values.active, &mut spare);
            values.pending_ack = false;
            spare
        };
        let mut returned = ReturnBatch {
            queue: self,
            batch: Some(batch),
        };
        returned.drain();
    }
}
struct ReturnBatch<'a> {
    queue: &'a ClockQueue,
    batch: Option<FixedVector<'static, ClockTick>>,
}
impl ReturnBatch<'_> {
    fn drain(&mut self) {
        loop {
            let Some(batch) = self.batch.as_mut() else {
                return;
            };
            // Tick/callback destruction is always outside the bookkeeping gate.
            // A single unwind resumes remaining ticks and requests through Drop.
            batch.clear();
            let mut values = self
                .queue
                .values
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if values.pending_ack {
                values.pending_ack = false;
                std::mem::swap(&mut values.active, batch);
            } else {
                // Restore atomically with the final pending check: a racing
                // acknowledgment either requests this drainer or takes spare.
                values.spare = self.batch.take();
                return;
            }
        }
    }
}
impl Drop for ReturnBatch<'_> {
    fn drop(&mut self) {
        self.drain();
    }
}
struct Node {
    // Immutable during every strong reference/clock loan. Only allocation and
    // last-reference retirement write this separate cell, never Meta reborrows.
    remote: UnsafeCell<Option<Remote>>,
    meta: Mutex<Meta>,
    clock: ClockQueue,
}
// SAFETY: remote publication/removal is serialized by Meta lifecycle; readers
// hold a node reference or the dedicated close loan. Remote payloads are Sync.
unsafe impl Sync for Node {}
struct ReleaseQueue {
    pending: FixedVector<'static, Id>,
    draining: bool,
}
struct ListenerState {
    generation: usize,
    leased: bool,
    retired: bool,
    node: Option<Id>,
    waker: Option<Waker>,
    bridge: Option<Dispatch>,
}
struct Listener {
    state: Mutex<ListenerState>,
}
#[derive(Clone, Copy)]
pub(super) struct ListenerId {
    index: usize,
    generation: usize,
}
/// Termination of observation is not acknowledgment of any native operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedCancellationObservation {
    Requested,
    Closed,
}
pub(super) struct Domain {
    nodes: FixedVector<'static, Node>,
    listeners: FixedVector<'static, Listener>,
    release: Mutex<ReleaseQueue>,
}
impl Domain {
    pub(super) fn payload(
        contexts: usize,
        listeners: usize,
        ticks: usize,
    ) -> Result<crate::memory::Bytes, MemoryError> {
        use crate::memory::Bytes;
        let nodes =
            std::alloc::Layout::array::<Node>(contexts).map_err(|_| MemoryError::InvalidLayout)?;
        let observers = std::alloc::Layout::array::<Listener>(listeners)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let queue =
            std::alloc::Layout::array::<Id>(contexts).map_err(|_| MemoryError::InvalidLayout)?;
        let tick = std::alloc::Layout::array::<ClockTick>(ticks)
            .map_err(|_| MemoryError::InvalidLayout)?;
        Bytes::new(nodes.size())
            .checked_add(Bytes::new(observers.size()))?
            .checked_add(Bytes::new(queue.size()))?
            .checked_add(
                Bytes::new(tick.size())
                    .checked_mul(contexts)?
                    .checked_mul(2)?,
            )
    }
    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        contexts: usize,
        listeners: usize,
        ticks: usize,
    ) -> Result<Self, MemoryError> {
        if contexts == 0 {
            return Err(MemoryError::NoSlot);
        }
        let mut nodes = FixedVector::prepare(memory.allocator(region)?, contexts)?;
        for _ in 0..contexts {
            nodes.push_reserved(Node {
                remote: UnsafeCell::new(None),
                meta: Mutex::new(Meta {
                    generation: 0,
                    references: 0,
                    releasing: false,
                    retired: false,
                    cancelled: false,
                    protected: false,
                    fiber: false,
                    closed: true,
                    cancellation: CancellationLink::Fresh,
                    clock_parent: None,
                    remote_clock: false,
                    clock_borrowed: false,
                }),
                clock: ClockQueue::prepare(memory, region, ticks)?,
            });
        }
        let mut slots = FixedVector::prepare(memory.allocator(region)?, listeners)?;
        for _ in 0..listeners {
            slots.push_reserved(Listener {
                state: Mutex::new(ListenerState {
                    generation: 0,
                    leased: false,
                    retired: false,
                    node: None,
                    waker: None,
                    bridge: None,
                }),
            });
        }
        Ok(Self {
            nodes,
            listeners: slots,
            release: Mutex::new(ReleaseQueue {
                pending: FixedVector::prepare(memory.allocator(region)?, contexts)?,
                draining: false,
            }),
        })
    }
    fn node(&self, id: Id) -> &Node {
        // SAFETY: Id is private and minted only by enumeration of this immutable
        // fixed array. Contexts retain the owning root; parent links retain node
        // generations. Cross-root selection is checked before accepting an Id.
        unsafe { self.nodes.as_slice().get_unchecked(id.index) }
    }
    fn listener_slot(&self, id: ListenerId) -> &Listener {
        // SAFETY: private ListenerId is minted only by enumeration of this fixed
        // root's listener array. Its owned waiter retains that root throughout.
        unsafe { self.listeners.as_slice().get_unchecked(id.index) }
    }
    fn valid(meta: &Meta, id: Id) -> bool {
        meta.generation == id.generation && meta.references != 0 && !meta.releasing
    }
    pub(super) fn retain(&self, id: Id) -> Result<(), MemoryError> {
        let mut meta = self
            .node(id)
            .meta
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !Self::valid(&meta, id) {
            return Err(MemoryError::Closed);
        }
        meta.references = meta
            .references
            .checked_add(1)
            .ok_or(MemoryError::OwnerLimit)?;
        Ok(())
    }
    pub(super) fn create(
        &self,
        kind: Kind,
        parent: Option<Id>,
        cancellation: Option<Id>,
        mut remote: Option<Remote>,
    ) -> Result<Id, MemoryError> {
        let clock_parent = if matches!(kind, Kind::Protected) && remote.is_none() {
            parent.map(|id| {
                let origin = self.clock_origin(id);
                // Share the actual local frontier directly. A foreign frontier
                // stays retained by its existing typed bridge/parent owner.
                if std::ptr::eq(origin.domain, self) {
                    origin.id
                } else {
                    id
                }
            })
        } else {
            None
        };
        let cancel_parent = if matches!(kind, Kind::Child | Kind::Parallel) && remote.is_none() {
            cancellation.or(parent)
        } else {
            None
        };
        if let Some(id) = clock_parent {
            self.retain(id)?;
        }
        if let Some(id) = cancel_parent
            && let Err(error) = self.retain(id)
        {
            if let Some(clock) = clock_parent {
                self.release(clock);
            }
            return Err(error);
        }
        let fiber = matches!(kind, Kind::Child)
            || matches!(kind, Kind::Parallel) && parent.is_some_and(|id| self.flags(id).0);
        for (index, node) in self.nodes.as_slice().iter().enumerate() {
            let mut meta = node
                .meta
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if meta.references != 0 || meta.releasing || meta.retired {
                continue;
            }
            let Some(generation) = meta.generation.checked_add(1) else {
                meta.retired = true;
                continue;
            };
            let id = Id { index, generation };
            if let Some(remote) = &mut remote {
                remote.attach(id);
            }
            let remote_present = remote.is_some();
            // SAFETY: this node has zero references, no close loan and is not
            // Releasing. No projection into the old/new remote can exist.
            unsafe {
                *node.remote.as_mut_unchecked() = remote.take();
            }
            *meta = Meta {
                generation,
                references: 1,
                releasing: false,
                retired: false,
                cancelled: false,
                protected: false,
                fiber,
                closed: false,
                cancellation: match cancel_parent {
                    _ if remote_present && matches!(kind, Kind::Parallel) => {
                        CancellationLink::RemoteSelected
                    }
                    _ if remote_present && matches!(kind, Kind::Child) => {
                        CancellationLink::RemoteInherit
                    }
                    Some(id) if matches!(kind, Kind::Parallel) => CancellationLink::Selected(id),
                    Some(id) => CancellationLink::Inherit(id),
                    None => CancellationLink::Fresh,
                },
                clock_parent,
                remote_clock: remote_present && matches!(kind, Kind::Protected),
                clock_borrowed: false,
            };
            return Ok(Id { index, generation });
        }
        if let Some(id) = cancel_parent {
            self.release(id);
        }
        if let Some(id) = clock_parent {
            self.release(id);
        }
        Err(MemoryError::NoSlot)
    }
    pub(super) fn release(&self, id: Id) {
        {
            let mut meta = self
                .node(id)
                .meta
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !Self::valid(&meta, id) {
                return;
            }
            meta.references -= 1;
            if meta.references != 0 {
                return;
            }
            meta.releasing = true;
            meta.closed = true;
            if meta.clock_borrowed {
                return;
            }
        }
        self.enqueue_release(id);
    }
    fn enqueue_release(&self, id: Id) {
        let drain = {
            let mut release = self
                .release
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // One Releasing entry per physical node; no node can be reused until
            // it is unlinked and cleared. Thus this fixed queue cannot grow.
            release.pending.push_reserved(id);
            if release.draining {
                false
            } else {
                release.draining = true;
                true
            }
        };
        if drain {
            self.drain();
        }
    }
    pub(super) fn finish_releasing(&self, id: Id) {
        self.node(id)
            .meta
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .releasing = false;
    }
    fn drain(&self) {
        let mut guard = DrainGuard {
            domain: self,
            active: true,
        };
        loop {
            let next = {
                let mut release = self
                    .release
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let next = release.pending.pop();
                if next.is_none() {
                    release.draining = false;
                }
                next
            };
            let Some(id) = next else {
                guard.active = false;
                return;
            };
            let node = &self.node(id);
            let parents = {
                let mut meta = node
                    .meta
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (
                    std::mem::replace(&mut meta.cancellation, CancellationLink::Fresh).parent(),
                    meta.clock_parent.take(),
                )
            };
            // SAFETY: last reference, no outstanding close loan; no remote
            // projection survives. The terminal frame now owns its settlement.
            let remote = unsafe { node.remote.replace(None) };
            let _released = ReturnNode {
                domain: self,
                id,
                parents,
                remote,
            };
            node.clock.acknowledge();
        }
    }
    fn remote(&self, id: Id) -> Option<&Remote> {
        // SAFETY: callers retain the start node (direct context, held listener
        // gate, or close loan). Each local/remote link retains its ancestor, so
        // the immutable chain cannot be removed/reused during this traversal.
        unsafe { self.node(id).remote.as_ref_unchecked().as_ref() }
    }
    pub(super) fn cancelled(&self, id: Id) -> bool {
        let mut current = View { domain: self, id };
        loop {
            let link = {
                let meta = current
                    .domain
                    .node(current.id)
                    .meta
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !Self::valid(&meta, current.id) || meta.cancelled {
                    return true;
                }
                meta.cancellation
            };
            match link {
                CancellationLink::Fresh => return false,
                CancellationLink::Inherit(id) | CancellationLink::Selected(id) => current.id = id,
                CancellationLink::RemoteInherit | CancellationLink::RemoteSelected => {
                    let Some(remote) = current.domain.remote(current.id) else {
                        return true;
                    };
                    current = remote.view();
                }
            }
        }
    }
    fn closed(&self, id: Id) -> bool {
        let meta = self
            .node(id)
            .meta
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !Self::valid(&meta, id) || meta.closed
    }
    pub(super) fn flags(&self, id: Id) -> (bool, bool) {
        let meta = self
            .node(id)
            .meta
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (meta.fiber, meta.protected)
    }
    pub(super) fn protected_failure(&self, id: Id, failed: bool) {
        self.node(id)
            .meta
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .protected = failed;
    }
    pub(super) fn request(&self, id: Id) -> bool {
        let mut current = View { domain: self, id };
        loop {
            let link = {
                let meta = current
                    .domain
                    .node(current.id)
                    .meta
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !Self::valid(&meta, current.id) {
                    return false;
                }
                meta.cancellation
            };
            match link {
                CancellationLink::Selected(id) => current.id = id,
                CancellationLink::RemoteSelected => {
                    let Some(remote) = current.domain.remote(current.id) else {
                        return false;
                    };
                    current = remote.view();
                }
                CancellationLink::Fresh
                | CancellationLink::Inherit(_)
                | CancellationLink::RemoteInherit => break,
            }
        }
        let inherited = current.domain.cancelled(current.id);
        let first = {
            let mut meta = current
                .domain
                .node(current.id)
                .meta
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let first = !meta.cancelled && !inherited;
            meta.cancelled = true;
            first
        };
        if first {
            current.domain.notify();
        }
        first
    }
    fn clock_origin(&self, id: Id) -> View<'_> {
        let mut current = View { domain: self, id };
        loop {
            let (parent, remote) = {
                let meta = current
                    .domain
                    .node(current.id)
                    .meta
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (meta.clock_parent, meta.remote_clock)
            };
            if let Some(id) = parent {
                current.id = id;
            } else if remote {
                let Some(parent) = current.domain.remote(current.id) else {
                    return current;
                };
                current = parent.view();
            } else {
                return current;
            }
        }
    }
    pub(super) fn acknowledge(&self, id: Id) {
        let origin = self.clock_origin(id);
        origin.domain.node(origin.id).clock.acknowledge();
    }
    pub(super) fn retain_tick(
        &self,
        id: Id,
        tick: ClockTick,
    ) -> Result<(), (MemoryError, ClockTick)> {
        if self.closed(id) {
            return Err((MemoryError::Closed, tick));
        }
        let origin = self.clock_origin(id);
        let mut values = origin
            .domain
            .node(origin.id)
            .clock
            .values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.closed(id) || origin.domain.closed(origin.id) {
            return Err((MemoryError::Closed, tick));
        }
        values
            .active
            .push(tick)
            .map_err(|tick| (MemoryError::NoSlot, tick))
    }
    pub(super) fn listener(&self, id: Id) -> Result<ListenerId, MemoryError> {
        for (index, slot) in self.listeners.as_slice().iter().enumerate() {
            let mut state = slot
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.leased || state.retired {
                continue;
            }
            let Some(generation) = state.generation.checked_add(1) else {
                state.retired = true;
                continue;
            };
            state.generation = generation;
            state.leased = true;
            state.node = Some(id);
            return Ok(ListenerId { index, generation });
        }
        Err(MemoryError::NoSlot)
    }
    pub(super) fn bind_bridge(&self, id: ListenerId, bridge: Dispatch) {
        self.listener_slot(id)
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .bridge = Some(bridge);
        self.notify();
    }
    pub(super) fn observe(
        &self,
        id: Id,
        listener: ListenerId,
        waker: &Waker,
    ) -> Option<PreparedCancellationObservation> {
        // Clone/drop/wake are external executor callbacks: never under a gate.
        let next = waker.clone();
        let old = {
            let mut state = self
                .listener_slot(listener)
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.waker.replace(next)
        };
        drop(old);
        if self.cancelled(id) {
            Some(PreparedCancellationObservation::Requested)
        } else if self.closed(id) {
            Some(PreparedCancellationObservation::Closed)
        } else {
            None
        }
    }
    pub(super) fn unlink(&self, id: ListenerId) {
        let old = {
            let mut state = self
                .listener_slot(id)
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.generation != id.generation || !state.leased {
                return;
            }
            state.leased = false;
            state.node = None;
            (state.waker.take(), state.bridge.take())
        };
        drop(old);
    }
    pub(super) fn notify(&self) {
        let mut sweep = WakeSweep {
            domain: self,
            next: 0,
        };
        sweep.run();
    }
    pub(super) fn close(&self) {
        for node in self.nodes.as_slice() {
            node.meta
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .closed = true;
        }
        let mut clocks = CloseClocks {
            domain: self,
            next: 0,
        };
        self.notify();
        clocks.run();
    }
    pub(super) fn reset(&mut self) -> Result<(), MemoryError> {
        for node in self.nodes.as_mut_slice() {
            let meta = node
                .meta
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if meta.references != 0 || meta.releasing {
                return Err(MemoryError::Busy);
            }
            meta.cancelled = false;
            meta.protected = false;
            meta.fiber = false;
            meta.closed = true;
            meta.cancellation = CancellationLink::Fresh;
            meta.clock_parent = None;
            meta.remote_clock = false;
            if meta.clock_borrowed {
                return Err(MemoryError::Busy);
            }
            let clock = node
                .clock
                .values
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            clock.active.clear();
            if let Some(spare) = &mut clock.spare {
                spare.clear();
            } else {
                return Err(MemoryError::Busy);
            }
            clock.pending_ack = false;
        }
        for listener in self.listeners.as_mut_slice() {
            let state = listener
                .state
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.leased {
                return Err(MemoryError::Busy);
            }
            drop(state.waker.take());
            drop(state.bridge.take());
            state.node = None;
        }
        Ok(())
    }
}
struct WakeSweep<'a> {
    domain: &'a Domain,
    next: usize,
}
impl WakeSweep<'_> {
    fn run(&mut self) {
        while let Some(slot) = self.domain.listeners.as_slice().get(self.next) {
            self.next += 1;
            let (waker, bridge) = {
                let mut state = slot
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // A leased registration's unregister must acquire this gate
                // BEFORE dropping its context. Thus node/remote ancestry stays
                // strongly retained during the query, not merely a weak Id.
                if let Some(id) = state.node {
                    let cancelled = self.domain.cancelled(id);
                    let closed = self.domain.closed(id);
                    let waker = if cancelled || closed {
                        state.waker.take()
                    } else {
                        None
                    };
                    let bridge = if cancelled { state.bridge.take() } else { None };
                    (waker, bridge)
                } else {
                    (None, None)
                }
            };
            let mut dispatch = WakeDispatch { waker, bridge };
            dispatch.run();
        }
    }
}
struct WakeDispatch {
    waker: Option<Waker>,
    bridge: Option<Dispatch>,
}
impl WakeDispatch {
    fn run(&mut self) {
        if let Some(bridge) = self.bridge.take() {
            bridge.fire();
        }
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}
impl Drop for WakeDispatch {
    fn drop(&mut self) {
        self.run();
    }
}
impl Drop for WakeSweep<'_> {
    fn drop(&mut self) {
        self.run();
    }
}
struct CloseClocks<'a> {
    domain: &'a Domain,
    next: usize,
}
impl CloseClocks<'_> {
    fn run(&mut self) {
        while let Some(node) = self.domain.nodes.as_slice().get(self.next) {
            let index = self.next;
            self.next += 1;
            let borrowed = {
                let mut meta = node
                    .meta
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if meta.references != 0 && !meta.releasing {
                    // Exactly one close sweep per admitted root generation. This
                    // physical loan bit defers last-reference retirement; no
                    // fallible reference-counter clone is attempted at shutdown.
                    meta.clock_borrowed = true;
                    Some(Id {
                        index,
                        generation: meta.generation,
                    })
                } else {
                    None
                }
            };
            if let Some(id) = borrowed {
                let _loan = CloseLoan {
                    domain: self.domain,
                    id,
                };
                self.domain.acknowledge(id);
            } else {
                node.clock.acknowledge();
            }
        }
    }
}
struct CloseLoan<'a> {
    domain: &'a Domain,
    id: Id,
}
impl Drop for CloseLoan<'_> {
    fn drop(&mut self) {
        let release = {
            let mut meta = self
                .domain
                .node(self.id)
                .meta
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            meta.clock_borrowed = false;
            meta.references == 0 && meta.releasing
        };
        if release {
            self.domain.enqueue_release(self.id);
        }
    }
}
impl Drop for CloseClocks<'_> {
    fn drop(&mut self) {
        self.run();
    }
}
struct DrainGuard<'a> {
    domain: &'a Domain,
    active: bool,
}
impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            let resume = {
                let mut release = self
                    .domain
                    .release
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let resume = !release.pending.is_empty();
                release.draining = resume;
                resume
            };
            if resume {
                self.domain.drain();
            }
        }
    }
}
struct ReturnNode<'a> {
    domain: &'a Domain,
    id: Id,
    parents: (Option<Id>, Option<Id>),
    remote: Option<Remote>,
}
impl Drop for ReturnNode<'_> {
    fn drop(&mut self) {
        if let Some(mut remote) = self.remote.take() {
            remote.retire();
            drop(remote);
        } else {
            self.domain.finish_releasing(self.id);
        }
        if let Some(id) = self.parents.0 {
            self.domain.release(id);
        }
        if let Some(id) = self.parents.1 {
            self.domain.release(id);
        }
    }
}
