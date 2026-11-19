//! Capabilities for task submission and cancellation.

use crate::admission::{CapacityPermit, Settlement};
use crate::internal::runtime_storage::ReservedOnce;
use crate::memory::PreparedUnitFuture;
use crate::{Cancellation, Defect, LibraryDefect};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::task::{Context, Poll};

/// Type alias for task future in execution.
type TaskFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Variants of task event used by execution.
pub(crate) enum TaskEvent {
    Started,
    Lost(Defect),
}

/// Type alias for task notification in execution.
enum TaskNotification {
    Ordinary(Box<dyn FnMut(TaskEvent) + Send>),
    Prepared(ReservedOnce<PreparedTaskCompletion, ()>),
}

pub(crate) enum PreparedTaskCompletion {
    Ready,
    Lost(Defect),
}

enum TaskBody {
    Ordinary(TaskFuture),
    Prepared(PreparedUnitFuture),
}

impl TaskBody {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        match self {
            Self::Ordinary(future) => future.as_mut().poll(cx),
            Self::Prepared(future) => Pin::new(future).poll(cx),
        }
    }
}

enum TaskSettlement {
    Ordinary(Settlement),
    Prepared(Option<CapacityPermit>),
}

/// Ownership-aware work submitted by Effect to an environment's task provider.
///
/// Poll through settlement. Dropping accepted work reports a typed loss defect
/// to its scope/observers after dropping its interpreter state; it does not
/// promise asynchronous release from Drop. Providers must never abort a task
/// as an implementation of cooperative Effect cancellation.
#[must_use = "accepted scheduled work must remain owned through settlement"]
pub struct ScheduledTask {
    pending: Option<TaskBody>,
    started: bool,
    settlement: TaskSettlement,
    notification: Option<TaskNotification>,
}

impl ScheduledTask {
    /// # Parameters
    /// - `pending` (`impl FnOnce(crate::admission::Settlement) -> TaskFuture`): The pending supplied to this operation.
    /// - `notification` (`impl FnMut(TaskEvent) + Send + 'static`): The notification supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new(
        pending: impl FnOnce(crate::admission::Settlement) -> TaskFuture,
        notification: impl FnMut(TaskEvent) + Send + 'static,
    ) -> Self {
        let settlement = crate::admission::Settlement::default();
        Self {
            pending: Some(TaskBody::Ordinary(pending(settlement.clone()))),
            settlement: TaskSettlement::Ordinary(settlement),
            started: false,
            notification: Some(TaskNotification::Ordinary(Box::new(notification))),
        }
    }

    pub(crate) fn prepared(
        pending: PreparedUnitFuture,
        notification: ReservedOnce<PreparedTaskCompletion, ()>,
    ) -> Self {
        Self {
            pending: Some(TaskBody::Prepared(pending)),
            started: false,
            settlement: TaskSettlement::Prepared(None),
            notification: Some(TaskNotification::Prepared(notification)),
        }
    }

    /// # Parameters
    /// - `action` (`impl FnOnce() + Send + 'static`): State transition selected for this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn with_permit(mut self, permit: CapacityPermit) -> Self {
        match &mut self.settlement {
            TaskSettlement::Ordinary(settlement) => settlement.add(move || drop(permit)),
            TaskSettlement::Prepared(settlement) => {
                *settlement = Some(permit);
            }
        }
        self
    }

    fn release(&mut self) {
        match &mut self.settlement {
            TaskSettlement::Ordinary(settlement) => settlement.release(),
            TaskSettlement::Prepared(settlement) => drop(settlement.take()),
        }
    }

    ///
    /// # Parameters
    /// - `defect` (`Defect`): The defect supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    fn fail(&mut self, defect: Defect) {
        let mut retirement = std::mem::DropGuard::new(self, |task| task.publish_loss(defect));
        drop(retirement.pending.take());
    }

    fn publish_loss(&mut self, defect: Defect) {
        self.release();
        if let Some(notification) = self.notification.take() {
            match notification {
                TaskNotification::Ordinary(mut notification) => {
                    notification(TaskEvent::Lost(defect))
                }
                TaskNotification::Prepared(notification) => {
                    notification.call(PreparedTaskCompletion::Lost(defect))
                }
            }
        }
    }
}

impl Future for ScheduledTask {
    /// Type alias for output in execution.
    type Output = ();

    ///
    /// # Parameters
    /// - `cx` (`&mut Context<'_>`): Rust task context used to register the current waker.
    ///
    /// # Returns
    /// Type: `Poll<()>`.
    /// Returns `Poll<()>`.
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if !this.started {
            this.started = true;
            if let Some(TaskNotification::Ordinary(notification)) = &mut this.notification {
                notification(TaskEvent::Started);
            }
        }
        let Some(pending) = &mut this.pending else {
            return Poll::Ready(());
        };
        match catch_unwind(AssertUnwindSafe(|| pending.poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(())) => {
                this.release();
                if matches!(this.notification, Some(TaskNotification::Ordinary(_))) {
                    this.notification = None;
                }
                drop(this.pending.take());
                if let Some(TaskNotification::Prepared(notification)) = this.notification.take() {
                    notification.call(PreparedTaskCompletion::Ready);
                }
                Poll::Ready(())
            }
            Err(panic) => {
                this.fail(Defect::from_panic(panic));
                Poll::Ready(())
            }
        }
    }
}

impl Drop for ScheduledTask {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        self.fail(Defect::from(LibraryDefect::ScheduledTaskDropped));
    }
}

/// Independently scheduled Effect tasks, supplied exclusively through R.
///
/// A provider must drive tasks independently, including synchronous callbacks
/// on separate workers. It must declare/configure its actual worker and admission
/// limits; a single-thread executor does not satisfy this capability. `spawn`
/// must not poll user work inline or block awaiting it. Ok transfers execution
/// ownership through settlement. Err rejects without polling, drops the task,
/// and reports the infrastructure defect. Shutdown drops queued/running drivers
/// so Effect can report loss, rather than leave an unresolved completion.
///
/// The ScheduledTask completion guard plus its scope provide ownership/loss
/// observation; this contract is more than a fire-and-forget future submission.
/// No runtime, pool, worker count, or admission policy is created by the core.
pub trait TaskSpawner: Send + Sync + 'static {
    /// # Parameters
    /// - `task` (`ScheduledTask`): The task supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect>;
}

/// Only operations that schedule children require this environment capability.
/// Providers may be concrete application types or public trait objects.
pub trait HasTaskSpawner {
    /// Task-spawning capability selected by this environment.
    type Tasks: TaskSpawner + ?Sized;

    /// # Returns
    /// Type: `&Self::Tasks`.
    fn task_spawner(&self) -> &Self::Tasks;
}

/// Environment capability used by [`crate::Effect::run_cancellable`].
///
/// Application services and execution providers are supplied through the same
/// environment `R`. Implementing this trait does not require an executor,
/// clock, or worker pool. Plain [`crate::Effect::run`] needs no such capability.
pub trait HasCancellation {
    /// The external request source for this run. Clones observe the same request.
    ///
    /// # Returns
    /// Type: `&Cancellation`.
    fn cancellation(&self) -> &Cancellation;
}

impl HasCancellation for Cancellation {
    ///
    /// # Returns
    /// Type: `&Cancellation`.
    fn cancellation(&self) -> &Cancellation {
        self
    }
}
