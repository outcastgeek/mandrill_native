//! Independent startup-prepared body completion records.
use crate::BodyOutcome;
use effect::memory::{
    Admission, AdmissionBundle, Bytes, CapacityUnit, FixedVector, MemoryError, PreparedMemory,
    RegionId, SharedSlot, SlotPool,
};
use effect::{Cancellation, PreparedLatch, PreparedLatchWait};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

const OPEN: u8 = 0;
const COMPLETED: u8 = 1;
const DISCONNECTED: u8 = 2;
const FAILED: u8 = 3;
struct Record {
    loans: Arc<Mutex<usize>>,
    outcome: AtomicU8,
    armed: AtomicBool,
    cancellation: Mutex<Option<Cancellation>>,
    quota: Mutex<Option<AdmissionBundle<'static, 1>>>,
}
struct Control {
    latch: PreparedLatch,
    record: SharedSlot<'static, Record>,
}
type Registry = Mutex<FixedVector<'static, Arc<Control>>>;
pub(crate) struct BodyPool {
    registry: SharedSlot<'static, Registry>,
    channels: Option<Arc<crate::prepared_streaming_channel::BodyChannelPool>>,
    admission: Option<Admission<'static, 1>>,
}
pub(crate) struct BodyRef {
    control: ControlOwner,
}
struct ControlOwner(Option<Arc<Control>>);
impl std::ops::Deref for ControlOwner {
    type Target = Arc<Control>;
    #[expect(clippy::unreachable, reason = "a live body reference retains its control until Drop")]
    fn deref(&self) -> &Self::Target {
        let Some(control) = &self.0 else { unreachable!("live body control"); };
        control
    }
}
/// Field order keeps the physical waiter until before its body owner retires.
pub(crate) struct BodyWait {
    waiter: PreparedLatchWait,
    owner: BodyRef,
}
pub(crate) struct PreparedBodyExitResources<E: Send + 'static> {
    pub(crate) mappers: [effect::PreparedExitMapper<E>; 3],
    pub(crate) cleanup: effect::CleanupFailures<E>,
    pub(crate) config: Arc<effect::PreparedCleanupConfig<E>>,
}
pub(crate) struct ReplyBodyVacancy<E: Send + 'static> {
    completion: BodyVacancy,
    channel: Option<crate::prepared_streaming_channel::BodyChannelVacancy>,
    resources: Option<PreparedBodyExitResources<E>>,
}
pub(crate) struct BodyVacancy {
    waiter: PreparedLatchWait,
    owner: BodyRef,
}

impl BodyPool {
    pub(crate) fn storage_bytes(records: usize, listeners: usize) -> Result<Bytes, MemoryError> {
        let vector = std::alloc::Layout::array::<Arc<Control>>(records)
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Record>::storage_bytes(records)?
            .checked_add(SlotPool::<Registry>::storage_bytes(1)?)?
            .checked_add(Bytes::new(vector.size()))?
            .checked_add(PreparedLatch::storage_bytes(listeners)?.checked_mul(records)?)
    }
    /// System Arc payload size; the Arc header and allocator overhead are additional.
    pub(crate) fn control_payload_layout() -> std::alloc::Layout {
        std::alloc::Layout::new::<Control>()
    }
    pub(crate) fn pool_payload_layout() -> std::alloc::Layout {
        std::alloc::Layout::new::<Self>()
    }
    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        records: usize,
        listeners: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(records, listeners)?;
        let slots = memory.slots(region, records, |_| {
            Ok(Record {
                loans: Arc::new(Mutex::new(0)),
                outcome: AtomicU8::new(OPEN),
                armed: AtomicBool::new(false),
                cancellation: Mutex::new(None),
                quota: Mutex::new(None),
            })
        })?;
        let mut controls = FixedVector::prepare(memory.allocator(region)?, records)?;
        for _ in 0..records {
            let control = Arc::new(Control {
                latch: PreparedLatch::prepare(memory, region, listeners)?,
                record: slots.checkout()?.share(),
            });
            if let Err(control) = controls.push(control) {
                drop(control);
                return Err(MemoryError::NoSlot);
            }
        }
        let mut controls = Some(controls);
        let registry = memory.slots(region, 1, |_| {
            Ok(Mutex::new(controls.take().ok_or(MemoryError::NoSlot)?))
        })?;
        Ok(Self {
            registry: registry.checkout()?.share(),
            channels: None,
            admission: None,
        })
    }
    pub(crate) fn install_admission(
        &mut self,
        admission: Admission<'static, 1>,
    ) -> Result<(), MemoryError> {
        if admission.capacities()[0].unit != CapacityUnit::Slots {
            return Err(MemoryError::UnsupportedRunContract);
        }
        self.admission = Some(admission);
        Ok(())
    }
    pub(crate) fn install_bytes_pool(&mut self, pool: Arc<crate::PreparedNativeBytesPool>) -> Result<(), MemoryError> {
        let channels = self.channels.as_mut().ok_or(MemoryError::UnsupportedRunContract)?;
        Arc::get_mut(channels).ok_or(MemoryError::Busy)?.install_bytes_pool(pool)
    }
    pub(crate) fn install_json_writers(&mut self, pool: Arc<effect::PreparedJsonWriterPool>) -> Result<(), MemoryError> {
        let channels = self.channels.as_mut().ok_or(MemoryError::UnsupportedRunContract)?;
        Arc::get_mut(channels).ok_or(MemoryError::Busy)?.install_json_writers(pool)
    }
    pub(crate) fn channels_configured(&self) -> bool { self.channels.is_some() }
    pub(crate) fn take_channel_failure(&self) -> Option<(crate::prepared_streaming_channel::FailurePhase, crate::prepared_streaming_channel::BodyChannelFailure)> {
        self.channels.as_ref()?.take_failure()
    }
    pub(crate) fn install_channels(&mut self, channels: crate::prepared_streaming_channel::BodyChannelPool) -> Result<(), MemoryError> {
        if self.channels.is_some() { return Err(MemoryError::Busy); }
        self.channels = Some(Arc::new(channels));
        Ok(())
    }
    pub(crate) fn reserve_reply<E: Send + 'static>(&self, config: &Arc<effect::PreparedCleanupConfig<E>>) -> Result<ReplyBodyVacancy<E>, MemoryError> {
        let completion = self.reserve()?;
        let channel = self.channels.as_ref().map(|pool| pool.reserve()).transpose()?;
        let resources = if channel.is_some() { Some(PreparedBodyExitResources { mappers: [effect::PreparedExitMapper::admit(config)?, effect::PreparedExitMapper::admit(config)?, effect::PreparedExitMapper::admit(config)?], cleanup: config.reserve_failures()?, config: Arc::clone(config) }) } else { None };
        Ok(ReplyBodyVacancy { completion, channel, resources })
    }
    /// Reserve both a body record and its first physical wait before callbacks.
    pub(crate) fn reserve(&self) -> Result<BodyVacancy, MemoryError> {
        let owner = {
            let mut registry = crate::try_admission_lock(&self.registry)?;
            let mut owner = None;
            for control in registry.as_mut_slice() {
                let Some(unique) = Arc::get_mut(control) else {
                    continue;
                };
                unique.latch.reset()?;
                unique.record.outcome.store(OPEN, Ordering::Release);
                unique.record.armed.store(false, Ordering::Release);
                *unique
                    .record
                    .loans
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = 1;
                owner = Some(BodyRef {
                    control: ControlOwner(Some(Arc::clone(control))),
                });
                break;
            }
            owner.ok_or(MemoryError::NoSlot)?
        };
        let waiter = owner.control.latch.wait()?;
        if let Some(admission) = &self.admission {
            let quota = admission.reserve([1], [0])?;
            *owner.control.record.quota.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(quota);
        }
        Ok(BodyVacancy { waiter, owner })
    }
}
impl<E: Send + 'static> ReplyBodyVacancy<E> {
    pub(crate) fn publish(self, cancellation: Cancellation) -> crate::body::BodyAdmission<E> {
        let (completion, wait) = self.completion.publish(cancellation);
        crate::body::BodyAdmission { completion, wait, channel: self.channel, resources: self.resources }
    }
}
impl BodyVacancy {
    /// Claim a further physical waiter before the body is published. The
    /// eventual publication supplies cancellation; this waiter is not polled here.
    pub(crate) fn admit_wait(&self) -> Result<BodyWait, MemoryError> {
        self.owner.admit_wait()
    }
    pub(crate) fn publish(self, cancellation: Cancellation) -> (BodyRef, BodyWait) {
        let Self { waiter, owner } = self;
        *owner
            .control
            .record
            .cancellation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cancellation);
        let wait_owner = owner.clone();
        (
            owner,
            BodyWait {
                waiter,
                owner: wait_owner,
            },
        )
    }
}
impl BodyRef {
    pub(crate) fn admit_wait(&self) -> Result<BodyWait, MemoryError> {
        let waiter = self.control.latch.wait()?;
        Ok(BodyWait {
            waiter,
            owner: self.clone(),
        })
    }
    pub(crate) fn arm(&self) {
        self.control.record.armed.store(true, Ordering::Release);
    }
    pub(crate) fn outcome(&self) -> BodyOutcome {
        match self.control.record.outcome.load(Ordering::Acquire) {
            COMPLETED => BodyOutcome::Completed,
            DISCONNECTED => BodyOutcome::Disconnected,
            FAILED | 4..=8 => BodyOutcome::Failed,
            _ => BodyOutcome::Open,
        }
    }
    pub(crate) fn output_limit(&self) -> Option<crate::OutputLimit> { crate::OutputLimit::from_code(self.control.record.outcome.load(Ordering::Acquire)) }
    pub(crate) fn fail_limit(&self, limit: crate::OutputLimit) {
        if self.control.record.outcome.compare_exchange(OPEN, limit.code(), Ordering::AcqRel, Ordering::Acquire).is_ok() { let _ = self.control.latch.open_now(); }
    }
    pub(crate) fn settle(&self, outcome: BodyOutcome) {
        let next = match outcome {
            BodyOutcome::Open => return,
            BodyOutcome::Completed => COMPLETED,
            BodyOutcome::Disconnected => DISCONNECTED,
            BodyOutcome::Failed => FAILED,
        };
        if self
            .control
            .record
            .outcome
            .compare_exchange(OPEN, next, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let _ = self.control.latch.open_now();
        if next == DISCONNECTED && self.control.record.armed.load(Ordering::Acquire) {
            let cancellation = self
                .control
                .record
                .cancellation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some(cancellation) = cancellation {
                let _ = cancellation.request();
            }
        }
    }
}
impl Clone for BodyRef {
    fn clone(&self) -> Self {
        let control = Arc::clone(&self.control);
        {
            let mut loans = control
                .record
                .loans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *loans += 1;
        }
        Self { control: ControlOwner(Some(control)) }
    }
}
impl Drop for BodyRef {
    fn drop(&mut self) {
        // The independent gate survives control retirement; quota drops last on unwind.
        let quota;
        let Some(control) = self.control.0.take() else { return; };
        let gate = Arc::clone(&control.record.loans);
        {
            let mut loans = gate.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if *loans != 1 {
                drop(control);
                *loans -= 1;
                return;
            }
            *loans = 0;
        }
        quota = control.record.quota.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner).take();
        let cancellation = control
            .record
            .cancellation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(cancellation);
        drop(control);
        drop(quota);
    }
}
impl std::future::Future for BodyWait {
    type Output = BodyOutcome;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        match std::pin::Pin::new(&mut this.waiter).poll(cx) {
            std::task::Poll::Ready(()) => std::task::Poll::Ready(this.owner.outcome()),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}
impl BodyWait {
    pub(crate) fn signal(&self) -> BodyRef {
        self.owner.clone()
    }
}
