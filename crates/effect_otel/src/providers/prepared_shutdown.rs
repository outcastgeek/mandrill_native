//! Independently reserved shutdown commands and failures on the fixed blocking provider.

use super::{BorrowedProviders, LifecyclePhase, ShutdownPolicy, Signal, TERMINAL_SIGNALS};
use crate::NativeError;
use effect::memory::{
    Bytes, FixedVector, FramePool, FrameReservation, MemoryError, PreparedMemory, RegionId,
    SharedSlot, SlotLease, SlotPool,
};
use effect::prepared_provider::{
    PreparedExecutionLease, PreparedProvider, PreparedSubmission, PreparedSubmitError,
    ProviderBudget, ProviderNativeEnvelope, ProviderRefusal, ProviderStartupError,
    ProviderWorkerPanics, ProviderWorkingSet,
};
use effect::{
    Exit, LibraryDefect, PreparedDeferred, PreparedPanicPool, PreparedPanicReservation,
    PreparedProviderDrain,
};
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use std::{
    alloc::Layout,
    fmt,
    future::Future,
    num::NonZeroUsize,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
    time::Instant,
};

// Three flush failures, and at most one deadline + one SDK failure for each
// shutdown. Startup rollback is strictly narrower (one acquisition + two closes).
const FAILURE_RECORDS: usize = 9;
const NATIVE_PANICS: usize = 6;
const PARKED_SHUTDOWN_PANICS: usize = NATIVE_PANICS + 1;
type Failures = FixedVector<'static, PreparedLifecycleFailure>;
type ShutdownExit = Exit<(), PreparedLifecycleError>;
type Staging = Mutex<Stage>;
type Worker<D> = PreparedProvider<ShutdownJob, (), WorkingSet<D>>;
type Submission<D> = PreparedSubmission<ShutdownJob, (), WorkingSet<D>>;
type WorkingSetSlot<D> = Mutex<Option<D>>;

struct WorkingSet<D: ProviderWorkingSet>(Option<SharedSlot<'static, WorkingSetSlot<D>>>);

impl<D: ProviderWorkingSet> WorkingSet<D> {
    fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self(
            self.0.as_ref().map(SharedSlot::try_clone).transpose()?,
        ))
    }
}

impl<D: ProviderWorkingSet> Drop for WorkingSet<D> {
    fn drop(&mut self) {
        if let Some(mut slot) = self.0.take().and_then(SharedSlot::into_last) {
            let working_set = slot
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(working_set);
        }
    }
}

// SAFETY: Both the worker and the escaped report hold counted aliases to the
// original admitted D, with no additional transitive working allocation. Only
// the final owner drops D, after its actual native report payload has retired.
unsafe impl<D: ProviderWorkingSet> ProviderWorkingSet for WorkingSet<D> {
    const CONTRACT: &'static str = D::CONTRACT;
}

struct ReportStorage {
    failures: Failures,
    working_set: Mutex<Option<PreparedExecutionLease>>,
}

impl ReportStorage {
    fn retire(&mut self) {
        self.failures.clear();
        drop(
            self.working_set
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
    }
}

/// A native cleanup cause retained in the selected report's actual backing.
#[derive(Debug)]
pub enum PreparedLifecycleCause {
    /// The SDK error stays in its original typed representation.
    Sdk(OTelSdkError),
    /// A native acquisition failure or a panic captured in prepared diagnostics.
    Native(NativeError),
    /// The shared shutdown budget expired before this operation.
    BudgetExpired,
}

/// One provider failure in the established lifecycle order.
#[derive(Debug)]
pub struct PreparedLifecycleFailure {
    /// Provider whose acknowledgment failed.
    pub signal: Signal,
    /// Operation that failed.
    pub phase: LifecyclePhase,
    /// Original typed cause.
    pub cause: PreparedLifecycleCause,
}

/// The actual bounded report retained across all prepared close observers.
pub struct PreparedLifecycleError {
    records: Option<SharedSlot<'static, ReportStorage>>,
    discarded: usize,
    refusal: Option<ProviderRefusal>,
}

impl PreparedLifecycleError {
    /// Borrow the primary and ordered subsequent failures.
    #[must_use]
    pub fn failures(&self) -> &[PreparedLifecycleFailure] {
        self.records
            .as_ref()
            .map_or(&[], |records| records.failures.as_slice())
    }

    /// Number of later causes beyond the retained report, saturating on overflow.
    #[must_use]
    pub const fn discarded(&self) -> usize {
        self.discarded
    }

    /// A failure to hand off the admitted worker command.
    #[must_use]
    pub const fn provider_refusal(&self) -> Option<ProviderRefusal> {
        self.refusal
    }

    pub(super) const fn refusal(refusal: ProviderRefusal) -> Self {
        Self {
            records: None,
            discarded: 0,
            refusal: Some(refusal),
        }
    }

    /// Retain the same batch without cloning native errors or their payloads.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            records: self
                .records
                .as_ref()
                .map(SharedSlot::try_clone)
                .transpose()?,
            discarded: self.discarded,
            refusal: self.refusal,
        })
    }
}

impl Drop for PreparedLifecycleError {
    fn drop(&mut self) {
        if let Some(mut owner) = self.records.take().and_then(SharedSlot::into_last) {
            owner.retire();
        }
    }
}

impl fmt::Debug for PreparedLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedLifecycleError")
            .field("failures", &self.failures())
            .field("discarded", &self.discarded)
            .field("refusal", &self.refusal)
            .finish()
    }
}

impl fmt::Display for PreparedLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(first) = self.failures().first() {
            write!(
                formatter,
                "telemetry lifecycle failed at {:?}/{:?} with {} retained and {} discarded failures",
                first.signal,
                first.phase,
                self.failures().len(),
                self.discarded,
            )
        } else if let Some(refusal) = self.refusal {
            write!(
                formatter,
                "telemetry shutdown worker refused the command: {refusal:?}"
            )
        } else {
            formatter.write_str("telemetry lifecycle report is empty")
        }
    }
}

impl std::error::Error for PreparedLifecycleError {}

pub(super) struct PreparedShutdownReport {
    records: Option<SlotLease<'static, ReportStorage>>,
    panics: [Option<PreparedPanicReservation>; PARKED_SHUTDOWN_PANICS],
    discarded: usize,
}

impl PreparedShutdownReport {
    fn next_panic(&mut self) -> Option<PreparedPanicReservation> {
        self.panics.iter_mut().find_map(Option::take)
    }

    fn record(&mut self, signal: Signal, phase: LifecyclePhase, cause: PreparedLifecycleCause) {
        if let Some(records) = &mut self.records
            && records
                .failures
                .push(PreparedLifecycleFailure {
                    signal,
                    phase,
                    cause,
                })
                .is_ok()
        {
            return;
        }
        self.discarded = self.discarded.saturating_add(1);
    }

    pub(super) fn catch_native<T>(
        &mut self,
        action: impl FnOnce() -> Result<T, NativeError>,
    ) -> Result<T, NativeError> {
        catch_unwind(AssertUnwindSafe(action)).unwrap_or_else(|panic| {
            let defect = match self.next_panic() {
                Some(slot) => slot.capture(panic),
                None => LibraryDefect::ProviderCapacityFull.into(),
            };
            Err(NativeError::Native(defect))
        })
    }

    pub(super) fn record_native(
        &mut self,
        signal: Signal,
        phase: LifecyclePhase,
        cause: NativeError,
    ) {
        self.record(signal, phase, PreparedLifecycleCause::Native(cause));
    }

    pub(super) fn record_sdk(
        &mut self,
        signal: Signal,
        phase: LifecyclePhase,
        action: impl FnOnce() -> OTelSdkResult,
    ) {
        match self.catch_native(|| Ok(action())) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => self.record(signal, phase, PreparedLifecycleCause::Sdk(error)),
            Err(error) => self.record_native(signal, phase, error),
        }
    }

    pub(super) fn budget_expired(&mut self, signal: Signal) {
        self.record(
            signal,
            LifecyclePhase::Shutdown,
            PreparedLifecycleCause::BudgetExpired,
        );
    }

    pub(super) fn finish(mut self) -> Result<(), PreparedLifecycleError> {
        let Some(mut records) = self.records.take() else {
            return Ok(());
        };
        if records.failures.is_empty() {
            records.retire();
            return Ok(());
        }
        Err(PreparedLifecycleError {
            records: Some(records.share()),
            discarded: self.discarded,
            refusal: None,
        })
    }
}
impl Drop for PreparedShutdownReport {
    fn drop(&mut self) {
        if let Some(records) = &mut self.records {
            records.retire();
        }
    }
}

/// Refusal before selected provider construction or SDK callbacks.
#[derive(Debug)]
pub enum PreparedShutdownRefusal {
    /// The selected report, frame, or diagnostic working set could not be admitted.
    Memory(MemoryError),
    /// The pre-started provider refused the physical command reservation.
    Provider(ProviderRefusal),
}
impl From<MemoryError> for PreparedShutdownRefusal {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}
impl From<ProviderRefusal> for PreparedShutdownRefusal {
    fn from(error: ProviderRefusal) -> Self {
        Self::Provider(error)
    }
}

/// A startup-prepared blocking worker pool dedicated to managed OTel shutdown.
pub struct PreparedShutdownWorkers<D: ProviderWorkingSet> {
    worker: Worker<D>,
    reports: SlotPool<'static, ReportStorage>,
    working_sets: SlotPool<'static, WorkingSetSlot<D>>,
    report_guards: FramePool<'static>,
    staging: SlotPool<'static, Staging>,
    publisher: FramePool<'static>,
    panics: PreparedPanicPool,
}

impl<D: ProviderWorkingSet> PreparedShutdownWorkers<D> {
    /// Checked owner layouts, excluding allocator headers and separately budgeted native storage.
    /// Each installation additionally holds up to seven slots from the selected panic pool.
    pub fn storage_bytes(
        installations: NonZeroUsize,
        workers: usize,
    ) -> Result<Bytes, MemoryError> {
        let installations = installations.get();
        let report = Layout::array::<PreparedLifecycleFailure>(FAILURE_RECORDS)
            .map_err(|_| MemoryError::InvalidLayout)?;
        Worker::<D>::storage_bytes(installations, workers)?
            .checked_add(SlotPool::<ReportStorage>::storage_bytes(installations)?)?
            .checked_add(SlotPool::<WorkingSetSlot<D>>::storage_bytes(installations)?)?
            .checked_add(FramePool::storage_bytes(
                installations,
                Layout::new::<WorkingSet<D>>(),
            )?)?
            .checked_add(Bytes::new(report.size()).checked_mul(installations)?)?
            .checked_add(SlotPool::<Staging>::storage_bytes(installations)?)?
            .checked_add(FramePool::storage_bytes(
                installations,
                Layout::new::<Publish>(),
            )?)
    }

    /// Prepare an independent native blocking lane and its typed report backing.
    /// The supplied native envelope and working-set witness cover the SDK and worker domains.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        installations: NonZeroUsize,
        native: ProviderNativeEnvelope,
        panics: &PreparedPanicPool,
    ) -> Result<Self, ProviderStartupError> {
        Self::storage_bytes(installations, native.workers)?;
        let count = installations.get();
        let reports = memory.slots(region, count, |_| {
            Ok(ReportStorage {
                failures: FixedVector::prepare(memory.allocator(region)?, FAILURE_RECORDS)?,
                working_set: Mutex::new(None),
            })
        })?;
        let working_sets = memory.slots(region, count, |_| Ok(Mutex::new(None)))?;
        let report_guards = memory.frames(region, count, Layout::new::<WorkingSet<D>>())?;
        let staging = memory.slots(region, count, |_| Ok(Mutex::new(Stage::default())))?;
        let publisher = memory.frames(region, count, Layout::new::<Publish>())?;
        let panics = panics.try_clone()?;
        let worker = Worker::prepare(memory, region, count, native)?;
        Ok(Self {
            worker,
            reports,
            working_sets,
            report_guards,
            staging,
            publisher,
            panics,
        })
    }

    /// Reserve a real worker command, report, completion frame, and diagnostics before SDK setup.
    pub fn reserve(
        &self,
        budget: ProviderBudget<D>,
    ) -> Result<PreparedShutdown<D>, PreparedShutdownRefusal> {
        let submission = self.worker.submitter()?.reserve()?;
        let mut records = self.reports.try_checkout()?;
        let mut working_set = self.working_sets.try_checkout()?;
        let report_guard = self.report_guards.reserve()?;
        let mut staged = self.staging.try_checkout()?;
        *staged
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Stage::default();
        let staged = staged.share();
        let publisher_stage = staged.try_clone()?;
        let worker_stage = staged.try_clone()?;
        let publisher_frame = self.publisher.reserve()?;
        let mut panics = self.panics.try_reserve_group::<PARKED_SHUTDOWN_PANICS>()?;
        let unexpected = panics
            .last_mut()
            .and_then(Option::take)
            .ok_or(MemoryError::NoSlot)?;
        let ProviderBudget {
            working_set: native_work,
            notification,
            worker_retirement,
            execution_retirement,
        } = budget;
        *working_set
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(native_work);
        let working_set = WorkingSet(Some(working_set.share()));
        *records
            .working_set
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(
            PreparedExecutionLease::new(report_guard, working_set.try_clone()?),
        );
        Ok(PreparedShutdown {
            submission,
            budget: ProviderBudget {
                working_set,
                notification,
                worker_retirement,
                execution_retirement,
            },
            report: PreparedShutdownReport {
                records: Some(records),
                panics,
                discarded: 0,
            },
            staged,
            publisher_stage,
            worker_stage,
            publisher_frame,
            unexpected,
        })
    }

    /// Join the dedicated native workers after every reserved lifecycle has settled or retired.
    pub fn join(&mut self) {
        self.worker.join();
    }

    /// Inspect any native worker failures after the worker owner joins.
    #[must_use]
    pub fn worker_panics(&self) -> &ProviderWorkerPanics {
        self.worker.worker_panics()
    }
}

/// One physical close command held independently of all measurement and export admission.
pub struct PreparedShutdown<D: ProviderWorkingSet> {
    submission: Submission<D>,
    budget: ProviderBudget<WorkingSet<D>>,
    report: PreparedShutdownReport,
    staged: SharedSlot<'static, Staging>,
    publisher_stage: SharedSlot<'static, Staging>,
    worker_stage: SharedSlot<'static, Staging>,
    publisher_frame: FrameReservation<'static, Publish>,
    unexpected: PreparedPanicReservation,
}

impl<D: ProviderWorkingSet> PreparedShutdown<D> {
    pub(super) fn submit(
        self,
        providers: BorrowedProviders,
        policy: ShutdownPolicy,
        drain: PreparedProviderDrain,
        completion: PreparedDeferred<(), PreparedLifecycleError>,
    ) -> Result<(), ProviderRefusal> {
        let Self {
            submission,
            budget,
            report,
            staged,
            publisher_stage,
            worker_stage,
            publisher_frame,
            unexpected,
        } = self;
        staged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .completion = Some(completion);
        let rollback = std::mem::DropGuard::new(&*staged, |stage| {
            update_stage(stage, |state| state.decided = true);
        });
        let publisher = PreparedExecutionLease::new(publisher_frame, Publish(publisher_stage));
        let submitted = submission
            .with_post_retirement_lease(publisher)
            .submit(
                || Ok::<_, !>(budget),
                |_, _| ShutdownJob {
                    drain: Some(drain),
                    job: Some((providers, policy, report, unexpected)),
                    staged: worker_stage,
                },
            )
            .map_err(|error| match error {
                PreparedSubmitError::Refused(refusal) => refusal,
                PreparedSubmitError::WorkingSet(never) => never,
            });
        update_stage(&staged, |state| {
            state.decided = true;
            if let Err(refusal) = &submitted {
                state.result = Some(Exit::Fail(PreparedLifecycleError::refusal(*refusal)));
            }
        });
        std::mem::DropGuard::dismiss(rollback);
        // Committed cleanup completes independently of cancellation requests.
        drop(submitted?);
        Ok(())
    }
}

#[derive(Default)]
struct Stage {
    completion: Option<PreparedDeferred<(), PreparedLifecycleError>>,
    result: Option<ShutdownExit>,
    decided: bool,
    retired: bool,
}

fn update_stage(stage: &Staging, update: impl FnOnce(&mut Stage)) {
    let ready = {
        let mut stage = stage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        update(&mut stage);
        if stage.decided && stage.retired {
            stage.completion.take().map(|completion| {
                let result = stage
                    .result
                    .take()
                    .unwrap_or_else(|| Exit::Die(LibraryDefect::ScheduledJobDropped.into()));
                (completion, result)
            })
        } else {
            None
        }
    };
    if let Some((completion, result)) = ready {
        let _ = completion.publish_now(result);
    }
}

// Core drops this frame only after actual command, result, budget and provider
// entry retirement. The submitter independently commits the handoff decision.
struct Publish(SharedSlot<'static, Staging>);

impl Drop for Publish {
    fn drop(&mut self) {
        update_stage(&self.0, |stage| stage.retired = true);
    }
}

struct ShutdownJob {
    drain: Option<PreparedProviderDrain>,
    job: Option<(
        BorrowedProviders,
        ShutdownPolicy,
        PreparedShutdownReport,
        PreparedPanicReservation,
    )>,
    staged: SharedSlot<'static, Staging>,
}

impl Future for ShutdownJob {
    type Output = ();
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if let Some(drain) = &mut this.drain
            && Pin::new(drain).poll(context).is_pending()
        {
            return Poll::Pending;
        }
        drop(this.drain.take());
        if let Some((providers, policy, mut report, unexpected)) = this.job.take() {
            let result = catch_unwind(AssertUnwindSafe(|| {
                let started = Instant::now();
                if policy.flush {
                    flush(&providers, &mut report);
                }
                shutdown(&providers, started, policy, &mut report);
                drop(providers);
                report.finish()
            }));
            let result = match result {
                Ok(Ok(())) => Exit::Success(()),
                Ok(Err(error)) => Exit::Fail(error),
                Err(panic) => Exit::Die(unexpected.capture(panic)),
            };
            this.staged
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .result = Some(result);
        }
        Poll::Ready(())
    }
}

fn flush(providers: &BorrowedProviders, report: &mut PreparedShutdownReport) {
    for signal in TERMINAL_SIGNALS {
        report.record_sdk(signal, LifecyclePhase::Flush, || {
            providers.flush_signal(signal)
        });
    }
}

fn shutdown(
    providers: &BorrowedProviders,
    started: Instant,
    policy: ShutdownPolicy,
    report: &mut PreparedShutdownReport,
) {
    for signal in TERMINAL_SIGNALS {
        let remaining = policy.budget.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            report.budget_expired(signal);
        }
        report.record_sdk(signal, LifecyclePhase::Shutdown, || {
            providers.shutdown_signal(signal, remaining)
        });
    }
}
