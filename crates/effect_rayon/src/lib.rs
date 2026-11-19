//! Rayon-backed CPU execution for the runtime-neutral Effect core.
//!
//! [`RayonCpu`] wraps a caller-owned `Arc<rayon::ThreadPool>` and an explicit
//! admission capacity. Put the adapter in an environment implementing
//! `HasCpuExecutor` to run CPU jobs; the same environment may use a different
//! provider for asynchronous tasks and blocking I/O. `stop` prevents new jobs,
//! and its drain waits for accepted jobs to settle. The adapter does not build
//! a pool or choose a worker count for the application.
//! Compose native CPU callbacks as Effect operations over `HasCpuExecutor`.
//! The caller supplies the typed error mapping and the selected pool. Job
//! admission is acquired before spawn and held through worker settlement.
//! Use [`RayonCpu::prepare`] when the application provisions fixed worker
//! storage explicitly.

use effect::{
    AdmissionCapacity, CpuExecutor, Defect, Diagnostics, ProviderAdmission, ProviderDrain,
    ScheduledJob,
};
use std::sync::Arc;
#[derive(Clone)]
/// CPU executor backed by a caller-owned Rayon pool with explicit admission.
pub struct RayonCpu {
    pool: Arc<rayon::ThreadPool>,
    admission: ProviderAdmission,
    diagnostics: Diagnostics,
}
impl RayonCpu {
    /// Prepare the compatible bounded CPU backend. Rayon ThreadPool::spawn
    /// cannot place HeapJobs into caller-provided storage, so this explicit
    /// route uses fixed workers/queue instead and never falls back to Rayon or
    /// its global pool. Rayon/dependency calls inside a callback require the
    /// callback's separately budgeted working-set contract.
    pub fn prepare<C, T, D>(
        memory: &effect::memory::PreparedMemory<'static>,
        region: effect::memory::RegionId,
        capacity: usize,
        native: effect::prepared_provider::ProviderNativeEnvelope,
    ) -> Result<
        effect::prepared_provider::PreparedProvider<
            effect::prepared_provider::SynchronousJob<C>,
            T,
            D,
        >,
        effect::prepared_provider::ProviderStartupError,
    >
    where
        C: FnOnce(effect::prepared_provider::PreparedCancellation) -> T + Send + 'static,
        T: Send + 'static,
        D: effect::prepared_provider::ProviderWorkingSet,
    {
        effect::prepared_provider::PreparedProvider::prepare(memory, region, capacity, native)
    }
    #[must_use]
    /// Attach an existing pool to Effect with explicit job admission and diagnostics.
    /// # Parameters
    /// - `pool` (`Arc<rayon::ThreadPool>`): The pool supplied to this operation.
    /// - `capacity` (`AdmissionCapacity`): The caller-supplied capacity bound.
    /// - `diagnostics` (`Diagnostics`): Separate sink for instrumentation or provider failures.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        pool: Arc<rayon::ThreadPool>,
        capacity: AdmissionCapacity,
        diagnostics: Diagnostics,
    ) -> Self {
        Self {
            pool,
            admission: ProviderAdmission::new(capacity),
            diagnostics,
        }
    }
    #[must_use]
    /// Reject future submissions and return a drain that awaits accepted jobs.
    /// # Returns
    /// Type: `ProviderDrain`.
    pub fn stop(&self) -> ProviderDrain {
        self.admission.stop()
    }
    #[must_use]
    /// Count jobs currently admitted through this adapter.
    /// # Returns
    /// Type: `usize`.
    pub fn active(&self) -> usize {
        self.admission.active()
    }
    #[must_use]
    /// Report the worker count configured on the supplied Rayon pool.
    /// # Returns
    /// Type: `usize`.
    pub fn worker_count(&self) -> usize {
        self.pool.current_num_threads()
    }
}
impl CpuExecutor for RayonCpu {
    ///
    /// # Parameters
    /// - `job` (`ScheduledJob`): Owned synchronous callback transferred to a provider.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        let permit = self.admission.admit()?;
        let job = permit.job(job, self.diagnostics.clone());
        self.pool.spawn(move || job.run());
        Ok(())
    }
}
