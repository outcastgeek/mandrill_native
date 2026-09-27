use effect::{
    AdmissionCapacity, CpuExecutor, Defect, Diagnostics, ProviderAdmission, ProviderDrain,
    ScheduledJob,
};
use std::sync::Arc;
#[derive(Clone)]
pub struct RayonCpu {
    pool: Arc<rayon::ThreadPool>,
    admission: ProviderAdmission,
    diagnostics: Diagnostics,
}
impl RayonCpu {
    #[must_use]
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
    pub fn stop(&self) -> ProviderDrain {
        self.admission.stop()
    }
    #[must_use]
    pub fn active(&self) -> usize {
        self.admission.active()
    }
    #[must_use]
    pub fn worker_count(&self) -> usize {
        self.pool.current_num_threads()
    }
}
impl CpuExecutor for RayonCpu {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        let permit = self.admission.admit()?;
        let job = permit.job(job, self.diagnostics.clone());
        self.pool.spawn(move || job.run());
        Ok(())
    }
}
