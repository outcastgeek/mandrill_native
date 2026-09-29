//! Native task execution backed by an explicit provider handle.
//!
//! [`NativeTasks`] adapts an existing `native_async_runtime::RuntimeHandle` to
//! Effect's `TaskSpawner` capability. The application retains shutdown
//! authority and keeps the native runtime alive while its scopes run. Task
//! ownership transfers to the provider on accepted submission; cancellation
//! requests are bridged to the native token and completion is observed before
//! settlement. Construct this adapter and put it in the application environment
//! when native tasks are desired alongside any other capabilities.

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
    /// Construct a `NativeTasks` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `handle` (`RuntimeHandle`): Owned or borrowed capability handle kept live through this operation.
    /// - `diagnostics` (`Diagnostics`): Separate sink for instrumentation or provider failures.
    ///
    /// # Returns
    /// Type: `Result<Self, Defect>`.
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
    ///
    /// # Parameters
    /// - `task` (`ScheduledTask`): The task supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), Defect>`.
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
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
///
/// # Parameters
/// - `native` (`Arc<native_async_runtime::Cancellation>`): The native supplied to this operation.
/// - `effect` (`Cancellation`): The effect supplied to this operation.
/// - `work` (`impl Future<Output = Exit<A, E>>`): The work supplied to this operation.
///
/// # Returns
/// Type: `Exit<A, E>`.
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
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
/// - `T`: Value or item type.
///
/// # Parameters
/// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
/// - `success` (`impl FnOnce(A) -> T`): Successful application value or its codec.
/// - `fail` (`impl FnOnce(E) -> T`): The fail supplied to this operation.
/// - `die` (`impl FnOnce(Defect) -> T`): The die supplied to this operation.
/// - `interrupted` (`impl FnOnce() -> T`): The interrupted supplied to this operation.
/// - `cleanup` (`impl FnOnce(effect::Finalization<E>) -> T`): The cleanup supplied to this operation.
///
/// # Returns
/// Type: `T`.
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
