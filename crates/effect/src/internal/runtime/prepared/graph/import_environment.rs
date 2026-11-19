//! Late selected-R publication into an ALREADY physically admitted foreign
//! body. Same Bind/Step/Imported driver and private consuming output handoff.
use super::super::run::PendingGraphInvocation;
use super::*;
use crate::internal::runtime::prepared::{OriginSnapshot, OriginSnapshotReservation};

type ImportEnvironmentTypes<A, E, I> = fn() -> (A, E, I);
pub struct ImportEnvironment<A, E, R, I, const KEY: u64> {
    value: R,
    marker: PhantomData<ImportEnvironmentTypes<A, E, I>>,
}
/// Existing bind grammar supplies selected R; native preflight supplies the
/// actual pending target through the ordinary finite construction registry.
pub fn import_with_environment<N, A, const KEY: u64>(
    source: N,
) -> impl Description<Value = A, Error = N::Error, Env = N::Env>
where
    N: Description,
    N::Value: Sync,
    A: Send + 'static,
{
    fn activate<
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
        I: Send + Sync + 'static,
        const KEY: u64,
    >(
        value: R,
    ) -> ImportEnvironment<A, E, R, I, KEY> {
        ImportEnvironment {
            value,
            marker: PhantomData,
        }
    }
    bind(source, activate::<A, N::Error, N::Value, N::Env, KEY>)
}
pub struct EnvironmentAdmission<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    I: Send + Sync + 'static,
    V: Send + 'static,
> {
    operation: FrameReservation<'static, EnvironmentOperation<A, E, R, I, V>>,
    pending: FrameReservation<'static, Imported<A, E, V>>,
    source: ConstructionValue<PendingGraphInvocation<A, E, R>>,
    ownership: PreparedOutputOwnerReservation<PreparedOutput<A, E>, V, E>,
    cancellation: PreparedCancellationWait<V, E>,
    context: PreparedRunContext<V, E>,
    snapshot: OriginSnapshotReservation<V, E>,
    prefix: OriginSnapshot,
}
struct EnvironmentOperation<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    I: Send + Sync + 'static,
    V: Send + 'static,
> {
    value: R,
    pending: FrameReservation<'static, Imported<A, E, V>>,
    source: ConstructionValue<PendingGraphInvocation<A, E, R>>,
    ownership: PreparedOutputOwnerReservation<PreparedOutput<A, E>, V, E>,
    cancellation: PreparedCancellationWait<V, E>,
    context: PreparedRunContext<V, E>,
    finish: Finish<A, E>,
    marker: PhantomData<fn() -> I>,
    snapshot: OriginSnapshotReservation<V, E>,
    prefix: OriginSnapshot,
}
impl<A: Send, E: Send, R: Send + Sync, I: Send + Sync, V: Send> Consume<(), Step>
    for EnvironmentOperation<A, E, R, I, V>
{
    fn consume(self, (): ()) -> Step {
        self.context
            .root
            .contexts
            .protected_failure(self.context.id, false);
        let source = match self.source.take() {
            Ok(source) => {
                // Transfer the REAL producer prefix into R's foreign phased
                // footer before publication. No counted parent-output cycle;
                // losing the parent cannot retire R's backing before R drops.
                self.snapshot.bind();
                source.publish_from(self.value, self.prefix)
            }
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
impl<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    I: Send + Sync + 'static,
    const KEY: u64,
> Description for ImportEnvironment<A, E, R, I, KEY>
{
}
impl<
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    I: Send + Sync + 'static,
    const KEY: u64,
> Spec for ImportEnvironment<A, E, R, I, KEY>
{
    type Value = A;
    type Error = E;
    type Env = I;
    type Storage = ();
    type Admission<V: Send + 'static> = EnvironmentAdmission<A, E, R, I, V>;
    fn provision<V: Send + 'static>(
        _: &PreparedMemory<'static>,
        _: RegionId,
        _: usize,
        _: &FramePool<'static>,
        _: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<(), MemoryError> {
        Ok(())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        Demand::cell::<EnvironmentOperation<A, E, R, I, V>>(false)
            .add(Demand::cell::<Imported<A, E, V>>(false))?
            .add(output_owner_shape::<PreparedOutput<A, E>, V, E>())?
            .add(output_origin_shape::<I, V, E>())?
            .add(Demand {
                frame_layout: OriginSnapshotReservation::<V, E>::layout(),
                frames_per_run: 1,
                continuations: 0,
                descriptions: 0,
                origin_frames: 1,
                environment_origin_frames: 0,
                record_payload: Bytes::new(0),
                cleanup_sites: 0,
                panic_sites: 0,
            })?
            .add(output_owner_shape::<OriginSnapshot, V, E>())
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &(),
        environment: &Environment<I>,
        context: &PreparedRunContext<V, E>,
        _: &crate::PreparedScope<E>,
        _: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        _: &(),
        environment: &Environment<I>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin = output_origin_reserve::<I, V, E>(frames, environment, context)?;
        let source = context.construction_value::<PendingGraphInvocation<A, E, R>>(KEY)?;
        if context.root.panic_pool.is_some() {
            source.validate_with(|source| {
                if source.has_panic_diagnostics() {
                    Ok(())
                } else {
                    Err(MemoryError::UnsupportedRunContract)
                }
            })?;
        }
        let (snapshot, prefix) = OriginSnapshotReservation::reserve_prefix(frames, context)?;
        let admission = EnvironmentAdmission {
            operation: frames.reserve()?,
            pending: frames.reserve()?,
            source,
            ownership: context.reserve_output_owner(frames)?,
            cancellation: context.cancelled()?,
            context: context.try_clone()?,
            snapshot,
            prefix,
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
            EnvironmentOperation {
                value: self.value,
                pending: admission.pending,
                source: admission.source,
                ownership: admission.ownership,
                cancellation: admission.cancellation,
                context: admission.context,
                finish: finish.0,
                marker: PhantomData,
                snapshot: admission.snapshot,
                prefix: admission.prefix,
            },
        )))
    }
}
