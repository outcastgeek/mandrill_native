//! Dynamically selected, physically admitted prepared-run imports. Type erasure
//! is the existing Step/Once storage, not an ordinary Effect/System interpreter.
use super::*;
#[path = "import_environment.rs"]
mod environment;
use crate::PreparedCancellationWait;
use crate::internal::runtime::prepared::construction::ConstructionValue;
use crate::internal::runtime::{DriveContext, Publication, poll_drive};
use crate::internal::runtime_storage::Pending;
pub use environment::{ImportEnvironment, import_with_environment};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

type ImportTypes<A, E, R> = fn() -> (A, E, R);
pub struct Import<A, E, R, const KEY: u64>(PhantomData<ImportTypes<A, E, R>>);
#[must_use]
pub fn import<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, const KEY: u64>()
-> Import<A, E, R, KEY> {
    Import(PhantomData)
}
pub struct ImportAdmission<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
> {
    operation: FrameReservation<'static, Operation<A, E, R, V>>,
    pending: FrameReservation<'static, Imported<A, E, V>>,
    source: ConstructionValue<PreparedInvocation<A, E>>,
    ownership: PreparedOutputOwnerReservation<PreparedOutput<A, E>, V, E>,
    cancellation: PreparedCancellationWait<V, E>,
    context: PreparedRunContext<V, E>,
}
struct Operation<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, V: Send + 'static>
{
    pending: FrameReservation<'static, Imported<A, E, V>>,
    source: ConstructionValue<PreparedInvocation<A, E>>,
    ownership: PreparedOutputOwnerReservation<PreparedOutput<A, E>, V, E>,
    cancellation: PreparedCancellationWait<V, E>,
    finish: Finish<A, E>,
    context: PreparedRunContext<V, E>,
    marker: PhantomData<fn() -> R>,
}
struct Imported<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    source: Option<PreparedInvocation<A, E>>,
    poll_context: crate::internal::runtime::PollContext,
    ownership: Option<PreparedOutputOwnerReservation<PreparedOutput<A, E>, V, E>>,
    cancellation: PreparedCancellationWait<V, E>,
    finish: Option<Finish<A, E>>,
    context: PreparedRunContext<V, E>,
}
// Only separately stored Step payloads are pinned. These are movable control
// handles; no projection ever pins the aggregate or its Option bookkeeping.
impl<A: Send, E: Send, V: Send> Unpin for Imported<A, E, V> {}
impl<A: Send, E: Send, V: Send> Future for Imported<A, E, V> {
    type Output = Step;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Step> {
        let this = self.get_mut();
        let cancel = Pin::new(&mut this.cancellation).poll(cx).is_ready();
        let Some(source) = this.source.as_mut() else {
            return Poll::Pending;
        };
        if cancel {
            let _ = source.context.request_cancellation();
        }
        let PreparedInvocation {
            step,
            context,
            output,
            ..
        } = source;
        if poll_drive(
            step,
            context,
            &mut this.poll_context,
            Publication::Publish,
            &mut || {
                let value = output.root.take();
                output.root.store(context.publish(value));
            },
            cx,
        )
        .is_pending()
        {
            this.context.acknowledge_clock();
            return Poll::Pending;
        }
        let (Some(source), Some(ownership), Some(finish)) = (
            this.source.take(),
            this.ownership.take(),
            this.finish.take(),
        ) else {
            return Poll::Ready(Step::Done);
        };
        // Only this sealed import owns the source invocation/output; it was never
        // returned to an external observer. Install the actual foreign output
        // owner before transferring its original Exit into our private Finish.
        let PreparedInvocation { output, .. } = source;
        let authority = ownership.install_for_invocation(output);
        let outcome = authority.root.take();
        let next = finish
            .call(this.context.publish(outcome))
            .unwrap_or(Step::Done);
        drop(authority);
        Poll::Ready(next)
    }
}
impl<A: Send, E: Send, R: Send + Sync, V: Send> Consume<(), Step> for Operation<A, E, R, V> {
    fn consume(self, (): ()) -> Step {
        self.context
            .root
            .contexts
            .protected_failure(self.context.id, false);
        let source = match self.source.take() {
            Ok(source) => source,
            // Native preflight verifies every vacancy BEFORE graph factories;
            // a missing affine value is an engine/construction contract defect,
            // not an unreported late capacity refusal in business E.
            Err(_) => {
                return self
                    .finish
                    .call(Exit::Die(Defect::from(
                        crate::LibraryDefect::PreparedConstructionUnbound,
                    )))
                    .unwrap_or(Step::Done);
            }
        };
        let poll_context = source.context.poll_context();
        Step::Suspended(Pending::prepared(
            self.pending,
            Imported {
                source: Some(source),
                poll_context,
                ownership: Some(self.ownership),
                cancellation: self.cancellation,
                finish: Some(self.finish),
                context: self.context,
            },
        ))
    }
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, const KEY: u64> Description
    for Import<A, E, R, KEY>
{
}
impl<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static, const KEY: u64> Spec
    for Import<A, E, R, KEY>
{
    type Value = A;
    type Error = E;
    type Env = R;
    type Storage = ();
    type Admission<V: Send + 'static> = ImportAdmission<A, E, R, V>;
    fn provision<V: Send + 'static>(
        _memory: &PreparedMemory<'static>,
        _region: RegionId,
        _capacity: usize,
        _frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<(), MemoryError> {
        let _ = cleanup;
        Ok(())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        Demand::cell::<Operation<A, E, R, V>>(false)
            .add(Demand::cell::<Imported<A, E, V>>(false))?
            .add(output_owner_shape::<PreparedOutput<A, E>, V, E>())?
            .add(output_origin_shape::<R, V, E>())
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &(),
        environment: &Environment<R>,
        context: &PreparedRunContext<V, E>,
        _scope: &crate::PreparedScope<E>,
        _cleanup: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        _storage: &(),
        environment: &Environment<R>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin = output_origin_reserve::<R, V, E>(frames, environment, context)?;
        let source = context.construction_value::<PreparedInvocation<A, E>>(KEY)?;
        if context.root.panic_pool.is_some() {
            source.validate_with(|source| {
                if source.context.root.panic_pool.is_some() {
                    Ok(())
                } else {
                    Err(MemoryError::UnsupportedRunContract)
                }
            })?;
        }
        let admission = ImportAdmission {
            operation: frames.reserve()?,
            pending: frames.reserve()?,
            source,
            ownership: context.reserve_output_owner(frames)?,
            cancellation: context.cancelled()?,
            context: context.try_clone()?,
        };
        origin
            .attach(environment.try_clone()?)
            .map_err(|(error, _)| error)?;
        Ok(admission)
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<A, E>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.operation,
            Operation::<A, E, R, V> {
                pending: admission.pending,
                source: admission.source,
                ownership: admission.ownership,
                cancellation: admission.cancellation,
                finish: finish.0,
                context: admission.context,
                marker: PhantomData,
            },
        )))
    }
}
