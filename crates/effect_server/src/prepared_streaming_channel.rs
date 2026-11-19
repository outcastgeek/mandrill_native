//! Independent startup-backed streaming channels. Native pull is not transport settlement.
use bytes::Bytes;
use effect::memory::{
    Bytes as MemoryBytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use effect::{
    Exit, PreparedCleanupAccumulator, PreparedCleanupConfig, PreparedLatch, PreparedLatchWait,
    PreparedOneshotPool, PreparedPanicPool, PreparedPanicReservation, PreparedQueue,
    PreparedQueueCapacity, PreparedQueueOfferReservation, PreparedQueueReceiver, PreparedReceiver,
    PreparedSender, QueueCapacity, QueueError, QueueShutdown,
};
use http_body::Frame;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

/// Explicit physical streaming-channel and per-channel operation capacities.
#[derive(Clone, Copy, Debug)]
pub struct PreparedBodyChannelCapacity {
    /// Total reusable channels in this host.
    pub channels: usize,
    /// Actual inline pending frame cells per channel.
    pub payloads_per_channel: usize,
    /// Simultaneous accepted writer/acknowledgement records per channel.
    pub writers_per_channel: usize,
    /// Simultaneous blocking codec sinks per channel.
    pub blocking_sinks_per_channel: usize,
}
/// Region payload and separately classified startup System Arc controls.
#[derive(Clone, Copy, Debug)]
pub struct PreparedBodyChannelDemand {
    /// Region payload and pool metadata; allocator overhead and frame backing are separate.
    pub region_bytes: MemoryBytes,
    /// Record controls plus the host-owned pool control.
    pub startup_system_arc_controls: usize,
    /// Requested Arc payload bytes, excluding headers, padding and allocator rounding.
    pub startup_system_arc_payload_bytes: MemoryBytes,
    /// Independently held queue cleanup carriers per channel before data operations.
    pub shutdown_cleanup_buffers_per_channel: usize,
    /// Independently held queue panic diagnostics per channel before data operations.
    pub shutdown_diagnostics_per_channel: usize,
    /// Queue, native poll, receiver-retirement and frame-retirement diagnostics.
    pub receiver_diagnostics_per_channel: usize,
    /// Additional receiver acknowledgement cleanup carrier.
    pub receiver_cleanup_buffers_per_channel: usize,
    /// E-side carriers preadmitted per reply channel: three failure mappers and one aggregate.
    pub application_cleanup_buffers_per_channel: usize,
    /// Queue diagnostics needed by each simultaneously admitted writer.
    pub diagnostics_per_writer: usize,
    /// Queue cleanup carrier needed by each simultaneously admitted writer.
    pub cleanup_buffers_per_writer: usize,
    /// E cleanup carriers per admitted blocking sink (mapper plus aggregate).
    pub application_cleanup_buffers_per_blocking_sink: usize,
}
impl PreparedBodyChannelCapacity {
    /// Derive selected-region storage and separately supplied control authorities.
    pub fn demand(
        self,
        policy: crate::BodyPolicy,
    ) -> Result<PreparedBodyChannelDemand, MemoryError> {
        BodyChannelPool::demand(policy, self)
    }
}
struct Delivery {
    frame: Frame<Bytes>,
    accepted: PreparedSender<()>,
}
struct State {
    first: bool,
    open: bool,
    active: usize,
    desired: Option<QueueShutdown>,
    executing: bool,
    drain_done: bool,
    discard_done: bool,
    receiver_failure_in_flight: bool,
    receiver_live: bool,
    failures: [Option<(u8, BodyChannelFailure)>; 3],
    next_failure: u8,
}
struct BlockingWake { thread: SharedSlot<'static, Mutex<Option<std::thread::Thread>>> }
impl std::task::Wake for BlockingWake {
    fn wake(self: Arc<Self>) { self.wake_by_ref(); }
    fn wake_by_ref(self: &Arc<Self>) { let thread = self.thread.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone(); if let Some(thread) = thread { thread.unpark(); } }
}
type BlockingRegistry = Mutex<FixedVector<'static, Arc<BlockingWake>>>;
struct Record {
    queue: Mutex<Option<PreparedQueue<Delivery>>>,
    acks: PreparedOneshotPool<()>,
    blocking_wakes: BlockingRegistry,
    blocking_failures: PreparedOneshotPool<RawBlockingFailure>,
    panics: PreparedPanicPool,
    cleanup: PreparedCleanupConfig<QueueError>,
    state: Mutex<State>,
}
struct Control {
    settlement: PreparedLatch,
    record: SharedSlot<'static, Record>,
}
type Registry = Mutex<FixedVector<'static, Option<Arc<Control>>>>;
/// Startup-owned registry; no channel handle owns this registry.
pub(crate) struct BodyChannelPool {
    registry: SharedSlot<'static, Registry>,
    bytes_pool: Option<Arc<crate::PreparedNativeBytesPool>>,
    json_writers: Option<Arc<effect::PreparedJsonWriterPool>>,
}
/// A reserved native receiver plus a writer sharing a startup control.
pub(crate) struct BodyChannelVacancy {
    receiver: NativeReceiver,
    writer: Writer,
    settlement: SettlementWait,
}
/// One preadmitted channel-retirement observer; its record outlives the waiter.
pub(crate) struct SettlementWait {
    wait: PreparedLatchWait,
    _control: Arc<Control>,
}
impl Future for SettlementWait {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut self.wait).poll(cx)
    }
}
/// Cloneable writer access; admitted writes have separate physical guards.
pub(crate) struct Writer {
    control: Arc<Control>,
    bytes_pool: Option<Arc<crate::PreparedNativeBytesPool>>,
    json_writers: Option<Arc<effect::PreparedJsonWriterPool>>,
}
/// Native consumer owns its receiver before any body program starts.
pub(crate) struct NativeReceiver {
    receiver: Option<PreparedQueueReceiver<Delivery>>,
    first: Option<PreparedPanicReservation>,
    second: Option<PreparedPanicReservation>,
    third: Option<PreparedPanicReservation>,
    cleanup: Option<effect::CleanupFailures<QueueError>>,
    control: Arc<Control>,
    ended: bool,
}
/// Raw queue failures are moved out for the host's typed aggregation.
#[derive(Clone, Copy)]
pub(crate) enum FailurePhase { Drain, Discard, Native }
pub(crate) enum BodyChannelFailure {
    Queue(Exit<(), QueueError>),
    Memory(MemoryError),
    MemoryWithCleanup {
        primary: MemoryError,
        cleanup: Exit<(), QueueError>,
    },
}
/// Result of an individual preadmitted frame write.
pub(crate) enum RawBlockingFailure { Memory(MemoryError), Disconnected, Queue(Exit<(), QueueError>) }
pub(crate) struct BlockingSink {
    writer: Writer,
    wake: Arc<BlockingWake>,
    failure: Option<PreparedSender<RawBlockingFailure>>,
}
pub(crate) enum WriteStatus {
    Accepted,
    Disconnected,
    QueueFailed(Exit<(), QueueError>),
}
/// Output of the native receiver; acknowledgement occurs only on Item.
pub(crate) enum Pull {
    Item(Frame<Bytes>),
    End,
    Failed,
}
fn array<T>(count: usize) -> Result<MemoryBytes, MemoryError> {
    Ok(MemoryBytes::new(
        std::alloc::Layout::array::<T>(count)
            .map_err(|_| MemoryError::InvalidLayout)?
            .size(),
    ))
}
impl BodyChannelPool {
    pub(crate) fn demand(
        policy: crate::BodyPolicy,
        capacity: PreparedBodyChannelCapacity,
    ) -> Result<PreparedBodyChannelDemand, MemoryError> {
        let q = PreparedQueue::<Delivery>::demand(
            QueueCapacity::Bounded(policy.frames),
            PreparedQueueCapacity {
                payloads: capacity.payloads_per_channel,
                producers: capacity.writers_per_channel,
                takers: 1,
            },
        )?;
        let blocking = SlotPool::<Mutex<Option<std::thread::Thread>>>::storage_bytes(capacity.blocking_sinks_per_channel)?
            .checked_add(array::<Arc<BlockingWake>>(capacity.blocking_sinks_per_channel)?)?
            .checked_add(PreparedOneshotPool::<RawBlockingFailure>::storage_bytes(capacity.blocking_sinks_per_channel)?)?;
        let per_channel = q
            .region_bytes
            .checked_add(PreparedOneshotPool::<()>::storage_bytes(
                capacity.writers_per_channel,
            )?)?.checked_add(blocking)?;
        let region_bytes = SlotPool::<Record>::storage_bytes(capacity.channels)?
            .checked_add(SlotPool::<Registry>::storage_bytes(1)?)?
            .checked_add(array::<Option<Arc<Control>>>(capacity.channels)?)?
            .checked_add(
                per_channel
                    .checked_add(PreparedLatch::storage_bytes(1)?)?
                    .checked_mul(capacity.channels)?,
            )?;
        let wake_count = capacity.channels.checked_mul(capacity.blocking_sinks_per_channel).ok_or(MemoryError::ArithmeticOverflow)?;
        let system = MemoryBytes::new(std::mem::size_of::<BlockingWake>()).checked_mul(wake_count)?.checked_add(MemoryBytes::new(std::mem::size_of::<Control>())
            .checked_mul(capacity.channels)?)?
            .checked_add(MemoryBytes::new(std::mem::size_of::<Self>()))?;
        Ok(PreparedBodyChannelDemand {
            region_bytes,
            startup_system_arc_controls: capacity
                .channels
                .checked_add(wake_count).and_then(|count| count.checked_add(1))
                .ok_or(MemoryError::ArithmeticOverflow)?,
            startup_system_arc_payload_bytes: system,
            shutdown_cleanup_buffers_per_channel: q.shutdown_cleanup_buffers,
            shutdown_diagnostics_per_channel: q.shutdown_panic_slots,
            receiver_diagnostics_per_channel: q
                .take_panic_slots
                .checked_add(3)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            receiver_cleanup_buffers_per_channel: 2,
            application_cleanup_buffers_per_channel: 4,
            diagnostics_per_writer: q.offer_panic_slots,
            cleanup_buffers_per_writer: 1,
            application_cleanup_buffers_per_blocking_sink: 2,
        })
    }
    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        policy: crate::BodyPolicy,
        capacity: PreparedBodyChannelCapacity,
        cleanup: &PreparedCleanupConfig<QueueError>,
        panics: &PreparedPanicPool,
    ) -> Result<Self, MemoryError> {
        let _ = Self::demand(policy, capacity)?;
        let slots = memory.slots(region, capacity.channels, |_| {
            let queue = PreparedQueue::prepare(
                memory,
                region,
                QueueCapacity::Bounded(policy.frames),
                PreparedQueueCapacity {
                    payloads: capacity.payloads_per_channel,
                    producers: capacity.writers_per_channel,
                    takers: 1,
                },
                cleanup,
                panics,
            )?;
            let acks = PreparedOneshotPool::prepare(memory, region, capacity.writers_per_channel)?;
            let wake_slots = memory.slots(region, capacity.blocking_sinks_per_channel, |_| Ok(Mutex::new(None)))?;
            let mut wakes = FixedVector::prepare(memory.allocator(region)?, capacity.blocking_sinks_per_channel)?;
            for _ in 0..capacity.blocking_sinks_per_channel {
                wakes.push(Arc::new(BlockingWake { thread: wake_slots.checkout()?.share() })).map_err(|_| MemoryError::NoSlot)?;
            }
            let blocking_failures = PreparedOneshotPool::prepare(memory, region, capacity.blocking_sinks_per_channel)?;
            Ok(Record {
                queue: Mutex::new(Some(queue)),
                acks,
                blocking_wakes: Mutex::new(wakes),
                blocking_failures,
                panics: panics.try_clone()?,
                cleanup: cleanup.try_clone()?,
                state: Mutex::new(State {
                    first: true,
                    open: false,
                    active: 0,
                    desired: None,
                    executing: false,
                    drain_done: false,
                    discard_done: false,
                    receiver_failure_in_flight: false,
                    receiver_live: false,
                    failures: [None, None, None],
                    next_failure: 0,
                }),
            })
        })?;
        let mut controls = FixedVector::prepare(memory.allocator(region)?, capacity.channels)?;
        for _ in 0..capacity.channels {
            controls
                .push(Some(Arc::new(Control {
                    settlement: PreparedLatch::prepare(memory, region, 1)?,
                    record: slots.checkout()?.share(),
                })))
                .map_err(|_| MemoryError::NoSlot)?;
        }
        let mut controls = Some(controls);
        let registry = memory.slots(region, 1, |_| {
            Ok(Mutex::new(controls.take().ok_or(MemoryError::NoSlot)?))
        })?;
        Ok(Self {
            registry: registry.checkout()?.share(),
            bytes_pool: None,
            json_writers: None,
        })
    }
}
struct RegistryClaim<'a> {
    registry: &'a Registry,
    index: usize,
    control: Option<Arc<Control>>,
}
impl Drop for RegistryClaim<'_> {
    #[allow(clippy::expect_used)]
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            let mut registry = self.registry.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = registry
                .as_mut_slice()
                .get_mut(self.index)
                .expect("reserved channel index");
            debug_assert!(entry.is_none());
            *entry = Some(control);
        }
    }
}
struct QueueRestore<'a> {
    record: &'a Record,
    queue: Option<PreparedQueue<Delivery>>,
}
impl Drop for QueueRestore<'_> {
    fn drop(&mut self) {
        if let Some(queue) = self.queue.take() {
            let mut gate = self
                .record
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            debug_assert!(gate.is_none());
            *gate = Some(queue);
        }
    }
}
struct ReopenRollback<'a> {
    record: &'a Record,
    queue: &'a mut PreparedQueue<Delivery>,
    armed: bool,
}
impl Drop for ReopenRollback<'_> {
    fn drop(&mut self) {
        if self.armed {
            let outcome = self.queue.shutdown_now(QueueShutdown::Discard);
            {
                let mut state = self
                    .record
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.first = false;
            }
            match outcome {
                Ok(Exit::Success(())) => {}
                Ok(outcome) => {
                    record_failure_record(self.record, 1, BodyChannelFailure::Queue(outcome))
                }
                Err(error) => {
                    record_failure_record(self.record, 1, BodyChannelFailure::Memory(error))
                }
            }
        }
    }
}
impl BodyChannelPool {
    /// Reserve a unique record, reopen its actual queue if needed, and admit
    /// the persistent native receiver before publishing either endpoint.
    #[allow(clippy::expect_used)]
    pub(crate) fn reserve(&self) -> Result<BodyChannelVacancy, MemoryError> {
        let (index, control) = {
            let mut registry = crate::try_admission_lock(&self.registry)?;
            let mut chosen = None;
            for (index, entry) in registry.as_mut_slice().iter_mut().enumerate() {
                if let Some(control) = entry.as_mut()
                    && Arc::get_mut(control).is_some()
                {
                    let state = control
                        .record
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let ready = state.failures.iter().all(Option::is_none)
                        && !state.open
                        && state.active == 0
                        && !state.executing
                        && !state.receiver_live;
                    drop(state);
                    if ready {
                        chosen = entry.take().map(|control| (index, control));
                        break;
                    }
                }
            }
            chosen.ok_or(MemoryError::NoSlot)?
        };
        let mut claim = RegistryClaim {
            registry: &self.registry,
            index,
            control: Some(control),
        };
        let unique =
            Arc::get_mut(claim.control.as_mut().expect("claimed channel")).expect("unique channel");
        unique.settlement.reset()?;
        let settlement = unique.settlement.wait()?;
        let record = &unique.record;
        let queue = record
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or(MemoryError::ConstructionUnbound)?;
        let mut queue = QueueRestore {
            record,
            queue: Some(queue),
        };
        let first_diagnostic = record.panics.try_reserve()?;
        let second_diagnostic = record.panics.try_reserve()?;
        let third_diagnostic = record.panics.try_reserve()?;
        let cleanup = record.cleanup.reserve_failures()?;
        let first = record
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .first;
        let queue_ref = queue.queue.as_mut().expect("held channel queue");
        if !first {
            queue_ref.rearm_quiescent()?;
        }
        let mut rollback = ReopenRollback {
            record,
            queue: queue_ref,
            armed: !first,
        };
        let receiver = rollback.queue.native_receiver()?;
        rollback.armed = false;
        drop(rollback);
        {
            let mut state = record
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.first = false;
            state.open = true;
            state.receiver_live = true;
            state.desired = None;
            state.drain_done = false;
            state.discard_done = false;
            state.next_failure = 0;
        }
        drop(queue);
        let control = claim.control.as_ref().expect("claimed channel");
        let vacancy = BodyChannelVacancy {
            receiver: NativeReceiver {
                receiver: Some(receiver),
                first: Some(first_diagnostic),
                second: Some(second_diagnostic),
                third: Some(third_diagnostic),
                cleanup: Some(cleanup),
                control: Arc::clone(control),
                ended: false,
            },
            writer: Writer {
                control: Arc::clone(control),
                bytes_pool: self.bytes_pool.clone(),
                json_writers: self.json_writers.clone(),
            },
            settlement: SettlementWait {
                wait: settlement,
                _control: Arc::clone(control),
            },
        };
        drop(claim);
        Ok(vacancy)
    }
    /// Recover one previously retained close or native-pull failure, if any.
    pub(crate) fn take_failure(&self) -> Option<(FailurePhase, BodyChannelFailure)> {
        let control = {
            let registry = self
                .registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registry
                .as_slice()
                .iter()
                .flatten()
                .find(|control| {
                    let state = control
                        .record
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    settled(&state)
                        && !state.receiver_failure_in_flight
                        && state.failures.iter().any(Option::is_some)
                })
                .map(Arc::clone)
        }?;
        take_failure(&control)
    }
}
impl BodyChannelPool {
    pub(crate) fn install_json_writers(&mut self, pool: Arc<effect::PreparedJsonWriterPool>) -> Result<(), MemoryError> {
        if self.json_writers.is_some() { return Err(MemoryError::Busy); }
        self.json_writers = Some(pool);
        Ok(())
    }
    pub(crate) fn install_bytes_pool(&mut self, pool: Arc<crate::PreparedNativeBytesPool>) -> Result<(), MemoryError> {
        if self.bytes_pool.is_some() { return Err(MemoryError::Busy); }
        self.bytes_pool = Some(pool);
        Ok(())
    }
}
impl BodyChannelVacancy {
    pub(crate) fn split(self) -> (NativeReceiver, Writer, SettlementWait) {
        (self.receiver, self.writer, self.settlement)
    }
}
impl Clone for Writer {
    fn clone(&self) -> Self {
        Self {
            control: Arc::clone(&self.control),
            bytes_pool: self.bytes_pool.clone(),
            json_writers: self.json_writers.clone(),
        }
    }
}

impl Writer {
    pub(crate) fn json_writers(&self) -> Option<&Arc<effect::PreparedJsonWriterPool>> { self.json_writers.as_ref() }
    pub(crate) fn bytes_pool(&self) -> Option<&Arc<crate::PreparedNativeBytesPool>> { self.bytes_pool.as_ref() }
    pub(crate) fn blocking_sink(&self) -> Result<(BlockingSink, PreparedReceiver<RawBlockingFailure>), MemoryError> {
        let wake = {
            let mut wakes = self.control.record.blocking_wakes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            wakes.as_mut_slice().iter_mut().find_map(|wake| if Arc::get_mut(wake).is_some() { Some(Arc::clone(wake)) } else { None }).ok_or(MemoryError::NoSlot)?
        };
        let (failure, receiver) = self.control.record.blocking_failures.try_channel()?;
        Ok((BlockingSink { writer: self.clone(), wake, failure: Some(failure) }, receiver))
    }
    /// Reserve acknowledgement and queue backing before constructing a frame.
    pub(crate) fn begin(&self) -> Result<WritePermit, MemoryError> {
        if !self
            .control
            .record
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .open
        {
            return Err(MemoryError::Closed);
        }
        let (accepted, wait) = self.control.record.acks.try_channel()?;
        let queue = {
            let queue = self
                .control
                .record
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            queue.as_ref().ok_or(MemoryError::Closed)?.try_clone()?
        };
        let reservation = queue.reserve_native_offer()?;
        drop(queue);
        let mut state = self
            .control
            .record
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.open {
            drop(state);
            return Err(MemoryError::Closed);
        }
        state.active = state
            .active
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        drop(state);
        Ok(WritePermit {
            reservation,
            accepted,
            wait,
            guard: WriteGuard {
                control: Arc::clone(&self.control),
            },
        })
    }
    /// Reject new writes; accepted writes retain their owners until retirement.
    pub(crate) fn finish(&self) {
        request_close(&self.control, QueueShutdown::Drain);
    }
    /// Take one raw close/native-pull failure for later host aggregation.
    pub(crate) fn take_failure(&self) -> Option<(FailurePhase, BodyChannelFailure)> {
        take_failure(&self.control)
    }
}
impl BlockingSink {
    pub(crate) fn write_chunk(&mut self, bytes: &[u8]) -> Result<(), ()> {
        if self.failure.is_none() { return Err(()); }
        let permit = match self.writer.begin() { Ok(permit) => permit, Err(MemoryError::Closed) => return self.fail(RawBlockingFailure::Disconnected), Err(error) => return self.fail(RawBlockingFailure::Memory(error)) };
        let bytes = match &self.writer.bytes_pool { Some(pool) => match pool.try_copy(bytes) { Ok(bytes) => bytes, Err(error) => return self.fail(RawBlockingFailure::Memory(error)) }, None => Bytes::copy_from_slice(bytes) };
        let frame = Frame::data(bytes);
        let future = permit.send(frame);
        let result = self.wait(future);
        match result {
            WriteStatus::Accepted => Ok(()),
            WriteStatus::Disconnected => self.fail(RawBlockingFailure::Disconnected),
            WriteStatus::QueueFailed(exit) => self.fail(RawBlockingFailure::Queue(exit)),
        }
    }
    fn fail(&mut self, failure: RawBlockingFailure) -> Result<(), ()> {
        if let Some(sender) = self.failure.take() { let _ = sender.send(failure); }
        Err(())
    }
    fn wait<F: Future<Output = WriteStatus>>(&self, future: F) -> WriteStatus {
        struct Binding<'a>(&'a BlockingWake);
        impl Drop for Binding<'_> { fn drop(&mut self) { let old = self.0.thread.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take(); drop(old); } }
        *self.wake.thread.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(std::thread::current());
        let binding = Binding(&self.wake);
        let outcome = {
            let waker = std::task::Waker::from(Arc::clone(&self.wake));
            let mut context = Context::from_waker(&waker);
            let mut future = std::pin::pin!(future);
            loop { match future.as_mut().poll(&mut context) { Poll::Ready(value) => break value, Poll::Pending => std::thread::park() } }
        };
        drop(binding);
        outcome
    }
}
/// Field order retires both physical offer and ack endpoints before the write count.
pub(crate) struct WritePermit {
    reservation: PreparedQueueOfferReservation<Delivery>,
    accepted: PreparedSender<()>,
    wait: PreparedReceiver<()>,
    guard: WriteGuard,
}
struct WriteGuard {
    control: Arc<Control>,
}
impl Drop for WriteGuard {
    fn drop(&mut self) {
        {
            let mut state = self
                .control
                .record
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            debug_assert!(state.active > 0);
            state.active -= 1;
        }
        drive_close(&self.control);
        notify_settled(&self.control);
    }
}
/// The pinned future is destroyed before the active write guard retires.
pub(crate) struct WriteFuture<F> {
    future: Option<F>,
    guard: Option<WriteGuard>,
}
impl<F: Future<Output = WriteStatus>> Future for WriteFuture<F> {
    type Output = WriteStatus;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: future is structurally pinned; set(None) drops it in place.
        let mut future = unsafe { self.as_mut().map_unchecked_mut(|this| &mut this.future) };
        let outcome = match future.as_mut().as_pin_mut() {
            Some(future) => match future.poll(cx) {
                Poll::Ready(value) => value,
                Poll::Pending => return Poll::Pending,
            },
            None => return Poll::Pending,
        };
        future.set(None);
        // SAFETY: guard is not pinned and is taken only after the future was destroyed.
        let guard = unsafe { self.get_unchecked_mut() }.guard.take();
        drop(guard);
        Poll::Ready(outcome)
    }
}
impl WritePermit {
    /// Install the frame after admission and await actual native pull acknowledgement.
    pub(crate) fn send(
        self,
        frame: Frame<Bytes>,
    ) -> WriteFuture<impl Future<Output = WriteStatus> + Send> {
        let Self {
            reservation,
            accepted,
            wait,
            guard,
        } = self;
        let offer = reservation.start(move || Delivery { frame, accepted });
        let future = async move {
            match offer.await {
                Exit::Success(()) => match wait.await {
                    Ok(()) => WriteStatus::Accepted,
                    Err(_) => WriteStatus::Disconnected,
                },
                failure @ (Exit::Fail(_)
                | Exit::Die(_)
                | Exit::Interrupted
                | Exit::FinalizationFailed(_)) => WriteStatus::QueueFailed(failure),
            }
        };
        WriteFuture {
            future: Some(future),
            guard: Some(guard),
        }
    }
}
enum Polled {
    Item,
    End,
    Queue(Exit<(), QueueError>),
    Memory(MemoryError),
}
impl NativeReceiver {
    /// Poll under a preadmitted panic boundary; acknowledgement occurs only on pull.
    #[allow(clippy::expect_used)]
    pub(crate) fn poll_pull(&mut self, cx: &mut Context<'_>) -> Poll<Pull> {
        if self.ended {
            return Poll::Ready(Pull::End);
        }
        let diagnostic = self
            .first
            .take()
            .expect("preadmitted native poll diagnostic");
        let mut frame = None;
        let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let receiver = self.receiver.as_mut().expect("live native receiver");
            match receiver.poll_take(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(Exit::Success(item))) => {
                    let Delivery {
                        frame: item,
                        accepted,
                    } = item.into_inner();
                    frame = Some(item);
                    let _ = accepted.send(());
                    Poll::Ready(Polled::Item)
                }
                Poll::Ready(Ok(Exit::Fail(QueueError::Shutdown))) => Poll::Ready(Polled::End),
                Poll::Ready(Ok(Exit::Die(defect))) => Poll::Ready(Polled::Queue(Exit::Die(defect))),
                Poll::Ready(Ok(Exit::Interrupted)) => Poll::Ready(Polled::Queue(Exit::Interrupted)),
                Poll::Ready(Ok(Exit::FinalizationFailed(details))) => {
                    Poll::Ready(Polled::Queue(Exit::FinalizationFailed(details)))
                }
                Poll::Ready(Err(error)) => Poll::Ready(Polled::Memory(error)),
            }
        }));
        match polled {
            Ok(Poll::Pending) => {
                self.first = Some(diagnostic);
                Poll::Pending
            }
            Ok(Poll::Ready(Polled::Item)) => {
                self.first = Some(diagnostic);
                Poll::Ready(Pull::Item(frame.take().expect("pulled frame")))
            }
            Ok(Poll::Ready(Polled::End)) => {
                self.first = Some(diagnostic);
                self.ended = true;
                Poll::Ready(Pull::End)
            }
            Ok(Poll::Ready(Polled::Queue(outcome))) => {
                self.first = Some(diagnostic);
                self.ended = true;
                record_failure(&self.control, 2, BodyChannelFailure::Queue(outcome));
                Poll::Ready(Pull::Failed)
            }
            Ok(Poll::Ready(Polled::Memory(error))) => {
                self.first = Some(diagnostic);
                self.ended = true;
                record_failure(&self.control, 2, BodyChannelFailure::Memory(error));
                Poll::Ready(Pull::Failed)
            }
            Err(payload) => {
                self.ended = true;
                let cleanup = self.cleanup.take().expect("preadmitted receiver cleanup");
                let receiver_retirement = self
                    .second
                    .take()
                    .expect("preadmitted receiver retirement diagnostic");
                let receiver = self.receiver.take();
                let receiver_panic =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(receiver))).err();
                let frame_retirement = self
                    .third
                    .take()
                    .expect("preadmitted frame retirement diagnostic");
                let frame_panic =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(frame))).err();
                let mut outcome = PreparedCleanupAccumulator::from_reserved(
                    Exit::Die(diagnostic.capture(payload)),
                    cleanup,
                );
                if let Some(payload) = receiver_panic {
                    outcome.append_cleanup(Exit::Die(receiver_retirement.capture(payload)));
                }
                if let Some(payload) = frame_panic {
                    outcome.append_cleanup(Exit::Die(frame_retirement.capture(payload)));
                }
                record_failure(
                    &self.control,
                    2,
                    BodyChannelFailure::Queue(outcome.finish()),
                );
                request_close(&self.control, QueueShutdown::Discard);
                Poll::Ready(Pull::Failed)
            }
        }
    }
}
impl Drop for NativeReceiver {
    #[allow(clippy::expect_used)] // A live receiver retains its preadmitted retirement authorities.
    fn drop(&mut self) {
        struct Disconnect<'a>(&'a Arc<Control>);
        impl Drop for Disconnect<'_> {
            fn drop(&mut self) {
                request_close(self.0, QueueShutdown::Discard);
            }
        }
        struct Retired<'a>(&'a Control);
        impl Drop for Retired<'_> {
            fn drop(&mut self) {
                let mut state = self
                    .0
                    .record
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.receiver_live = false;
                drop(state);
                notify_settled(self.0);
            }
        }
        let retired = Retired(&self.control);
        let close = Disconnect(&self.control);
        if let Some(receiver) = self.receiver.take() {
            let mut failure = ReceiverFailureGuard::new(&self.control);
            let diagnostic = self
                .second
                .take()
                .expect("preadmitted receiver retirement diagnostic");
            if let Err(payload) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(receiver)))
            {
                let cleanup = self.cleanup.take().expect("preadmitted receiver cleanup");
                failure.append(diagnostic.capture(payload), cleanup);
            }
            drop(failure);
        }
        drop(close);
        drop(retired);
    }
}

// The three fixed positions are independent authorities: Drain, Discard, and
// the receiver's one terminal native failure. No phase can publish twice.
#[allow(clippy::expect_used)] // Indices are the three private, fixed phase constants.
fn store_failure(state: &mut State, index: usize, failure: BodyChannelFailure) {
    let slot = state.failures.get_mut(index).expect("valid failure phase");
    debug_assert!(slot.is_none());
    let order = state.next_failure;
    state.next_failure += 1;
    *slot = Some((order, failure));
}
fn record_failure_record(record: &Record, index: usize, failure: BodyChannelFailure) {
    let mut state = record
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    store_failure(&mut state, index, failure);
}
fn record_failure(control: &Control, index: usize, failure: BodyChannelFailure) {
    record_failure_record(&control.record, index, failure);
}
fn take_failure(control: &Control) -> Option<(FailurePhase, BodyChannelFailure)> {
    let mut state = control
        .record
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if state.receiver_failure_in_flight || !settled(&state) {
        return None;
    }
    let index = state
        .failures
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.as_ref().map(|(order, _)| (index, *order)))
        .min_by_key(|(_, order)| *order)
        .map(|(index, _)| index)?;
    state
        .failures
        .get_mut(index)?
        .take()
        .map(|(_, failure)| (match index { 0 => FailurePhase::Drain, 1 => FailurePhase::Discard, _ => FailurePhase::Native }, failure))
}
// During receiver retirement keep an existing primary owned outside the gate;
// observers cannot overtake it until its cleanup is rejoined and restored.
struct ReceiverFailureGuard<'a> {
    control: &'a Control,
    previous: Option<(u8, BodyChannelFailure)>,
}
impl<'a> ReceiverFailureGuard<'a> {
    fn new(control: &'a Control) -> Self {
        let mut state = control
            .record
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert!(!state.receiver_failure_in_flight);
        state.receiver_failure_in_flight = true;
        let previous = state.failures[2].take();
        drop(state);
        Self { control, previous }
    }
    #[allow(clippy::unreachable)] // Unique receiver retirement can append only once.
    fn append(&mut self, defect: effect::Defect, cleanup: effect::CleanupFailures<QueueError>) {
        let (order, primary) = match self.previous.take() {
            Some((order, BodyChannelFailure::Queue(primary))) => (Some(order), Ok(primary)),
            Some((order, BodyChannelFailure::Memory(primary))) => (Some(order), Err(primary)),
            Some((_order, BodyChannelFailure::MemoryWithCleanup { .. })) => {
                unreachable!("receiver retirement appends only once")
            }
            None => (None, Ok(Exit::Success(()))),
        };
        let result = match primary {
            Ok(primary) => {
                let mut accumulator = PreparedCleanupAccumulator::from_reserved(primary, cleanup);
                accumulator.append_cleanup(Exit::Die(defect));
                BodyChannelFailure::Queue(accumulator.finish())
            }
            Err(primary) => {
                let mut accumulator =
                    PreparedCleanupAccumulator::from_reserved(Exit::Success(()), cleanup);
                accumulator.append_cleanup(Exit::Die(defect));
                BodyChannelFailure::MemoryWithCleanup {
                    primary,
                    cleanup: accumulator.finish(),
                }
            }
        };
        if let Some(order) = order {
            self.previous = Some((order, result));
        } else {
            let mut state = self
                .control
                .record
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let order = state.next_failure;
            state.next_failure += 1;
            drop(state);
            self.previous = Some((order, result));
        }
    }
}
impl Drop for ReceiverFailureGuard<'_> {
    fn drop(&mut self) {
        let mut state = self
            .control
            .record
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert!(state.failures[2].is_none());
        state.failures[2] = self.previous.take();
        state.receiver_failure_in_flight = false;
    }
}
fn settled(state: &State) -> bool {
    !state.open
        && state.discard_done
        && !state.receiver_live
        && state.active == 0
        && !state.executing
        && state.desired.is_none()
        && !state.receiver_failure_in_flight
}
fn notify_settled(control: &Control) {
    let ready = {
        let state = control
            .record
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        settled(&state)
    };
    if ready {
        let _ = control.settlement.open_now();
    }
}
fn request_close(control: &Arc<Control>, mode: QueueShutdown) {
    {
        let mut state = control
            .record
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.open = false;
        match mode {
            QueueShutdown::Drain
                if !state.drain_done
                    && !state.discard_done
                    && !matches!(state.desired, Some(QueueShutdown::Discard)) =>
            {
                state.desired = Some(QueueShutdown::Drain)
            }
            QueueShutdown::Discard if !state.discard_done => {
                state.desired = Some(QueueShutdown::Discard)
            }
            QueueShutdown::Drain | QueueShutdown::Discard => {}
        }
    }
    drive_close(control);
}
struct CloseContinuation<'a> {
    control: &'a Arc<Control>,
    mode: QueueShutdown,
    armed: bool,
}
impl Drop for CloseContinuation<'_> {
    fn drop(&mut self) {
        if self.armed {
            {
                let mut state = self
                    .control
                    .record
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match self.mode {
                    QueueShutdown::Discard if !state.discard_done => {
                        state.desired = Some(QueueShutdown::Discard)
                    }
                    QueueShutdown::Drain
                        if !state.drain_done
                            && !state.discard_done
                            && !matches!(state.desired, Some(QueueShutdown::Discard)) =>
                    {
                        state.desired = Some(QueueShutdown::Drain)
                    }
                    QueueShutdown::Drain | QueueShutdown::Discard => {}
                }
                state.executing = false;
            }
            drive_close(self.control);
        }
    }
}
fn drive_close(control: &Arc<Control>) {
    loop {
        let mode = {
            let mut state = control
                .record
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.executing {
                return;
            }
            match state.desired {
                Some(QueueShutdown::Drain) if state.active != 0 => return,
                Some(mode) => {
                    state.desired = None;
                    state.executing = true;
                    mode
                }
                None => return,
            }
        };
        let mut continuation = CloseContinuation {
            control,
            mode,
            armed: true,
        };
        let queue = control
            .record
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let result = match queue {
            Some(queue) => {
                let queue = QueueRestore {
                    record: &control.record,
                    queue: Some(queue),
                };
                #[allow(clippy::expect_used)]
                // QueueRestore uniquely owns this queue until restoration.
                let result = queue.queue.as_ref().expect("held queue").shutdown_now(mode);
                drop(queue);
                result
            }
            None => Err(MemoryError::Busy),
        };
        {
            let mut state = control
                .record
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let index = match mode {
                QueueShutdown::Drain => 0,
                QueueShutdown::Discard => 1,
            };
            match result {
                Ok(Exit::Success(())) => {}
                Ok(outcome) => store_failure(&mut state, index, BodyChannelFailure::Queue(outcome)),
                Err(error) => store_failure(&mut state, index, BodyChannelFailure::Memory(error)),
            }
            match mode {
                QueueShutdown::Drain => state.drain_done = true,
                QueueShutdown::Discard => state.discard_done = true,
            }
            if state.discard_done
                || state.drain_done && matches!(state.desired, Some(QueueShutdown::Drain))
            {
                state.desired = None;
            }
            state.executing = false;
        }
        continuation.armed = false;
        notify_settled(control);
    }
}
