//! Finally-settled provider value handoff, not a scheduler/fork implementation.
//! The sealed operation is the only caller of the private bare-value Finish
//! transfer; no public observer/extractor can invoke an arbitrary owned callback.
use super::*;
use crate::prepared_provider::{
    PreparedOwnedProviderResult, PreparedProviderOutputLease, ProviderWorkingSet,
};
use std::future::Future;

type Types<E, R> = fn() -> (E, R);
pub struct ProviderResultDescription<F: 'static, A: 'static, D: 'static, E, R> {
    result: PreparedOwnedProviderResult<F, A, D>,
    marker: PhantomData<Types<E, R>>,
}
#[must_use]
pub fn provider_result<F, A, D, E, R>(
    result: PreparedOwnedProviderResult<F, A, D>,
) -> ProviderResultDescription<F, A, D, E, R>
where
    F: Future<Output = A> + Send + 'static,
    A: Send + 'static,
    D: ProviderWorkingSet,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ProviderResultDescription {
        result,
        marker: PhantomData,
    }
}
pub struct ProviderResultAdmission<
    F: Send + 'static,
    A: Send + 'static,
    D: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
> {
    operation: FrameReservation<'static, Operation<F, A, D, E, R, V>>,
    ownership: PreparedOutputOwnerReservation<PreparedProviderOutputLease<F, A, D>, V, E>,
    context: PreparedRunContext<V, E>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct Operation<
    F: Send + 'static,
    A: Send + 'static,
    D: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
> {
    description: ProviderResultDescription<F, A, D, E, R>,
    ownership: PreparedOutputOwnerReservation<PreparedProviderOutputLease<F, A, D>, V, E>,
    finish: Finish<A, E>,
    context: PreparedRunContext<V, E>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<F, A, D, E, R, V> Consume<(), Step> for Operation<F, A, D, E, R, V>
where
    F: Future<Output = A> + Send + 'static,
    A: Send + 'static,
    D: ProviderWorkingSet,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        self.context
            .root
            .contexts
            .protected_failure(self.context.id, false);
        if self.context.is_cancelled() {
            // No business value is extracted. Dropping the exclusive provider
            // view follows the already installed phased entry retirement lane.
            return self.finish.call(Exit::Interrupted).unwrap_or(Step::Done);
        }
        // Only sealed dispatch constructs this operation. Its origin admission
        // and Finish belong to this same live invocation, whose owned output
        // remains a consumer through the entire drive, cancellation and unwind.
        self.description
            .result
            .handoff(self.ownership, &self.context, self.panic, self.finish)
    }
}
impl<F, A, D, E, R> Description for ProviderResultDescription<F, A, D, E, R>
where
    F: Future<Output = A> + Send + 'static,
    A: Send + 'static,
    D: ProviderWorkingSet,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
}
impl<F, A, D, E, R> Spec for ProviderResultDescription<F, A, D, E, R>
where
    F: Future<Output = A> + Send + 'static,
    A: Send + 'static,
    D: ProviderWorkingSet,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    type Value = A;
    type Error = E;
    type Env = R;
    type Storage = ();
    type Admission<V: Send + 'static> = ProviderResultAdmission<F, A, D, E, R, V>;
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
        Demand::cell::<Operation<F, A, D, E, R, V>>(false)
            .add(Demand::panic_sites(1))?
            .add(output_owner_shape::<
                PreparedProviderOutputLease<F, A, D>,
                V,
                E,
            >())?
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
        // Full concrete frame/entry-D transfer/consumer and actual environment
        // origin exist before the graph factory produces any result descriptor.
        let origin = output_origin_reserve::<R, V, E>(frames, environment, context)?;
        let admission = ProviderResultAdmission {
            operation: frames.reserve()?,
            ownership: context.reserve_output_owner(frames)?,
            context: context.try_clone()?,
            panic: context.reserve_panic()?,
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
            Operation {
                description: self,
                ownership: admission.ownership,
                finish: finish.0,
                context: admission.context,
                panic: admission.panic,
            },
        )))
    }
}
