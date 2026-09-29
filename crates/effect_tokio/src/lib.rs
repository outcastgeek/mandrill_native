//! Tokio-backed task, blocking, and clock providers.
//!
//! [`TokioTasks`], [`TokioBlocking`], and [`TokioClock`] retain a caller-supplied
//! `Arc<tokio::runtime::Runtime>`; none creates a hidden runtime. An application
//! can put all three in `R`, combine them with `effect_rayon::RayonCpu`, or
//! supply its own capabilities. Task and blocking adapters enforce explicit
//! admission and expose a stop/drain boundary. Keep the runtime alive until
//! application scopes, accepted work, and provider drains have settled.

use effect::{
    AdmissionCapacity, BlockingExecutor, Cancellation, Clock, ClockError, ClockFuture, ClockTick,
    Defect, Diagnostics, LibraryDefect, ProviderAdmission, ProviderDrain, ScheduledJob,
    ScheduledTask, TaskSpawner,
};
use futures_util::future::{Either, select};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::{Runtime, RuntimeFlavor};

/// Retains the supplied runtime. Its owner keeps it alive until stop().wait()
/// and all application scopes settle. No runtime or worker policy is inferred.
#[derive(Clone)]
pub struct TokioTasks {
    runtime: Arc<Runtime>,
    admission: ProviderAdmission,
}
impl TokioTasks {
    /// Attach a multithread Tokio runtime to Effect task spawning.
    /// Reject runtimes that cannot drive independent child tasks.
    /// # Parameters
    /// - `runtime` (`Arc<Runtime>`): The runtime supplied to this operation.
    /// - `capacity` (`AdmissionCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Result<Self, Defect>`.
    pub fn new(runtime: Arc<Runtime>, capacity: AdmissionCapacity) -> Result<Self, Defect> {
        if runtime.handle().runtime_flavor() != RuntimeFlavor::MultiThread
            || runtime.metrics().num_workers() <= NonZeroUsize::MIN.get()
        {
            return Err(Defect::from(LibraryDefect::ProviderWorkersInsufficient));
        }
        Ok(Self {
            runtime,
            admission: ProviderAdmission::new(capacity),
        })
    }
    /// Borrow the same runtime owner for a synchronous application's entry point.
    /// This exposes no additional provider configuration.
    #[must_use]
    /// # Returns
    /// Type: `&Runtime`.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }
    #[must_use]
    /// Reject new tasks and return a drain for already accepted tasks.
    /// # Returns
    /// Type: `ProviderDrain`.
    pub fn stop(&self) -> ProviderDrain {
        self.admission.stop()
    }
    #[must_use]
    /// Count tasks currently admitted through this adapter.
    /// # Returns
    /// Type: `usize`.
    pub fn active(&self) -> usize {
        self.admission.active()
    }
}
impl TaskSpawner for TokioTasks {
    ///
    /// # Parameters
    /// - `task` (`ScheduledTask`): The task supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        let permit = self.admission.admit()?;
        let task = permit.task(task);
        drop(self.runtime.spawn(task));
        Ok(())
    }
}
#[derive(Clone)]
/// Blocking-job executor backed by a caller-owned Tokio runtime.
pub struct TokioBlocking {
    runtime: Arc<Runtime>,
    admission: ProviderAdmission,
    diagnostics: Diagnostics,
}
impl TokioBlocking {
    #[must_use]
    /// Attach an existing runtime's blocking pool with explicit admission.
    /// # Parameters
    /// - `runtime` (`Arc<Runtime>`): The runtime supplied to this operation.
    /// - `capacity` (`AdmissionCapacity`): The caller-supplied capacity bound.
    /// - `diagnostics` (`Diagnostics`): Separate sink for instrumentation or provider failures.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        runtime: Arc<Runtime>,
        capacity: AdmissionCapacity,
        diagnostics: Diagnostics,
    ) -> Self {
        Self {
            runtime,
            admission: ProviderAdmission::new(capacity),
            diagnostics,
        }
    }
    #[must_use]
    /// Reject new blocking jobs and return a drain for accepted jobs.
    /// # Returns
    /// Type: `ProviderDrain`.
    pub fn stop(&self) -> ProviderDrain {
        self.admission.stop()
    }
    #[must_use]
    /// Count blocking jobs currently admitted through this adapter.
    /// # Returns
    /// Type: `usize`.
    pub fn active(&self) -> usize {
        self.admission.active()
    }
}
impl BlockingExecutor for TokioBlocking {
    ///
    /// # Parameters
    /// - `job` (`ScheduledJob`): Owned synchronous callback transferred to a provider.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        let permit = self.admission.admit()?;
        let job = permit.job(job, self.diagnostics.clone());
        drop(self.runtime.spawn_blocking(move || job.run()));
        Ok(())
    }
}
#[derive(Clone)]
/// Monotonic clock and cancellable sleep backed by a caller-owned Tokio runtime.
pub struct TokioClock {
    runtime: Arc<Runtime>,
    origin: Instant,
}
impl TokioClock {
    #[must_use]
    /// Bind sleeps to a caller-owned runtime and establish a monotonic origin.
    /// # Parameters
    /// - `runtime` (`Arc<Runtime>`): The runtime supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self {
            runtime,
            origin: Instant::now(),
        }
    }
}
impl Clock for TokioClock {
    ///
    /// # Returns
    /// Type: `Duration`.
    /// Returns `Duration`.
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
    ///
    /// # Parameters
    /// - `duration` (`Duration`): The elapsed-time duration to use.
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `ClockFuture`.
    /// Returns `ClockFuture`.
    fn sleep(&self, duration: Duration, cancellation: Cancellation) -> ClockFuture {
        let runtime = Arc::clone(&self.runtime);
        Box::pin(async move {
            if cancellation.is_requested() {
                return Err(ClockError::Interrupted);
            }
            let deadline = tokio::time::Instant::now()
                .checked_add(duration)
                .ok_or(ClockError::Overflow)?;
            let timer = {
                let _entered = runtime.enter();
                tokio::time::sleep_until(deadline)
            };
            let elapsed = select(Box::pin(timer), Box::pin(cancellation.cancelled())).await;
            drop(runtime);
            match elapsed {
                Either::Left(((), _)) if cancellation.is_requested() => {
                    Err(ClockError::Interrupted)
                }
                Either::Left(((), _)) => Ok(ClockTick::elapsed()),
                Either::Right(((), _)) => Err(ClockError::Interrupted),
            }
        })
    }
}
