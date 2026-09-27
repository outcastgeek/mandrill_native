use effect::{Effect, Exit, HasCancellation};
use effect_native_runtime::linked;
use native_async_runtime::{AdmissionError, Completion, RuntimeHandle};
use std::convert::Infallible;
use std::sync::Arc;
/// The native caller owns admission, cancellation, terminal arbitration and its
/// payload representation. Effect owns and awaits its application's cleanup.
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
