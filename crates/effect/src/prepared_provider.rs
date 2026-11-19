//! Typed provider submission with physical, startup-prepared storage.
//!
//! This compatible backend does not call Tokio spawn, Rayon HeapJob, or the
//! native runtime's allocating submit API. Ordinary providers remain available.
//! Each reservation covers one queue cell, concrete future/capture, completion,
//! cancellation flag, notification registration and retained result. Worker
//! stacks and thread/waker controls are separately budgeted native storage.
//! Arbitrary future/output pointees are NOT covered by their inline layouts.
#![doc = include_str!("prepared_provider/README.md")]

use crate::internal::destruction::{DestructionNode, enqueue_prepared};
use crate::memory::{
    Bytes, FixedVector, FrameReservation, MemoryError, MemoryPolicy, PreparedMemory, RegionId,
    SharedSlot, SlotLease, SlotPool,
};
use std::any::Any;
use std::cell::UnsafeCell;
use std::future::Future;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, JoinHandle};

mod context;
mod diagnostic;
mod task;
pub use context::{PlainWorker, ProviderWorkerContext, SynchronousJob};
pub use diagnostic::{
    PreparedDiagnosticLease, ProviderBudget, ProviderDiagnostic, ProviderDiagnosticSinkLease,
    ProviderDiagnosticSite,
};
pub use task::{
    BlockingProvider, ConstructionProvider, CpuProvider, HasPreparedProvider,
    PreparedProviderConstruction, PreparedTaskFuture, TaskProvider,
};

/// A lease for the separately budgeted transitive working set. Implementors
/// must keep capture/input/output pointee and dependency allocation budgets
/// alive through destruction of the last result consumer. This is a native
/// contract, not a claim that size_of::<Self/F/T>() covers those allocations.
///
/// # Safety
/// The named contract must bound all storage outside this module's region and
/// configured worker stacks, including callback/future internals and any cloned
/// caller waker. No unbounded or System fallback is allowed under that contract.
/// The implementation must retain the actual backing/budget, not just a count.
pub unsafe trait ProviderWorkingSet: Send + 'static {
    const CONTRACT: &'static str;
}

/// Explicit native startup envelope. The application separately budgets the
/// fixed number of OS threads, one Arc wake control per capacity cell,
/// mutex/condvar OS internals and one startup handshake per worker under
/// `contract`. JoinHandle and worker-panic pointer arrays use region backing.
/// These opaque
/// platform costs are not reported as region bytes or pointer-sized proofs.
#[derive(Clone, Copy, Debug)]
pub struct ProviderNativeEnvelope {
    pub workers: usize,
    pub stack_bytes: usize,
    pub contract: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub enum ProviderRefusal {
    Memory(MemoryError),
    Closed,
    InvalidEnvelope,
    InsufficientWorkers,
}
impl From<MemoryError> for ProviderRefusal {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

/// Raw owner-thread failures retain their startup-provisioned result storage;
/// no formatting, default diagnostic loss, or native copying is performed.
pub struct ProviderWorkerPanics(Option<FixedVector<'static, Box<dyn Any + Send>>>);
impl ProviderWorkerPanics {
    #[must_use]
    pub fn payloads(&self) -> &[Box<dyn Any + Send>] {
        match &self.0 {
            Some(values) => values.as_slice(),
            None => &[],
        }
    }
}
impl std::fmt::Debug for ProviderWorkerPanics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderWorkerPanics")
            .field("count", &self.payloads().len())
            .finish()
    }
}
#[derive(Debug)]
pub enum ProviderStartupError {
    Reservation(ProviderRefusal),
    DuringStartup(ProviderRefusal, ProviderWorkerPanics),
    Thread(std::io::Error, ProviderWorkerPanics),
    Context(Box<dyn Any + Send>, ProviderWorkerPanics),
    WorkerStartupLost(ProviderWorkerPanics),
}
impl From<MemoryError> for ProviderStartupError {
    fn from(error: MemoryError) -> Self {
        Self::Reservation(error.into())
    }
}

/// Panic payloads are retained without copying/formatting or selecting a new
/// diagnostic saturation policy. Expected errors (including E = !) stay in T.
pub enum ProviderSettlement<T> {
    Returned(T),
    Panicked(Box<dyn Any + Send>),
}

struct CancelRecord {
    requested: AtomicBool,
    settled: AtomicBool,
    worker: Mutex<Option<Waker>>,
}
/// Cooperative request authority with separately prepared notification backing.
/// Retaining a token retains its cancellation slot, never a future/result cycle.
pub struct PreparedCancellation {
    record: SharedSlot<'static, CancelRecord>,
}
impl PreparedCancellation {
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            record: self.record.try_clone()?,
        })
    }
    #[must_use]
    pub fn is_requested(&self) -> bool {
        self.record.requested.load(Ordering::Acquire)
    }
    pub fn request(&self) {
        self.record.requested.store(true, Ordering::Release);
        let waker = lock(&self.record.worker).as_ref().cloned();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
/// Exact-layout execution ownership (for example native parent/drain permits).
/// Destruction uses the prepared FlatDrop lane, with no erased heap allocation.
/// Unlike result ownership, it ends only after completion publication/notification.
pub struct PreparedExecutionLease {
    _frame: crate::memory::RuntimeFrame,
}
impl PreparedExecutionLease {
    pub fn new<T: Send + 'static>(reservation: FrameReservation<'static, T>, value: T) -> Self {
        Self {
            _frame: reservation.erase(value),
        }
    }
}
struct State<T, D> {
    result: Option<ProviderSettlement<T>>,
    budget: Option<D>,
    graph_origins: Option<crate::internal::runtime::NativeGraphOrigins<D>>,
    waiter: Option<Waker>,
    // The entry's actual cancellation/control slot survives result/budget
    // descendant retirement, independent of any observer's field-drop order.
    control: Option<PreparedCancellation>,
    post_retirement: Option<PreparedExecutionLease>,
}
struct Entry<F: 'static, T: 'static, D: 'static> {
    state: Mutex<State<T, D>>,
    future: UnsafeCell<MaybeUninit<F>>,
    initialized: AtomicBool,
    signal: Arc<Signal<F, T, D>>,
    retirement: UnsafeCell<DestructionNode>,
    retiring_owner: UnsafeCell<MaybeUninit<SlotLease<'static, Entry<F, T, D>>>>,
}
// SAFETY: future access is private: unique prepublication reservation places F;
// one Work owner leaves the locked queue to poll/drop F in place. Last-owner
// retirement is exclusive and cannot overlap a Work owner. Shared receivers
// access only locked result state, never F. F/T/D must transfer between workers.
unsafe impl<F: Send, T: Send, D: Send> Send for Entry<F, T, D> {}
// SAFETY: the same unique queue-owner protocol serializes all future mutation;
// result/budget/waiter state is separately mutex protected, even for !Sync T/D.
unsafe impl<F: Send, T: Send, D: Send> Sync for Entry<F, T, D> {}
impl<F, T, D> Entry<F, T, D> {
    unsafe fn clear_future(&self) {
        if self.initialized.swap(false, Ordering::AcqRel) {
            // SAFETY: caller owns the private Work or unique final retirement.
            // Clear before user Drop, so unwinding never leaves a live flag.
            unsafe { std::ptr::drop_in_place((*self.future.get()).as_mut_ptr()) };
        }
    }
}
struct RetireState<'a, F: 'static, T: 'static, D: 'static>(&'a Entry<F, T, D>);
impl<F, T, D> Drop for RetireState<'_, F, T, D> {
    fn drop(&mut self) {
        let payloads = {
            let mut state = lock(&self.0.state);
            (state.result.take(), state.waiter.take())
        };
        drop(payloads);
    }
}

unsafe fn retire_entry<F: 'static, T: 'static, D: 'static>(node: NonNull<DestructionNode>) {
    // SAFETY: EntryOwner last-owner transfer installed this exact live entry.
    let entry = unsafe { &*DestructionNode::context(node).cast::<Entry<F, T, D>>() };
    let result = RetireState(entry);
    // SAFETY: no Work or observer remains; flag clears before user destruction.
    unsafe { entry.clear_future() };
    drop(result);
}
unsafe fn retire_budget<F: 'static, T: 'static, D: 'static>(node: NonNull<DestructionNode>) {
    // SAFETY: payload and all nested prepared descendants finished; entry is
    // still retained, and no result observer or work owner can access it.
    let entry = unsafe { &*DestructionNode::context(node).cast::<Entry<F, T, D>>() };
    let (budget, origins) = {
        let mut state = lock(&entry.state);
        (state.budget.take(), state.graph_origins.take())
    };
    match origins {
        Some(origins) => origins.retire(budget),
        None => drop(budget),
    }
}
unsafe fn release_entry<F: 'static, T: 'static, D: 'static>(node: NonNull<DestructionNode>) {
    // SAFETY: both payload/footer descendant trees have finished; consume the
    // unique queued owner once. Its pool storage may disappear during Drop.
    unsafe {
        let entry = &*DestructionNode::context(node).cast::<Entry<F, T, D>>();
        let (control, post_retirement) = {
            let mut state = lock(&entry.state);
            (state.control.take(), state.post_retirement.take())
        };
        let owner = (*entry.retiring_owner.get()).assume_init_read();
        drop(owner);
        drop(control);
        // Real F/T/D payloads and both descendant phases, PLUS entry/control
        // return, precede a serving actor's final settlement notification.
        drop(post_retirement);
    }
}

// SharedSlot normally preserves initialized values between checkouts. This
// wrapper retires all operation payloads outside the pool metadata lock before
// returning the final slot, including on abandoned receivers and panic unwind.
struct EntryOwner<F: 'static, T: 'static, D: 'static>(
    ManuallyDrop<SharedSlot<'static, Entry<F, T, D>>>,
);
impl<F, T, D> EntryOwner<F, T, D> {
    fn entry(&self) -> &Entry<F, T, D> {
        &self.0
    }
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self(ManuallyDrop::new(self.0.try_clone()?)))
    }
}
impl<F, T, D> Drop for EntryOwner<F, T, D> {
    fn drop(&mut self) {
        // SAFETY: Drop consumes the one live owner exactly once; no access to
        // self.0 follows this transfer, including on payload unwind.
        let owner = unsafe { ManuallyDrop::take(&mut self.0) };
        if let Some(slot) = owner.into_last() {
            // SAFETY: the unique last-owner slot has no Work/readers. Actual
            // constructors require Send futures/results/budgets; the queued
            // record retains that exact backing until postorder retirement.
            unsafe {
                let entry = &*slot;
                let node = NonNull::new_unchecked(entry.retirement.get());
                (*node.as_ptr()).set_context(std::ptr::from_ref(entry).cast_mut().cast());
                (*entry.retiring_owner.get()).write(slot);
                enqueue_prepared(node);
            }
        }
    }
}

struct Work<F: 'static, T: 'static, D: 'static> {
    entry: EntryOwner<F, T, D>,
    cancellation: PreparedCancellation,
    execution: Option<PreparedExecutionLease>,
    diagnostics: (
        PreparedDiagnosticLease,
        PreparedDiagnosticLease,
        PreparedDiagnosticLease,
    ),
}
struct QueueCell<F: 'static, T: 'static, D: 'static> {
    occupied: bool,
    work: Option<Work<F, T, D>>,
}
struct Queue<F: 'static, T: 'static, D: 'static> {
    cells: FixedVector<'static, QueueCell<F, T, D>>,
    next: usize,
    stopped: bool,
    preparing: usize,
    running: usize,
}
struct Control<F: 'static, T: 'static, D: 'static> {
    queue: Mutex<Queue<F, T, D>>,
    changed: Condvar,
}
struct Signal<F: 'static, T: 'static, D: 'static> {
    ready: AtomicBool,
    control: SharedSlot<'static, Control<F, T, D>>,
}
impl<F: Send, T: Send, D: Send> Wake for Signal<F, T, D> {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        // Synchronize with the worker's condition check to avoid lost wakeups.
        let queue = lock(&self.control.queue);
        self.ready.store(true, Ordering::Release);
        drop(queue);
        self.control.changed.notify_one();
    }
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn wait<'a, T>(condvar: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    condvar
        .wait(guard)
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Shutdown authority. Dropping requests stop but never cancels or discards
/// accepted work. `join` is the synchronous owner boundary; use outside poll
/// workers. Submission handles do not have shutdown authority.
pub struct PreparedProvider<F: 'static, T: 'static, D: 'static> {
    handle: PreparedSubmitter<F, T, D>,
    workers: FixedVector<'static, JoinHandle<()>>,
    worker_panics: ProviderWorkerPanics,
}
/// Typed submission authority. No erased Box/Arc callback is constructed before
/// physical reservation. Sharing is explicit/fallible, without a new Clone rule.
pub struct PreparedSubmitter<F: 'static, T: 'static, D: 'static> {
    control: SharedSlot<'static, Control<F, T, D>>,
    entries: SlotPool<'static, Entry<F, T, D>>,
    cancellations: SlotPool<'static, CancelRecord>,
    policy: MemoryPolicy,
}

impl<F, T, D> PreparedProvider<F, T, D>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    D: ProviderWorkingSet,
{
    /// Provision all region storage and start exactly the declared workers.
    /// Any partial startup is stopped and joined before returning an error.
    /// PreparedMemory determines the allocation policy; after sealing, this
    /// backend only reuses slots, even in bounded-allocation mode. No fallback
    /// to an ordinary provider or to native spawn occurs on refusal.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        native: ProviderNativeEnvelope,
    ) -> Result<Self, ProviderStartupError> {
        if capacity == 0
            || native.workers == 0
            || native.stack_bytes == 0
            || native.contract.is_empty()
            || D::CONTRACT.is_empty()
        {
            return Err(ProviderStartupError::Reservation(
                ProviderRefusal::InvalidEnvelope,
            ));
        }
        Self::prepare_with_context(memory, region, capacity, native, PlainWorker)
    }
    /// Same physical backend, with an explicit provider-owned worker context.
    /// The context is installed on the worker thread, never carried as a guard
    /// from the submitting/polling thread.
    pub fn prepare_with_context<C: ProviderWorkerContext>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        native: ProviderNativeEnvelope,
        context: C,
    ) -> Result<Self, ProviderStartupError> {
        if capacity == 0
            || native.workers == 0
            || native.stack_bytes == 0
            || native.contract.is_empty()
            || D::CONTRACT.is_empty()
        {
            return Err(ProviderStartupError::Reservation(
                ProviderRefusal::InvalidEnvelope,
            ));
        }
        let cancellations = memory.slots(region, capacity, |_| {
            Ok(CancelRecord {
                requested: AtomicBool::new(false),
                settled: AtomicBool::new(false),
                worker: Mutex::new(None),
            })
        })?;
        let mut cells = FixedVector::prepare(memory.allocator(region)?, capacity)?;
        for _ in 0..capacity {
            cells.push_reserved(QueueCell {
                occupied: false,
                work: None,
            });
        }
        let mut cells = Some(cells);
        let controls = memory.slots(region, 1, |_| {
            Ok(Control {
                queue: Mutex::new(Queue {
                    cells: cells.take().ok_or(MemoryError::NoSlot)?,
                    next: 0,
                    stopped: false,
                    preparing: 0,
                    running: 0,
                }),
                changed: Condvar::new(),
            })
        })?;
        let control = controls.checkout()?.share();
        let entries = memory.slots(region, capacity, |_| {
            Ok(Entry {
                state: Mutex::new(State {
                    result: None,
                    budget: None,
                    graph_origins: None,
                    waiter: None,
                    control: None,
                    post_retirement: None,
                }),
                future: UnsafeCell::new(MaybeUninit::uninit()),
                initialized: AtomicBool::new(false),
                retirement: UnsafeCell::new(DestructionNode::new(
                    retire_entry::<F, T, D>,
                    retire_budget::<F, T, D>,
                    release_entry::<F, T, D>,
                )),
                retiring_owner: UnsafeCell::new(MaybeUninit::uninit()),
                // One native wake control per physical cell, only at startup.
                signal: Arc::new(Signal {
                    ready: AtomicBool::new(false),
                    control: control.try_clone()?,
                }),
            })
        })?;
        let mut provider = Self {
            handle: PreparedSubmitter {
                control,
                entries,
                cancellations,
                policy: memory.policy(),
            },
            workers: FixedVector::prepare(memory.allocator(region)?, native.workers)?,
            worker_panics: ProviderWorkerPanics(Some(FixedVector::prepare(
                memory.allocator(region)?,
                native.workers,
            )?)),
        };
        for _ in 0..native.workers {
            let control = match provider.handle.control.try_clone() {
                Ok(control) => control,
                Err(error) => {
                    provider.join();
                    return Err(ProviderStartupError::DuringStartup(
                        error.into(),
                        ProviderWorkerPanics(provider.worker_panics.0.take()),
                    ));
                }
            };
            let context = match catch_unwind(AssertUnwindSafe(|| context.clone())) {
                Ok(context) => context,
                Err(payload) => {
                    provider.join();
                    return Err(ProviderStartupError::Context(
                        payload,
                        ProviderWorkerPanics(provider.worker_panics.0.take()),
                    ));
                }
            };
            // One bounded startup-only handshake under the native envelope.
            // No submission handle escapes before every worker context enters.
            let (started, startup) = std::sync::mpsc::sync_channel(1);
            match thread::Builder::new()
                .stack_size(native.stack_bytes)
                .spawn(
                    move || match catch_unwind(AssertUnwindSafe(|| context.enter())) {
                        Ok(guard) => {
                            if started.send(Ok(())).is_ok() {
                                worker(control);
                            }
                            drop(guard);
                        }
                        Err(payload) => {
                            let _ = started.send(Err(payload));
                        }
                    },
                ) {
                Ok(worker) => provider.workers.push_reserved(worker),
                Err(error) => {
                    provider.join();
                    return Err(ProviderStartupError::Thread(
                        error,
                        ProviderWorkerPanics(provider.worker_panics.0.take()),
                    ));
                }
            }
            match startup.recv() {
                Ok(Ok(())) => {}
                Ok(Err(payload)) => {
                    provider.join();
                    return Err(ProviderStartupError::Context(
                        payload,
                        ProviderWorkerPanics(provider.worker_panics.0.take()),
                    ));
                }
                Err(_) => {
                    provider.join();
                    return Err(ProviderStartupError::WorkerStartupLost(
                        ProviderWorkerPanics(provider.worker_panics.0.take()),
                    ));
                }
            }
        }
        Ok(provider)
    }
    pub fn submitter(&self) -> Result<PreparedSubmitter<F, T, D>, MemoryError> {
        self.handle.try_clone()
    }
}
impl<F, T, D> PreparedProvider<F, T, D> {
    /// Concrete inline region allocations for this backend. Allocator
    /// headers/padding appear additionally in MemoryUsage; worker stacks,
    /// native controls and F/T/D pointees require their explicit contracts.
    pub fn storage_bytes(capacity: usize, workers: usize) -> Result<Bytes, MemoryError> {
        fn array<T>(count: usize) -> Result<Bytes, MemoryError> {
            std::alloc::Layout::array::<T>(count)
                .map(|layout| Bytes::new(layout.size()))
                .map_err(|_| MemoryError::InvalidLayout)
        }
        SlotPool::<Entry<F, T, D>>::storage_bytes(capacity)?
            .checked_add(SlotPool::<CancelRecord>::storage_bytes(capacity)?)?
            .checked_add(SlotPool::<Control<F, T, D>>::storage_bytes(1)?)?
            .checked_add(array::<QueueCell<F, T, D>>(capacity)?)?
            .checked_add(array::<JoinHandle<()>>(workers)?)?
            .checked_add(array::<Box<dyn Any + Send>>(workers)?)
    }
    pub fn stop(&self) {
        lock(&self.handle.control.queue).stopped = true;
        self.handle.entries.close();
        self.handle.cancellations.close();
        self.handle.control.changed.notify_all();
    }
    /// Waits for accepted execution, not retained consumer output. Output leases
    /// can outlive shutdown and continue to retain their complete slot/budget.
    pub fn join(&mut self) {
        self.stop();
        while let Some(worker) = self.workers.pop() {
            if let Err(payload) = worker.join() {
                match &mut self.worker_panics.0 {
                    Some(failures) => failures.push_reserved(payload),
                    // Storage transfers into a startup error only after every
                    // handle was joined. Never silently lose a raw payload if
                    // that private ownership invariant is violated.
                    None => std::panic::resume_unwind(payload),
                }
            }
        }
    }
    /// Inspect raw worker/context failures after join. The owner retains all
    /// payloads until its own destruction; no diagnostic policy is selected.
    #[must_use]
    pub fn worker_panics(&self) -> &ProviderWorkerPanics {
        &self.worker_panics
    }
}
impl<F, T, D> Drop for PreparedProvider<F, T, D> {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A physical operation reservation acquired before capture/future construction.
/// Dropping it rolls back all owned resources. Stop rejects new reservations;
/// an accepted reservation can still hand off once, and shutdown waits for it.
/// Refusal is independent of the workflow's error type.
struct Preparation<F: 'static, T: 'static, D: 'static> {
    control: SharedSlot<'static, Control<F, T, D>>,
    armed: bool,
}
impl<F, T, D> Drop for Preparation<F, T, D> {
    fn drop(&mut self) {
        if self.armed {
            lock(&self.control.queue).preparing -= 1;
            self.control.changed.notify_all();
        }
    }
}
pub(crate) type ProviderDiagnostics = (
    PreparedDiagnosticLease,
    PreparedDiagnosticLease,
    PreparedDiagnosticLease,
);
#[must_use = "accepted preparation must hand off or roll back before shutdown settles"]
pub struct PreparedSubmission<F: 'static, T: 'static, D: 'static> {
    producer: EntryOwner<F, T, D>,
    receiver: EntryOwner<F, T, D>,
    cancellation: PreparedCancellation,
    worker_cancellation: PreparedCancellation,
    receiver_cancellation: PreparedCancellation,
    execution: Option<PreparedExecutionLease>,
    // Release accepted-preparation/drain ownership AFTER all rollback drops.
    preparation: Preparation<F, T, D>,
}
impl<F, T, D> PreparedSubmitter<F, T, D> {
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            control: self.control.try_clone()?,
            entries: self.entries.try_clone()?,
            cancellations: self.cancellations.try_clone()?,
            policy: self.policy,
        })
    }
    /// Explicit region allocation policy chosen by the supplied memory plan.
    /// This backend reuses its fixed typed records under either policy; bounded
    /// working-set allocations require a separate real lease before factories.
    #[must_use]
    pub fn memory_policy(&self) -> MemoryPolicy {
        self.policy
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.entries.active()
    }
    pub fn reserve(&self) -> Result<PreparedSubmission<F, T, D>, ProviderRefusal> {
        let control = self.control.try_clone()?;
        if lock(&control.queue).stopped {
            return Err(ProviderRefusal::Closed);
        }
        let entry = self.entries.try_checkout()?;
        let cancellation = self.cancellations.try_checkout()?;
        cancellation.requested.store(false, Ordering::Release);
        cancellation.settled.store(false, Ordering::Release);
        *lock(&cancellation.worker) = None;
        let cancellation = PreparedCancellation {
            record: cancellation.share(),
        };
        let worker_cancellation = cancellation.try_clone()?;
        let receiver_cancellation = cancellation.try_clone()?;
        let control_cancellation = cancellation.try_clone()?;
        lock(&entry.state).control = Some(control_cancellation);
        let producer = EntryOwner(ManuallyDrop::new(entry.share()));
        let receiver = producer.try_clone()?;
        {
            let mut queue = lock(&control.queue);
            if queue.stopped {
                return Err(ProviderRefusal::Closed);
            }
            // Each preparation retains one distinct pool slot, so this count
            // cannot exceed the already representable configured capacity.
            queue.preparing += 1;
        }
        Ok(PreparedSubmission {
            preparation: Preparation {
                control,
                armed: true,
            },
            producer,
            receiver,
            cancellation,
            worker_cancellation,
            receiver_cancellation,
            execution: None,
        })
    }
}
impl<F, T, D> PreparedSubmission<F, T, D> {
    /// Native actor receipt-retirement companion. Unlike execution publication,
    /// this owner survives all F/T/D descendants and actual entry/control return.
    /// It is acquired/installed before constructing the accepted native Future.
    pub fn with_post_retirement_lease(self, lease: PreparedExecutionLease) -> Self {
        let previous = lock(&self.producer.entry().state)
            .post_retirement
            .replace(lease);
        drop(previous);
        self
    }
    /// Attach a physically reserved settlement owner before constructing work.
    pub fn with_execution_lease(mut self, lease: PreparedExecutionLease) -> Self {
        let previous = self.execution.replace(lease);
        drop(previous);
        self
    }
}
impl<F, T, D> PreparedSubmission<F, T, D>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    D: ProviderWorkingSet,
{
    pub(crate) fn install_graph_origins(
        &self,
        origins: crate::internal::runtime::NativeGraphOrigins<D>,
    ) {
        // Fresh private submission, no Work/result aliases; installed once.
        lock(&self.producer.entry().state).graph_origins = Some(origins);
    }
    /// Private guard reserved before business factories. It never reads or
    /// extracts a result; it keeps D live through task-cell waiter operations
    /// even when another observer transfers the receipt concurrently.
    pub(crate) fn retain_graph_origin_preflight(
        &self,
    ) -> Result<PreparedProviderOutputLease<F, T, D>, MemoryError> {
        Ok(PreparedProviderOutputLease {
            _owner: self.receiver.try_clone()?,
            _settlement: self.receiver_cancellation.try_clone()?,
        })
    }
    /// Acquire the actual transitive working-set lease BEFORE constructing F.
    /// Both hooks are caller code under D's named native contract. A budget
    /// refusal runs no factory. No provider allocation occurs in this method.
    pub fn submit<X>(
        self,
        budget: impl FnOnce() -> Result<ProviderBudget<D>, X>,
        factory: impl FnOnce(&D, PreparedCancellation) -> F,
    ) -> Result<PreparedReceipt<F, T, D>, PreparedSubmitError<X>> {
        let diagnostics =
            self.install_graph_budget(budget().map_err(PreparedSubmitError::WorkingSet)?);
        self.submit_graph(diagnostics, factory)
            .map_err(PreparedSubmitError::Refused)
    }
    /// The generated graph installs D during native preflight, before even the
    /// outer descriptor factory. Ordinary submit uses this same installation.
    pub(crate) fn install_graph_budget(&self, budget: ProviderBudget<D>) -> ProviderDiagnostics {
        let ProviderBudget {
            working_set,
            notification,
            worker_retirement,
            execution_retirement,
        } = budget;
        // Fresh unpublished reservation, called exactly once by either submit
        // or PreparedProviderConstruction. No prior D or public result aliases.
        lock(&self.producer.entry().state).budget = Some(working_set);
        (notification, worker_retirement, execution_retirement)
    }
    pub(crate) fn submit_graph(
        self,
        diagnostics: ProviderDiagnostics,
        factory: impl FnOnce(&D, PreparedCancellation) -> F,
    ) -> Result<PreparedReceipt<F, T, D>, ProviderRefusal> {
        let (notification, worker_retirement, execution_retirement) = diagnostics;
        let entry = self.producer.entry();
        let working_set = {
            let mut state = lock(&entry.state);
            match state.budget.as_mut() {
                Some(budget) => NonNull::from(budget),
                None => return Err(MemoryError::UnsupportedRunContract.into()),
            }
        };
        // SAFETY: the unpublished submission exclusively controls budget writes.
        // No metadata lock spans user code; the borrow ends with this call. If
        // construction unwinds, EntryOwner's payload/descendant/footer retirement
        // owns D, including partially constructed prepared captures queued by Drop.
        let future = factory(unsafe { working_set.as_ref() }, self.cancellation);
        // SAFETY: no Work/receipt has been published; this unique submission
        // initializes exactly F at its final region-owned address before pinning.
        unsafe {
            (*entry.future.get()).write(future);
        }
        entry.initialized.store(true, Ordering::Release);
        let Self {
            mut preparation,
            producer,
            receiver,
            worker_cancellation,
            receiver_cancellation,
            execution,
            ..
        } = self;
        producer.entry().signal.ready.store(true, Ordering::Release);
        let work = Work {
            entry: producer,
            cancellation: worker_cancellation,
            execution,
            diagnostics: (notification, worker_retirement, execution_retirement),
        };
        let mut queue = lock(&preparation.control.queue);
        // Every pending/running/result operation owns a distinct entry; queue
        // capacity equals the entry pool. Thus an admitted entry has a cell.
        let Some(cell) = queue
            .cells
            .as_mut_slice()
            .iter_mut()
            .find(|cell| !cell.occupied)
        else {
            // No user drops while holding the provider queue mutex, even if an
            // internal capacity invariant is violated. Return a typed refusal.
            drop(queue);
            return Err(MemoryError::NoSlot.into());
        };
        cell.occupied = true;
        cell.work = Some(work);
        queue.preparing -= 1;
        preparation.armed = false;
        drop(queue);
        preparation.control.changed.notify_one();
        Ok(PreparedReceipt {
            owner: receiver,
            cancellation: receiver_cancellation,
        })
    }
}
#[derive(Debug)]
pub enum PreparedSubmitError<X> {
    Refused(ProviderRefusal),
    WorkingSet(X),
}

/// Caller ownership is separate from execution ownership. Drop requests
/// cooperative cancellation but the worker retains storage until actual return.
#[must_use = "dropping requests cancellation, not execution settlement"]
pub struct PreparedReceipt<F: 'static, T: 'static, D: 'static> {
    owner: EntryOwner<F, T, D>,
    cancellation: PreparedCancellation,
}
impl<F, T, D> PreparedReceipt<F, T, D> {
    /// Retain published output without requesting cancellation when this view
    /// is dropped. This clones only prepared owner metadata, not T or D.
    pub fn retain_result(&self) -> Result<Option<PreparedOutput<F, T, D>>, MemoryError> {
        if lock(&self.owner.entry().state).result.is_none() {
            return Ok(None);
        }
        Ok(Some(PreparedOutput {
            owner: self.owner.try_clone()?,
            settlement: self.cancellation.try_clone()?,
        }))
    }
    pub fn request_cancel(&self) {
        self.cancellation.request();
    }
    #[must_use]
    pub fn is_cancel_requested(&self) -> bool {
        self.cancellation.is_requested()
    }
    /// Await publication without transferring the result out of its backing.
    /// Waker clone/drop obeys D's separately budgeted notification contract.
    pub async fn settled(&mut self) {
        std::future::poll_fn(|cx| {
            let state = lock(&self.owner.entry().state);
            if state.result.is_some() {
                return Poll::Ready(());
            }
            if state
                .waiter
                .as_ref()
                .is_some_and(|w| w.will_wake(cx.waker()))
            {
                return Poll::Pending;
            }
            drop(state);
            // Caller Waker operations can allocate/panic/reenter under the
            // selected native contract, never under provider metadata locks.
            let waiter = cx.waker().clone();
            let mut state = lock(&self.owner.entry().state);
            if state.result.is_some() {
                drop(state);
                drop(waiter);
                return Poll::Ready(());
            }
            let previous = state.waiter.replace(waiter);
            drop(state);
            drop(previous);
            Poll::Pending
        })
        .await
    }
    /// Actual execution/queue retirement, not merely result publication. Shares
    /// the receipt's single waiter; callers must not poll two observations at once.
    /// Native serving/drain companion: wait for real execution and queue
    /// retirement, not merely publication. Uses this receipt's one waiter and
    /// retains its actual output/working-set owner until the caller drops it.
    pub fn poll_execution_settled(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.poll_final(cx)
    }
    pub(crate) fn poll_final(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let state = lock(&self.owner.entry().state);
        if self.cancellation.record.settled.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        if state
            .waiter
            .as_ref()
            .is_some_and(|w| w.will_wake(cx.waker()))
        {
            return Poll::Pending;
        }
        drop(state);
        let waiter = cx.waker().clone();
        let mut state = lock(&self.owner.entry().state);
        if self.cancellation.record.settled.load(Ordering::Acquire) {
            drop(state);
            drop(waiter);
            return Poll::Ready(());
        }
        let old = state.waiter.replace(waiter);
        drop(state);
        drop(old);
        Poll::Pending
    }
    /// # Safety
    /// Only graph-private task storage may call this after poll_final Ready,
    /// without ever exposing/cloning a public result observer. Worker result
    /// writes have ended before the final flag; its remaining owner only wakes
    /// and retires metadata. No public receipt/result alias may access T.
    pub(crate) unsafe fn into_owned_for_graph(self) -> PreparedOwnedProviderResult<F, T, D> {
        let this = ManuallyDrop::new(self);
        // SAFETY: consume the sole graph observer exactly once, avoiding caller
        // Drop cancellation after settlement; no later access to this follows.
        unsafe {
            PreparedOwnedProviderResult {
                lease: PreparedProviderOutputLease {
                    _owner: std::ptr::read(&this.owner),
                    _settlement: std::ptr::read(&this.cancellation),
                },
            }
        }
    }
    /// Nonblocking observation; result never moves out of its leased backing.
    #[must_use]
    pub fn result(&self) -> Option<ProviderResult<'_, T, D>> {
        let state = lock(&self.owner.entry().state);
        if state.result.is_some() {
            Some(ProviderResult { state })
        } else {
            None
        }
    }
}
impl<F, T, D> Drop for PreparedReceipt<F, T, D> {
    fn drop(&mut self) {
        self.request_cancel();
    }
}
/// Independently retained consumer of already-published output. The final
/// consumer releases the payload and its working-set budget together.
#[must_use = "retains output backing until its final consumer drops"]
pub struct PreparedOutput<F: 'static, T: 'static, D: 'static> {
    owner: EntryOwner<F, T, D>,
    settlement: PreparedCancellation,
}
impl<F, T, D> PreparedOutput<F, T, D> {
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            owner: self.owner.try_clone()?,
            settlement: self.settlement.try_clone()?,
        })
    }
    #[must_use]
    pub fn result(&self) -> ProviderResult<'_, T, D> {
        ProviderResult {
            state: lock(&self.owner.entry().state),
        }
    }
}

/// Physical provider result/budget owner used only by checked output transfer.
/// No public operation separates this owner from an extracted settlement.
pub struct PreparedProviderOutputLease<F: 'static, T: 'static, D: 'static> {
    _owner: EntryOwner<F, T, D>,
    _settlement: PreparedCancellation,
}
/// Exclusive finally-settled result view; the settlement stays in its actual
/// provider entry rather than being exposed as a bare owned T.
pub struct PreparedOwnedProviderResult<F: 'static, T: 'static, D: 'static> {
    lease: PreparedProviderOutputLease<F, T, D>,
}
impl<F, T, D> PreparedOutput<F, T, D> {
    /// Nonblocking/recoverable exclusive-view admission after final worker
    /// settlement. No T is moved, cloned, or mutated through another observer.
    pub fn try_take(self) -> Result<PreparedOwnedProviderResult<F, T, D>, Self> {
        if !self.settlement.record.settled.load(Ordering::Acquire) || !self.owner.0.is_unique() {
            return Err(self);
        }
        if lock(&self.owner.entry().state).result.is_none() {
            return Err(self);
        }
        Ok(PreparedOwnedProviderResult {
            lease: PreparedProviderOutputLease {
                _owner: self.owner,
                _settlement: self.settlement,
            },
        })
    }
}
impl<F, T, D> PreparedProviderOutputLease<F, T, D> {
    /// Borrow the settlement under its original provider entry gate.
    #[must_use]
    pub fn result(&self) -> ProviderResult<'_, T, D> {
        ProviderResult {
            state: lock(&self._owner.entry().state),
        }
    }
}
pub type PreparedProviderTransfer<F, T, D, A, E> = Result<
    crate::PreparedOutputOwnerView<PreparedProviderOutputLease<F, T, D>, A, E>,
    (MemoryError, PreparedOwnedProviderResult<F, T, D>),
>;
impl<F: Send, T: Send, D: Send> PreparedOwnedProviderResult<F, T, D> {
    #[must_use]
    pub fn settlement(&self) -> ProviderResult<'_, T, D> {
        self.lease.result()
    }
    /// Transfer the actual entry/D into a pre-reserved origin cell and return
    /// its pre-admitted output consumer. Only borrowed result access is exposed;
    /// neither concurrent output Drop nor context closure can detach its budget.
    pub fn transfer<A: Send + 'static, E: Send + 'static>(
        self,
        reservation: crate::PreparedOutputOwnerReservation<
            PreparedProviderOutputLease<F, T, D>,
            A,
            E,
        >,
    ) -> PreparedProviderTransfer<F, T, D, A, E> {
        match reservation.attach_retained(self.lease) {
            Ok(view) => Ok(view),
            Err((error, lease)) => Err((error, Self { lease })),
        }
    }
}

impl<F: Send, A: Send, D: Send> PreparedOwnedProviderResult<F, A, D> {
    /// Only the sealed provider-result graph operation calls this: its admission
    /// pre-holds the transfer cell/consumer and its Finish is supplied by the SAME
    /// live PreparedInvocation drive. This affine result was finally settled and
    /// uniquely checked by try_take; no public operation clones/removes its value.
    /// Install actual entry/D before moving non-Clone A into the private Finish.
    /// The temporary consumer spans publication and is then dropped, never stored
    /// as an internal same-root consumer inside the outcome or origin record.
    pub(crate) fn handoff<V: Send + 'static, E: Send + 'static>(
        self,
        reservation: crate::PreparedOutputOwnerReservation<
            PreparedProviderOutputLease<F, A, D>,
            V,
            E,
        >,
        context: &crate::PreparedRunContext<V, E>,
        panic: Option<crate::PreparedPanicReservation>,
        finish: crate::internal::runtime::Finish<A, E>,
    ) -> crate::internal::runtime::Step {
        let authority = reservation.install_for_invocation(self.lease);
        let settlement = lock(&authority._owner.entry().state).result.take();
        let outcome = match settlement {
            Some(ProviderSettlement::Returned(value)) => crate::Exit::Success(value),
            // Same native graph panic-to-Die boundary as existing native leaves;
            // diagnostic allocation remains its explicit unresolved contract.
            Some(ProviderSettlement::Panicked(payload)) => {
                crate::Exit::Die(crate::internal::runtime::panic_defect(panic, payload))
            }
            None => crate::Exit::Die(crate::Defect::from(
                crate::LibraryDefect::MissingInterpreterOutcome,
            )),
        };
        let next = finish
            .call(context.publish(outcome))
            .unwrap_or(crate::internal::runtime::Step::Done);
        drop(authority);
        next
    }
}

impl<F: Send, A: Send, E: Send, D: Send> PreparedOwnedProviderResult<F, crate::Exit<A, E>, D> {
    pub(crate) fn graph_driver_lost(&self) -> bool {
        match lock(&self.lease._owner.entry().state).result.as_ref() {
            Some(ProviderSettlement::Returned(_)) => false,
            Some(ProviderSettlement::Panicked(_)) | None => true,
        }
    }
    pub(crate) fn handoff_exit<V: Send + 'static>(
        self,
        reservation: crate::PreparedOutputOwnerReservation<
            PreparedProviderOutputLease<F, crate::Exit<A, E>, D>,
            V,
            E,
        >,
        context: &crate::PreparedRunContext<V, E>,
        panic: Option<crate::PreparedPanicReservation>,
        finish: crate::internal::runtime::Finish<A, E>,
    ) -> crate::internal::runtime::Step {
        let authority = reservation.install_for_invocation(self.lease);
        let settlement = lock(&authority._owner.entry().state).result.take();
        let outcome = match settlement {
            Some(ProviderSettlement::Returned(exit)) => exit,
            Some(ProviderSettlement::Panicked(payload)) => {
                crate::Exit::Die(crate::internal::runtime::panic_defect(panic, payload))
            }
            None => crate::Exit::Die(crate::Defect::from(
                crate::LibraryDefect::MissingInterpreterOutcome,
            )),
        };
        let next = finish
            .call(context.publish(outcome))
            .unwrap_or(crate::internal::runtime::Step::Done);
        drop(authority);
        next
    }
}

/// Borrow of immutable result backing. Owned sharing retains the same entry;
/// result payloads themselves need not implement Clone or Sync.
pub struct ProviderResult<'a, T, D> {
    state: MutexGuard<'a, State<T, D>>,
}
impl<T, D> ProviderResult<'_, T, D> {
    #[must_use]
    pub fn settlement(&self) -> Option<&ProviderSettlement<T>> {
        self.state.result.as_ref()
    }
}

// One polling actor owns this guard. Ready/panic drops F at its stable address;
// Pending disarms retirement so another worker can poll the same pinned cell.
struct PollFuture<'a, F: 'static, T: 'static, D: 'static> {
    entry: &'a Entry<F, T, D>,
    pending: bool,
}
impl<F, T, D> Drop for PollFuture<'_, F, T, D> {
    fn drop(&mut self) {
        if !self.pending {
            // SAFETY: the caller owns the one Work removed from its queue cell.
            unsafe { self.entry.clear_future() };
        }
    }
}
fn worker<F, T, D>(control: SharedSlot<'static, Control<F, T, D>>)
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    D: ProviderWorkingSet,
{
    loop {
        let (index, work) = {
            let mut queue = lock(&control.queue);
            loop {
                let len = queue.cells.len();
                let next = queue.next;
                let cells = queue.cells.as_slice();
                let chosen = cells
                    .iter()
                    .enumerate()
                    .skip(next)
                    .chain(cells.iter().enumerate().take(next))
                    .find_map(|(index, cell)| {
                        cell.work
                            .as_ref()
                            .filter(|work| work.entry.entry().signal.ready.load(Ordering::Acquire))
                            .map(|_| index)
                    });
                let chosen = chosen.and_then(|index| {
                    queue
                        .cells
                        .as_mut_slice()
                        .get_mut(index)
                        .and_then(|cell| cell.work.take().map(|work| (index, work)))
                });
                if let Some((index, work)) = chosen {
                    work.entry
                        .entry()
                        .signal
                        .ready
                        .store(false, Ordering::Release);
                    queue.next = if index + 1 == len { 0 } else { index + 1 };
                    queue.running += 1;
                    break (index, work);
                }
                if queue.stopped
                    && queue.preparing == 0
                    && queue.running == 0
                    && queue.cells.as_slice().iter().all(|cell| !cell.occupied)
                {
                    return;
                }
                queue = wait(&control.changed, queue);
            }
        };
        // Arc::clone only increments the startup-owned per-cell wake control.
        // Stale external wakers may cause harmless spurious polls, never access
        // old F or prevent generation-checked entry reclamation.
        let waker = Waker::from(Arc::clone(&work.entry.entry().signal));
        *lock(&work.cancellation.record.worker) = Some(waker.clone());
        let result = catch_unwind(AssertUnwindSafe(|| {
            let entry = work.entry.entry();
            let mut retirement = PollFuture {
                entry,
                pending: false,
            };
            // SAFETY: this worker exclusively owns Work outside the queue. F was
            // initialized once before publication; no mutable/pinned reference
            // to it can exist on any other thread. Cell address never changes.
            let future =
                unsafe { Pin::new_unchecked((&mut *entry.future.get()).assume_init_mut()) };
            let result = future.poll(&mut Context::from_waker(&waker));
            retirement.pending = result.is_pending();
            drop(retirement);
            result
        }));
        let result = match result {
            Ok(Poll::Pending) => {
                let mut queue = lock(&control.queue);
                // SAFETY: index came from this fixed vector's enumeration.
                // Its length is immutable, control retains its backing, and no
                // other producer can claim this occupied polling cell.
                unsafe { queue.cells.as_mut_slice().get_unchecked_mut(index) }.work = Some(work);
                queue.running -= 1;
                drop(queue);
                control.changed.notify_all();
                continue;
            }
            Ok(Poll::Ready(value)) => ProviderSettlement::Returned(value),
            Err(payload) => ProviderSettlement::Panicked(payload),
        };
        let Work {
            entry,
            execution,
            diagnostics,
            cancellation,
        } = work;
        {
            let mut state = lock(&entry.entry().state);
            *lock(&cancellation.record.worker) = None;
            state.result = Some(result);
        }
        // All failure delivery owners were physically admitted before captures.
        // Delivery transfers raw payload and actual event ownership; it does not
        // allocate/copy a Defect or choose a diagnostic saturation/loss policy.
        let (notification, retirement, execution_retirement) = diagnostics;
        match catch_unwind(AssertUnwindSafe(|| drop(execution))) {
            Ok(()) => drop(execution_retirement),
            Err(payload) => {
                execution_retirement.report(ProviderDiagnosticSite::ExecutionRetirement, payload)
            }
        }
        {
            let mut queue = lock(&control.queue);
            // SAFETY: same immutable-length, worker-owned cell proof as on
            // Pending; completion releases the exact cell selected above.
            unsafe { queue.cells.as_mut_slice().get_unchecked_mut(index) }.occupied = false;
            queue.running -= 1;
        }
        // Published results can precede native execution/queue retirement. Only
        // this final boundary enables once-only output transfer; consumer drop
        // or an earlier result notification cannot manufacture settlement.
        let waiter = {
            let mut state = lock(&entry.entry().state);
            cancellation.record.settled.store(true, Ordering::Release);
            state.waiter.take()
        };
        match catch_unwind(AssertUnwindSafe(|| {
            if let Some(waiter) = waiter {
                waiter.wake();
            }
        })) {
            Ok(()) => drop(notification),
            Err(payload) => notification.report(ProviderDiagnosticSite::Notification, payload),
        }
        match catch_unwind(AssertUnwindSafe(|| drop(entry))) {
            Ok(()) => drop(retirement),
            Err(payload) => retirement.report(ProviderDiagnosticSite::WorkerRetirement, payload),
        }

        control.changed.notify_all();
    }
}
