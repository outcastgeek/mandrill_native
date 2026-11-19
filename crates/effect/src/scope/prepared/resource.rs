//! Actual prepared acquire/register-or-release ownership, not pure finalizer
//! registration. Every physical promise/cell/callback/abandonment/handle is held
//! before entering acquisition; raw success installs ownership before publish.
pub(super) mod entry;
mod future;
mod value;
use super::{PreparedScope, capacity_outcome, capacity_outcome_with};
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::internal::runtime_storage::{Consume, Settlement};
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool};
use crate::scope::{Inner, Registered, RegistrationError};
use crate::{Defect, Effect, Exit, LibraryDefect, PrimaryOutcomeRef, Release};
use entry::{Entry, PreflightError};
pub use future::{PreparedFutureResourceBinding, PreparedFutureResourcePlan};
use std::alloc::Layout;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, RwLock};
pub use value::PreparedResourceValue;
use value::{Cell, Owner};

struct Recipe<T: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static, F, M> {
    acquire: Effect<T, E, R>,
    release: F,
    capacity: M,
}
/// Concrete counted preparation demand. Callback storage is supplied by the
/// scope, using callback_layout and callback_frames; description Arcs remain an
/// explicit legacy interpreter domain rather than hidden in these byte totals.
#[derive(Clone, Copy, Debug)]
pub struct PreparedResourceDemand {
    pub storage_payload: Bytes,
    pub allocation_requests: usize,
    pub maximum_live_resources: usize,
    pub callback_layout: Layout,
    pub callback_frames: usize,
    /// One independently reserved refusal/cleanup accumulator per resource.
    pub cleanup_accumulators: usize,
    /// Release and refusal-mapper diagnostics per admitted resource.
    pub panic_sites: usize,
    /// Some only when the concrete cleanup future working set is known and
    /// physically acquired in preflight, not at cleanup dispatch.
    pub cleanup: Option<PreparedCleanupDemand>,
}
#[derive(Clone, Copy, Debug)]
pub struct PreparedCleanupDemand {
    pub pending_layout: Layout,
    pub completion_layout: Layout,
    pub start_layout: Layout,
    pub native_context: crate::memory::PreparedContextualDemand,
    pub frames: usize,
}
impl PreparedCleanupDemand {
    pub fn frame_layout(self) -> Result<Layout, MemoryError> {
        let native = self.native_context.maximum_layout()?;
        Layout::from_size_align(
            self.pending_layout
                .size()
                .max(self.completion_layout.size())
                .max(self.start_layout.size())
                .max(native.size()),
            self.pending_layout
                .align()
                .max(self.completion_layout.align())
                .max(self.start_layout.align())
                .max(native.align()),
        )
        .map(|layout| layout.pad_to_align())
        .map_err(|_| MemoryError::InvalidLayout)
    }
}
/// Typed startup recipe: reusable acquisition, primary-aware release and the
/// explicit capacity mapper are held in counted prepared metadata. Scoped release
/// receives the same Succeeded marker as ordinary acquire_scoped, not a fabricated
/// borrowed body outcome. Constructing the plan does not invoke either factory.
pub struct PreparedResourcePlan<
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F,
    M,
> {
    acquire: Effect<T, E, R>,
    release: F,
    capacity: M,
}
/// Reusable/concurrent resource admission over the same Scope lifecycle engine.
/// Native acquisition cannot begin until its physical ownership bundle exists.
/// Counted sharing is fallible; the acquisition Effect's legacy operation Arc is
/// shared only for dispatch, not silently charged as prepared description storage.
pub struct PreparedResourceBinding<
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Send + Sync + 'static,
    M: Send + Sync + 'static,
> {
    recipe: SharedSlot<'static, Recipe<T, E, R, F, M>>,
    values: SlotPool<'static, Cell<T>>,
}
impl<T, E, R, F, M> PreparedResourcePlan<T, E, R, F, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: for<'a> Fn(T, PrimaryOutcomeRef<'a, E>) -> Release<E, R> + Send + Sync + 'static,
    M: Fn(MemoryError) -> E + Send + Sync + 'static,
{
    pub fn new(acquire: Effect<T, E, R>, release: F, capacity: M) -> Self {
        Self {
            acquire,
            release,
            capacity,
        }
    }
    pub fn demand(&self, resources: usize) -> Result<PreparedResourceDemand, MemoryError> {
        let operation = Layout::new::<Settlement<Operation<T, E, R, F, M>, Owner<T>>>();
        let abandon = Layout::new::<Abandon<T>>();
        let callback_layout = Layout::from_size_align(
            operation.size().max(abandon.size()),
            operation.align().max(abandon.align()),
        )
        .map_err(|_| MemoryError::InvalidLayout)?
        .pad_to_align();
        Ok(PreparedResourceDemand {
            storage_payload: SlotPool::<Cell<T>>::storage_bytes(resources)?
                .checked_add(SlotPool::<Recipe<T, E, R, F, M>>::storage_bytes(1)?)?,
            allocation_requests: 2,
            maximum_live_resources: resources,
            cleanup_accumulators: resources,
            panic_sites: resources
                .checked_mul(2)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            callback_layout,
            cleanup: None,
            callback_frames: resources
                .checked_mul(2)
                .ok_or(MemoryError::ArithmeticOverflow)?,
        })
    }
    pub fn prepare(
        self,
        memory: &PreparedMemory<'static>,
        metadata: RegionId,
        resources: usize,
    ) -> Result<PreparedResourceBinding<T, E, R, F, M>, MemoryError> {
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
        });
        let recipes = memory.slots(metadata, 1, |_| recipe.take().ok_or(MemoryError::NoSlot))?;
        let recipe = recipes.try_checkout()?.share();
        Ok(PreparedResourceBinding { recipe, values })
    }
}
struct Operation<
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Send + Sync + 'static,
    M: Send + Sync + 'static,
> {
    value: PreparedResourceValue<T>,
    recipe: SharedSlot<'static, Recipe<T, E, R, F, M>>,
    environment: Arc<R>,
    context: Context,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<T, E, R, F, M> Consume<Finish<(), E>, Step> for Operation<T, E, R, F, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: for<'a> Fn(T, PrimaryOutcomeRef<'a, E>) -> Release<E, R> + Send + Sync + 'static,
    M: Fn(MemoryError) -> E + Send + Sync + 'static,
{
    fn consume(self, finish: Finish<(), E>) -> Step {
        let cleanup = catch_unwind(AssertUnwindSafe(|| {
            self.value
                .invalidate()
                .map(|resource| (self.recipe.release)(resource, PrimaryOutcomeRef::Succeeded))
        }));
        match cleanup {
            Ok(Some(cleanup)) => cleanup.execute(self.environment, self.context, finish),
            Ok(None) => runtime::finish(
                finish,
                Exit::Die(Defect::from(LibraryDefect::ResourceReleasedTwice)),
            ),
            Err(payload) => runtime::finish(
                finish,
                Exit::Die(crate::internal::runtime::panic_defect(self.panic, payload)),
            ),
        }
    }
}
struct Abandon<T: Send + Sync + 'static> {
    value: PreparedResourceValue<T>,
}
impl<T: Send + Sync + 'static> Consume<(), ()> for Abandon<T> {
    fn consume(self, (): ()) {
        drop(self.value.invalidate());
    }
}
type Admission<T, E, R, F, M> = Result<Bundle<T, E, Recipe<T, E, R, F, M>>, PreflightError>;
trait Capacity<E> {
    fn map_capacity(&self, error: MemoryError) -> E;
}
impl<T, E, R, F, M> Capacity<E> for Recipe<T, E, R, F, M>
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
struct Bundle<T: Send + Sync + 'static, E: Send + 'static, Q: Send + Sync + 'static> {
    recipe: SharedSlot<'static, Q>,
    entry: Entry<E>,
    finalizer: Registered<E>,
    output: PreparedResourceValue<T>,
    closing: SharedSlot<'static, Inner<E>>,
    cleanup_buffer: crate::CleanupFailures<E>,
    mapper_panic: Option<crate::PreparedPanicReservation>,
}
impl<T, E, R, F, M> PreparedResourceBinding<T, E, R, F, M>
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: for<'a> Fn(T, PrimaryOutcomeRef<'a, E>) -> Release<E, R> + Send + Sync + 'static,
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
    ) -> Admission<T, E, R, F, M> {
        let cleanup_buffer = scope.reserve_cleanup_buffer()?;
        let entry = Entry::reserve(scope)?;
        let closing = scope.inner.try_clone()?;
        let recipe = self.recipe.try_clone()?;
        let acquisition = self.recipe.try_clone()?;
        let output = PreparedResourceValue {
            cell: self.values.try_checkout()?.share(),
        };
        // All fallible owner acquisition precedes native acquisition. None of
        // these are payload Clone and the caller output is never published early.
        let operation_value = output.try_clone()?;
        let guard_value = output.try_clone()?;
        let abandoned = output.try_clone()?;
        let operation = scope
            .callbacks
            .reserve::<Settlement<Operation<T, E, R, F, M>, Owner<T>>>()?;
        let abandon = scope.callbacks.reserve::<Abandon<T>>()?;
        let finalizer = Registered::from_prepared_settlement(
            operation,
            Settlement {
                operation: Operation {
                    value: operation_value,
                    recipe,
                    environment,
                    context: context.protected(),
                    panic: release_panic,
                },
                guard: Owner { value: guard_value },
            },
            abandon,
            Abandon { value: abandoned },
        );
        Ok(Bundle {
            entry,
            finalizer,
            output,
            recipe: acquisition,
            closing,
            cleanup_buffer,
            mapper_panic: None,
        })
    }
    /// Admit diagnostic authorities before returning one lazy acquisition Effect.
    /// Refusal preserves this binding; no native acquisition runs at admission.
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
                        capacity_outcome_with(error, &binding.recipe.capacity, Some(mapper_panic)),
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
    /// Reserve before acquisition. Preclosed admission reports the existing
    /// registration defect without starting acquisition; a close after admission
    /// transfers raw success into awaited immediate protected cleanup. Full and
    /// other storage refusals map explicitly to E, never a capacity-to-Die cast.
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
fn complete<T, E, Q>(
    acquired: Exit<T, E>,
    bundle: Bundle<T, E, Q>,
    finish: Finish<PreparedResourceValue<T>, E>,
) -> Step
where
    T: Send + Sync + 'static,
    E: Send + 'static,
    Q: Capacity<E> + Send + Sync + 'static,
{
    match acquired.into_failure() {
        Err(failure) => runtime::finish(finish, failure),
        Ok(resource) => {
            let Bundle {
                entry,
                finalizer,
                output,
                recipe,
                closing,
                cleanup_buffer,
                mapper_panic,
            } = bundle;
            // Raw ownership precedes cancellation/publication in BOTH adapters.
            output.install(resource);
            match entry.commit(finalizer) {
                Ok(()) => runtime::finish(finish, Exit::Success(output)),
                Err(RegistrationError::Closed(finalizer)) => refused(
                    finalizer,
                    output,
                    closing,
                    Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
                    finish,
                    cleanup_buffer,
                ),
                Err(RegistrationError::Full(finalizer)) => refused(
                    finalizer,
                    output,
                    closing,
                    capacity_outcome_with(
                        MemoryError::NoSlot,
                        &|error| recipe.map_capacity(error),
                        mapper_panic,
                    ),
                    finish,
                    cleanup_buffer,
                ),
            }
        }
    }
}
fn refused<T: Send + Sync + 'static, E: Send + 'static>(
    finalizer: Registered<E>,
    output: PreparedResourceValue<T>,
    closing: SharedSlot<'static, Inner<E>>,
    primary: Exit<PreparedResourceValue<T>, E>,
    finish: Finish<PreparedResourceValue<T>, E>,
    cleanup_buffer: crate::CleanupFailures<E>,
) -> Step {
    finalizer.execute_with(move |frame| {
        runtime::continuation_retained(
            move |cleanup| {
                drop(output);
                drop(closing);
                let mut accumulator =
                    crate::PreparedCleanupAccumulator::from_reserved(primary, cleanup_buffer);
                accumulator.append_cleanup(cleanup);
                runtime::finish(finish, accumulator.finish())
            },
            frame,
        )
    })
}
