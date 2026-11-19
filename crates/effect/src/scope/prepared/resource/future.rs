//! Concrete cleanup futures: reserve BOTH pending and completion payloads during
//! acquisition preflight, never by attempting another capacity claim at release.
use super::super::{PreparedScope, capacity_outcome};
use super::{
    Abandon, Bundle, Capacity, Cell, Entry, Owner, PreflightError, PreparedCleanupDemand,
    PreparedResourceDemand, PreparedResourceValue, complete,
};
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::internal::runtime_storage::{
    Consume, FutureReservation, InstrumentationReservation, Once, Settlement,
};
use crate::memory::{
    FrameReservation, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use crate::scope::Registered;
use crate::{Defect, Effect, Exit, LibraryDefect, NativeContext, PrimaryOutcomeRef};
use std::alloc::Layout;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

type Admission<T, E, R, F, P, M> = Result<Bundle<T, E, Recipe<T, E, R, F, P, M>>, PreflightError>;
struct Recipe<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static, F, P, M> {
    acquire: Effect<T, E, R>,
    release: F,
    capacity: M,
    pending: PhantomData<fn() -> P>,
}
/// Concrete-future resource recipe. The future type's actual compiler layout and
/// completion payload are known without invoking release or acquiring T. The
/// primary argument follows the existing scoped Succeeded-marker contract.
pub struct PreparedFutureResourcePlan<
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F,
    P,
    M,
> {
    acquire: Effect<T, E, R>,
    release: F,
    capacity: M,
    pending: PhantomData<fn() -> P>,
}
/// Early working-set admission for one concrete cleanup future. Callback,
/// abandonment and seven cleanup cells plus resource/entry/shared owners precede
/// native acquisition; native-context entry/restore boundaries remain unchanged
/// while their tag/restore/invoke payloads use physical reserved storage.
/// This is not an automatic transitive demand proof for arbitrary nested native
/// callbacks; complete generated run/context/provider contracts remain separate.
pub struct PreparedFutureResourceBinding<
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Send + Sync + 'static,
    P: 'static,
    M: Send + Sync + 'static,
> {
    recipe: SharedSlot<'static, Recipe<T, E, R, F, P, M>>,
    values: SlotPool<'static, Cell<T>>,
}
impl<T, E, R, F, P, M> Capacity<E> for Recipe<T, E, R, F, P, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    M: Fn(MemoryError) -> E,
{
    fn map_capacity(&self, error: MemoryError) -> E {
        (self.capacity)(error)
    }
}
impl<T, E, R, F, P, M> PreparedFutureResourcePlan<T, E, R, F, P, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: for<'a> Fn(T, PrimaryOutcomeRef<'a, E>, Arc<R>, NativeContext) -> P + Send + Sync + 'static,
    P: Future<Output = Exit<(), E>> + Send + 'static,
    M: Fn(MemoryError) -> E + Send + Sync + 'static,
{
    pub fn new(acquire: Effect<T, E, R>, release: F, capacity: M) -> Self {
        Self {
            acquire,
            release,
            capacity,
            pending: PhantomData,
        }
    }
    pub fn demand(&self, resources: usize) -> Result<PreparedResourceDemand, MemoryError> {
        let operation = Layout::new::<Settlement<Operation<T, E, R, F, P, M>, Owner<T>>>();
        let abandon = Layout::new::<Abandon<T>>();
        let callback_layout = Layout::from_size_align(
            operation.size().max(abandon.size()),
            operation.align().max(abandon.align()),
        )
        .map_err(|_| MemoryError::InvalidLayout)?
        .pad_to_align();
        let (pending_layout, completion_layout) = FutureReservation::<P, (), E>::layouts();
        let frames = resources
            .checked_mul(2)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let cleanup_frames = resources
            .checked_mul(7)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let start_layout = Layout::new::<Start<T, E, R, F, P, M>>();
        Ok(PreparedResourceDemand {
            storage_payload: SlotPool::<Cell<T>>::storage_bytes(resources)?
                .checked_add(SlotPool::<Recipe<T, E, R, F, P, M>>::storage_bytes(1)?)?,
            allocation_requests: 2,
            maximum_live_resources: resources,
            cleanup_accumulators: resources,
            panic_sites: resources
                .checked_mul(2)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            callback_layout,
            callback_frames: frames,
            cleanup: Some(PreparedCleanupDemand {
                pending_layout,
                completion_layout,
                start_layout,
                native_context: InstrumentationReservation::<(), E>::layouts(),
                frames: cleanup_frames,
            }),
        })
    }
    pub fn prepare(
        self,
        memory: &PreparedMemory<'static>,
        metadata: RegionId,
        resources: usize,
    ) -> Result<PreparedFutureResourceBinding<T, E, R, F, P, M>, MemoryError> {
        let _demand = self.demand(resources)?;
        let values = memory.slots(metadata, resources, |_| {
            Ok(Cell {
                value: RwLock::new(None),
            })
        })?;
        let mut recipe = Some(Recipe {
            acquire: self.acquire,
            release: self.release,
            capacity: self.capacity,
            pending: PhantomData,
        });
        let recipes = memory.slots(metadata, 1, |_| recipe.take().ok_or(MemoryError::NoSlot))?;
        Ok(PreparedFutureResourceBinding {
            recipe: recipes.try_checkout()?.share(),
            values,
        })
    }
}
struct Operation<T, E, R, F, P, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Send + Sync + 'static,
    P: Future<Output = Exit<(), E>> + Send + 'static,
    M: Send + Sync + 'static,
{
    value: PreparedResourceValue<T>,
    recipe: SharedSlot<'static, Recipe<T, E, R, F, P, M>>,
    environment: Arc<R>,
    context: NativeContext,
    reservation: FutureReservation<P, (), E>,
    start: FrameReservation<'static, Start<T, E, R, F, P, M>>,
    instrumentation: InstrumentationReservation<(), E>,
}
impl<T, E, R, F, P, M> Consume<Finish<(), E>, Step> for Operation<T, E, R, F, P, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: for<'a> Fn(T, PrimaryOutcomeRef<'a, E>, Arc<R>, NativeContext) -> P + Send + Sync + 'static,
    P: Future<Output = Exit<(), E>> + Send + 'static,
    M: Send + Sync + 'static,
{
    fn consume(self, finish: Finish<(), E>) -> Step {
        let context = self.context.context.clone();
        let (finish, tag) = self.instrumentation.finish(finish, context.clone());
        let step = match self.value.invalidate() {
            Some(resource) => Step::More(Once::prepared(
                self.start,
                Start {
                    resource,
                    recipe: self.recipe,
                    environment: self.environment,
                    context: self.context,
                    reservation: self.reservation,
                    finish,
                },
            )),
            None => self.reservation.complete(
                Exit::Die(Defect::from(LibraryDefect::ResourceReleasedTwice)),
                finish,
            ),
        };
        runtime::instrument_step_reserved(step, context, tag)
    }
}
struct Start<T, E, R, F, P, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Send + Sync + 'static,
    P: Future<Output = Exit<(), E>> + Send + 'static,
    M: Send + Sync + 'static,
{
    resource: T,
    recipe: SharedSlot<'static, Recipe<T, E, R, F, P, M>>,
    environment: Arc<R>,
    context: NativeContext,
    reservation: FutureReservation<P, (), E>,
    finish: Finish<(), E>,
}
impl<T, E, R, F, P, M> Consume<(), Step> for Start<T, E, R, F, P, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: for<'a> Fn(T, PrimaryOutcomeRef<'a, E>, Arc<R>, NativeContext) -> P + Send + Sync + 'static,
    P: Future<Output = Exit<(), E>> + Send + 'static,
    M: Send + Sync + 'static,
{
    fn consume(self, (): ()) -> Step {
        if self.context.context.cancelled() {
            return self.reservation.complete(Exit::Interrupted, self.finish);
        }
        self.reservation.start(
            move || {
                (self.recipe.release)(
                    self.resource,
                    PrimaryOutcomeRef::Succeeded,
                    self.environment,
                    self.context,
                )
            },
            self.finish,
        )
    }
}
impl<T, E, R, F, P, M> PreparedFutureResourceBinding<T, E, R, F, P, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: for<'a> Fn(T, PrimaryOutcomeRef<'a, E>, Arc<R>, NativeContext) -> P + Send + Sync + 'static,
    P: Future<Output = Exit<(), E>> + Send + 'static,
    M: Fn(MemoryError) -> E + Send + Sync + 'static,
{
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            recipe: self.recipe.try_clone()?,
            values: self.values.try_clone()?,
        })
    }
    fn reserve(
        &self,
        scope: &PreparedScope<E>,
        environment: Arc<R>,
        context: &Context,
        release_panic: Option<crate::PreparedPanicReservation>,
    ) -> Admission<T, E, R, F, P, M> {
        let cleanup_buffer = scope.reserve_cleanup_buffer()?;
        let entry = Entry::reserve(scope)?;
        let closing = scope.inner.try_clone()?;
        let recipe = self.recipe.try_clone()?;
        let acquisition = self.recipe.try_clone()?;
        let output = PreparedResourceValue {
            cell: self.values.try_checkout()?.share(),
        };
        let operation_value = output.try_clone()?;
        let guard_value = output.try_clone()?;
        let abandoned = output.try_clone()?;
        let operation = scope
            .callbacks
            .reserve::<Settlement<Operation<T, E, R, F, P, M>, Owner<T>>>()?;
        let abandon = scope.callbacks.reserve::<Abandon<T>>()?;
        let protected = context.protected();
        let pool = protected
            .frames()
            .ok_or(MemoryError::MissingRuntimeStorage)?;
        let reservation = FutureReservation::<P, (), E>::reserve(pool)?.with_panic(release_panic);
        let start = pool.reserve::<Start<T, E, R, F, P, M>>()?;
        let instrumentation = InstrumentationReservation::<(), E>::reserve(pool)?;
        let finalizer = Registered::from_prepared_settlement(
            operation,
            Settlement {
                operation: Operation {
                    value: operation_value,
                    recipe,
                    environment,
                    context: NativeContext { context: protected },
                    reservation,
                    start,
                    instrumentation,
                },
                guard: Owner { value: guard_value },
            },
            abandon,
            Abandon { value: abandoned },
        );
        Ok(Bundle {
            recipe: acquisition,
            entry,
            finalizer,
            output,
            closing,
            cleanup_buffer,
            mapper_panic: None,
        })
    }
    /// Admit release and refusal diagnostics before returning one lazy Effect.
    pub fn admit_acquire_scoped(
        &self,
        scope: PreparedScope<E>,
    ) -> Result<Effect<PreparedResourceValue<T>, E, R>, MemoryError> {
        let release_panic = scope
            .reserve_panic()?
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let mapper_panic = scope
            .reserve_panic()?
            .ok_or(MemoryError::UnsupportedRunContract)?;
        let binding = self.try_clone()?;
        let candidate = std::sync::Mutex::new(Some((binding, scope, release_panic, mapper_panic)));
        Ok(Effect::new(move |environment, context, finish| {
            let candidate = candidate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let Some((binding, scope, release_panic, mapper_panic)) = candidate else {
                return runtime::finish(
                    finish,
                    Exit::Die(Defect::from(LibraryDefect::ScopeOutcomeConsumed)),
                );
            };
            let mut bundle = match binding.reserve(
                &scope,
                Arc::clone(&environment),
                &context,
                Some(release_panic),
            ) {
                Ok(bundle) => bundle,
                Err(PreflightError::Capacity(error)) => {
                    return runtime::finish(
                        finish,
                        super::super::capacity_outcome_with(
                            error,
                            &binding.recipe.capacity,
                            Some(mapper_panic),
                        ),
                    );
                }
                Err(PreflightError::Closed) => {
                    return runtime::finish(
                        finish,
                        Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
                    );
                }
            };
            bundle.mapper_panic = Some(mapper_panic);
            bundle.recipe.acquire.clone().execute(
                environment,
                context,
                runtime::continuation(move |acquired| complete(acquired, bundle, finish)),
            )
        }))
    }
    pub fn acquire_scoped(self, scope: PreparedScope<E>) -> Effect<PreparedResourceValue<T>, E, R> {
        Effect::new(move |environment, context, finish| {
            let panic = match scope.reserve_panic() {
                Ok(panic) => panic,
                Err(error) => {
                    return runtime::finish(finish, capacity_outcome(error, &self.recipe.capacity));
                }
            };
            let bundle = match self.reserve(&scope, Arc::clone(&environment), &context, panic) {
                Ok(bundle) => bundle,
                Err(PreflightError::Capacity(error)) => {
                    return runtime::finish(finish, capacity_outcome(error, &self.recipe.capacity));
                }
                Err(PreflightError::Closed) => {
                    return runtime::finish(
                        finish,
                        Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
                    );
                }
            };
            bundle.recipe.acquire.clone().execute(
                environment,
                context,
                runtime::continuation(move |acquired| complete(acquired, bundle, finish)),
            )
        })
    }
}
