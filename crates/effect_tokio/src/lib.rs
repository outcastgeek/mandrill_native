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
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }
    #[must_use]
    pub fn stop(&self) -> ProviderDrain {
        self.admission.stop()
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.admission.active()
    }
}
impl TaskSpawner for TokioTasks {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        let permit = self.admission.admit()?;
        let task = permit.task(task);
        drop(self.runtime.spawn(task));
        Ok(())
    }
}
#[derive(Clone)]
pub struct TokioBlocking {
    runtime: Arc<Runtime>,
    admission: ProviderAdmission,
    diagnostics: Diagnostics,
}
impl TokioBlocking {
    #[must_use]
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
    pub fn stop(&self) -> ProviderDrain {
        self.admission.stop()
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.admission.active()
    }
}
impl BlockingExecutor for TokioBlocking {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        let permit = self.admission.admit()?;
        let job = permit.job(job, self.diagnostics.clone());
        drop(self.runtime.spawn_blocking(move || job.run()));
        Ok(())
    }
}
#[derive(Clone)]
pub struct TokioClock {
    runtime: Arc<Runtime>,
    origin: Instant,
}
impl TokioClock {
    #[must_use]
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self {
            runtime,
            origin: Instant::now(),
        }
    }
}
impl Clock for TokioClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
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
