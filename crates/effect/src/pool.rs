//! Scoped resource pooling, leases, and shutdown.

use crate::cancellation::{LoaderCancellation, PreparedCancellationBank};
use crate::effect_uninterruptible;
use crate::execution::PreparedTaskCompletion;
use crate::internal::gate::{Gate, GateError};
use crate::internal::notification::notify_all;
use crate::internal::outcome;
use crate::internal::runtime::{
    self, Context, Finish, ParallelContextReservation, PreparedNestedReservation, Step,
};
use crate::internal::runtime_storage::{ReservedOnce, ReservedOnceReservation};
use crate::internal::work_list::WorkList;
use crate::memory::prepared_notification;
use crate::memory::{
    Bytes, FixedVector, FramePool, FrameVacancy, MemoryError, PreparedMemory, PreparedUnitFuture,
    RegionId, SharedSlot, SlotLease, SlotPool,
};
use crate::scope::Registered;
use crate::{
    Cancellation, CleanupFailures, Clock, Defect, Effect, Exit, HasClock, LibraryDefect,
    PreparedCleanupAccumulator, PreparedCleanupConfig, PreparedCleanupPool, Scope,
};
use crate::{PreparedGate, PreparedSemaphoreError, PreparedSemaphorePermit, SemaphoreError};
use event_listener::Event;
use futures_util::{
    FutureExt,
    future::{Either, poll_fn, select},
    task::AtomicWaker,
};
use std::collections::VecDeque;
use std::future::Future;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::task::Poll;
use std::time::Duration;

type Create<T, E> = Arc<dyn Fn(&Context, Finish<T, E>) -> Step + Send + Sync>;
type Validate<T, E> = Arc<dyn Fn(&EntryRef<T, E>, &Context, Finish<bool, E>) -> Step + Send + Sync>;
type Dispose<T, E> = Arc<dyn Fn(T, &Context, Finish<(), E>) -> Step + Send + Sync>;
type Evaluation<A, E> = Option<PreparedNestedReservation<A, E>>;
type CleanupReservation<E> = Option<CleanupFailures<E>>;

enum PoolCleanup<A, E: Send + 'static> {
    Ordinary(Exit<A, E>),
    Prepared(PreparedCleanupAccumulator<A, E>),
}
impl<A, E: Send + 'static> PoolCleanup<A, E> {
    fn new(primary: Exit<A, E>, reserve: CleanupReservation<E>) -> Self {
        match reserve {
            Some(reserve) => {
                Self::Prepared(PreparedCleanupAccumulator::from_reserved(primary, reserve))
            }
            None => Self::Ordinary(primary),
        }
    }

    fn append(self, cleanup: Exit<(), E>) -> Self {
        match self {
            Self::Ordinary(primary) => Self::Ordinary(outcome::append_cleanup(primary, cleanup)),
            Self::Prepared(mut accumulator) => {
                accumulator.append_cleanup(cleanup);
                Self::Prepared(accumulator)
            }
        }
    }

    fn finish(self) -> Exit<A, E> {
        match self {
            Self::Ordinary(outcome) => outcome,
            Self::Prepared(accumulator) => accumulator.finish(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for pool.
pub enum PoolCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The status reported when a pool lease or owner reaches its end boundary.
pub enum PoolEndStatus {
    /// Cleanup settled without a retained failure.
    Clean,
    /// The operation completed with a failure.
    Failed,
}
/// Typed state used by pool.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct State<T: Send + Sync + 'static, E: Send + 'static> {
    closed: bool,
    resources: usize,
    capacity: PoolCapacity,
    idle: IdleQueue<EntryRef<T, E>>,
    shutdown_queue: Option<IdleQueue<EntryRef<T, E>>>,
    diagnostics: Option<WorkList<PoolDiagnostic>>,
    shutdown_cleanup: CleanupReservation<E>,
    shutdown: Shutdown<E>,
    population: WorkList<PopulationJob<E>>,
}
impl<T: Send + Sync + 'static, E: Send + 'static> State<T, E> {
    fn population_target(&self, target: usize) -> usize {
        match self.capacity {
            PoolCapacity::Unbounded => target,
            PoolCapacity::Bounded(limit) => target.min(limit.get()),
        }
    }
}
/// Variants of shutdown used by pool.
///
/// # Type parameters
/// - `E`: Domain-error type.
enum Shutdown<E> {
    Open,
    Closing,
    Closed(Option<Exit<(), E>>),
}
/// Typed inner used by pool.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Inner<T: Send + Sync + 'static, E: Send + 'static> {
    state: Mutex<State<T, E>>,
    changed: PoolSignal,
    shutdown_changed: PoolSignal,
    acquiring: AcquisitionGate<E>,
    prepared_storage: Option<PreparedStorage<T, E>>,
    cancellation: LoaderCancellation,
    ttl: Option<Duration>,
    now: Arc<dyn Fn() -> Duration + Send + Sync>,
    create: Create<T, E>,
    validate: Validate<T, E>,
    dispose: Dispose<T, E>,
    spawn: Arc<dyn Fn(crate::ScheduledTask) -> Result<(), Defect> + Send + Sync>,
}
/// A scoped collection of reusable resources with explicit capacity and disposal.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct Pool<T: Send + Sync + 'static, E: Send + 'static> {
    inner: Arc<Inner<T, E>>,
}

type IdleSnapshot<T, E> = FixedVector<'static, EntryRef<T, E>>;

enum IdleQueue<T> {
    Ordinary(VecDeque<T>),
    Prepared(crate::internal::waiting::Waiting<T>),
}

impl<T> IdleQueue<T> {
    fn prepared(
        policy_memory: &PreparedMemory<'static>,
        region: RegionId,
        limit: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        crate::internal::waiting::Waiting::prepare(policy_memory, region, limit.get())
            .map(Self::Prepared)
    }
    fn iter(&self) -> impl Iterator<Item = &T> {
        let (ordinary, prepared) = match self {
            Self::Ordinary(queue) => (Some(queue), None),
            Self::Prepared(queue) => (None, Some(queue)),
        };
        ordinary
            .into_iter()
            .flatten()
            .chain(prepared.into_iter().flat_map(|queue| queue.iter()))
    }
    fn len(&self) -> usize {
        match self {
            Self::Ordinary(q) => q.len(),
            Self::Prepared(q) => q.len(),
        }
    }
    fn pop_front(&mut self) -> Option<T> {
        match self {
            Self::Ordinary(q) => q.pop_front(),
            Self::Prepared(q) => q.pop_front(),
        }
    }
    fn push_back(&mut self, entry: T) {
        match self {
            Self::Ordinary(q) => q.push_back(entry),
            // Each entry retains one resource slot. The prepared pool refuses
            // capacity changes above its queue width, including during close.
            Self::Prepared(q) => {
                q.push_back(entry);
            }
        }
    }
}

struct PoolControls<T: Send + Sync + 'static, E: Send + 'static> {
    gate: AcquisitionGate<E>,
    changed: PoolSignal,
    shutdown_changed: PoolSignal,
    idle: IdleQueue<EntryRef<T, E>>,
    shutdown: IdleQueue<EntryRef<T, E>>,
    population: WorkList<PopulationJob<E>>,
    prepared_storage: Option<PreparedStorage<T, E>>,
    diagnostics: WorkList<PoolDiagnostic>,
    shutdown_cleanup: CleanupReservation<E>,
}

enum PoolSignal {
    Ordinary(Event),
    Prepared {
        observers: prepared_notification::Event,
        required: AtomicWaker,
    },
}

impl PoolSignal {
    fn ordinary() -> Self {
        Self::Ordinary(Event::new())
    }

    fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        observers: usize,
    ) -> Result<Self, MemoryError> {
        Ok(Self::Prepared {
            observers: prepared_notification::Event::prepare(memory, region, observers)?,
            required: AtomicWaker::new(),
        })
    }

    fn observer(&self) -> Result<Option<prepared_notification::Registration>, MemoryError> {
        match self {
            Self::Ordinary(_) => Ok(None),
            Self::Prepared { observers, .. } => observers.listen().map(Some),
        }
    }

    async fn wait_observer(&self, ready: impl Fn() -> bool) -> Result<(), MemoryError> {
        if !ready() {
            let observer = self.observer()?;
            self.wait_registered(observer.as_ref(), ready).await;
        }
        Ok(())
    }

    #[allow(clippy::expect_used)] // Prepared callers reserve before selecting this wait path.
    async fn wait_registered(
        &self,
        observer: Option<&prepared_notification::Registration>,
        ready: impl Fn() -> bool,
    ) {
        match self {
            Self::Ordinary(changed) => loop {
                let listener = changed.listen();
                if ready() {
                    return;
                }
                listener.await;
            },
            Self::Prepared { .. } => {
                let listener = observer.expect("admitted pool observer");
                loop {
                    listener.reset();
                    if ready() {
                        return;
                    }
                    poll_fn(|cx| listener.poll(cx)).await;
                }
            }
        }
    }

    async fn wait_required(&self, ready: impl Fn() -> bool) {
        self.wait_required_with(None, ready).await;
    }

    async fn wait_required_with(&self, alternate: Option<&AtomicWaker>, ready: impl Fn() -> bool) {
        match self {
            Self::Ordinary(_) => self.wait_registered(None, ready).await,
            Self::Prepared { required, .. } => {
                // The designated cleanup driver is the only borrower of its waker.
                let required = alternate.unwrap_or(required);
                let cleanup = std::mem::DropGuard::new(required, |waker| drop(waker.take()));
                poll_fn(|cx| {
                    cleanup.register(cx.waker());
                    if ready() {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
        }
    }

    fn notify(&self) {
        match self {
            Self::Ordinary(changed) => notify_all(changed),
            Self::Prepared {
                observers,
                required,
            } => {
                let _wake_required = std::mem::DropGuard::new(required, AtomicWaker::wake);
                observers.notify_all();
            }
        }
    }
}

struct PoolDiagnostic {
    reason: LibraryDefect,
    // A lost resource retains its preadmitted record until shutdown observes it.
    _reservation: Option<crate::memory::SlotLease<'static, ()>>,
}

fn cancellation_demand(
    capacity: NonZeroUsize,
    waiters: usize,
) -> Result<(usize, usize), MemoryError> {
    let listeners = waiters
        .checked_add(1)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    let nodes = capacity
        .get()
        .checked_add(listeners)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    Ok((nodes, listeners))
}

fn diagnostic_capacity(resources: NonZeroUsize) -> Result<usize, MemoryError> {
    // At most one retained resource loss per permit, one abandonment per live
    // lease after the pool closes, and one abandonment of the owning scope.
    resources
        .get()
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or(MemoryError::ArithmeticOverflow)
}

type PopulationRows<T> = SlotPool<'static, FixedVector<'static, T>>;
struct PendingPopulation<T: Send + Sync + 'static, E: Send + 'static> {
    job: PopulationJob<E>,
    slot: Option<Slot<T, E>>,
    cleanup: CleanupReservation<E>,
    cancellation: Cancellation,
    origin: Option<ParallelContextReservation>,
    task: Option<PopulationTaskFrames<E>>,
    committed: bool,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Drop for PendingPopulation<T, E> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            let job = &self.job;
            let _settle = std::mem::DropGuard::new(self.committed, |committed| {
                if committed {
                    job.finish_dispatch(Some(Defect::from(LibraryDefect::ScheduledTaskDropped)));
                }
            });
            drop(slot);
        }
    }
}

struct PopulationTaskFrames<E: Send + 'static> {
    future: FrameVacancy<'static>,
    notice: ReservedOnceReservation<PopulationJob<E>, PreparedTaskCompletion, ()>,
}

struct PopulationWorker<T: Send + Sync + 'static, E: Send + 'static> {
    pool: Pool<T, E>,
    job: PopulationJob<E>,
    slot: Slot<T, E>,
    cleanup: CleanupReservation<E>,
    context: Context,
    operation: Cancellation,
}

impl<T: Send + Sync + 'static, E: Send + 'static> PopulationWorker<T, E> {
    fn future_layout() -> std::alloc::Layout {
        fn layout<I, F: Future<Output = ()>>(_: fn(I) -> F) -> std::alloc::Layout {
            std::alloc::Layout::new::<F>()
        }
        layout(Self::run_prepared)
    }

    async fn run(self) -> Exit<(), E> {
        let Self {
            pool,
            mut slot,
            cleanup,
            context,
            operation,
            ..
        } = self;
        let work = async {
            let created = pool.create_value(&context, slot.evaluation.take()).await;
            match created.into_failure() {
                Ok(value) => {
                    pool.return_entry(slot.install(value), context.cancelled(), context, cleanup)
                        .await
                }
                Err(failure) => {
                    drop(slot);
                    Pool::<T, E>::retain_cleanup(failure, cleanup)
                }
            }
        };
        let outcome = AssertUnwindSafe(work)
            .catch_unwind()
            .await
            .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
        operation.unlink();
        outcome
    }

    async fn run_prepared(self) {
        let job = self.job.clone();
        let outcome = self.run().await;
        *job.produced
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
    }

    fn task(
        self,
        frames: Option<PopulationTaskFrames<E>>,
    ) -> Result<crate::ScheduledTask, MemoryError> {
        let job = self.job.clone();
        match frames {
            Some(frames) => {
                let work = self.run_prepared();
                let future = frames.future.typed(&work)?;
                Ok(crate::ScheduledTask::prepared(
                    PreparedUnitFuture::initialize(future, work),
                    frames
                        .notice
                        .initialize(job, |job, event| job.complete_prepared(event)),
                ))
            }
            None => {
                let lost = job.clone();
                Ok(crate::ScheduledTask::new(
                    move |settlement| {
                        Box::pin(async move {
                            let completed = self.run().await;
                            settlement.release();
                            job.publish(completed);
                        })
                    },
                    move |event| match event {
                        crate::execution::TaskEvent::Started => {}
                        crate::execution::TaskEvent::Lost(defect) => {
                            lost.publish(Exit::Die(defect))
                        }
                    },
                ))
            }
        }
    }
}

fn evaluation_layout<A: Send + 'static, B: Send + 'static, E: Send + 'static>()
-> Result<std::alloc::Layout, MemoryError> {
    let (output_a, finish_a) = PreparedNestedReservation::<A, E>::layouts();
    let (output_b, finish_b) = PreparedNestedReservation::<B, E>::layouts();
    let layout = [output_a, finish_a, output_b, finish_b];
    let size = layout.iter().map(|layout| layout.size()).max().unwrap_or(0);
    let alignment = layout
        .iter()
        .map(|layout| layout.align())
        .max()
        .unwrap_or(1);
    std::alloc::Layout::from_size_align(size, alignment).map_err(|_| MemoryError::InvalidLayout)
}

fn population_task_layout<T: Send + Sync + 'static, E: Send + 'static>()
-> Result<std::alloc::Layout, MemoryError> {
    let future = PopulationWorker::<T, E>::future_layout();
    let notice = ReservedOnce::<PreparedTaskCompletion, ()>::layout::<PopulationJob<E>>();
    std::alloc::Layout::from_size_align(
        future.size().max(notice.size()),
        future.align().max(notice.align()),
    )
    .map_err(|_| MemoryError::InvalidLayout)
}

struct PopulationObservation<E: Send + 'static> {
    // Return the observer before its last job handle can recycle the control.
    listener: Option<prepared_notification::Registration>,
    job: PopulationJob<E>,
}
impl<E: Send + 'static> PopulationObservation<E> {
    fn new(job: PopulationJob<E>) -> Result<Self, MemoryError> {
        Ok(Self {
            listener: job.changed.observer()?,
            job,
        })
    }

    async fn wait(&self) {
        self.job
            .changed
            .wait_registered(self.listener.as_ref(), || self.job.settled())
            .await;
    }
}

enum PopulationJob<E: Send + 'static> {
    Ordinary(Arc<Population<E>>),
    Prepared(Option<SharedSlot<'static, Population<E>>>),
}

impl<E: Send + 'static> Clone for PopulationJob<E> {
    #[allow(clippy::expect_used)] // These infallible owners use the same address-space owner bound as Arc.
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(job) => Self::Ordinary(Arc::clone(job)),
            Self::Prepared(job) => Self::Prepared(Some(
                job.as_ref()
                    .expect("live population job")
                    .try_clone()
                    .expect("population job owner limit"),
            )),
        }
    }
}

impl<E: Send + 'static> std::ops::Deref for PopulationJob<E> {
    type Target = Population<E>;
    #[allow(clippy::expect_used)] // The owner is present until its own destructor runs.
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(job) => job,
            Self::Prepared(job) => job.as_ref().expect("live population job"),
        }
    }
}

impl<E: Send + 'static> Drop for PopulationJob<E> {
    fn drop(&mut self) {
        if let Self::Prepared(job) = self
            && let Some(mut cell) = job.take().and_then(SharedSlot::into_last)
        {
            let produced = cell
                .produced
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let state = cell
                .state
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let state = std::mem::replace(state, PopulationState::Submitting(None));
            drop((produced, state));
        }
    }
}

struct PreparedStorage<T: Send + Sync + 'static, E: Send + 'static> {
    resources: SlotPool<'static, Entry<T, E>>,
    leases: SlotPool<'static, LeaseInner<T, E>>,
    diagnostic_permits: SlotPool<'static, ()>,
    controls: SlotPool<'static, Population<E>>,
    references: PopulationRows<PopulationJob<E>>,
    observations: PopulationRows<PopulationObservation<E>>,
    jobs: PopulationRows<PendingPopulation<T, E>>,
    cleanup: PreparedCleanupConfig<E>,
    cancellation: PreparedCancellationBank,
    origins: FramePool<'static>,
    population_frames: FramePool<'static>,
    evaluation_frames: FramePool<'static>,
    disposal_frames: FramePool<'static>,
    on_error: Arc<dyn Fn(PreparedSemaphoreError) -> E + Send + Sync>,
}

fn population_tables<T: Send + 'static>(
    memory: &PreparedMemory<'static>,
    region: RegionId,
    count: usize,
    width: usize,
) -> Result<PopulationRows<T>, MemoryError> {
    memory.slots(region, count, |_| {
        FixedVector::prepare(memory.allocator(region)?, width)
    })
}

fn population_table_bytes<T>(count: usize, width: usize) -> Result<Bytes, MemoryError> {
    let values = std::alloc::Layout::array::<T>(width).map_err(|_| MemoryError::InvalidLayout)?;
    SlotPool::<FixedVector<'_, T>>::storage_bytes(count)?
        .checked_add(Bytes::new(values.size()).checked_mul(count)?)
}

/// Reserve idle-entry queues and a private FIFO acquisition gate for each scoped pool.
pub struct PreparedPoolStorage<E> {
    /// Maximum resource count and physical capacity of each idle-entry queue.
    pub capacity: NonZeroUsize,
    /// Startup memory used by the acquisition gate.
    pub memory: Arc<PreparedMemory<'static>>,
    /// Region that holds the idle queues, gate and queued waiter records.
    pub region: RegionId,
    /// Queued acquisition requests and, independently, observers per lease or shutdown.
    pub waiters: usize,
    /// Lease controls held by callers and their scope finalizers, including ended leases.
    pub lease_controls: NonZeroUsize,
    /// Simultaneous ensure_size operations with their own observation, retirement and dispatch lists.
    pub population_calls: NonZeroUsize,
    /// Failure buffers for active acquisitions, population work and retained outcomes.
    /// Acquisitions reserve two, and population reserves two plus one per new resource.
    pub cleanup_outcomes: NonZeroUsize,
    /// Secondary cleanup entries per outcome, with a separate overflow record.
    pub cleanup_entries: usize,
    /// Map preparation and admission failures into the pool's error type.
    pub on_error: Arc<dyn Fn(PreparedSemaphoreError) -> E + Send + Sync>,
}

enum AcquisitionGate<E> {
    Ordinary(Gate),
    Prepared(
        PreparedGate,
        NonZeroUsize,
        Arc<dyn Fn(PreparedSemaphoreError) -> E + Send + Sync>,
    ),
}

type AcquisitionPermit = Either<crate::internal::gate::Permit, PreparedSemaphorePermit>;

impl<E> AcquisitionGate<E> {
    async fn enter(&self, cancellation: Cancellation) -> Exit<AcquisitionPermit, E> {
        match self {
            Self::Ordinary(gate) => match gate.enter(cancellation).await {
                Ok(permit) => Exit::Success(Either::Left(permit)),
                Err(GateError::Interrupted) => Exit::Interrupted,
                Err(GateError::Closed) => Exit::Die(Defect::from(LibraryDefect::PoolClosed)),
            },
            Self::Prepared(gate, _, on_error) => {
                if cancellation.is_requested() {
                    return Exit::Interrupted;
                }
                let acquire = match gate.enter() {
                    Ok(acquire) => acquire,
                    Err(PreparedSemaphoreError::Semaphore(SemaphoreError::Closed)) => {
                        return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                    }
                    Err(error) => return Exit::Fail(on_error(error)),
                };
                let cancelled = match cancellation.try_cancelled() {
                    Ok(cancelled) => cancelled,
                    Err(error) => {
                        drop(acquire);
                        return Exit::Fail(on_error(PreparedSemaphoreError::Memory(error)));
                    }
                };
                match select(acquire, std::pin::pin!(cancelled)).await {
                    Either::Left((Ok(permit), _)) => Exit::Success(Either::Right(permit)),
                    Either::Left((Err(SemaphoreError::Closed), _)) => {
                        Exit::Die(Defect::from(LibraryDefect::PoolClosed))
                    }
                    Either::Left((Err(error), _)) => {
                        Exit::Fail(on_error(PreparedSemaphoreError::Semaphore(error)))
                    }
                    Either::Right(_) => Exit::Interrupted,
                }
            }
        }
    }

    fn close(&self) {
        match self {
            Self::Ordinary(gate) => gate.close(),
            Self::Prepared(gate, _, _) => {
                let _ = gate.close_now();
            }
        }
    }
}

/// Startup storage for simultaneous inspections of a pool's idle entries.
pub struct PreparedPoolSnapshots<T: Send + Sync + 'static, E: Send + 'static> {
    slots: SlotPool<'static, IdleSnapshot<T, E>>,
}

impl<T: Send + Sync + 'static, E: Send + 'static> PreparedPoolSnapshots<T, E> {
    /// Return slot storage plus idle entry references for sizing the selected region.
    /// Region alignment padding depends on preceding allocations.
    pub fn storage_bytes(
        simultaneous: NonZeroUsize,
        entries_per_snapshot: NonZeroUsize,
    ) -> Result<Bytes, MemoryError> {
        let entries = std::alloc::Layout::array::<EntryRef<T, E>>(entries_per_snapshot.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<IdleSnapshot<T, E>>::storage_bytes(simultaneous.get())?
            .checked_add(Bytes::new(entries.size()).checked_mul(simultaneous.get())?)
    }

    /// Reserve one entry-reference table per concurrent inspection in the selected region.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        simultaneous: NonZeroUsize,
        entries_per_snapshot: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(simultaneous, entries_per_snapshot)?;
        let slots = memory.slots(region, simultaneous.get(), |_| {
            FixedVector::prepare(memory.allocator(region)?, entries_per_snapshot.get())
        })?;
        Ok(Self { slots })
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static> Clone for Pool<T, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
/// Typed slot used by pool.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Slot<T: Send + Sync + 'static, E: Send + 'static> {
    entry: Option<SlotLease<'static, Entry<T, E>>>,
    evaluation: Evaluation<T, E>,
    disposal: Evaluation<(), E>,
    ownership: ResourceOwnership<T, E>,
}

struct ResourceOwnership<T: Send + Sync + 'static, E: Send + 'static> {
    pool: Weak<Inner<T, E>>,
    diagnostic: Option<SlotLease<'static, ()>>,
}

impl<T: Send + Sync + 'static, E: Send + 'static> Slot<T, E> {
    fn ordinary() -> Self {
        Self {
            entry: None,
            evaluation: None,
            disposal: None,
            ownership: ResourceOwnership {
                pool: Weak::new(),
                diagnostic: None,
            },
        }
    }

    fn reserve(storage: &PreparedStorage<T, E>) -> Result<Self, MemoryError> {
        Ok(Self {
            entry: Some(storage.resources.try_checkout()?),
            evaluation: Some(PreparedNestedReservation::reserve(
                &storage.evaluation_frames,
            )?),
            disposal: Some(PreparedNestedReservation::reserve(
                &storage.disposal_frames,
            )?),
            ownership: ResourceOwnership {
                pool: Weak::new(),
                diagnostic: Some(storage.diagnostic_permits.try_checkout()?),
            },
        })
    }

    fn install(self, value: T) -> EntryRef<T, E> {
        match self.entry {
            Some(mut entry) => {
                *entry
                    .data
                    .value
                    .get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value);
                *entry
                    .data
                    .returned
                    .get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Duration::ZERO;
                *entry
                    .disposal
                    .get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = self.disposal;
                entry.ownership = Some(self.ownership);
                EntryRef::Prepared(Some(entry.share()))
            }
            None => EntryRef::Ordinary(Arc::new(Entry {
                data: ResourceData {
                    value: RwLock::new(Some(value)),
                    returned: Mutex::new(Duration::ZERO),
                },
                disposal: Mutex::new(None),
                ownership: Some(self.ownership),
            })),
        }
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static> Drop for ResourceOwnership<T, E> {
    fn drop(&mut self) {
        drop(self.diagnostic.take());
        if let Some(pool) = self.pool.upgrade() {
            pool.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .resources -= NonZeroUsize::MIN.get();
            pool.changed.notify();
        }
    }
}

struct ResourceData<T> {
    value: RwLock<Option<T>>,
    returned: Mutex<Duration>,
}

struct Entry<T: Send + Sync + 'static, E: Send + 'static> {
    data: ResourceData<T>,
    disposal: Mutex<Evaluation<(), E>>,
    ownership: Option<ResourceOwnership<T, E>>,
}

impl<T: Send + Sync + 'static, E: Send + 'static> Entry<T, E> {
    fn empty() -> Self {
        Self {
            data: ResourceData {
                value: RwLock::new(None),
                returned: Mutex::new(Duration::ZERO),
            },
            disposal: Mutex::new(None),
            ownership: None,
        }
    }

    fn remove(&mut self) -> (Option<T>, Option<ResourceOwnership<T, E>>) {
        drop(
            self.disposal
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
        let value = self
            .data
            .value
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let mut ownership = self.ownership.take();
        if value.is_some()
            && let Some(ownership) = &mut ownership
            && let Some(pool) = ownership.pool.upgrade()
        {
            let mut state = pool
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(diagnostics) = &mut state.diagnostics {
                diagnostics.push_reserved(PoolDiagnostic {
                    reason: LibraryDefect::PoolResourceDropped,
                    _reservation: ownership.diagnostic.take(),
                });
            }
        }
        (value, ownership)
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static> Drop for Entry<T, E> {
    fn drop(&mut self) {
        // Prepared cells are emptied before they are returned to their bank.
        let (value, ownership) = self.remove();
        drop(value);
        drop(ownership);
    }
}

enum EntryRef<T: Send + Sync + 'static, E: Send + 'static> {
    Ordinary(Arc<Entry<T, E>>),
    Prepared(Option<SharedSlot<'static, Entry<T, E>>>),
}

impl<T: Send + Sync + 'static, E: Send + 'static> EntryRef<T, E> {
    #[allow(clippy::expect_used)] // The live slot is present until its owning ref is dropped.
    fn take_disposal(&self) -> Evaluation<(), E> {
        let entry: &Entry<T, E> = match self {
            Self::Ordinary(entry) => entry,
            Self::Prepared(entry) => entry.as_ref().expect("live pool resource"),
        };
        entry
            .disposal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static> Clone for EntryRef<T, E> {
    #[allow(clippy::expect_used)] // These infallible owners use the same address-space owner bound as Arc.
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(entry) => Self::Ordinary(Arc::clone(entry)),
            Self::Prepared(entry) => Self::Prepared(Some(
                entry
                    .as_ref()
                    .expect("live pool resource")
                    .try_clone()
                    .expect("pool resource owner limit"),
            )),
        }
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static> std::ops::Deref for EntryRef<T, E> {
    type Target = ResourceData<T>;

    #[allow(clippy::expect_used)] // The owner is present until its own destructor runs.
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(entry) => &entry.data,
            Self::Prepared(entry) => &entry.as_ref().expect("live pool resource").data,
        }
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static> Drop for EntryRef<T, E> {
    fn drop(&mut self) {
        if let Self::Prepared(entry) = self
            && let Some(mut cell) = entry.take().and_then(SharedSlot::into_last)
        {
            let (value, ownership) = cell.remove();
            // Tuple order also returns the physical cell before the count on unwind.
            let retired = (cell, ownership);
            drop(value);
            drop(retired);
        }
    }
}
/// Typed lease state used by pool.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct LeaseState<T: Send + Sync + 'static, E: Send + 'static> {
    entry: Option<(EntryRef<T, E>, CleanupReservation<E>)>,
    phase: LeasePhase<E>,
}
/// Variants of lease phase used by pool.
///
/// # Type parameters
/// - `E`: Domain-error type.
enum LeasePhase<E> {
    Open,
    Closing,
    Closed(Option<Exit<(), E>>),
}
/// Typed lease inner used by pool.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct LeaseInner<T: Send + Sync + 'static, E: Send + 'static> {
    state: Mutex<LeaseState<T, E>>,
    changed: PoolSignal,
    interrupted: AtomicWaker,
}
impl<T: Send + Sync + 'static, E: Send + 'static> LeaseInner<T, E> {
    fn new(state: LeaseState<T, E>, changed: PoolSignal) -> Self {
        Self {
            state: Mutex::new(state),
            changed,
            interrupted: AtomicWaker::new(),
        }
    }

    fn notify(&self) {
        let _wake_interrupted = std::mem::DropGuard::new(&self.interrupted, AtomicWaker::wake);
        self.changed.notify();
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static> LeaseState<T, E> {
    fn ended() -> Self {
        Self {
            entry: None,
            phase: LeasePhase::Closed(None),
        }
    }
}

enum LeaseWait {
    Observer,
    Scope,
    InterruptedAcquisition,
}

type LeaseReservation<T, E> = Option<SlotLease<'static, LeaseInner<T, E>>>;

struct LeaseClaim<T: Send + Sync + 'static, E: Send + 'static> {
    entry: EntryRef<T, E>,
    control: LeaseReservation<T, E>,
    cleanup: [CleanupReservation<E>; 2],
}

enum LeaseControl<T: Send + Sync + 'static, E: Send + 'static> {
    Ordinary(Arc<LeaseInner<T, E>>),
    Prepared(Option<SharedSlot<'static, LeaseInner<T, E>>>),
}
impl<T: Send + Sync + 'static, E: Send + 'static> Clone for LeaseControl<T, E> {
    #[allow(clippy::expect_used)] // These infallible owners use the same address-space owner bound as Arc.
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(lease) => Self::Ordinary(Arc::clone(lease)),
            Self::Prepared(lease) => Self::Prepared(Some(
                lease
                    .as_ref()
                    .expect("live pool lease")
                    .try_clone()
                    .expect("pool lease owner limit"),
            )),
        }
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static> std::ops::Deref for LeaseControl<T, E> {
    type Target = LeaseInner<T, E>;

    #[allow(clippy::expect_used)] // The owner is present until its own destructor runs.
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(lease) => lease,
            Self::Prepared(lease) => lease.as_ref().expect("live pool lease"),
        }
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static> Drop for LeaseControl<T, E> {
    fn drop(&mut self) {
        if let Self::Prepared(lease) = self
            && let Some(mut cell) = lease.take().and_then(SharedSlot::into_last)
        {
            let state = cell
                .state
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(std::mem::replace(state, LeaseState::ended()));
        }
    }
}

/// An invalidatable borrowed view of one resource owned by a pool.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct PoolLease<T: Send + Sync + 'static, E: Send + 'static> {
    inner: LeaseControl<T, E>,
    pool: Pool<T, E>,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Clone for PoolLease<T, E> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            pool: self.pool.clone(),
        }
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static> PoolLease<T, E> {
    /// Borrow the contained value only for the duration of the supplied callback.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    /// - `A`: Application value type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `inspect` (`impl FnOnce(&T) -> A`): The inspect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<A, Defect>`.
    pub fn with<A>(&self, inspect: impl FnOnce(&T) -> A) -> Result<A, Defect> {
        let entry = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry
            .as_ref()
            .map(|(entry, _)| entry.clone())
            .ok_or_else(|| Defect::from(LibraryDefect::PoolLeaseEnded))?;
        let value = entry
            .value
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match value.as_ref() {
            Some(value) => {
                catch_unwind(AssertUnwindSafe(|| inspect(value))).map_err(Defect::from_panic)
            }
            None => Err(Defect::from(LibraryDefect::PoolLeaseEnded)),
        }
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn abandon(&self) {
        let entry = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !matches!(state.phase, LeasePhase::Open) {
                return;
            }
            state.phase = LeasePhase::Closed(Some(Exit::Die(Defect::from(
                LibraryDefect::ScopeOwnerDropped,
            ))));
            state.entry.take()
        };
        let entry = entry.map(|(entry, cleanup)| {
            drop(cleanup);
            entry
        });
        // Retain ownership for an explicit recovery drain; Drop cannot
        // invoke and await the native disposer.
        self.pool.abandon_entry(entry);
        self.inner.notify();
    }
    ///
    /// # Returns
    /// Type: `PoolEndStatus`.
    /// Returns `PoolEndStatus`.
    fn status(&self) -> PoolEndStatus {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &state.phase {
            LeasePhase::Closed(Some(Exit::Success(()))) | LeasePhase::Closed(None) => {
                PoolEndStatus::Clean
            }
            LeasePhase::Closed(Some(
                Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_),
            ))
            | LeasePhase::Open
            | LeasePhase::Closing => PoolEndStatus::Failed,
        }
    }
    ///
    /// # Parameters
    /// - `retire` (`bool`): The retire supplied to this operation.
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `()`.
    async fn end(
        &self,
        retire: bool,
        context: Context,
        wait: LeaseWait,
    ) -> Result<(), MemoryError> {
        let entry = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.phase {
                LeasePhase::Open => {
                    state.phase = LeasePhase::Closing;
                    state.entry.take()
                }
                LeasePhase::Closing | LeasePhase::Closed(_) => None,
            }
        };
        if let Some((entry, cleanup)) = entry {
            let mut guard = LeaseDriver {
                lease: self.clone(),
                settled: false,
            };
            let outcome = self
                .pool
                .return_entry(entry, retire, context.protected(), cleanup)
                .await;
            self.inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .phase = LeasePhase::Closed(Some(outcome));
            guard.settled = true;
            self.inner.notify();
        } else {
            let settled = || {
                matches!(
                    self.inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .phase,
                    LeasePhase::Closed(_)
                )
            };
            match wait {
                LeaseWait::Observer => self.inner.changed.wait_observer(settled).await?,
                LeaseWait::Scope => self.inner.changed.wait_required(settled).await,
                LeaseWait::InterruptedAcquisition => {
                    self.inner
                        .changed
                        .wait_required_with(Some(&self.inner.interrupted), settled)
                        .await
                }
            }
        }
        Ok(())
    }
    /// End this claim or lease and return its settlement status.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<PoolEndStatus, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn release<R: Send + Sync + 'static>(&self) -> Effect<PoolEndStatus, E, R> {
        self.end_effect(false)
    }
    /// Remove the leased resource from reuse and start its disposal path.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<PoolEndStatus, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn discard<R: Send + Sync + 'static>(&self) -> Effect<PoolEndStatus, E, R> {
        self.end_effect(true)
    }
    ///
    /// # Type parameters
    /// - `R`: Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `retire` (`bool`): The retire supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<PoolEndStatus, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn end_effect<R: Send + Sync + 'static>(&self, retire: bool) -> Effect<PoolEndStatus, E, R> {
        let lease = self.clone();
        effect_uninterruptible! { program: Effect::new(move |_, context, finish| {
            let lease = lease.clone();
            Step::suspended(Box::pin(async move {
                if let Err(error) = lease.end(retire, context, LeaseWait::Observer).await {
                    return runtime::finish(finish, Exit::Fail(lease.pool.storage_error(error)));
                }
                runtime::finish(finish, Exit::Success(lease.status()))
            }))
        }), }
    }
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `Exit<(), E>`.
    /// Returns `Exit<(), E>`.
    async fn finalize(&self, context: Context, wait: LeaseWait) -> Exit<(), E> {
        if let Err(error) = self.end(false, context, wait).await {
            return Exit::Fail(self.pool.storage_error(error));
        }
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut state.phase {
            LeasePhase::Closed(outcome) => outcome.take().unwrap_or(Exit::Success(())),
            LeasePhase::Open | LeasePhase::Closing => {
                Exit::Die(Defect::from(LibraryDefect::PoolLeaseEnded))
            }
        }
    }
}
/// Typed lease driver used by pool.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct LeaseDriver<T: Send + Sync + 'static, E: Send + 'static> {
    lease: PoolLease<T, E>,
    settled: bool,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Drop for LeaseDriver<T, E> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if !self.settled {
            self.lease
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .phase = LeasePhase::Closed(Some(Exit::Die(Defect::from(
                LibraryDefect::PoolReleaseDriverDropped,
            ))));
            self.lease.inner.notify();
        }
    }
}
impl<T: Send + Sync + 'static, E: Send + 'static> Pool<T, E> {
    /// Create a `Pool` through the declared Effect and environment capabilities.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `HasClock + crate::HasTaskSpawner + Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `scope` (`Scope<E>`): Owner that joins child work and releases resources at close.
    /// - `capacity` (`PoolCapacity`): The caller-supplied capacity bound.
    /// - `idle_ttl` (`Option<Duration>`): The idle ttl supplied to this operation.
    /// - `create` (`Effect<T, E, R>`): The create supplied to this operation.
    /// - `validate` (`impl Fn(&T) -> Effect<bool, E, R> + Send + Sync + 'static`): The validate supplied to this operation.
    /// - `dispose` (`impl Fn(T) -> Effect<(), E, R> + Send + Sync + 'static`): The dispose supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<Self, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make<R: HasClock + crate::HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<E>,
        capacity: PoolCapacity,
        idle_ttl: Option<Duration>,
        create: Effect<T, E, R>,
        validate: impl Fn(&T) -> Effect<bool, E, R> + Send + Sync + 'static,
        dispose: impl Fn(T) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Effect<Self, E, R> {
        Self::make_with_controls(scope, capacity, idle_ttl, create, validate, dispose, || {
            Ok(PoolControls {
                gate: AcquisitionGate::Ordinary(Gate::new()),
                changed: PoolSignal::ordinary(),
                shutdown_changed: PoolSignal::ordinary(),
                idle: IdleQueue::Ordinary(VecDeque::new()),
                shutdown: IdleQueue::Ordinary(VecDeque::new()),
                population: WorkList::default(),
                prepared_storage: None,
                diagnostics: WorkList::default(),
                shutdown_cleanup: None,
            })
        })
    }

    /// Region payload for idle and shutdown queues, the acquisition gate, population
    /// controls and the requested number of concurrent population workspaces.
    /// Includes the independently reserved shutdown outcome. Region allocator
    /// headers, padding and the pool's startup Arc are additional.
    pub fn prepared_storage_bytes(
        capacity: NonZeroUsize,
        waiters: usize,
        population_calls: NonZeroUsize,
        lease_controls: NonZeroUsize,
        cleanup_outcomes: NonZeroUsize,
        cleanup_entries: usize,
    ) -> Result<Bytes, MemoryError> {
        let (cancellation_nodes, cancellation_listeners) = cancellation_demand(capacity, waiters)?;
        let queue =
            crate::internal::waiting::Waiting::<EntryRef<T, E>>::payload_bytes(capacity.get())?;
        let references = population_calls
            .get()
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        // Shutdown always has its own buffer and frame, even when work outcomes are retained.
        let shutdown_cleanup =
            PreparedCleanupPool::<E>::storage_bytes(1, cleanup_entries)?.checked_add(
                FramePool::storage_bytes(1, PreparedCleanupPool::<E>::frame_layout())?,
            )?;
        let work_cleanup =
            PreparedCleanupPool::<E>::storage_bytes(cleanup_outcomes.get(), cleanup_entries)?
                .checked_add(FramePool::storage_bytes(
                    cleanup_outcomes.get(),
                    PreparedCleanupPool::<E>::frame_layout(),
                )?)?;
        let evaluation_count = capacity
            .get()
            .checked_mul(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        queue
            .checked_mul(2)?
            .checked_add(shutdown_cleanup)?
            .checked_add(work_cleanup)?
            .checked_add(PreparedCancellationBank::storage_bytes(
                cancellation_nodes,
                cancellation_listeners,
            )?)?
            .checked_add(FramePool::storage_bytes(
                cancellation_nodes,
                ParallelContextReservation::layout(),
            )?)?
            .checked_add(FramePool::storage_bytes(
                capacity
                    .get()
                    .checked_mul(2)
                    .ok_or(MemoryError::ArithmeticOverflow)?,
                population_task_layout::<T, E>()?,
            )?)?
            .checked_add(FramePool::storage_bytes(
                evaluation_count,
                evaluation_layout::<T, bool, E>()?,
            )?)?
            .checked_add(FramePool::storage_bytes(
                evaluation_count,
                evaluation_layout::<(), (), E>()?,
            )?)?
            .checked_add(SlotPool::<Population<E>>::storage_bytes(capacity.get())?)?
            .checked_add(SlotPool::<Entry<T, E>>::storage_bytes(capacity.get())?)?
            .checked_add(SlotPool::<LeaseInner<T, E>>::storage_bytes(
                lease_controls.get(),
            )?)?
            .checked_add(
                prepared_notification::Event::storage_bytes(waiters)?
                    .checked_mul(lease_controls.get())?,
            )?
            .checked_add(SlotPool::<()>::storage_bytes(capacity.get())?)?
            .checked_add(population_table_bytes::<PoolDiagnostic>(
                1,
                diagnostic_capacity(capacity)?,
            )?)?
            .checked_add(
                prepared_notification::Event::storage_bytes(population_calls.get())?
                    .checked_mul(capacity.get())?,
            )?
            .checked_add(PreparedGate::storage_bytes(waiters)?)?
            .checked_add(prepared_notification::Event::storage_bytes(1)?)?
            .checked_add(prepared_notification::Event::storage_bytes(waiters)?)?
            .checked_add(population_table_bytes::<PopulationJob<E>>(
                references,
                capacity.get(),
            )?)?
            .checked_add(population_table_bytes::<PopulationObservation<E>>(
                population_calls.get(),
                capacity.get(),
            )?)?
            .checked_add(population_table_bytes::<PendingPopulation<T, E>>(
                population_calls.get(),
                capacity.get(),
            )?)
    }

    /// Provision queues, FIFO waiters and population controls before registering the pool.
    pub fn make_with_prepared_storage<
        R: HasClock + crate::HasTaskSpawner + Send + Sync + 'static,
    >(
        scope: Scope<E>,
        policy: PreparedPoolStorage<E>,
        idle_ttl: Option<Duration>,
        create: Effect<T, E, R>,
        validate: impl Fn(&T) -> Effect<bool, E, R> + Send + Sync + 'static,
        dispose: impl Fn(T) -> Effect<(), E, R> + Send + Sync + 'static,
    ) -> Effect<Self, E, R> {
        Self::make_with_controls(
            scope,
            PoolCapacity::Bounded(policy.capacity),
            idle_ttl,
            create,
            validate,
            dispose,
            move || {
                let map = |error| (policy.on_error)(PreparedSemaphoreError::Memory(error));
                Self::prepared_storage_bytes(
                    policy.capacity,
                    policy.waiters,
                    policy.population_calls,
                    policy.lease_controls,
                    policy.cleanup_outcomes,
                    policy.cleanup_entries,
                )
                .map_err(map)?;
                let queue = || {
                    IdleQueue::prepared(&policy.memory, policy.region, policy.capacity).map_err(map)
                };
                let idle = queue()?;
                let shutdown = queue()?;
                let calls = policy.population_calls.get();
                let count = calls
                    .checked_add(1)
                    .ok_or_else(|| map(MemoryError::ArithmeticOverflow))?;
                let references =
                    population_tables(&policy.memory, policy.region, count, policy.capacity.get())
                        .map_err(map)?;
                let observations =
                    population_tables(&policy.memory, policy.region, calls, policy.capacity.get())
                        .map_err(map)?;
                let jobs =
                    population_tables(&policy.memory, policy.region, calls, policy.capacity.get())
                        .map_err(map)?;
                let population = WorkList::checkout(&references).map_err(map)?;
                let controls = policy
                    .memory
                    .slots(policy.region, policy.capacity.get(), |_| {
                        Ok(Population::new(PoolSignal::prepare(
                            &policy.memory,
                            policy.region,
                            calls,
                        )?))
                    })
                    .map_err(map)?;
                let gate = PreparedGate::prepare(&policy.memory, policy.region, policy.waiters)
                    .map_err(|error| (policy.on_error)(error))?;
                let diagnostic_tables = population_tables(
                    &policy.memory,
                    policy.region,
                    1,
                    diagnostic_capacity(policy.capacity).map_err(map)?,
                )
                .map_err(map)?;
                let prepare_cleanup = |count| -> Result<PreparedCleanupConfig<E>, MemoryError> {
                    Ok(PreparedCleanupConfig::new(
                        PreparedCleanupPool::prepare(
                            &policy.memory,
                            policy.region,
                            count,
                            policy.cleanup_entries,
                        )?,
                        policy.memory.frames(
                            policy.region,
                            count,
                            PreparedCleanupPool::<E>::frame_layout(),
                        )?,
                    ))
                };
                let shutdown_cleanup = prepare_cleanup(1)
                    .and_then(|config| config.reserve_failures())
                    .map_err(map)?;
                let cleanup = prepare_cleanup(policy.cleanup_outcomes.get()).map_err(map)?;
                let (cancellation_nodes, cancellation_listeners) =
                    cancellation_demand(policy.capacity, policy.waiters).map_err(map)?;
                let cancellation = PreparedCancellationBank::prepare(
                    &policy.memory,
                    policy.region,
                    cancellation_nodes,
                    cancellation_listeners,
                )
                .map_err(map)?;
                let evaluation_count = policy
                    .capacity
                    .get()
                    .checked_mul(2)
                    .ok_or_else(|| map(MemoryError::ArithmeticOverflow))?;
                Ok(PoolControls {
                    gate: AcquisitionGate::Prepared(
                        gate,
                        policy.capacity,
                        Arc::clone(&policy.on_error),
                    ),
                    changed: PoolSignal::prepare(&policy.memory, policy.region, 1).map_err(map)?,
                    shutdown_changed: PoolSignal::prepare(
                        &policy.memory,
                        policy.region,
                        policy.waiters,
                    )
                    .map_err(map)?,
                    idle,
                    shutdown,
                    population,
                    diagnostics: WorkList::checkout(&diagnostic_tables).map_err(map)?,
                    shutdown_cleanup: Some(shutdown_cleanup),
                    prepared_storage: Some(PreparedStorage {
                        leases: policy
                            .memory
                            .slots(policy.region, policy.lease_controls.get(), |_| {
                                Ok(LeaseInner::new(
                                    LeaseState::ended(),
                                    PoolSignal::prepare(
                                        &policy.memory,
                                        policy.region,
                                        policy.waiters,
                                    )?,
                                ))
                            })
                            .map_err(map)?,
                        diagnostic_permits: policy
                            .memory
                            .slots(policy.region, policy.capacity.get(), |_| Ok(()))
                            .map_err(map)?,
                        resources: policy
                            .memory
                            .slots(policy.region, policy.capacity.get(), |_| Ok(Entry::empty()))
                            .map_err(map)?,
                        controls,
                        references,
                        observations,
                        jobs,
                        cleanup,
                        cancellation,
                        origins: policy
                            .memory
                            .frames(
                                policy.region,
                                cancellation_nodes,
                                ParallelContextReservation::layout(),
                            )
                            .map_err(map)?,
                        population_frames: policy
                            .memory
                            .frames(
                                policy.region,
                                policy
                                    .capacity
                                    .get()
                                    .checked_mul(2)
                                    .ok_or_else(|| map(MemoryError::ArithmeticOverflow))?,
                                population_task_layout::<T, E>().map_err(map)?,
                            )
                            .map_err(map)?,
                        evaluation_frames: policy
                            .memory
                            .frames(
                                policy.region,
                                evaluation_count,
                                evaluation_layout::<T, bool, E>().map_err(map)?,
                            )
                            .map_err(map)?,
                        disposal_frames: policy
                            .memory
                            .frames(
                                policy.region,
                                evaluation_count,
                                evaluation_layout::<(), (), E>().map_err(map)?,
                            )
                            .map_err(map)?,
                        on_error: Arc::clone(&policy.on_error),
                    }),
                })
            },
        )
    }

    fn reserve_population_task(&self) -> Result<Option<PopulationTaskFrames<E>>, MemoryError> {
        self.inner
            .prepared_storage
            .as_ref()
            .map(|storage| {
                Ok(PopulationTaskFrames {
                    future: storage
                        .population_frames
                        .reserve_layout(PopulationWorker::<T, E>::future_layout())?,
                    notice: ReservedOnce::reserve(&storage.population_frames)?,
                })
            })
            .transpose()
    }

    fn reserve_parallel(&self) -> Result<Option<ParallelContextReservation>, MemoryError> {
        self.inner
            .prepared_storage
            .as_ref()
            .map(|storage| ParallelContextReservation::reserve(&storage.origins))
            .transpose()
    }

    fn parallel_context(
        context: &Context,
        cancellation: Cancellation,
        origin: Option<ParallelContextReservation>,
    ) -> Context {
        match origin {
            Some(origin) => context.parallel_prepared(cancellation, origin),
            None => context.parallel(cancellation),
        }
    }

    fn cancellation_wait<'a>(
        &self,
        cancellation: &'a Cancellation,
    ) -> Result<impl Future<Output = ()> + 'a + use<'a, T, E>, MemoryError> {
        if self.inner.prepared_storage.is_some() {
            cancellation.try_cancelled().map(Either::Left)
        } else {
            Ok(Either::Right(cancellation.cancelled()))
        }
    }

    fn reserve_cleanup(&self) -> Result<CleanupReservation<E>, MemoryError> {
        self.inner
            .prepared_storage
            .as_ref()
            .map_or(Ok(None), |storage| {
                storage.cleanup.reserve_failures().map(Some)
            })
    }

    fn append_cleanup<A>(
        primary: Exit<A, E>,
        cleanup: Exit<(), E>,
        reserve: CleanupReservation<E>,
    ) -> Exit<A, E> {
        if matches!(cleanup, Exit::Success(())) {
            primary
        } else {
            PoolCleanup::new(primary, reserve).append(cleanup).finish()
        }
    }

    fn retain_cleanup<A>(outcome: Exit<A, E>, reserve: CleanupReservation<E>) -> Exit<A, E> {
        if matches!(outcome, Exit::FinalizationFailed(_)) {
            PoolCleanup::new(outcome, reserve).finish()
        } else {
            outcome
        }
    }

    #[allow(clippy::expect_used)] // Ordinary control operations cannot return a memory error.
    fn storage_error(&self, error: MemoryError) -> E {
        let storage = self
            .inner
            .prepared_storage
            .as_ref()
            .expect("prepared pool storage");
        (storage.on_error)(PreparedSemaphoreError::Memory(error))
    }

    fn make_with_controls<R: HasClock + crate::HasTaskSpawner + Send + Sync + 'static>(
        scope: Scope<E>,
        capacity: PoolCapacity,
        idle_ttl: Option<Duration>,
        create: Effect<T, E, R>,
        validate: impl Fn(&T) -> Effect<bool, E, R> + Send + Sync + 'static,
        dispose: impl Fn(T) -> Effect<(), E, R> + Send + Sync + 'static,
        prepare_controls: impl Fn() -> Result<PoolControls<T, E>, E> + Send + Sync + 'static,
    ) -> Effect<Self, E, R> {
        let validate = Arc::new(validate);
        let dispose = Arc::new(dispose);
        Effect::new(move |env: Arc<R>, context, finish| {
            let controls = match prepare_controls() {
                Ok(controls) => controls,
                Err(error) => return runtime::finish(finish, Exit::Fail(error)),
            };
            let cancellation = match &controls.prepared_storage {
                Some(storage) => match storage.cancellation.bind(&context.cancellation) {
                    Ok(cancellation) => cancellation,
                    Err(error) => {
                        return runtime::finish(
                            finish,
                            Exit::Fail((storage.on_error)(PreparedSemaphoreError::Memory(error))),
                        );
                    }
                },
                None => LoaderCancellation::ordinary(&context.cancellation),
            };
            let create_env = Arc::clone(&env);
            let validate_env = Arc::clone(&env);
            let dispose_env = Arc::clone(&env);
            let spawn_env = Arc::clone(&env);
            let now_env = env;
            let create = create.clone();
            let validate = Arc::clone(&validate);
            let dispose = Arc::clone(&dispose);
            let pool = Self {
                inner: Arc::new(Inner {
                    state: Mutex::new(State {
                        closed: false,
                        resources: usize::default(),
                        capacity,
                        idle: controls.idle,
                        shutdown_queue: Some(controls.shutdown),
                        diagnostics: Some(controls.diagnostics),
                        shutdown_cleanup: controls.shutdown_cleanup,
                        shutdown: Shutdown::Open,
                        population: controls.population,
                    }),
                    changed: controls.changed,
                    shutdown_changed: controls.shutdown_changed,
                    acquiring: controls.gate,
                    prepared_storage: controls.prepared_storage,
                    cancellation,
                    ttl: idle_ttl,
                    now: Arc::new(move || now_env.clock().now()),
                    spawn: Arc::new(move |task| {
                        use crate::TaskSpawner;
                        spawn_env.task_spawner().spawn(task)
                    }),
                    create: Arc::new(move |context, finish| {
                        create
                            .clone()
                            .execute(create_env.clone(), context.clone(), finish)
                    }),
                    validate: Arc::new(move |entry, context, finish| {
                        let program = {
                            let value = entry
                                .value
                                .read()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            match value.as_ref() {
                                Some(value) => validate(value),
                                None => {
                                    return runtime::finish(
                                        finish,
                                        Exit::Die(Defect::from(LibraryDefect::PoolLeaseEnded)),
                                    );
                                }
                            }
                        };
                        program.execute(validate_env.clone(), context.clone(), finish)
                    }),
                    dispose: Arc::new(move |value, context, finish| {
                        dispose(value).execute(dispose_env.clone(), context.clone(), finish)
                    }),
                }),
            };
            let close = pool.clone();
            let close_context = context.protected();
            let abandoned = pool.clone();
            let finalizer = Registered::with_abandon(
                move |finish| {
                    Step::suspended(Box::pin(async move {
                        let cleanup = close.shutdown_owned(close_context).await;
                        runtime::finish(finish, cleanup)
                    }))
                },
                move || abandoned.abandon(),
            );
            scope.register_or_release(finalizer, pool, finish)
        })
    }
    ///
    /// # Returns
    /// Type: `()`.
    fn abandon(&self) {
        self.abandon_entry(None);
    }
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `entry` (`Option<EntryRef<T, E>>`): Stored record or event selected for this transition.
    ///
    /// # Returns
    /// Type: `()`.
    fn abandon_entry(&self, entry: Option<EntryRef<T, E>>) {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            if let Some(diagnostics) = &mut state.diagnostics {
                diagnostics.push_reserved(PoolDiagnostic {
                    reason: LibraryDefect::ScopeOwnerDropped,
                    _reservation: None,
                });
            }
            if let Some(entry) = entry {
                state.idle.push_back(entry);
            }
        }
        self.inner.acquiring.close();
        let _ = self.inner.cancellation.request();
        self.inner.changed.notify();
    }
    async fn create_value(&self, context: &Context, reserve: Evaluation<T, E>) -> Exit<T, E> {
        let entry = |finish| (self.inner.create)(context, finish);
        match reserve {
            Some(reserve) => runtime::evaluate_raw_reserved(context, reserve, entry).await,
            None => runtime::evaluate_raw_with(context, entry).await,
        }
    }

    async fn evaluate<A: Send + 'static>(
        context: &Context,
        reserve: Evaluation<A, E>,
        entry: impl FnOnce(Finish<A, E>) -> Step,
    ) -> Exit<A, E> {
        match reserve {
            Some(reserve) => runtime::evaluate_context_reserved(context, reserve, entry).await,
            None => runtime::evaluate_context_with(context, entry).await,
        }
    }

    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `entry` (`EntryRef<T, E>`): Stored record or event selected for this transition.
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `Exit<(), E>`.
    /// Returns `Exit<(), E>`.
    async fn dispose_entry(&self, entry: EntryRef<T, E>, context: Context) -> Exit<(), E> {
        let value = entry
            .value
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let result = match value {
            Some(value) => {
                let reserve = entry.take_disposal();
                AssertUnwindSafe(async {
                    let context = context.protected();
                    Self::evaluate(&context, reserve, |finish| {
                        (self.inner.dispose)(value, &context, finish)
                    })
                    .await
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)))
            }
            None => Exit::Success(()),
        };
        drop(entry);
        result
    }
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `entry` (`EntryRef<T, E>`): Stored record or event selected for this transition.
    /// - `retire` (`bool`): The retire supplied to this operation.
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `Exit<(), E>`.
    /// Returns `Exit<(), E>`.
    async fn return_entry(
        &self,
        entry: EntryRef<T, E>,
        retire: bool,
        context: Context,
        reserve: CleanupReservation<E>,
    ) -> Exit<(), E> {
        let returned = catch_unwind(AssertUnwindSafe(|| (self.inner.now)()));
        if !retire && let Ok(returned_time) = returned.as_ref() {
            *entry
                .returned
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = *returned_time;
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let excess = match state.capacity {
                PoolCapacity::Unbounded => false,
                PoolCapacity::Bounded(limit) => state.resources > limit.get(),
            };
            if !state.closed && !excess {
                state.idle.push_back(entry);
                drop(state);
                self.inner.changed.notify();
                return Exit::Success(());
            }
        }
        let cleanup = self.dispose_entry(entry, context).await;
        match returned {
            Ok(_) => Self::retain_cleanup(cleanup, reserve),
            Err(panic) => {
                Self::append_cleanup(Exit::Die(Defect::from_panic(panic)), cleanup, reserve)
            }
        }
    }
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `Exit<LeaseClaim<T, E>, E>`.
    async fn acquire_entry(&self, context: Context) -> Exit<LeaseClaim<T, E>, E> {
        if self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed
        {
            return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
        }
        let mut turn = None;
        loop {
            if turn.is_none() {
                turn = Some(
                    match self
                        .inner
                        .acquiring
                        .enter(context.cancellation.clone())
                        .await
                        .into_failure()
                    {
                        Ok(permit) => permit,
                        Err(failure) => return failure,
                    },
                );
            }
            if context.cancelled() {
                return Exit::Interrupted;
            }
            let selection = {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                }
                let room = match state.capacity {
                    PoolCapacity::Unbounded => true,
                    PoolCapacity::Bounded(limit) => state.resources < limit.get(),
                };
                if state.idle.len() == 0 && !room {
                    None
                } else {
                    let control = match &self.inner.prepared_storage {
                        Some(storage) => match storage.leases.try_checkout() {
                            Ok(control) => Some(control),
                            Err(error) => {
                                drop(state);
                                drop(turn.take());
                                return Exit::Fail((storage.on_error)(
                                    PreparedSemaphoreError::Memory(error),
                                ));
                            }
                        },
                        None => None,
                    };
                    let cleanup = (|| Ok([self.reserve_cleanup()?, self.reserve_cleanup()?]))();
                    let cleanup = match cleanup {
                        Ok(cleanup) => cleanup,
                        Err(error) => {
                            drop(state);
                            drop(control);
                            drop(turn.take());
                            return Exit::Fail(self.storage_error(error));
                        }
                    };
                    let validation = if state.idle.len() == 0 {
                        Ok(None)
                    } else {
                        self.inner
                            .prepared_storage
                            .as_ref()
                            .map(|storage| {
                                PreparedNestedReservation::reserve(&storage.evaluation_frames)
                            })
                            .transpose()
                    };
                    let validation = match validation {
                        Ok(validation) => validation,
                        Err(error) => {
                            drop(state);
                            drop(control);
                            drop(cleanup);
                            drop(turn.take());
                            return Exit::Fail(self.storage_error(error));
                        }
                    };
                    if let Some(entry) = state.idle.pop_front() {
                        Some((Ok((entry, validation)), control, cleanup))
                    } else {
                        let Some(count) = state.resources.checked_add(NonZeroUsize::MIN.get())
                        else {
                            return Exit::Die(Defect::from(LibraryDefect::PoolCountOverflow));
                        };
                        let mut slot = match &self.inner.prepared_storage {
                            Some(storage) => match Slot::reserve(storage) {
                                Ok(slot) => slot,
                                Err(error) => {
                                    drop(state);
                                    drop(control);
                                    drop(cleanup);
                                    drop(turn.take());
                                    return Exit::Fail((storage.on_error)(
                                        PreparedSemaphoreError::Memory(error),
                                    ));
                                }
                            },
                            None => Slot::ordinary(),
                        };
                        state.resources = count;
                        slot.ownership.pool = Arc::downgrade(&self.inner);
                        Some((Err(slot), control, cleanup))
                    }
                }
            };
            match selection {
                Some((Err(mut slot), control, cleanup)) => {
                    drop(turn.take());
                    let created =
                        AssertUnwindSafe(self.create_value(&context, slot.evaluation.take()))
                            .catch_unwind()
                            .await
                            .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    match created.into_failure() {
                        Ok(value) => {
                            return Exit::Success(LeaseClaim {
                                entry: slot.install(value),
                                control,
                                cleanup,
                            });
                        }
                        Err(failure) => {
                            drop(slot);
                            return failure;
                        }
                    }
                }
                Some((Ok((entry, reservation)), control, cleanup_reserves)) => {
                    drop(turn.take());
                    let validation = AssertUnwindSafe(async {
                        let expired = self.inner.ttl.is_some_and(|ttl| {
                            (self.inner.now)().saturating_sub(
                                *entry
                                    .returned
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                            ) >= ttl
                        });
                        if expired {
                            Exit::Success(false)
                        } else {
                            Self::evaluate(&context, reservation, |finish| {
                                (self.inner.validate)(&entry, &context, finish)
                            })
                            .await
                        }
                    })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                    match validation {
                        Exit::Success(true) => {
                            return Exit::Success(LeaseClaim {
                                entry,
                                control,
                                cleanup: cleanup_reserves,
                            });
                        }
                        Exit::Success(false) => {
                            let cleanup = self.dispose_entry(entry, context.clone()).await;
                            if let Err(failure) = cleanup.into_failure() {
                                return failure;
                            }
                        }
                        failure @ (Exit::Fail(_)
                        | Exit::Die(_)
                        | Exit::Interrupted
                        | Exit::FinalizationFailed(_)) => {
                            let cleanup = self.dispose_entry(entry, context.clone()).await;
                            let [reserve, _] = cleanup_reserves;
                            return Self::append_cleanup(
                                failure.into_failure().err().unwrap_or(Exit::Interrupted),
                                cleanup,
                                reserve,
                            );
                        }
                    }
                }
                None => {
                    // Only the FIFO permit holder waits for a returned resource.
                    let changed = self.inner.changed.wait_observer(|| {
                        let state = self
                            .inner
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        state.closed
                            || state.idle.len() != 0
                            || match state.capacity {
                                PoolCapacity::Unbounded => true,
                                PoolCapacity::Bounded(limit) => state.resources < limit.get(),
                            }
                    });
                    let cancelled = match self.cancellation_wait(&context.cancellation) {
                        Ok(cancelled) => cancelled,
                        Err(error) => {
                            drop(turn.take());
                            return Exit::Fail(self.storage_error(error));
                        }
                    };
                    match select(std::pin::pin!(changed), std::pin::pin!(cancelled)).await {
                        Either::Left((Ok(()), _)) => {}
                        Either::Left((Err(error), _)) => {
                            drop(turn.take());
                            return Exit::Fail(self.storage_error(error));
                        }
                        Either::Right(((), _)) => return Exit::Interrupted,
                    }
                }
            }
        }
    }
    /// Acquire a resource and register the matching release obligation.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `scope` (`Scope<E>`): Owner that joins child work and releases resources at close.
    ///
    /// # Returns
    /// Type: `Effect<PoolLease<T, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn acquire<R: Send + Sync + 'static>(
        &self,
        scope: Scope<E>,
    ) -> Effect<PoolLease<T, E>, E, R> {
        let pool = self.clone();
        Effect::new(move |_, context, finish| {
            let pool = pool.clone();
            let scope = scope.clone();
            Step::suspended(Box::pin(async move {
                if pool
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .closed
                {
                    return runtime::finish(
                        finish,
                        Exit::Die(Defect::from(LibraryDefect::PoolClosed)),
                    );
                }
                let cancelled = match pool.cancellation_wait(&context.cancellation) {
                    Ok(cancelled) => cancelled,
                    Err(error) => {
                        return runtime::finish(finish, Exit::Fail(pool.storage_error(error)));
                    }
                };
                let operation = match pool.inner.cancellation.child() {
                    Ok(operation) => operation,
                    Err(error) => {
                        drop(cancelled);
                        return runtime::finish(finish, Exit::Fail(pool.storage_error(error)));
                    }
                };
                let origin = match pool.reserve_parallel() {
                    Ok(origin) => origin,
                    Err(error) => {
                        drop(operation);
                        drop(cancelled);
                        return runtime::finish(finish, Exit::Fail(pool.storage_error(error)));
                    }
                };
                let child = Self::parallel_context(&context, operation.clone(), origin);
                let acquired = match select(
                    std::pin::pin!(pool.acquire_entry(child)),
                    std::pin::pin!(cancelled),
                )
                .await
                {
                    Either::Left((outcome, _)) => outcome,
                    Either::Right(((), remaining)) => {
                        let _ = operation.request();
                        remaining.await
                    }
                };
                operation.unlink();
                match acquired.into_failure() {
                    Err(failure) => runtime::finish(finish, failure),
                    Ok(LeaseClaim {
                        entry,
                        control,
                        cleanup: [acquisition_cleanup, lease_cleanup],
                    }) => {
                        let state = LeaseState {
                            entry: Some((entry, lease_cleanup)),
                            phase: LeasePhase::Open,
                        };
                        let inner = match control {
                            Some(mut cell) => {
                                *cell
                                    .state
                                    .get_mut()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner) = state;
                                LeaseControl::Prepared(Some(cell.share()))
                            }
                            None => LeaseControl::Ordinary(Arc::new(LeaseInner::new(
                                state,
                                PoolSignal::ordinary(),
                            ))),
                        };
                        let lease = PoolLease { inner, pool };
                        let owned = lease.clone();
                        let cleanup_context = context.protected();
                        let abandoned = lease.clone();
                        let finalizer = Registered::with_abandon(
                            move |finish| {
                                Step::suspended(Box::pin(async move {
                                    let result =
                                        owned.finalize(cleanup_context, LeaseWait::Scope).await;
                                    runtime::finish(finish, result)
                                }))
                            },
                            move || abandoned.abandon(),
                        );
                        match scope.try_register(finalizer) {
                            Err(finalizer) => {
                                finalizer.execute(runtime::continuation(move |cleanup| {
                                    runtime::finish(
                                        finish,
                                        Self::append_cleanup(
                                            Exit::Die(Defect::from(
                                                LibraryDefect::ScopeRegistrationClosed,
                                            )),
                                            cleanup,
                                            acquisition_cleanup,
                                        ),
                                    )
                                }))
                            }
                            Ok(())
                                if context.cancelled()
                                    || lease
                                        .pool
                                        .inner
                                        .state
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                                        .closed =>
                            {
                                let cleanup = lease
                                    .finalize(
                                        context.protected(),
                                        LeaseWait::InterruptedAcquisition,
                                    )
                                    .await;
                                runtime::finish(
                                    finish,
                                    Self::append_cleanup(
                                        Exit::Interrupted,
                                        cleanup,
                                        acquisition_cleanup,
                                    ),
                                )
                            }
                            Ok(()) => runtime::finish(finish, Exit::Success(lease)),
                        }
                    }
                }
            }))
        })
    }
    /// Change the pool’s permitted resource count under its ownership rules.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `capacity` (`PoolCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Effect<(), E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn set_capacity<R: Send + Sync + 'static>(
        &self,
        capacity: PoolCapacity,
    ) -> Effect<(), E, R> {
        let pool = self.clone();
        Effect::ready(move |_, _| {
            let mut state = pool
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
            }
            if let AcquisitionGate::Prepared(_, limit, on_error) = &pool.inner.acquiring
                && !matches!(capacity, PoolCapacity::Bounded(size) if size <= *limit)
            {
                drop(state);
                return Exit::Fail(on_error(PreparedSemaphoreError::Semaphore(
                    SemaphoreError::CapacityExceeded,
                )));
            }
            state.capacity = capacity;
            drop(state);
            pool.inner.changed.notify();
            Exit::Success(())
        })
    }
    /// Check whether the pool has an idle resource available for reuse.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `predicate` (`impl Fn(&T) -> bool + Send + Sync + 'static`): The condition used to select or validate values.
    ///
    /// # Returns
    /// Type: `Effect<bool, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn exists_idle<R: Send + Sync + 'static>(
        &self,
        predicate: impl Fn(&T) -> bool + Send + Sync + 'static,
    ) -> Effect<bool, E, R> {
        let pool = self.clone();
        Effect::ready(move |_, context| {
            let idle = {
                let state = pool
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                }
                state.idle.iter().cloned().collect::<Vec<_>>()
            };
            let found = Self::idle_matches(&idle, &predicate);
            if context.cancelled() {
                Exit::Interrupted
            } else {
                Exit::Success(found)
            }
        })
    }
    /// Inspect idle entries with startup-reserved snapshot storage. Refuse before
    /// invoking the predicate if the snapshot cannot hold the whole idle set.
    pub fn exists_idle_prepared<R: Send + Sync + 'static>(
        &self,
        snapshots: Arc<PreparedPoolSnapshots<T, E>>,
        predicate: impl Fn(&T) -> bool + Send + Sync + 'static,
        on_error: impl Fn(MemoryError) -> E + Send + Sync + 'static,
    ) -> Effect<bool, E, R> {
        let pool = self.clone();
        Effect::ready(move |_, context| {
            let slot = match snapshots.slots.try_checkout() {
                Ok(slot) => slot,
                Err(error) => return Exit::Fail(on_error(error)),
            };
            let mut idle = std::mem::DropGuard::new(slot, |mut slot| slot.clear());
            {
                let state = pool
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed {
                    return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                }
                if state.idle.len() > idle.capacity() {
                    drop(state);
                    drop(idle);
                    return Exit::Fail(on_error(MemoryError::NoSlot));
                }
                for entry in state.idle.iter() {
                    idle.push_reserved(entry.clone());
                }
            }
            let found = Self::idle_matches(idle.as_slice(), &predicate);
            if context.cancelled() {
                Exit::Interrupted
            } else {
                Exit::Success(found)
            }
        })
    }

    fn idle_matches(entries: &[EntryRef<T, E>], predicate: &impl Fn(&T) -> bool) -> bool {
        entries.iter().any(|entry| {
            entry
                .value
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_some_and(predicate)
        })
    }
    /// Populate the pool toward the requested size through its task provider.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `target` (`usize`): The target supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<(), E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn ensure_size<R: Send + Sync + 'static>(&self, target: usize) -> Effect<(), E, R> {
        let pool = self.clone();
        Effect::new(move |_, context, finish| {
            let pool = pool.clone();
            Step::composed(Box::pin(async move {
                let work = async {
                    if pool.inner.prepared_storage.is_some() {
                        let state = pool
                            .inner
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if state.closed {
                            return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                        }
                        if state.resources >= state.population_target(target)
                            && state.population.len() == 0
                        {
                            return Exit::Success(());
                        }
                    }
                    let (mut relevant, mut jobs, retired) = match &pool.inner.prepared_storage {
                        Some(storage) => {
                            let admitted = (|| {
                                Ok((
                                    WorkList::checkout(&storage.observations)?,
                                    WorkList::checkout(&storage.jobs)?,
                                    WorkList::checkout(&storage.references)?,
                                ))
                            })();
                            match admitted {
                                Ok(work) => work,
                                Err(error) => {
                                    return Exit::Fail((storage.on_error)(
                                        PreparedSemaphoreError::Memory(error),
                                    ));
                                }
                            }
                        }
                        None => (
                            WorkList::default(),
                            WorkList::default(),
                            WorkList::default(),
                        ),
                    };
                    let cancelled = match pool.cancellation_wait(&context.cancellation) {
                        Ok(cancelled) => cancelled,
                        Err(error) => return Exit::Fail(pool.storage_error(error)),
                    };
                    let [population_cleanup, interrupted_cleanup] = {
                        let mut state = pool
                            .inner
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if state.closed {
                            return Exit::Die(Defect::from(LibraryDefect::PoolClosed));
                        }
                        let maximum = state.population_target(target);
                        let new_jobs = maximum.saturating_sub(state.resources);
                        if let Some(storage) = &pool.inner.prepared_storage
                            && new_jobs > state.population.remaining()
                        {
                            drop(state);
                            drop(relevant);
                            drop(jobs);
                            drop(retired);
                            drop(cancelled);
                            return Exit::Fail((storage.on_error)(PreparedSemaphoreError::Memory(
                                MemoryError::NoSlot,
                            )));
                        }
                        let stage = (|| {
                            for job in state.population.iter() {
                                relevant.push_reserved(PopulationObservation::new(job.clone())?);
                            }
                            for _ in 0..new_jobs {
                                let (job, slot) = match &pool.inner.prepared_storage {
                                    Some(storage) => (
                                        PopulationJob::Prepared(Some(
                                            storage.controls.try_checkout()?.share(),
                                        )),
                                        Slot::reserve(storage)?,
                                    ),
                                    None => (
                                        PopulationJob::Ordinary(Arc::new(Population::new(
                                            PoolSignal::ordinary(),
                                        ))),
                                        Slot::ordinary(),
                                    ),
                                };
                                relevant.push_reserved(PopulationObservation::new(job.clone())?);
                                jobs.push_reserved(PendingPopulation {
                                    job,
                                    slot: Some(slot),
                                    cleanup: pool.reserve_cleanup()?,
                                    cancellation: pool.inner.cancellation.child()?,
                                    origin: pool.reserve_parallel()?,
                                    task: pool.reserve_population_task()?,
                                    committed: false,
                                });
                            }
                            Ok([pool.reserve_cleanup()?, pool.reserve_cleanup()?])
                        })();
                        let cleanup = match stage {
                            Ok(cleanup) => cleanup,
                            Err(error) => {
                                drop(state);
                                drop(relevant);
                                drop(jobs);
                                drop(retired);
                                drop(cancelled);
                                return Exit::Fail(pool.storage_error(error));
                            }
                        };
                        if context.cancelled() {
                            return Exit::Interrupted;
                        }
                        for pending in jobs.iter_mut() {
                            // new_jobs is bounded by maximum - state.resources.
                            state.resources += NonZeroUsize::MIN.get();
                            state.population.push_reserved(pending.job.clone());
                            if let Some(slot) = &mut pending.slot {
                                slot.ownership.pool = Arc::downgrade(&pool.inner);
                                pending.committed = true;
                            }
                        }
                        cleanup
                    };
                    let mut cancelled = std::pin::pin!(cancelled);
                    // Unwind and cancellation retire any observed records without dropping
                    // payloads under the registry gate.
                    let _retire_observed = std::mem::DropGuard::new(retired, |mut retired| {
                        let mut state = pool
                            .inner
                            .state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        state.population.retain(|job| {
                            if job.observed() {
                                retired.push_reserved(job.clone());
                                false
                            } else {
                                true
                            }
                        });
                        drop(state);
                        drop(retired);
                    });
                    // All real slots and completion owners exist before dispatch.
                    for mut pending in jobs.drain() {
                        let job = &pending.job;
                        let Some(slot) = pending.slot.take() else {
                            continue;
                        };
                        let cleanup = pending.cleanup.take();
                        let operation = pending.cancellation.clone();
                        let child_context = Self::parallel_context(
                            &context,
                            operation.clone(),
                            pending.origin.take(),
                        );
                        let worker = PopulationWorker {
                            pool: pool.clone(),
                            job: job.clone(),
                            slot,
                            cleanup,
                            context: child_context,
                            operation,
                        };
                        let task = match worker.task(pending.task.take()) {
                            Ok(task) => task,
                            Err(_) => {
                                // Both preparation and placement use the exact same future type.
                                job.finish_dispatch(Some(Defect::from(
                                    LibraryDefect::PreparedConstructionUnbound,
                                )));
                                continue;
                            }
                        };
                        let rejection =
                            match catch_unwind(AssertUnwindSafe(|| (pool.inner.spawn)(task))) {
                                Ok(result) => result.err(),
                                Err(panic) => Some(Defect::from_panic(panic)),
                            };
                        job.finish_dispatch(rejection);
                    }
                    let mut primary = population_cleanup;
                    let mut accumulated = None;
                    for observation in relevant.drain() {
                        match select(std::pin::pin!(observation.wait()), cancelled.as_mut()).await {
                            Either::Left(((), _)) => {
                                if let Some(completed) = observation.job.take()
                                    && !matches!(completed, Exit::Success(()))
                                {
                                    accumulated = Some(match accumulated {
                                        Some(accumulator) => {
                                            PoolCleanup::append(accumulator, completed)
                                        }
                                        None => PoolCleanup::new(completed, primary.take()),
                                    });
                                }
                            }
                            Either::Right(((), _)) => {
                                let completed =
                                    accumulated.map_or(Exit::Success(()), PoolCleanup::finish);
                                return Self::append_cleanup(
                                    Exit::Interrupted,
                                    completed,
                                    interrupted_cleanup,
                                );
                            }
                        }
                    }
                    accumulated.map_or(Exit::Success(()), PoolCleanup::finish)
                };
                let outcome = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                runtime::finish(finish, outcome)
            }))
        })
    }
    ///
    /// # Parameters
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `()`.
    async fn shutdown_start(
        &self,
        context: Context,
        owning_scope: bool,
    ) -> Result<(), MemoryError> {
        let idle = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.shutdown {
                Shutdown::Open => {
                    state.closed = true;
                    state.shutdown = Shutdown::Closing;
                    state.shutdown_queue.take().map(|spare| {
                        (
                            std::mem::replace(&mut state.idle, spare),
                            std::mem::take(&mut state.shutdown_cleanup),
                        )
                    })
                }
                Shutdown::Closing | Shutdown::Closed(_) => None,
            }
        };
        if let Some((mut idle, shutdown_cleanup)) = idle {
            let mut guard = ShutdownDriver {
                pool: self.clone(),
                settled: false,
            };
            let _ = self.inner.cancellation.request();
            self.inner.acquiring.close();
            self.inner.changed.notify();
            let mut cleanup = PoolCleanup::new(Exit::Success(()), shutdown_cleanup);
            while let Some(entry) = idle.pop_front() {
                cleanup = cleanup.append(self.dispose_entry(entry, context.clone()).await);
            }
            loop {
                std::mem::swap(
                    &mut idle,
                    &mut self
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .idle,
                );
                while let Some(entry) = idle.pop_front() {
                    cleanup = cleanup.append(self.dispose_entry(entry, context.clone()).await);
                }
                let ready = || {
                    let state = self
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.resources == 0 || state.idle.len() != 0
                };
                self.inner.changed.wait_required(ready).await;
                if self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .resources
                    == 0
                {
                    break;
                }
            }
            let mut population = std::mem::take(
                &mut self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .population,
            );
            for job in population.drain() {
                job.changed.wait_required(|| job.settled()).await;
                if let Some(outcome) = job.take() {
                    cleanup = cleanup.append(outcome);
                }
            }
            let diagnostics = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .diagnostics
                .take();
            if let Some(mut diagnostics) = diagnostics {
                for diagnostic in diagnostics.drain() {
                    cleanup = cleanup.append(Exit::Die(Defect::from(diagnostic.reason)));
                }
            }
            let outcome = cleanup.finish();
            self.inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .shutdown = Shutdown::Closed(Some(outcome));
            guard.settled = true;
            self.inner.cancellation.unlink();
            self.inner.shutdown_changed.notify();
        } else {
            let settled = || {
                matches!(
                    self.inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .shutdown,
                    Shutdown::Closed(_)
                )
            };
            if owning_scope {
                self.inner.shutdown_changed.wait_required(settled).await;
            } else {
                self.inner.shutdown_changed.wait_observer(settled).await?;
            }
        }
        Ok(())
    }
    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    ///
    /// # Returns
    /// Type: `Exit<(), E>`.
    /// Returns `Exit<(), E>`.
    async fn shutdown_owned(&self, context: Context) -> Exit<(), E> {
        if let Err(error) = self.shutdown_start(context, true).await {
            return Exit::Fail(self.storage_error(error));
        }
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut state.shutdown {
            Shutdown::Closed(outcome) => outcome.take().unwrap_or(Exit::Success(())),
            Shutdown::Open | Shutdown::Closing => {
                Exit::Die(Defect::from(LibraryDefect::PoolShutdownDriverDropped))
            }
        }
    }
    /// Stop accepting new work and describe the settlement of owned work.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<PoolEndStatus, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn shutdown<R: Send + Sync + 'static>(&self) -> Effect<PoolEndStatus, E, R> {
        let pool = self.clone();
        effect_uninterruptible! { program: Effect::new(move |_, context, finish| {
            let pool = pool.clone();
            Step::suspended(Box::pin(async move {
                if let Err(error) = pool.shutdown_start(context.protected(), false).await {
                    return runtime::finish(finish, Exit::Fail(pool.storage_error(error)));
                }
                let status = {
                    let state = pool
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    match &state.shutdown {
                        Shutdown::Closed(Some(Exit::Success(()))) | Shutdown::Closed(None) => {
                            PoolEndStatus::Clean
                        }
                        Shutdown::Open
                        | Shutdown::Closing
                        | Shutdown::Closed(Some(
                            Exit::Fail(_)
                            | Exit::Die(_)
                            | Exit::Interrupted
                            | Exit::FinalizationFailed(_),
                        )) => PoolEndStatus::Failed,
                    }
                };
                runtime::finish(finish, Exit::Success(status))
            }))
        }), }
    }
}
/// Typed shutdown driver used by pool.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + Sync + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct ShutdownDriver<T: Send + Sync + 'static, E: Send + 'static> {
    pool: Pool<T, E>,
    settled: bool,
}
impl<T: Send + Sync + 'static, E: Send + 'static> Drop for ShutdownDriver<T, E> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if !self.settled {
            self.pool
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .shutdown = Shutdown::Closed(Some(Exit::Die(Defect::from(
                LibraryDefect::PoolShutdownDriverDropped,
            ))));
            self.pool.inner.shutdown_changed.notify();
        }
    }
}

/// Variants of population state used by pool.
///
/// # Type parameters
/// - `E`: Domain-error type.
enum PopulationState<E> {
    Submitting(Option<Exit<(), E>>),
    Pending,
    Settled(Option<Exit<(), E>>),
}
/// Typed population used by pool.
///
/// # Type parameters
/// - `E`: Domain-error type.
struct Population<E> {
    state: Mutex<PopulationState<E>>,
    produced: Mutex<Option<Exit<(), E>>>,
    changed: PoolSignal,
}
impl<E> Population<E> {
    fn new(changed: PoolSignal) -> Self {
        Self {
            state: Mutex::new(PopulationState::Submitting(None)),
            produced: Mutex::new(None),
            changed,
        }
    }

    fn settled(&self) -> bool {
        matches!(
            *self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            PopulationState::Settled(_)
        )
    }
    fn complete_prepared(&self, event: PreparedTaskCompletion) {
        let produced = self
            .produced
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        match event {
            PreparedTaskCompletion::Ready => self.publish(produced.unwrap_or_else(|| {
                Exit::Die(Defect::from(LibraryDefect::MissingInterpreterOutcome))
            })),
            PreparedTaskCompletion::Lost(defect) => {
                self.publish(Exit::Die(defect));
                drop(produced);
            }
        }
    }

    fn publish(&self, outcome: Exit<(), E>) {
        let ready = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &mut *state {
                PopulationState::Submitting(provisional) if provisional.is_none() => {
                    *provisional = Some(outcome);
                    false
                }
                PopulationState::Pending => {
                    *state = PopulationState::Settled(Some(outcome));
                    true
                }
                PopulationState::Submitting(_) | PopulationState::Settled(_) => false,
            }
        };
        if ready {
            self.changed.notify();
        }
    }

    fn finish_dispatch(&self, rejection: Option<Defect>) {
        let (ready, discarded) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let PopulationState::Submitting(provisional) = &mut *state else {
                return;
            };
            if let Some(defect) = rejection {
                let discarded = provisional.take();
                *state = PopulationState::Settled(Some(Exit::Die(defect)));
                (true, discarded)
            } else if let Some(outcome) = provisional.take() {
                *state = PopulationState::Settled(Some(outcome));
                (true, None)
            } else {
                *state = PopulationState::Pending;
                (false, None)
            }
        };
        if ready {
            self.changed.notify();
        }
        drop(discarded);
    }

    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Option<Exit<(), E>>`.
    /// Returns the value when present, or `None`.
    fn take(&self) -> Option<Exit<(), E>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut *state {
            PopulationState::Submitting(_) | PopulationState::Pending => None,
            PopulationState::Settled(outcome) => Some(outcome.take().unwrap_or(Exit::Success(()))),
        }
    }
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    fn observed(&self) -> bool {
        matches!(
            *self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            PopulationState::Settled(None)
        )
    }
}
