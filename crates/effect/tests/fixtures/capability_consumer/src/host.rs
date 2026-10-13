use effect::{BlockingExecutor, CpuExecutor, Defect, ScheduledJob, ScheduledTask, TaskSpawner};
use std::num::NonZeroUsize;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;

use crate::drive;

enum Work {
    Task(ScheduledTask),
    Job(ScheduledJob),
}

/// Application-authored, bounded provider. Its worker and queue capacities
/// are constructor inputs; Effect creates neither workers nor an executor.
pub struct WorkerPool {
    sender: Option<mpsc::SyncSender<Work>>,
    workers: Mutex<Vec<thread::JoinHandle<()>>>,
    failures: Arc<Mutex<Vec<Defect>>>,
    pending: Arc<(Mutex<usize>, Condvar)>,
}

impl WorkerPool {
    pub fn new(workers: NonZeroUsize, queued: NonZeroUsize) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(queued.get());
        let receiver = Arc::new(Mutex::new(receiver));
        let failures = Arc::new(Mutex::new(Vec::new()));
        let pending = Arc::new((Mutex::new(0_usize), Condvar::new()));
        let mut handles = Vec::with_capacity(workers.get());
        for _ in 0..workers.get() {
            let receiver = Arc::clone(&receiver);
            let failures = Arc::clone(&failures);
            let pending = Arc::clone(&pending);
            let worker = thread::Builder::new()
                .spawn(move || {
                    loop {
                        let next = receiver
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .recv();
                        match next {
                            Ok(Work::Task(task)) => drive(task),
                            Ok(Work::Job(job)) => {
                                if let Err(error) = job.run() {
                                    failures
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                                        .push(error);
                                }
                            }
                            Err(_) => break,
                        }
                        let (count, changed) = &*pending;
                        let mut count = count
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        *count -= 1;
                        changed.notify_all();
                    }
                })
                .map_err(|error| error.to_string())?;
            handles.push(worker);
        }
        Ok(Self {
            sender: Some(sender),
            workers: Mutex::new(handles),
            failures,
            pending,
        })
    }

    fn submit(&self, work: Work) -> Result<(), Defect> {
        let (count, changed) = &*self.pending;
        let mut count = count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *count += 1;
        let result = self
            .sender
            .as_ref()
            .ok_or_else(|| Defect::new("worker pool is closed"))
            .and_then(|sender| {
                sender
                    .try_send(work)
                    .map_err(|error| Defect::new(error.to_string()))
            });
        if result.is_err() {
            *count -= 1;
            changed.notify_all();
        }
        result
    }

    pub fn wait_idle(&self) {
        let (count, changed) = &*self.pending;
        let mut count = count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *count != 0 {
            count = changed
                .wait(count)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    pub fn assert_healthy(&self) {
        assert!(
            self.failures
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "provider worker failed"
        );
    }
}

impl TaskSpawner for WorkerPool {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        self.submit(Work::Task(task))
    }
}

impl CpuExecutor for WorkerPool {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        self.submit(Work::Job(job))
    }
}

impl BlockingExecutor for WorkerPool {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        self.submit(Work::Job(job))
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.sender.take();
        let workers = std::mem::take(
            self.workers
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for worker in workers {
            assert!(worker.join().is_ok(), "provider worker panicked");
        }
    }
}
