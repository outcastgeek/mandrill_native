use crate::{Cancellation, Defect, LibraryDefect};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::task::{Context, Poll};

type TaskFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

pub(crate) enum TaskEvent {
    Started,
    Lost(Defect),
}

type TaskNotification = Box<dyn FnMut(TaskEvent) + Send>;

/// Ownership-aware work submitted by Effect to an environment's task provider.
///
/// Poll through settlement. Dropping accepted work reports a typed loss defect
/// to its scope/observers after dropping its interpreter state; it does not
/// promise asynchronous release from Drop. Providers must never abort a task
/// as an implementation of cooperative Effect cancellation.
#[must_use = "accepted scheduled work must remain owned through settlement"]
pub struct ScheduledTask {
    pending: Option<TaskFuture>,
    started: bool,
    settlement: crate::admission::Settlement,
    notification: Option<TaskNotification>,
}

impl ScheduledTask {
    pub(crate) fn new(
        pending: impl FnOnce(crate::admission::Settlement) -> TaskFuture,
        notification: impl FnMut(TaskEvent) + Send + 'static,
    ) -> Self {
        let settlement = crate::admission::Settlement::default();
        Self {
            pending: Some(pending(settlement.clone())),
            settlement,
            started: false,
            notification: Some(Box::new(notification)),
        }
    }

    pub(crate) fn on_settled(self, action: impl FnOnce() + Send + 'static) -> Self {
        self.settlement.add(action);
        self
    }

    fn fail(&mut self, defect: Defect) {
        drop(self.pending.take());
        self.settlement.release();
        if let Some(mut notification) = self.notification.take() {
            notification(TaskEvent::Lost(defect));
        }
    }
}

impl Future for ScheduledTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if !this.started {
            this.started = true;
            if let Some(notification) = &mut this.notification {
                notification(TaskEvent::Started);
            }
        }
        let Some(pending) = &mut this.pending else {
            return Poll::Ready(());
        };
        match catch_unwind(AssertUnwindSafe(|| pending.as_mut().poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(())) => {
                this.settlement.release();
                this.notification = None;
                this.pending = None;
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
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect>;
}

/// Only operations that schedule children require this environment capability.
/// Providers may be concrete application types or public trait objects.
pub trait HasTaskSpawner {
    type Tasks: TaskSpawner + ?Sized;

    fn task_spawner(&self) -> &Self::Tasks;
}

/// Environment capability used by [`crate::Effect::run_cancellable`].
///
/// Application services and execution providers are supplied through the same
/// environment `R`. Implementing this trait does not require an executor,
/// clock, or worker pool. Plain [`crate::Effect::run`] needs no such capability.
pub trait HasCancellation {
    /// The external request source for this run. Clones observe the same request.
    fn cancellation(&self) -> &Cancellation;
}

impl HasCancellation for Cancellation {
    fn cancellation(&self) -> &Cancellation {
        self
    }
}
