//! Native providers assembled for the application.

use effect::{Effect, Exit, HasCancellation};
use effect_native_runtime::linked;
use native_async_runtime::{AdmissionError, Completion, RuntimeHandle};
use std::convert::Infallible;
use std::sync::Arc;
/// The native caller owns admission, cancellation, terminal arbitration and its
/// payload representation. Effect owns and awaits its application's cleanup.
///
/// # Type parameters
/// - `A`: Application value type. Additional bound: `Send + 'static`.
/// - `E`: Domain-error type. Additional bound: `Send + 'static`.
/// - `R`: Environment type. Additional bound: `HasCancellation + Send + Sync + 'static`.
///
/// # Parameters
/// - `handle` (`&RuntimeHandle`): Owned or borrowed capability handle kept live through this operation.
/// - `native` (`Arc<native_async_runtime::Cancellation>`): The native supplied to this operation.
/// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
/// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
/// - `done` (`impl FnOnce(Exit<A, E>) + Send + 'static`): The done supplied to this operation.
///
/// # Returns
/// Type: `Result<(), AdmissionError>`.
pub fn submit<A, E, R>(
    handle: &RuntimeHandle,
    native: Arc<native_async_runtime::Cancellation>,
    program: Effect<A, E, R>,
    env: Arc<R>,
    done: impl FnOnce(Exit<A, E>) + Send + 'static,
) -> Result<(), AdmissionError>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasCancellation + Send + Sync + 'static,
{
    handle.submit(
        native,
        move |native| async move {
            let cancellation = env.cancellation().clone();
            Ok::<_, Infallible>(linked(native, cancellation, program.run_cancellable(env)).await)
        },
        move |completion| {
            let outcome = match completion {
                Completion::Success(outcome) => outcome,
                Completion::Failure(never) => match never {},
                Completion::Panicked => Exit::Die(effect::Defect::from(
                    effect::LibraryDefect::NativeCompletionPanicked,
                )),
            };
            done(outcome);
        },
    )
}
