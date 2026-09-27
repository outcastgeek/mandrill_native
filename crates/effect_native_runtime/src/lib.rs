use effect::{Cancellation, Defect, Diagnostics, Exit, LibraryDefect, ScheduledTask, TaskSpawner};
use futures_util::future::{Either, select};
use native_async_runtime::{AdmissionError, Completion, RuntimeHandle};
use std::future::Future;
use std::sync::Arc;

/// The native owner retains shutdown authority. This adapter only submits.
#[derive(Clone)]
pub struct NativeTasks {
    handle: RuntimeHandle,
    diagnostics: Diagnostics,
}
impl NativeTasks {
    pub fn new(handle: RuntimeHandle, diagnostics: Diagnostics) -> Result<Self, Defect> {
        if handle.worker_count() <= std::num::NonZeroUsize::MIN.get() {
            return Err(Defect::from(LibraryDefect::ProviderWorkersInsufficient));
        }
        Ok(Self {
            handle,
            diagnostics,
        })
    }
}
impl TaskSpawner for NativeTasks {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        let diagnostics = self.diagnostics.clone();
        self.handle
            .submit(
                native_async_runtime::Cancellation::new(),
                move |_| async move {
                    task.await;
                    Ok::<(), Defect>(())
                },
                move |completion| match completion {
                    Completion::Success(()) => {}
                    Completion::Failure(defect) => diagnostics.record(defect),
                    Completion::Panicked => {
                        diagnostics.record(Defect::from(LibraryDefect::NativeCompletionPanicked))
                    }
                },
            )
            .map_err(|error| {
                Defect::from(match error {
                    AdmissionError::Saturated => LibraryDefect::ProviderCapacityFull,
                    AdmissionError::Stopping => LibraryDefect::ProviderClosed,
                })
            })
    }
}
/// Request translation is persistent and bidirectional while the owned workflow
/// runs. Neither token is given another owner's private terminal authority.
pub async fn linked<A, E>(
    native: Arc<native_async_runtime::Cancellation>,
    effect: Cancellation,
    work: impl Future<Output = Exit<A, E>>,
) -> Exit<A, E> {
    if native.is_cancelled() {
        let _ = effect.request();
    }
    if effect.is_requested() {
        let _ = native.request();
    }
    let requests = async {
        match select(Box::pin(native.cancelled()), Box::pin(effect.cancelled())).await {
            Either::Left(((), _)) => {
                let _ = effect.request();
            }
            Either::Right(((), _)) => {
                let _ = native.request();
            }
        }
    };
    match select(Box::pin(work), Box::pin(requests)).await {
        Either::Left((outcome, _)) => outcome,
        Either::Right(((), remaining)) => remaining.await,
    }
}
/// Preserve all five exit families at a native boundary. The consumer chooses
/// its payload/ABI representation and handles cleanup details explicitly.
pub fn complete<A, E, T>(
    outcome: Exit<A, E>,
    success: impl FnOnce(A) -> T,
    fail: impl FnOnce(E) -> T,
    die: impl FnOnce(Defect) -> T,
    interrupted: impl FnOnce() -> T,
    cleanup: impl FnOnce(effect::Finalization<E>) -> T,
) -> T {
    match outcome {
        Exit::Success(value) => success(value),
        Exit::Fail(error) => fail(error),
        Exit::Die(defect) => die(defect),
        Exit::Interrupted => interrupted(),
        Exit::FinalizationFailed(details) => cleanup(details),
    }
}
