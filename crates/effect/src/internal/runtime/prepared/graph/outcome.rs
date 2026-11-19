//! Prepared exit transformations using the existing continuation evaluator.
use super::*;
use crate::{CleanupFailure, Finalization, PrimaryOutcome};

#[doc(hidden)]
pub trait Transform<A, E>: Send + 'static {
    type Value: Send + 'static;
    fn apply(self, input: Exit<A, E>) -> Exit<Self::Value, E>;
}
#[doc(hidden)]
pub struct EitherTransform;
impl<A: Send + 'static, E: Send + 'static> Transform<A, E> for EitherTransform {
    type Value = Result<A, E>;
    fn apply(self, input: Exit<A, E>) -> Exit<Self::Value, E> {
        match input {
            Exit::Success(value) => Exit::Success(Ok(value)),
            Exit::Fail(error) => Exit::Success(Err(error)),
            Exit::Die(defect) => Exit::Die(defect),
            Exit::Interrupted => Exit::Interrupted,
            Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
        }
    }
}
#[doc(hidden)]
pub struct CaptureTransform<F, Y> {
    enabled: bool,
    classify: F,
    marker: PhantomData<fn() -> Y>,
}
impl<
    A: Send + 'static,
    E: Send + 'static,
    Y: Send + 'static,
    F: Fn(E) -> Result<Y, E> + Send + 'static,
> Transform<A, E> for CaptureTransform<F, Y>
{
    type Value = Result<A, Y>;
    fn apply(self, input: Exit<A, E>) -> Exit<Self::Value, E> {
        match input {
            Exit::Success(value) => Exit::Success(Ok(value)),
            Exit::Fail(error) if self.enabled => match (self.classify)(error) {
                Ok(value) => Exit::Success(Err(value)),
                Err(error) => Exit::Fail(error),
            },
            Exit::Fail(error) => Exit::Fail(error),
            Exit::Die(defect) => Exit::Die(defect),
            Exit::Interrupted => Exit::Interrupted,
            Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
        }
    }
}
#[doc(hidden)]
pub struct ErrorTransform<F>(F);
impl<A: Send + 'static, E: Send + 'static, F: Fn(E) -> E + Send + 'static> Transform<A, E>
    for ErrorTransform<F>
{
    type Value = A;
    fn apply(self, input: Exit<A, E>) -> Exit<A, E> {
        let map = self.0;
        let cleanup = |failure| match failure {
            CleanupFailure::ReleaseFailure(error) => CleanupFailure::ReleaseFailure(map(error)),
            other @ (CleanupFailure::ReleaseDefect(_) | CleanupFailure::ReleaseInterrupted) => {
                other
            }
        };
        match input {
            Exit::Success(value) => Exit::Success(value),
            Exit::Fail(error) => Exit::Fail(map(error)),
            Exit::Die(defect) => Exit::Die(defect),
            Exit::Interrupted => Exit::Interrupted,
            Exit::FinalizationFailed(Finalization {
                primary,
                first,
                mut rest,
            }) => {
                let primary = match primary {
                    PrimaryOutcome::Failed(error) => PrimaryOutcome::Failed(map(error)),
                    other @ (PrimaryOutcome::Succeeded
                    | PrimaryOutcome::Defected(_)
                    | PrimaryOutcome::WasInterrupted) => other,
                };
                let first = cleanup(first);
                rest.map_same(&map);
                Exit::FinalizationFailed(Finalization {
                    primary,
                    first,
                    rest,
                })
            }
        }
    }
}
#[doc(hidden)]
pub struct Outcome<N, T> {
    source: N,
    transform: T,
}
pub fn either<N: Description>(source: N) -> Outcome<N, EitherTransform> {
    Outcome {
        source,
        transform: EitherTransform,
    }
}
pub fn capture<
    N: Description,
    Y: Send + 'static,
    F: Fn(N::Error) -> Result<Y, N::Error> + Send + 'static,
>(
    source: N,
    enabled: bool,
    classify: F,
) -> Outcome<N, CaptureTransform<F, Y>> {
    Outcome {
        source,
        transform: CaptureTransform {
            enabled,
            classify,
            marker: PhantomData,
        },
    }
}
pub fn map_error<N: Description, F: Fn(N::Error) -> N::Error + Send + 'static>(
    source: N,
    map: F,
) -> Outcome<N, ErrorTransform<F>> {
    Outcome {
        source,
        transform: ErrorTransform(map),
    }
}
impl<N: Description, T: Transform<N::Value, N::Error>> Description for Outcome<N, T> {}
pub struct Admission<N: Description, T: Transform<N::Value, N::Error>, V: Send + 'static> {
    entry: FrameReservation<'static, Entry<N, T, V>>,
    controls: Controls<N, T, V>,
}
struct Controls<N: Description, T: Transform<N::Value, N::Error>, V: Send + 'static> {
    continuation: FrameReservation<'static, Continue<N, T, V>>,
    resume: FrameReservation<'static, Resume<N, T, V>>,
    terminal: FrameReservation<'static, Terminal<T::Value, N::Error, V>>,
    source: N::Admission<V>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct Entry<N: Description, T: Transform<N::Value, N::Error>, V: Send + 'static> {
    description: Outcome<N, T>,
    controls: Controls<N, T, V>,
    finish: Finish<T::Value, N::Error>,
}
struct Continue<N: Description, T: Transform<N::Value, N::Error>, V: Send + 'static> {
    resume: FrameReservation<'static, Resume<N, T, V>>,
    transform: T,
    terminal: FrameReservation<'static, Terminal<T::Value, N::Error, V>>,
    finish: Finish<T::Value, N::Error>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct Resume<N: Description, T: Transform<N::Value, N::Error>, V: Send + 'static> {
    outcome: Exit<N::Value, N::Error>,
    transform: T,
    terminal: FrameReservation<'static, Terminal<T::Value, N::Error, V>>,
    finish: Finish<T::Value, N::Error>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<N: Description, T: Transform<N::Value, N::Error>, V: Send + 'static> Consume<(), Step>
    for Entry<N, T, V>
{
    fn consume(self, (): ()) -> Step {
        let Controls {
            continuation,
            resume,
            terminal,
            source,
            context,
            panic,
        } = self.controls;
        let next = Continue {
            resume,
            transform: self.description.transform,
            terminal,
            finish: self.finish,
            context,
            panic,
        };
        self.description
            .source
            .dispatch(source, Continuation(Once::prepared(continuation, next)))
            .0
    }
}
impl<N: Description, T: Transform<N::Value, N::Error>, V: Send + 'static>
    Consume<Exit<N::Value, N::Error>, Step> for Continue<N, T, V>
{
    fn consume(self, outcome: Exit<N::Value, N::Error>) -> Step {
        let Self {
            resume,
            transform,
            terminal,
            finish,
            context,
            panic,
        } = self;
        Step::More(Once::prepared(
            resume,
            Resume {
                outcome,
                transform,
                terminal,
                finish,
                context,
                panic,
            },
        ))
    }
}

impl<N: Description, T: Transform<N::Value, N::Error>, V: Send + 'static> Consume<(), Step>
    for Resume<N, T, V>
{
    fn consume(self, (): ()) -> Step {
        let outcome = catch_unwind(AssertUnwindSafe(|| self.transform.apply(self.outcome)))
            .unwrap_or_else(|panic| Exit::Die(super::super::panic_defect(self.panic, panic)));
        Step::More(Once::prepared(
            self.terminal,
            Terminal {
                outcome,
                finish: self.finish,
                context: self.context,
            },
        ))
    }
}
struct Terminal<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    outcome: Exit<A, E>,
    finish: Finish<A, E>,
    context: PreparedRunContext<V, E>,
}
impl<A: Send + 'static, E: Send + 'static, V: Send + 'static> Consume<(), Step>
    for Terminal<A, E, V>
{
    fn consume(self, (): ()) -> Step {
        let Self {
            outcome,
            finish,
            context,
        } = self;
        let step = finish.call(outcome).unwrap_or(Step::Done);
        drop(context);
        step
    }
}
impl<N: Description, T: Transform<N::Value, N::Error>> Spec for Outcome<N, T> {
    type Value = T::Value;
    type Error = N::Error;
    type Env = N::Env;
    type Admission<V: Send + 'static> = Admission<N, T, V>;
    type Storage = N::Storage;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        N::provision::<V>(memory, region, capacity, frames, cleanup)
    }
    fn storage_bytes<V: Send + 'static>(
        capacity: usize,
        layout: Layout,
    ) -> Result<Bytes, MemoryError> {
        N::storage_bytes::<V>(capacity, layout)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        N::shape::<V>()?
            .add(Demand::cell::<Entry<N, T, V>>(false))?
            .add(Demand::cell::<Continue<N, T, V>>(true))?
            .add(Demand::cell::<Resume<N, T, V>>(true))?
            .add(Demand::cell::<Terminal<T::Value, N::Error, V>>(true))?
            .add(Demand::panic_sites(1))?
            .add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        N::scope_finalizers()
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
        enclosing: &crate::PreparedScope<Self::Error>,
        enclosing_cleanup: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admission = Admission {
            entry: frames.reserve()?,
            controls: Controls {
                continuation: frames.reserve()?,
                resume: frames.reserve()?,
                terminal: frames.reserve()?,
                source: N::reserve_scoped(
                    frames,
                    storage,
                    environment,
                    context,
                    enclosing,
                    enclosing_cleanup,
                )?,
                context: context.try_clone()?,
                panic: context.reserve_panic()?,
            },
        };
        origin
            .attach(environment.try_clone()?)
            .map_err(|(error, _)| error)?;
        Ok(admission)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admission = Admission {
            entry: frames.reserve()?,
            controls: Controls {
                continuation: frames.reserve()?,
                resume: frames.reserve()?,
                terminal: frames.reserve()?,
                source: N::reserve(frames, storage, environment, context)?,
                context: context.try_clone()?,
                panic: context.reserve_panic()?,
            },
        };
        origin
            .attach(environment.try_clone()?)
            .map_err(|(error, _)| error)?;
        Ok(admission)
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<Self::Value, Self::Error>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            Entry {
                description: self,
                controls: admission.controls,
                finish: finish.0,
            },
        )))
    }
}
