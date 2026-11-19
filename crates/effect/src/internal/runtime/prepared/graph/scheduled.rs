//! R-selected physical native scheduling with recursively reserved typed payload,
//! provider queue/result/cancel records, D/diagnostics and final output transfer.
use super::future::{FutureContext, context_shape, reserve_context};
use super::*;
use crate::internal::runtime::DriveContext;
use crate::internal::runtime_storage::Pending;
use crate::prepared_provider::{
    HasPreparedProvider, PreparedCancellation, PreparedProviderConstruction,
    PreparedProviderOutputLease, PreparedReceipt, PreparedTaskFuture, ProviderRefusal,
};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};

type Budget<R, A, E, K> = <R as HasPreparedProvider<A, E, K>>::WorkingSet;
type Job<A, E> = PreparedTaskFuture<A, E>;
type Lease<R, A, E, K> = PreparedProviderOutputLease<Job<A, E>, Exit<A, E>, Budget<R, A, E, K>>;
type Receipt<R, A, E, K> = PreparedReceipt<Job<A, E>, Exit<A, E>, Budget<R, A, E, K>>;
type Ownership<R, A, E, K, V> = PreparedOutputOwnerReservation<Lease<R, A, E, K>, V, E>;
type Preparation<R, A, E, K> = PreparedProviderConstruction<A, E, Budget<R, A, E, K>>;
type PreparationSource<R, A, E, K> =
    super::super::construction::ConstructionValue<Preparation<R, A, E, K>>;
type Types<P, A, E, R, K> = fn() -> (P, A, E, R, K);
pub struct Scheduled<F, P, A, E, R, K> {
    factory: F,
    marker: PhantomData<Types<P, A, E, R, K>>,
}
#[must_use]
pub fn scheduled<F, P, A, E, R, K>(factory: F) -> Scheduled<F, P, A, E, R, K>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    F: FnOnce(Environment<R>, FutureContext<E>) -> P + Send + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
{
    Scheduled {
        factory,
        marker: PhantomData,
    }
}
struct NativeTask<
    F,
    P,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
> {
    factory: Option<F>,
    pending: Option<P>,
    environment: Option<Environment<R>>,
    native: Option<FutureContext<E>>,
    cancellation: PreparedCancellation,
    poll_context: crate::internal::runtime::PollContext,
    context: PreparedRunContext<V, E>,
    marker: PhantomData<fn() -> A>,
}
impl<F, P, A, E, R, V> Future for NativeTask<F, P, A, E, R, V>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    V: Send + 'static,
    F: FnOnce(Environment<R>, FutureContext<E>) -> P,
    P: Future<Output = Exit<A, E>>,
{
    type Output = Exit<A, E>;
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        // SAFETY: payload placed once in its typed stable frame. Only pending
        // is structurally pinned; factory/control Options are never projected.
        let Self {
            factory,
            pending,
            environment,
            native,
            cancellation,
            poll_context,
            context,
            ..
        } = unsafe { self.get_unchecked_mut() };
        // Worker polling is independent of the parent trampoline. Enter here
        // on every poll, including the first factory and interrupted outcomes.
        let guard = poll_context
            .instrumentation
            .as_ref()
            .map(|hook| hook.enter(&poll_context.trace));
        let mut run = || {
            if cancellation.is_requested() {
                let _ = context.request_cancellation();
            }
            if pending.is_none() {
                context.root.contexts.protected_failure(context.id, false);
                if context.is_cancelled() {
                    return Poll::Ready(Exit::Interrupted);
                }
                let (Some(factory), Some(environment), Some(native)) =
                    (factory.take(), environment.take(), native.take())
                else {
                    return Poll::Ready(Exit::Die(Defect::from(
                        crate::LibraryDefect::FutureFactoryConsumed,
                    )));
                };
                // The factory runs on the selected worker after native context entry.
                *pending = Some(factory(environment, native));
            }
            let Some(pending) = pending.as_mut() else {
                return Poll::Pending;
            };
            // SAFETY: the pending field is stable and never moved after first poll.
            let result = unsafe { Pin::new_unchecked(pending) }.poll(cx);
            context.acknowledge_clock();
            result.map(|exit| context.publish(exit))
        };
        if guard.is_none() && let Some((hook, trace)) = context.prepared_instrumentation() {
            return hook.with_scope(trace, run);
        }
        run()
    }
}
pub struct Controls<
    F: Send + 'static,
    P: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    V: Send + 'static,
> {
    pending: FrameReservation<'static, Await<A, E, R, K, V>>,
    payload: FrameReservation<'static, NativeTask<F, P, A, E, R, V>>,
    preparation: PreparationSource<R, A, E, K>,
    snapshot: super::super::OriginSnapshotReservation<V, E>,
    origins: super::super::NativeGraphOrigins<Budget<R, A, E, K>>,
    ownership: PreparedOutputOwnerReservation<Lease<R, A, E, K>, V, E>,
    environment: Environment<R>,
    native: FutureContext<E>,
    cancellation: crate::PreparedCancellationWait<V, E>,
    worker: PreparedRunContext<V, E>,
    poll_context: crate::internal::runtime::PollContext,
    context: PreparedRunContext<V, E>,
    panic: Option<crate::PreparedPanicReservation>,
}
pub struct Admission<
    F: Send + 'static,
    P: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    V: Send + 'static,
> {
    operation: FrameReservation<'static, Operation<F, P, A, E, R, K, V>>,
    controls: Controls<F, P, A, E, R, K, V>,
}
struct Operation<
    F: Send + 'static,
    P: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    V: Send + 'static,
> {
    description: Scheduled<F, P, A, E, R, K>,
    admission: Controls<F, P, A, E, R, K, V>,
    finish: Finish<A, E>,
}
struct Await<
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    V: Send + 'static,
> {
    cancellation: crate::PreparedCancellationWait<V, E>,
    finish: Option<Finish<A, E>>,
    ownership: Option<Ownership<R, A, E, K, V>>,
    context: PreparedRunContext<V, E>,
    panic: Option<crate::PreparedPanicReservation>,
    // Last: native waiter/capture descendants retire before this entry/D.
    receipt: Option<Receipt<R, A, E, K>>,
}
// No field is structurally pinned: all payloads live in separately retained
// stable cells, and these fields are only movable observation/control handles.
impl<
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    V: Send + 'static,
> Unpin for Await<A, E, R, K, V>
{
}
impl<A, E, R, K, V> Future for Await<A, E, R, K, V>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    V: Send + 'static,
{
    type Output = Step;
    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Step> {
        let cancelled = Pin::new(&mut self.cancellation).poll(cx).is_ready();
        let Some(receipt) = self.receipt.as_mut() else {
            return Poll::Pending;
        };
        if cancelled {
            receipt.request_cancel();
        }
        if receipt.poll_final(cx).is_pending() {
            self.context.acknowledge_clock();
            return Poll::Pending;
        }
        let (Some(receipt), Some(ownership), Some(finish)) = (
            self.receipt.take(),
            self.ownership.take(),
            self.finish.take(),
        ) else {
            return Poll::Ready(Step::Done);
        };
        // SAFETY: this private operation never exposes its sole receipt or any
        // public result aliases. poll_final excludes further worker result writes;
        // its remaining notification owner cannot read/extract the settlement.
        let output = unsafe { receipt.into_owned_for_graph() };
        let panic = self.panic.take();
        Poll::Ready(output.handoff_exit(ownership, &self.context, panic, finish))
    }
}
impl<F, P, A, E, R, K, V> Consume<(), Step> for Operation<F, P, A, E, R, K, V>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    V: Send + 'static,
    F: FnOnce(Environment<R>, FutureContext<E>) -> P + Send + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let Controls {
            pending,
            payload,
            preparation,
            snapshot,
            origins,
            ownership,
            environment,
            native,
            cancellation,
            worker,
            poll_context,
            context,
            panic,
            ..
        } = self.admission;
        let PreparedProviderConstruction {
            submission,
            diagnostics,
            retention: _retention,
        } = match preparation.take() {
            Ok(value) => value,
            Err(_) => {
                return self
                    .finish
                    .call(Exit::Die(Defect::from(
                        crate::LibraryDefect::PreparedConstructionUnbound,
                    )))
                    .unwrap_or(Step::Done);
            }
        };
        snapshot.bind();
        submission.install_graph_origins(origins);
        // Later local than the guards: captures retire before D on cancellation.
        let factory = self.description.factory;
        if context.is_cancelled() {
            return self.finish.call(Exit::Interrupted).unwrap_or(Step::Done);
        }
        let receipt = submission.submit_graph(diagnostics, move |_, cancellation| {
            PreparedTaskFuture::place(
                payload,
                NativeTask {
                    factory: Some(factory),
                    pending: None,
                    environment: Some(environment),
                    native: Some(native),
                    cancellation,
                    poll_context,
                    context: worker,
                    marker: PhantomData,
                },
            )
        });
        match receipt {
            Ok(receipt) => Step::Suspended(Pending::prepared(
                pending,
                Await::<A, E, R, K, V> {
                    receipt: Some(receipt),
                    cancellation,
                    ownership: Some(ownership),
                    finish: Some(self.finish),
                    context,
                    panic,
                },
            )),
            // Physical reservation guarantees a queue cell; this is an internal
            // invariant, NOT the typed pre-execution capacity refusal path.
            Err(_) => self
                .finish
                .call(Exit::Die(Defect::from(
                    crate::LibraryDefect::PreparedProviderHandoffRefused,
                )))
                .unwrap_or(Step::Done),
        }
    }
}
fn memory_refusal(error: ProviderRefusal) -> MemoryError {
    match error {
        ProviderRefusal::Memory(error) => error,
        ProviderRefusal::Closed => MemoryError::Closed,
        ProviderRefusal::InvalidEnvelope | ProviderRefusal::InsufficientWorkers => {
            MemoryError::UnsupportedRunContract
        }
    }
}
impl<F, P, A, E, R, K> Description for Scheduled<F, P, A, E, R, K>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    F: FnOnce(Environment<R>, FutureContext<E>) -> P + Send + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
{
}
impl<F, P, A, E, R, K> Spec for Scheduled<F, P, A, E, R, K>
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: Send + 'static,
    F: FnOnce(Environment<R>, FutureContext<E>) -> P + Send + 'static,
    P: Future<Output = Exit<A, E>> + Send + 'static,
{
    type Value = A;
    type Error = E;
    type Env = R;
    type Storage = ();
    type Admission<V: Send + 'static> = Admission<F, P, A, E, R, K, V>;
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
        let demand = Demand::cell::<Operation<F, P, A, E, R, K, V>>(false)
            .add(Demand::panic_sites(1))?
            .add(Demand::cell::<Await<A, E, R, K, V>>(false))?
            .add(Demand::cell::<NativeTask<F, P, A, E, R, V>>(false))?
            .add(context_shape::<V, E>()?)?
            .add(output_owner_shape::<Lease<R, A, E, K>, V, E>())?
            .add(output_origin_shape::<R, V, E>())?
            .add(native_origins_shape::<Budget<R, A, E, K>, V, E>()?)?;
        if <R as HasPreparedProvider<A, E, K>>::CONSTRUCTION_KEY.is_none() {
            demand.add(Demand {
                frame_layout: PreparationSource::<R, A, E, K>::layout::<V, E>(),
                frames_per_run: 1,
                continuations: 0,
                descriptions: 0,
                origin_frames: 0,
                environment_origin_frames: 0,
                record_payload: Bytes::new(0),
                cleanup_sites: 0,
                panic_sites: 0,
            })
        } else {
            Ok(demand)
        }
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
        let preparation = if let Some(key) = <R as HasPreparedProvider<A, E, K>>::CONSTRUCTION_KEY {
            context.construction_value::<Preparation<R, A, E, K>>(key)?
        } else {
            // Same actual prepared indirection as a foreign construction site.
            // The typed frame/node is acquired before provider/budget callbacks;
            // no heap Box, large inline variant or metadata fallback is needed.
            PreparationSource::<R, A, E, K>::native(frames, context, || {
                environment
                    .with(|env| {
                        let submission =
                            env.prepared_provider().reserve().map_err(memory_refusal)?;
                        let retention = submission.retain_graph_origin_preflight()?;
                        let budget = env.reserve_provider_budget()?;
                        let diagnostics = submission.install_graph_budget(budget);
                        Ok::<_, MemoryError>(PreparedProviderConstruction {
                            submission,
                            diagnostics,
                            retention,
                        })
                    })
                    .map_err(|_| MemoryError::UnsupportedRunContract)?
            })?
        };
        let worker = context.child()?;
        let (snapshot, origins) =
            super::super::OriginSnapshotReservation::reserve(frames, &worker)?;
        let operation = frames.reserve()?;
        let controls = Controls {
            pending: frames.reserve()?,
            payload: frames.reserve()?,
            preparation,
            snapshot,
            origins,
            ownership: worker.reserve_output_owner(frames)?,
            environment: environment.try_clone()?,
            native: reserve_context(frames, &worker)?,
            cancellation: context.cancelled()?,
            poll_context: worker.poll_context(),
            worker,
            context: context.try_clone()?,
            panic: context.reserve_panic()?,
        };
        origin
            .attach(environment.try_clone()?)
            .map_err(|(error, _)| error)?;
        Ok(Admission {
            operation,
            controls,
        })
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<A, E>,
    ) -> Transition {
        let Admission {
            operation,
            controls,
        } = admission;
        Transition(Step::More(Once::prepared(
            operation,
            Operation {
                description: self,
                admission: controls,
                finish: finish.0,
            },
        )))
    }
}
