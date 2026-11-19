//! Physically prepared FIFO queue storage and operation ownership.
//!
//! Logical unbounded capacity does not remove the caller-selected physical
//! admission limits. Items retain their actual payload cells through every
//! escaped owner; backing owned inside `T` remains the caller's responsibility.
//! Offer factories run on first poll only after admission and a cancellation
//! check. Scratch reserves at least one notification slot even with no
//! producers or takers, for a pending operation's own withdrawal.
//!
//! Cleanup buffers and panic diagnostics are supplied and reserved explicitly.
//! Shutdown admits separate cleanup carriers for Drain and a later Discard;
//! Drain leaves committed buffered items available to future takes.
use crate::internal::runtime::prepared_graph::{
    PreparedCancellationControl, PreparedCancellationProbe,
};
use crate::internal::waiting::{Position, Waiting};
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease, SlotPool,
    prepared_notification,
};
use crate::{
    Exit, PreparedCancellation, PreparedCancellationSignalWait, PreparedCleanupConfig,
    PreparedPanicPool, PreparedPanicReservation, QueueCapacity, QueueError,
};
use std::ops::Deref;
use std::sync::Mutex;

/// Caller-selected physical payload and simultaneous operation capacities.
#[derive(Clone, Copy, Debug)]
pub struct PreparedQueueCapacity {
    /// Physical inline payload cells, including escaped items.
    pub payloads: usize,
    /// Simultaneous admitted offer operations.
    pub producers: usize,
    /// Simultaneous admitted take operations.
    pub takers: usize,
}
/// An owned payload retaining its actual prepared cell until destruction.
pub struct PreparedQueueItem<T: Send + 'static> {
    cell: SlotLease<'static, Option<T>>,
}
impl<T: Send + 'static> PreparedQueueItem<T> {
    /// Move out the uniquely owned value and return its now-empty prepared cell.
    #[must_use]
    #[allow(clippy::expect_used)] // Only a filled cell can create an item.
    pub fn into_inner(mut self) -> T {
        self.cell.take().expect("live prepared queue item")
    }
}
impl<T: Send + 'static> Deref for PreparedQueueItem<T> {
    type Target = T;
    #[allow(clippy::expect_used)] // Only a filled payload cell can construct an item.
    fn deref(&self) -> &T {
        self.cell.as_ref().expect("live prepared queue item")
    }
}
impl<T: Send + 'static> Drop for PreparedQueueItem<T> {
    fn drop(&mut self) {
        let value = self.cell.take();
        drop(value);
    }
}
enum Probe {
    Context(PreparedCancellationProbe),
    Standalone(PreparedCancellation),
    Uncancelled,
}
impl Probe {
    fn is_requested(&self) -> bool {
        match self {
            Self::Context(probe) => probe.is_requested(),
            Self::Standalone(token) => token.is_requested(),
            Self::Uncancelled => false,
        }
    }
}
enum CancellationControl {
    Context(PreparedCancellationControl),
    Uncancelled,
    Standalone {
        wait: PreparedCancellationSignalWait,
        token: PreparedCancellation,
    },
}
impl CancellationControl {
    fn is_requested(&self) -> bool {
        match self {
            Self::Context(control) => control.is_requested(),
            Self::Standalone { token, .. } => token.is_requested(),
            Self::Uncancelled => false,
        }
    }
    fn poll(&mut self, cx: &mut std::task::Context<'_>) {
        match self {
            Self::Context(control) => {
                let _ = control.poll_cancelled(cx);
            }
            Self::Standalone { wait, .. } => {
                let _ = std::future::Future::poll(std::pin::Pin::new(wait), cx);
            }
            Self::Uncancelled => {}
        }
    }
}
struct Control<O: Send + 'static> {
    outcome: Mutex<Option<Exit<O, QueueError>>>,
    changed: prepared_notification::Event,
}
struct ControlOwner<O: Send + 'static> {
    slot: Option<SharedSlot<'static, Control<O>>>,
}
impl<O: Send + 'static> ControlOwner<O> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slot: Some(self.slot.as_ref().ok_or(MemoryError::Closed)?.try_clone()?),
        })
    }
    #[allow(clippy::expect_used)] // A live control authority always retains its slot.
    fn control(&self) -> &Control<O> {
        self.slot.as_deref().expect("live queue control")
    }
    fn same(&self, other: &Self) -> bool {
        match (&self.slot, &other.slot) {
            (Some(a), Some(b)) => a.same_slot(b),
            _ => false,
        }
    }
    fn take(&self) -> Option<Exit<O, QueueError>> {
        self.control()
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
    fn set(&self, outcome: Exit<O, QueueError>) {
        let mut current = self
            .control()
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A control is checked out empty and its unique Waiting entry is
        // removed before this single completion; no second writer exists.
        debug_assert!(current.is_none());
        *current = Some(outcome);
    }
}
impl<O: Send + 'static> Drop for ControlOwner<O> {
    fn drop(&mut self) {
        if let Some(lease) = self.slot.take().and_then(SharedSlot::into_last) {
            let outcome = lease
                .outcome
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(outcome);
            drop(lease);
        }
    }
}
struct Offer<T: Send + 'static> {
    item: PreparedQueueItem<T>,
    control: ControlOwner<()>,
    probe: Probe,
}
struct Taker<T: Send + 'static> {
    control: ControlOwner<PreparedQueueItem<T>>,
    probe: Probe,
}
struct State<T: Send + 'static> {
    capacity: QueueCapacity,
    buffer: FixedVector<'static, PreparedQueueItem<T>>,
    offers: Waiting<Offer<T>>,
    takers: Waiting<Taker<T>>,
    closed: bool,
}
// Each notification carries the removed entry's original probe so its final
// context/node retirement cannot occur under the queue gate.
enum Notify<T: Send + 'static> {
    Offer(ControlOwner<()>, Probe),
    Take(ControlOwner<PreparedQueueItem<T>>, Probe),
}
struct Scratch<T: Send + 'static> {
    notify: FixedVector<'static, Notify<T>>,
    discard: FixedVector<'static, PreparedQueueItem<T>>,
    panics: FixedVector<'static, PreparedPanicReservation>,
    defects: FixedVector<'static, crate::Defect>,
}
struct ScratchOwner<T: Send + 'static> {
    slot: Option<SlotLease<'static, Scratch<T>>>,
}
enum ScratchDrop<T: Send + 'static> {
    Notify(Notify<T>),
    Item(PreparedQueueItem<T>),
    Panic(PreparedPanicReservation),
    Defect(crate::Defect),
}
struct ContinueScratch<'a, T: Send + 'static> {
    owner: &'a mut ScratchOwner<T>,
    armed: bool,
}
struct ContinuePhase<'a, T: Send + 'static> {
    owner: &'a mut ScratchOwner<T>,
    armed: bool,
}
impl<T: Send + 'static> Drop for ContinuePhase<'_, T> {
    fn drop(&mut self) {
        if self.armed {
            self.owner.clear_phase();
        }
    }
}
impl<T: Send + 'static> Drop for ContinueScratch<'_, T> {
    fn drop(&mut self) {
        if self.armed {
            self.owner.clear();
        }
    }
}
impl<T: Send + 'static> ScratchOwner<T> {
    #[allow(clippy::expect_used)] // An admitted operation retains its scratch lease until retirement.
    fn get_mut(&mut self) -> &mut Scratch<T> {
        self.slot.as_deref_mut().expect("live queue scratch")
    }
    fn clear_phase(&mut self) {
        loop {
            let next = self.slot.as_mut().and_then(|slot| {
                slot.notify
                    .remove_first()
                    .map(ScratchDrop::Notify)
                    .or_else(|| slot.discard.remove_first().map(ScratchDrop::Item))
                    .or_else(|| slot.defects.remove_first().map(ScratchDrop::Defect))
            });
            let Some(next) = next else { break };
            let mut continuation = ContinuePhase {
                owner: self,
                armed: true,
            };
            match next {
                ScratchDrop::Notify(value) => drop(value),
                ScratchDrop::Item(value) => drop(value),
                ScratchDrop::Defect(value) => drop(value),
                ScratchDrop::Panic(value) => drop(value),
            }
            continuation.armed = false;
        }
    }
    fn clear(&mut self) {
        loop {
            let next = self.slot.as_mut().and_then(|slot| {
                slot.notify
                    .remove_first()
                    .map(ScratchDrop::Notify)
                    .or_else(|| slot.discard.remove_first().map(ScratchDrop::Item))
                    .or_else(|| slot.panics.remove_first().map(ScratchDrop::Panic))
                    .or_else(|| slot.defects.remove_first().map(ScratchDrop::Defect))
            });
            let Some(next) = next else { break };
            let mut continuation = ContinueScratch {
                owner: self,
                armed: true,
            };
            match next {
                ScratchDrop::Notify(value) => drop(value),
                ScratchDrop::Item(value) => drop(value),
                ScratchDrop::Panic(value) => drop(value),
                ScratchDrop::Defect(value) => drop(value),
            }
            continuation.armed = false;
        }
        drop(self.slot.take());
    }
}
impl<T: Send + 'static> Drop for ScratchOwner<T> {
    fn drop(&mut self) {
        self.clear();
    }
}
struct Shutdown<T: Send + 'static> {
    scratch: SlotLease<'static, Scratch<T>>,
    cleanup: crate::CleanupFailures<QueueError>,
    spare: Option<crate::CleanupFailures<QueueError>>,
}
struct Resources<T: Send + 'static> {
    scratch: ScratchOwner<T>,
    cleanup: Option<crate::CleanupFailures<QueueError>>,
}
// Keeps the independently preadmitted shutdown batch owned across callbacks
// and restores it on unwind, without holding either queue or shutdown gate.
struct ShutdownGuard<'a, T: Send + 'static> {
    state: &'a Mutex<State<T>>,
    owner: &'a Mutex<Option<Shutdown<T>>>,
    resources: Option<Resources<T>>,
    spare: Option<crate::CleanupFailures<QueueError>>,
    retain: bool,
}
impl<T: Send + 'static> Drop for ShutdownGuard<'_, T> {
    fn drop(&mut self) {
        let Some(mut resources) = self.resources.take() else {
            return;
        };
        resources.scratch.clear_phase();
        let Some(cleanup) = resources.cleanup.take().or_else(|| self.spare.take()) else {
            return;
        };
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.retain || (state.closed && state.buffer.is_empty()) {
            drop(state);
            return;
        }
        if let Some(scratch) = resources.scratch.slot.take() {
            let batch = Shutdown {
                scratch,
                cleanup,
                spare: self.spare.take(),
            };
            *self
                .owner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(batch);
        }
        drop(state);
    }
}
struct ChangeDrain<'a, T: Send + 'static> {
    scratch: &'a mut Scratch<T>,
    active: bool,
}
impl<T: Send + 'static> ChangeDrain<'_, T> {
    #[allow(clippy::expect_used)] // Admission reserves the proven maximum for this complete operation.
    fn run(&mut self) {
        while let Some(notification) = self.scratch.notify.remove_first() {
            let panic = self
                .scratch
                .panics
                .remove_first()
                .expect("preadmitted notification diagnostic");
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match notification {
                    Notify::Offer(control, probe) => {
                        control.control().changed.notify_all();
                        drop(probe);
                        drop(control);
                    }
                    Notify::Take(control, probe) => {
                        control.control().changed.notify_all();
                        drop(probe);
                        drop(control);
                    }
                }));
            match result {
                Ok(()) => self.scratch.panics.push_reserved(panic),
                Err(payload) => self.scratch.defects.push_reserved(panic.capture(payload)),
            }
        }
        while let Some(item) = self.scratch.discard.remove_first() {
            let panic = self
                .scratch
                .panics
                .remove_first()
                .expect("preadmitted item diagnostic");
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(item)));
            match result {
                Ok(()) => self.scratch.panics.push_reserved(panic),
                Err(payload) => self.scratch.defects.push_reserved(panic.capture(payload)),
            }
        }
        self.active = false;
    }
}
impl<T: Send + 'static> Drop for ChangeDrain<'_, T> {
    fn drop(&mut self) {
        if self.active {
            self.run();
        }
    }
}
impl<T: Send + 'static> Resources<T> {
    #[allow(clippy::expect_used, clippy::unreachable)] // Complete admission and W05 rejoin preserve the reserved cleanup owner.
    fn finish<B>(&mut self, outcome: Exit<B, QueueError>) -> Exit<B, QueueError> {
        let scratch = self.scratch.get_mut();
        ChangeDrain {
            scratch,
            active: true,
        }
        .run();
        if scratch.defects.is_empty() {
            return outcome;
        }
        match outcome {
            Exit::Success(value) => {
                if std::mem::needs_drop::<B>() {
                    let panic = scratch
                        .panics
                        .remove_first()
                        .expect("preadmitted successful-value diagnostic");
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(value))) {
                        Ok(()) => scratch.panics.push_reserved(panic),
                        Err(payload) => scratch.defects.push_reserved(panic.capture(payload)),
                    }
                } else {
                    drop(value);
                }
                let spare = self.cleanup.take();
                let mut accumulator =
                    match crate::PreparedCleanupAccumulator::rejoin(Exit::Success(()), spare) {
                        Ok(value) => value,
                        Err(_) => unreachable!("admitted cleanup owner"),
                    };
                while let Some(defect) = scratch.defects.remove_first() {
                    accumulator.append_cleanup(Exit::Die(defect));
                }
                let (result, spare) = accumulator.into_parts();
                self.cleanup = spare;
                match result {
                    Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
                    Exit::Success(()) | Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted => {
                        unreachable!("nonempty cleanup failures")
                    }
                }
            }
            other @ (Exit::Fail(_)
            | Exit::Die(_)
            | Exit::Interrupted
            | Exit::FinalizationFailed(_)) => {
                let spare = self.cleanup.take();
                let mut accumulator = match crate::PreparedCleanupAccumulator::rejoin(other, spare)
                {
                    Ok(value) => value,
                    Err(_) => unreachable!("admitted cleanup owner"),
                };
                while let Some(defect) = scratch.defects.remove_first() {
                    accumulator.append_cleanup(Exit::Die(defect));
                }
                let (result, spare) = accumulator.into_parts();
                self.cleanup = spare;
                result
            }
        }
    }
}
struct QueueState<T: Send + 'static> {
    state: Mutex<State<T>>,
    payloads: SlotPool<'static, Option<T>>,
    offers: SlotPool<'static, Control<()>>,
    takers: SlotPool<'static, Control<PreparedQueueItem<T>>>,
    // Serialize checkout's pre-gate owner clone, including failed admissions.
    taker_admission: Mutex<()>,
    scratch: SlotPool<'static, Scratch<T>>,
    shutdown: Mutex<Option<Shutdown<T>>>,
    cleanup: PreparedCleanupConfig<QueueError>,
    panics: PreparedPanicPool,
    capacity: PreparedQueueCapacity,
}
type ReservedTaker<T> = (
    prepared_notification::Registration,
    ControlOwner<PreparedQueueItem<T>>,
    ControlOwner<PreparedQueueItem<T>>,
);
/// Prepared FIFO with explicit physical operation and payload limits.
pub struct PreparedQueue<T: Send + 'static> {
    state: SharedSlot<'static, QueueState<T>>,
}
/// A preadmitted offer. The candidate factory is consumed only when first polled.
pub struct PreparedQueueOffer<T: Send + 'static, F: Send + 'static> {
    listener: Option<prepared_notification::Registration>,
    control: ControlOwner<()>,
    queue: PreparedQueue<T>,
    entry: Option<ControlOwner<()>>,
    probe: Option<Probe>,
    cancellation: CancellationControl,
    payload: Option<SlotLease<'static, Option<T>>>,
    factory: Option<F>,
    apply: fn(F) -> Result<T, crate::Defect>,
    position: Option<Position>,
    resources: Resources<T>,
    done: bool,
}
/// Physical native offer backing held before the candidate factory is supplied.
#[doc(hidden)]
pub struct PreparedQueueOfferReservation<T: Send + 'static> {
    listener: prepared_notification::Registration,
    control: ControlOwner<()>,
    entry: ControlOwner<()>,
    payload: SlotLease<'static, Option<T>>,
    queue: PreparedQueue<T>,
    resources: Resources<T>,
}
impl<T: Send + 'static> PreparedQueueOfferReservation<T> {
    /// Install an affine candidate only after all physical owners exist.
    #[doc(hidden)]
    pub fn start<F: FnOnce() -> T + Send + 'static>(self, factory: F) -> PreparedQueueOffer<T, F> {
        PreparedQueueOffer {
            listener: Some(self.listener),
            control: self.control,
            queue: self.queue,
            entry: Some(self.entry),
            probe: Some(Probe::Uncancelled),
            cancellation: CancellationControl::Uncancelled,
            payload: Some(self.payload),
            factory: Some(factory),
            apply: |factory: F| Ok(factory()),
            position: None,
            resources: self.resources,
            done: false,
        }
    }
}
/// A preadmitted take retaining its result's actual payload cell.
pub struct PreparedQueueTake<T: Send + 'static> {
    listener: Option<prepared_notification::Registration>,
    control: ControlOwner<PreparedQueueItem<T>>,
    queue: PreparedQueue<T>,
    entry: Option<ControlOwner<PreparedQueueItem<T>>>,
    probe: Option<Probe>,
    cancellation: CancellationControl,
    position: Option<Position>,
    resources: Resources<T>,
    started: bool,
    done: bool,
}
/// A native receiver retaining its actual listener, control and cleanup backing
/// across successful pulls. Dropping it withdraws any pending take.
#[doc(hidden)]
pub struct PreparedQueueReceiver<T: Send + 'static> {
    take: PreparedQueueTake<T>,
    rearm: bool,
    terminal: bool,
}
// Neither future exposes a structurally pinned field. T remains in its stable
// prepared payload cell and the affine factory may be moved before invocation.
impl<T: Send + 'static, F: Send + 'static> Unpin for PreparedQueueOffer<T, F> {}
impl<T: Send + 'static> Unpin for PreparedQueueTake<T> {}
fn reserve_control<O: Send + 'static>(
    pool: &SlotPool<'static, Control<O>>,
) -> Result<
    (
        prepared_notification::Registration,
        ControlOwner<O>,
        ControlOwner<O>,
    ),
    MemoryError,
> {
    let lease = pool.checkout()?;
    let listener = lease.changed.listen()?;
    let owner = ControlOwner {
        slot: Some(lease.share()),
    };
    let entry = owner.try_clone()?;
    Ok((listener, owner, entry))
}

/// Derived physical storage and panic-reservation bounds for a prepared queue.
#[derive(Clone, Copy, Debug)]
pub struct PreparedQueueDemand {
    /// Region payload and pool metadata, excluding allocator overhead and T-owned pointees.
    pub region_bytes: Bytes,
    /// Physical buffer entries for the selected logical policy.
    pub buffer_slots: usize,
    /// Preadmitted diagnostic authorities per offer, including its factory.
    pub offer_panic_slots: usize,
    /// Preadmitted diagnostic authorities per take.
    pub take_panic_slots: usize,
    /// Independently held diagnostic authorities for shutdown.
    pub shutdown_panic_slots: usize,
    /// Separate cleanup buffers for a failing Drain followed by a failing Discard.
    pub shutdown_cleanup_buffers: usize,
}
impl<T: Send + 'static> PreparedQueue<T> {
    fn reserve_taker(&self) -> Result<ReservedTaker<T>, MemoryError> {
        let gate = self
            .state
            .taker_admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Fresh controls are empty: the final owner takes any prior outcome
        // outside its gate before returning the slot. Listener retirement also
        // clears its Waker before the listener slot can return.
        let result = reserve_control(&self.state.takers);
        drop(gate);
        result
    }
    fn reserve_resources(&self, panic_count: usize) -> Result<Resources<T>, MemoryError> {
        let mut scratch = ScratchOwner {
            slot: Some(self.state.scratch.checkout()?),
        };
        for _ in 0..panic_count {
            scratch
                .get_mut()
                .panics
                .push_reserved(self.state.panics.try_reserve()?);
        }
        let cleanup = self.state.cleanup.reserve_failures()?;
        Ok(Resources {
            scratch,
            cleanup: Some(cleanup),
        })
    }
    /// Compute actual inline/pool demand. Allocator headers and T-owned pointees are additional.
    pub fn demand(
        logical: QueueCapacity,
        capacity: PreparedQueueCapacity,
    ) -> Result<PreparedQueueDemand, MemoryError> {
        // Taker-pool handles are private: at most two per physical taker,
        // plus the total physically provisioned notification scratch entries
        // (including one removed into a drain local per scratch), plus the
        // pool owner and serialized checkout/rearm transient margin.
        // This bound makes an internal receiver's checked owner retain unable
        // to reach SharedSlot's isize::MAX owner limit without allocation.
        let operations = capacity
            .producers
            .checked_add(capacity.takers)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let aliases = operations
            .checked_add(1)
            .and_then(|n| n.checked_mul(operations.max(1)))
            .and_then(|n| {
                capacity
                    .takers
                    .checked_mul(2)
                    .and_then(|live| n.checked_add(live))
            })
            .and_then(|n| n.checked_add(4))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        if aliases >= isize::MAX as usize {
            return Err(MemoryError::OwnerLimit);
        }
        fn array<T>(count: usize) -> Result<Bytes, MemoryError> {
            Ok(Bytes::new(
                std::alloc::Layout::array::<T>(count)
                    .map_err(|_| MemoryError::InvalidLayout)?
                    .size(),
            ))
        }
        let buffer_slots = match logical {
            QueueCapacity::Unbounded => capacity.payloads,
            QueueCapacity::Bounded(limit) => capacity.payloads.min(limit.get()),
            QueueCapacity::Rendezvous => 0,
        };
        let offer_panic_slots = capacity
            .takers
            .checked_add(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let take_panic_slots = capacity
            .producers
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let shutdown_panic_slots = capacity
            .producers
            .checked_mul(2)
            .and_then(|n| n.checked_add(capacity.takers))
            .and_then(|n| n.checked_add(buffer_slots))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let operations = capacity
            .producers
            .checked_add(capacity.takers)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let notify = operations.max(1);
        let discard = capacity
            .producers
            .checked_add(buffer_slots)
            .ok_or(MemoryError::ArithmeticOverflow)?
            .max(1);
        let panic_slots = offer_panic_slots
            .max(take_panic_slots)
            .max(shutdown_panic_slots);
        let scratch_count = operations
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let scratch_arrays = array::<Notify<T>>(notify)?
            .checked_add(array::<PreparedQueueItem<T>>(discard)?)?
            .checked_add(array::<PreparedPanicReservation>(panic_slots)?)?
            .checked_add(array::<crate::Defect>(panic_slots)?)?
            .checked_mul(scratch_count)?;
        let region_bytes = SlotPool::<QueueState<T>>::storage_bytes(1)?
            .checked_add(SlotPool::<Option<T>>::storage_bytes(capacity.payloads)?)?
            .checked_add(SlotPool::<Control<()>>::storage_bytes(capacity.producers)?)?
            .checked_add(SlotPool::<Control<PreparedQueueItem<T>>>::storage_bytes(
                capacity.takers,
            )?)?
            .checked_add(prepared_notification::Event::storage_bytes(1)?.checked_mul(operations)?)?
            .checked_add(Waiting::<Offer<T>>::payload_bytes(capacity.producers)?)?
            .checked_add(Waiting::<Taker<T>>::payload_bytes(capacity.takers)?)?
            .checked_add(array::<PreparedQueueItem<T>>(buffer_slots)?)?
            .checked_add(SlotPool::<Scratch<T>>::storage_bytes(scratch_count)?)?
            .checked_add(scratch_arrays)?;
        Ok(PreparedQueueDemand {
            region_bytes,
            buffer_slots,
            offer_panic_slots,
            take_panic_slots,
            shutdown_panic_slots,
            shutdown_cleanup_buffers: 2,
        })
    }
    /// Prepare real payload, waiter, scratch and independent shutdown backing.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        logical: QueueCapacity,
        capacity: PreparedQueueCapacity,
        cleanup: &PreparedCleanupConfig<QueueError>,
        panics: &PreparedPanicPool,
    ) -> Result<Self, MemoryError> {
        let demand = Self::demand(logical, capacity)?;
        let operations = capacity
            .producers
            .checked_add(capacity.takers)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let notify = operations.max(1);
        let discard = capacity
            .producers
            .checked_add(demand.buffer_slots)
            .ok_or(MemoryError::ArithmeticOverflow)?
            .max(1);
        let panic_slots = demand
            .offer_panic_slots
            .max(demand.take_panic_slots)
            .max(demand.shutdown_panic_slots);
        let scratch = memory.slots(
            region,
            operations
                .checked_add(1)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            |_| {
                Ok(Scratch {
                    notify: FixedVector::prepare(memory.allocator(region)?, notify)?,
                    discard: FixedVector::prepare(memory.allocator(region)?, discard)?,
                    panics: FixedVector::prepare(memory.allocator(region)?, panic_slots)?,
                    defects: FixedVector::prepare(memory.allocator(region)?, panic_slots)?,
                })
            },
        )?;
        let mut shutdown_scratch = scratch.checkout()?;
        for _ in 0..demand.shutdown_panic_slots {
            shutdown_scratch.panics.push_reserved(panics.try_reserve()?);
        }
        let shutdown_cleanup = cleanup.reserve_failures()?;
        let shutdown_spare = cleanup.reserve_failures()?;
        let payloads = memory.slots(region, capacity.payloads, |_| Ok(None))?;
        let offers = memory.slots(region, capacity.producers, |_| {
            Ok(Control {
                outcome: Mutex::new(None),
                changed: prepared_notification::Event::prepare(memory, region, 1)?,
            })
        })?;
        let takers = memory.slots(region, capacity.takers, |_| {
            Ok(Control {
                outcome: Mutex::new(None),
                changed: prepared_notification::Event::prepare(memory, region, 1)?,
            })
        })?;
        let state = State {
            capacity: logical,
            buffer: FixedVector::prepare(memory.allocator(region)?, demand.buffer_slots)?,
            offers: Waiting::prepare(memory, region, capacity.producers)?,
            takers: Waiting::prepare(memory, region, capacity.takers)?,
            closed: false,
        };
        let mut record = Some(QueueState {
            state: Mutex::new(state),
            payloads,
            offers,
            takers,
            taker_admission: Mutex::new(()),
            scratch,
            shutdown: Mutex::new(Some(Shutdown {
                scratch: shutdown_scratch,
                cleanup: shutdown_cleanup,
                spare: Some(shutdown_spare),
            })),
            cleanup: cleanup.try_clone()?,
            panics: panics.try_clone()?,
            capacity,
        });
        let root = memory.slots(region, 1, |_| record.take().ok_or(MemoryError::NoSlot))?;
        Ok(Self {
            state: root.checkout()?.share(),
        })
    }
    /// Retain the same queue without allocating.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            state: self.state.try_clone()?,
        })
    }
    /// Return the number of buffered items, excluding pending producers.
    pub fn size_now(&self) -> usize {
        self.state
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .buffer
            .len()
    }
    /// Admit a native offer whose cancellation is withdrawal on future drop.
    /// Refusal returns the original, uninvoked factory.
    #[doc(hidden)]
    #[allow(clippy::result_large_err)]
    pub fn offer_native<F: FnOnce() -> T + Send + 'static>(
        &self,
        factory: F,
    ) -> Result<PreparedQueueOffer<T, F>, (MemoryError, F)> {
        match self.reserve_native_offer() {
            Ok(reservation) => Ok(reservation.start(factory)),
            Err(error) => Err((error, factory)),
        }
    }
    /// Reserve the native offer's complete physical backing without a factory.
    #[doc(hidden)]
    pub fn reserve_native_offer(&self) -> Result<PreparedQueueOfferReservation<T>, MemoryError> {
        let payload = self.state.payloads.checkout()?;
        let (listener, control, entry) = reserve_control(&self.state.offers)?;
        let queue = self.try_clone()?;
        let count = self
            .state
            .capacity
            .takers
            .checked_add(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let resources = self.reserve_resources(count)?;
        Ok(PreparedQueueOfferReservation {
            listener,
            control,
            entry,
            payload,
            queue,
            resources,
        })
    }
    /// Admit one native take whose cancellation is withdrawal on future drop.
    #[doc(hidden)]
    pub fn take_native(&self) -> Result<PreparedQueueTake<T>, MemoryError> {
        let (listener, control, entry) = self.reserve_taker()?;
        let queue = self.try_clone()?;
        let count = self
            .state
            .capacity
            .producers
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let resources = self.reserve_resources(count)?;
        Ok(PreparedQueueTake {
            listener: Some(listener),
            control,
            queue,
            entry: Some(entry),
            probe: Some(Probe::Uncancelled),
            cancellation: CancellationControl::Uncancelled,
            position: None,
            resources,
            started: false,
            done: false,
        })
    }
    /// Admit a receiver once before native pull-driven delivery begins.
    #[doc(hidden)]
    pub fn native_receiver(&self) -> Result<PreparedQueueReceiver<T>, MemoryError> {
        Ok(PreparedQueueReceiver {
            take: self.take_native()?,
            rearm: false,
            terminal: false,
        })
    }
    /// Admit an affine offer under the native context before invoking `factory`.
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn offer_with_context<E: Send + 'static, F: FnOnce() -> T + Send + 'static>(
        &self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
        factory: F,
    ) -> Result<
        PreparedQueueOffer<T, F>,
        (
            MemoryError,
            crate::internal::runtime::prepared_graph::FutureContext<E>,
            F,
        ),
    > {
        self.offer_context_inner(context, factory, |factory| Ok(factory()))
    }
    /// Admit a candidate whose environment access may return an existing defect.
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn offer_try_with_context<
        E: Send + 'static,
        F: FnOnce() -> Result<T, crate::Defect> + Send + 'static,
    >(
        &self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
        factory: F,
    ) -> Result<
        PreparedQueueOffer<T, F>,
        (
            MemoryError,
            crate::internal::runtime::prepared_graph::FutureContext<E>,
            F,
        ),
    > {
        self.offer_context_inner(context, factory, |factory| factory())
    }
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    fn offer_context_inner<E: Send + 'static, F: Send + 'static>(
        &self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
        factory: F,
        apply: fn(F) -> Result<T, crate::Defect>,
    ) -> Result<
        PreparedQueueOffer<T, F>,
        (
            MemoryError,
            crate::internal::runtime::prepared_graph::FutureContext<E>,
            F,
        ),
    > {
        let admitted = (|| {
            let probe = context.try_probe()?;
            let payload = self.state.payloads.checkout()?;
            let (listener, control, entry) = reserve_control(&self.state.offers)?;
            let queue = self.try_clone()?;
            let resources = self.reserve_resources(
                self.state
                    .capacity
                    .takers
                    .checked_add(2)
                    .ok_or(MemoryError::ArithmeticOverflow)?,
            )?;
            Ok::<_, MemoryError>((probe, payload, listener, control, entry, queue, resources))
        })();
        match admitted {
            Ok((probe, payload, listener, control, entry, queue, resources)) => {
                Ok(PreparedQueueOffer {
                    listener: Some(listener),
                    control,
                    queue,
                    entry: Some(entry),
                    probe: Some(Probe::Context(probe)),
                    cancellation: CancellationControl::Context(context.into_cancellation_control()),
                    payload: Some(payload),
                    factory: Some(factory),
                    apply,
                    position: None,
                    resources,
                    done: false,
                })
            }
            Err(error) => Err((error, context, factory)),
        }
    }
    /// Admit an affine offer using a standalone prepared cancellation token.
    #[allow(clippy::result_large_err)]
    pub fn offer_with<F: FnOnce() -> T + Send + 'static>(
        &self,
        token: &PreparedCancellation,
        factory: F,
    ) -> Result<PreparedQueueOffer<T, F>, (MemoryError, F)> {
        let admitted = (|| {
            let probe = token.try_clone()?;
            let cancellation = token.try_clone()?;
            let wait = token.cancelled()?;
            let payload = self.state.payloads.checkout()?;
            let (listener, control, entry) = reserve_control(&self.state.offers)?;
            let queue = self.try_clone()?;
            let resources = self.reserve_resources(
                self.state
                    .capacity
                    .takers
                    .checked_add(2)
                    .ok_or(MemoryError::ArithmeticOverflow)?,
            )?;
            Ok::<_, MemoryError>((
                probe,
                cancellation,
                wait,
                payload,
                listener,
                control,
                entry,
                queue,
                resources,
            ))
        })();
        match admitted {
            Ok((probe, token, wait, payload, listener, control, entry, queue, resources)) => {
                Ok(PreparedQueueOffer {
                    listener: Some(listener),
                    control,
                    queue,
                    entry: Some(entry),
                    probe: Some(Probe::Standalone(probe)),
                    cancellation: CancellationControl::Standalone { wait, token },
                    payload: Some(payload),
                    factory: Some(factory),
                    apply: |factory: F| Ok(factory()),
                    position: None,
                    resources,
                    done: false,
                })
            }
            Err(error) => Err((error, factory)),
        }
    }
    /// Admit a take under the native context before starting the operation.
    #[allow(clippy::result_large_err)]
    pub fn take_with_context<E: Send + 'static>(
        &self,
        context: crate::internal::runtime::prepared_graph::FutureContext<E>,
    ) -> Result<
        PreparedQueueTake<T>,
        (
            MemoryError,
            crate::internal::runtime::prepared_graph::FutureContext<E>,
        ),
    > {
        let admitted = (|| {
            let probe = context.try_probe()?;
            let (listener, control, entry) = self.reserve_taker()?;
            let queue = self.try_clone()?;
            let count = self
                .state
                .capacity
                .producers
                .checked_mul(2)
                .and_then(|n| n.checked_add(1))
                .ok_or(MemoryError::ArithmeticOverflow)?;
            let resources = self.reserve_resources(count)?;
            Ok::<_, MemoryError>((probe, listener, control, entry, queue, resources))
        })();
        match admitted {
            Ok((probe, listener, control, entry, queue, resources)) => Ok(PreparedQueueTake {
                listener: Some(listener),
                control,
                queue,
                entry: Some(entry),
                probe: Some(Probe::Context(probe)),
                cancellation: CancellationControl::Context(context.into_cancellation_control()),
                position: None,
                resources,
                started: false,
                done: false,
            }),
            Err(error) => Err((error, context)),
        }
    }
    /// Admit a take using a standalone prepared cancellation token.
    pub fn take(&self, token: &PreparedCancellation) -> Result<PreparedQueueTake<T>, MemoryError> {
        let probe = token.try_clone()?;
        let cancellation = token.try_clone()?;
        let wait = token.cancelled()?;
        let (listener, control, entry) = self.reserve_taker()?;
        let queue = self.try_clone()?;
        let count = self
            .state
            .capacity
            .producers
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let resources = self.reserve_resources(count)?;
        Ok(PreparedQueueTake {
            listener: Some(listener),
            control,
            queue,
            entry: Some(entry),
            probe: Some(Probe::Standalone(probe)),
            cancellation: CancellationControl::Standalone {
                wait,
                token: cancellation,
            },
            position: None,
            resources,
            started: false,
            done: false,
        })
    }
}

// Every entry owns one physically admitted control slot, hence a waiter list
// cannot reach its prepared capacity while another admitted entry is outside it.
#[allow(clippy::unreachable)]
fn insert_waiter<V>(list: &mut Waiting<V>, value: V) -> Position {
    match list.try_push_back(value) {
        Ok(position) => position,
        Err(_) => unreachable!("preadmitted queue control bounds waiter count"),
    }
}
fn fail_offer<T: Send + 'static>(
    entry: Offer<T>,
    scratch: &mut Scratch<T>,
    result: Exit<(), QueueError>,
) {
    let Offer {
        item,
        control,
        probe,
    } = entry;
    control.set(result);
    scratch.notify.push_reserved(Notify::Offer(control, probe));
    scratch.discard.push_reserved(item);
}
fn complete_taker<T: Send + 'static>(
    entry: Taker<T>,
    scratch: &mut Scratch<T>,
    result: Exit<PreparedQueueItem<T>, QueueError>,
) {
    entry.control.set(result);
    scratch
        .notify
        .push_reserved(Notify::Take(entry.control, entry.probe));
}
fn live_offer<T: Send + 'static>(
    state: &mut State<T>,
    scratch: &mut Scratch<T>,
) -> Option<Offer<T>> {
    while let Some(entry) = state.offers.pop_front() {
        if entry.probe.is_requested() {
            fail_offer(entry, scratch, Exit::Interrupted);
        } else {
            return Some(entry);
        }
    }
    None
}
fn complete_offer<T: Send + 'static>(
    entry: Offer<T>,
    scratch: &mut Scratch<T>,
) -> PreparedQueueItem<T> {
    let Offer {
        item,
        control,
        probe,
    } = entry;
    control.set(Exit::Success(()));
    scratch.notify.push_reserved(Notify::Offer(control, probe));
    item
}
fn permits_buffer(capacity: QueueCapacity, len: usize) -> bool {
    match capacity {
        QueueCapacity::Rendezvous => false,
        QueueCapacity::Bounded(max) => len < max.get(),
        QueueCapacity::Unbounded => true,
    }
}
enum Admission<B> {
    Ready(Exit<B, QueueError>),
    Pending(Position),
}
impl<T: Send + 'static> PreparedQueue<T> {
    fn admit_offer(
        &self,
        item: PreparedQueueItem<T>,
        entry: ControlOwner<()>,
        probe: Probe,
        scratch: &mut Scratch<T>,
    ) -> Admission<()> {
        let mut state = self
            .state
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let requested = probe.is_requested();
        if requested || state.closed {
            let exit = if requested {
                Exit::Interrupted
            } else {
                Exit::Fail(QueueError::Shutdown)
            };
            scratch.discard.push_reserved(item);
            drop(state);
            drop(probe);
            drop(entry);
            return Admission::Ready(exit);
        }
        while let Some(taker) = state.takers.pop_front() {
            if taker.probe.is_requested() {
                complete_taker(taker, scratch, Exit::Interrupted);
            } else {
                complete_taker(taker, scratch, Exit::Success(item));
                // This control is not a registered waiter and must not wake.
                drop(state);
                drop(probe);
                drop(entry);
                return Admission::Ready(Exit::Success(()));
            }
        }
        if permits_buffer(state.capacity, state.buffer.len()) {
            state.buffer.push_reserved(item);
            drop(state);
            drop(probe);
            drop(entry);
            Admission::Ready(Exit::Success(()))
        } else {
            Admission::Pending(insert_waiter(
                &mut state.offers,
                Offer {
                    item,
                    control: entry,
                    probe,
                },
            ))
        }
    }
    #[allow(clippy::unreachable)] // Each non-pending branch constructs an outcome.
    fn admit_take(
        &self,
        entry: ControlOwner<PreparedQueueItem<T>>,
        probe: Probe,
        scratch: &mut Scratch<T>,
    ) -> Admission<PreparedQueueItem<T>> {
        let mut state = self
            .state
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let outcome = if probe.is_requested() {
            Some(Exit::Interrupted)
        } else if let Some(item) = state.buffer.remove_first() {
            if !state.closed
                && let Some(offer) = live_offer(&mut state, scratch)
            {
                let replacement = complete_offer(offer, scratch);
                state.buffer.push_reserved(replacement);
            }
            Some(Exit::Success(item))
        } else if !state.closed {
            if let Some(offer) = live_offer(&mut state, scratch) {
                Some(Exit::Success(complete_offer(offer, scratch)))
            } else {
                return Admission::Pending(insert_waiter(
                    &mut state.takers,
                    Taker {
                        control: entry,
                        probe,
                    },
                ));
            }
        } else {
            Some(Exit::Fail(QueueError::Shutdown))
        };
        let drained = state.closed && state.buffer.is_empty();
        drop(state);
        if drained {
            let retired = self
                .state
                .shutdown
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(Shutdown {
                scratch,
                cleanup,
                spare,
            }) = retired
            {
                drop(ScratchOwner {
                    slot: Some(scratch),
                });
                drop(cleanup);
                drop(spare);
            }
        }
        drop(probe);
        drop(entry);
        match outcome {
            Some(value) => Admission::Ready(value),
            None => unreachable!("take admission produces an outcome"),
        }
    }
    fn withdraw_offer(
        &self,
        position: Position,
        control: &ControlOwner<()>,
        scratch: &mut Scratch<T>,
    ) {
        let mut state = self
            .state
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = state
            .offers
            .remove_where(position, |entry| entry.control.same(control))
        {
            fail_offer(entry, scratch, Exit::Interrupted);
        }
    }
    fn withdraw_take(
        &self,
        position: Position,
        control: &ControlOwner<PreparedQueueItem<T>>,
        scratch: &mut Scratch<T>,
    ) {
        let mut state = self
            .state
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = state
            .takers
            .remove_where(position, |entry| entry.control.same(control))
        {
            complete_taker(entry, scratch, Exit::Interrupted);
        }
    }
}

impl<T: Send + 'static, F: Send + 'static> std::future::Future for PreparedQueueOffer<T, F> {
    type Output = Exit<(), QueueError>;
    #[allow(clippy::expect_used)] // Every option is taken once, under the future's unique poll authority.
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = &mut *self;
        if let Some(factory) = this.factory.take() {
            if this.cancellation.is_requested() {
                this.done = true;
                return std::task::Poll::Ready(this.resources.finish(Exit::Interrupted));
            }
            let panic = this
                .resources
                .scratch
                .get_mut()
                .panics
                .remove_first()
                .expect("admitted factory diagnostic");
            let value = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (this.apply)(factory)
            })) {
                Ok(Ok(value)) => {
                    this.resources.scratch.get_mut().panics.push_reserved(panic);
                    value
                }
                Ok(Err(defect)) => {
                    this.resources.scratch.get_mut().panics.push_reserved(panic);
                    this.done = true;
                    return std::task::Poll::Ready(this.resources.finish(Exit::Die(defect)));
                }
                Err(payload) => {
                    this.done = true;
                    return std::task::Poll::Ready(
                        this.resources.finish(Exit::Die(panic.capture(payload))),
                    );
                }
            };
            let mut cell = this.payload.take().expect("admitted payload");
            *cell = Some(value);
            let item = PreparedQueueItem { cell };
            let entry = this.entry.take().expect("admitted offer entry");
            let probe = this.probe.take().expect("admitted offer probe");
            match this
                .queue
                .admit_offer(item, entry, probe, this.resources.scratch.get_mut())
            {
                Admission::Ready(outcome) => {
                    this.done = true;
                    return std::task::Poll::Ready(this.resources.finish(outcome));
                }
                Admission::Pending(position) => {
                    this.position = Some(position);
                    let result = this.resources.finish(Exit::Success(()));
                    if !matches!(result, Exit::Success(())) {
                        this.queue.withdraw_offer(
                            position,
                            &this.control,
                            this.resources.scratch.get_mut(),
                        );
                        this.position = None;
                        this.done = true;
                        let committed = this.control.take();
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            drop(committed)
                        }));
                        return std::task::Poll::Ready(this.resources.finish(result));
                    }
                }
            }
        }
        let listener = this.listener.as_ref().expect("admitted listener");
        listener.reset();
        let _ = listener.poll(cx);
        this.cancellation.poll(cx);
        if let Some(outcome) = this.control.take() {
            this.position = None;
            this.done = true;
            return std::task::Poll::Ready(this.resources.finish(outcome));
        }
        if this.cancellation.is_requested() {
            if let Some(position) = this.position.take() {
                this.queue.withdraw_offer(
                    position,
                    &this.control,
                    this.resources.scratch.get_mut(),
                );
            }
            if let Some(outcome) = this.control.take() {
                this.done = true;
                return std::task::Poll::Ready(this.resources.finish(outcome));
            }
        }
        std::task::Poll::Pending
    }
}
impl<T: Send + 'static> std::future::Future for PreparedQueueTake<T> {
    type Output = Exit<PreparedQueueItem<T>, QueueError>;
    #[allow(clippy::expect_used, clippy::unreachable)] // Options belong to a unique admitted future; cleanup failure cannot become success.
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = &mut *self;
        if !this.started {
            this.started = true;
            let entry = this.entry.take().expect("admitted take entry");
            let probe = this.probe.take().expect("admitted take probe");
            match this
                .queue
                .admit_take(entry, probe, this.resources.scratch.get_mut())
            {
                Admission::Ready(outcome) => {
                    this.done = true;
                    return std::task::Poll::Ready(this.resources.finish(outcome));
                }
                Admission::Pending(position) => {
                    this.position = Some(position);
                    let result = this.resources.finish(Exit::Success(()));
                    if !matches!(result, Exit::Success(())) {
                        this.queue.withdraw_take(
                            position,
                            &this.control,
                            this.resources.scratch.get_mut(),
                        );
                        this.position = None;
                        this.done = true;
                        let committed = this.control.take();
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            drop(committed)
                        }));
                        return std::task::Poll::Ready(match this.resources.finish(result) {
                            Exit::Success(()) => unreachable!("cleanup failure remains a failure"),
                            Exit::Fail(error) => Exit::Fail(error),
                            Exit::Die(defect) => Exit::Die(defect),
                            Exit::Interrupted => Exit::Interrupted,
                            Exit::FinalizationFailed(details) => Exit::FinalizationFailed(details),
                        });
                    }
                }
            }
        }
        let listener = this.listener.as_ref().expect("admitted listener");
        listener.reset();
        let _ = listener.poll(cx);
        this.cancellation.poll(cx);
        if let Some(outcome) = this.control.take() {
            this.position = None;
            this.done = true;
            return std::task::Poll::Ready(this.resources.finish(outcome));
        }
        if this.cancellation.is_requested() {
            if let Some(position) = this.position.take() {
                this.queue
                    .withdraw_take(position, &this.control, this.resources.scratch.get_mut());
            }
            if let Some(outcome) = this.control.take() {
                this.done = true;
                return std::task::Poll::Ready(this.resources.finish(outcome));
            }
        }
        std::task::Poll::Pending
    }
}
impl<T: Send + 'static> PreparedQueueReceiver<T> {
    /// Poll the next item using the same preadmitted control, listener,
    /// cleanup carrier and diagnostics. A non-success outcome is terminal.
    /// Removed entry aliases held by older notifications can only wake; they
    /// cannot write the new outcome after their queue-gated removal.
    #[doc(hidden)]
    pub fn poll_take(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<Exit<PreparedQueueItem<T>, QueueError>, MemoryError>> {
        if self.terminal {
            return std::task::Poll::Ready(Err(MemoryError::Closed));
        }
        if self.rearm {
            // Preparation checked the maximum private taker/notification
            // owner count; this checked retain allocates no backing or control.
            self.take.entry = Some(self.take.control.try_clone()?);
            self.take.probe = Some(Probe::Uncancelled);
            self.take.started = false;
            self.take.done = false;
            self.rearm = false;
        }
        match std::future::Future::poll(std::pin::Pin::new(&mut self.take), cx) {
            std::task::Poll::Pending => std::task::Poll::Pending,
            std::task::Poll::Ready(outcome) => {
                self.rearm = matches!(&outcome, Exit::Success(_));
                self.terminal = !self.rearm;
                std::task::Poll::Ready(Ok(outcome))
            }
        }
    }
}
impl<T: Send + 'static, F: Send + 'static> Drop for PreparedQueueOffer<T, F> {
    fn drop(&mut self) {
        if !self.done {
            if let Some(position) = self.position.take() {
                self.queue.withdraw_offer(
                    position,
                    &self.control,
                    self.resources.scratch.get_mut(),
                );
                drop(self.resources.finish(Exit::Success(())));
            }
            let outcome = self.control.take();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(outcome)));
        }
    }
}
impl<T: Send + 'static> Drop for PreparedQueueTake<T> {
    fn drop(&mut self) {
        if !self.done {
            if let Some(position) = self.position.take() {
                self.queue
                    .withdraw_take(position, &self.control, self.resources.scratch.get_mut());
                drop(self.resources.finish(Exit::Success(())));
            }
            let outcome = self.control.take();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(outcome)));
        }
    }
}

impl<T: Send + 'static> PreparedQueue<T> {
    /// Reopen a closed, drained queue only when this is the sole root owner.
    /// The independent shutdown backing is reacquired before any state change;
    /// escaped item cells retain their own physical pool slots.
    #[doc(hidden)]
    #[allow(clippy::expect_used)] // Complete resources hold their exact reserved fields.
    pub fn rearm_quiescent(&mut self) -> Result<(), MemoryError> {
        if !self.state.is_unique() {
            return Err(MemoryError::Busy);
        }
        {
            let state = self
                .state
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !state.closed
                || !state.buffer.is_empty()
                || !state.offers.is_empty()
                || !state.takers.is_empty()
            {
                return Err(MemoryError::Busy);
            }
        }
        if self
            .state
            .shutdown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
        {
            return Err(MemoryError::Busy);
        }
        let demand = Self::demand(
            self.state
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .capacity,
            self.state.capacity,
        )?;
        let mut resources = self.reserve_resources(demand.shutdown_panic_slots)?;
        let spare = self.state.cleanup.reserve_failures()?;
        let mut state = self
            .state
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut shutdown = self
            .state
            .shutdown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.closed
            || !state.buffer.is_empty()
            || !state.offers.is_empty()
            || !state.takers.is_empty()
            || shutdown.is_some()
        {
            return Err(MemoryError::Busy);
        }
        let cleanup = resources.cleanup.take().expect("reserved shutdown cleanup");
        let scratch = resources
            .scratch
            .slot
            .take()
            .expect("reserved shutdown scratch");
        *shutdown = Some(Shutdown {
            scratch,
            cleanup,
            spare: Some(spare),
        });
        state.closed = false;
        Ok(())
    }
    /// Close admission and settle pending operations using independently reserved shutdown backing.
    /// Drain preserves committed buffered items for subsequent takes.
    pub fn shutdown_now(
        &self,
        mode: crate::QueueShutdown,
    ) -> Result<Exit<(), QueueError>, MemoryError> {
        let no_work = |state: &State<T>| {
            state.closed && (matches!(mode, crate::QueueShutdown::Drain) || state.buffer.is_empty())
        };
        if no_work(
            &self
                .state
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        ) {
            return Ok(Exit::Success(()));
        }
        let reserved = self
            .state
            .shutdown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(reserved) = reserved else {
            if no_work(
                &self
                    .state
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ) {
                return Ok(Exit::Success(()));
            }
            return Err(MemoryError::Busy);
        };
        let mut guard = ShutdownGuard {
            state: &self.state.state,
            owner: &self.state.shutdown,
            resources: Some(Resources {
                scratch: ScratchOwner {
                    slot: Some(reserved.scratch),
                },
                cleanup: Some(reserved.cleanup),
            }),
            spare: reserved.spare,
            retain: true,
        };
        #[allow(clippy::expect_used)]
        // Guard owns resources until its destructor restores or retires them.
        let resources = guard.resources.as_mut().expect("shutdown resources");
        let mut state = self
            .state
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if no_work(&state) {
            drop(state);
            return Ok(Exit::Success(()));
        }
        state.closed = true;
        if matches!(mode, crate::QueueShutdown::Discard) {
            while let Some(item) = state.buffer.remove_first() {
                resources.scratch.get_mut().discard.push_reserved(item);
            }
        }
        while let Some(offer) = state.offers.pop_front() {
            let outcome = if offer.probe.is_requested() {
                Exit::Interrupted
            } else {
                Exit::Fail(QueueError::Shutdown)
            };
            fail_offer(offer, resources.scratch.get_mut(), outcome);
        }
        while let Some(taker) = state.takers.pop_front() {
            let outcome = if taker.probe.is_requested() {
                Exit::Interrupted
            } else {
                Exit::Fail(QueueError::Shutdown)
            };
            complete_taker(taker, resources.scratch.get_mut(), outcome);
        }
        let remaining = !state.buffer.is_empty();
        drop(state);
        let outcome = resources.finish(Exit::Success(()));
        guard.retain = remaining;
        Ok(outcome)
    }
}
