//! Linear construction permits: actual retained target/vacancy, not copied IDs.
//! Construction capacity is distinct from the returned execution tree's demand.
use super::layer::{LayerSpec, Scoped, ScopedExecution};
use super::*;
use crate::{PreparedScope, PrimaryOutcomeRef, scope::PreparedScopeEntry};

pub struct ScopePermit<E: Send + 'static> {
    entry: PreparedScopeEntry<E>,
}
pub struct ScopedRecipe<A, F, C> {
    acquire: A,
    release: F,
    marker: PhantomData<fn() -> C>,
}
pub struct BoundScoped<A: Description, F, C> {
    // User recipe/captures drop before their actual target ownership.
    recipe: ScopedRecipe<A, F, C>,
    permit: ScopePermit<A::Error>,
}
pub struct TargetMismatch<E: Send + 'static, A, F, C> {
    pub recipe: ScopedRecipe<A, F, C>,
    pub permit: ScopePermit<E>,
}
pub fn recipe<A, F, C>(acquire: A, release: F) -> ScopedRecipe<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
    ScopedRecipe {
        acquire,
        release,
        marker: PhantomData,
    }
}
pub type FinalizerRecipe<C, F> =
    ScopedRecipe<Success<(), <C as Spec>::Error, <C as Spec>::Env>, F, C>;
pub type CheckedBound<E, A, F, C> = Result<BoundScoped<A, F, C>, TargetMismatch<E, A, F, C>>;
pub trait FinalizerFactory<C: Description<Value = ()>>:
    for<'a> FnOnce((), PrimaryOutcomeRef<'a, C::Error>) -> C + Send + 'static
{
}
impl<C, F> FinalizerFactory<C> for F
where
    C: Description<Value = ()>,
    F: for<'a> FnOnce((), PrimaryOutcomeRef<'a, C::Error>) -> C + Send + 'static,
{
}
pub fn finalizer_recipe<F, C>(factory: F) -> FinalizerRecipe<C, impl FinalizerFactory<C>>
where
    C: Description<Value = ()>,
    F: FnOnce() -> C + Send + 'static,
{
    ScopedRecipe {
        acquire: Success {
            value: (),
            marker: PhantomData,
        },
        release: move |(), _primary: PrimaryOutcomeRef<'_, C::Error>| factory(),
        marker: PhantomData,
    }
}
impl<E: Send + 'static> ScopePermit<E> {
    pub(super) fn into_entry(self) -> PreparedScopeEntry<E> {
        self.entry
    }
    /// Explicit host/pre-business target reservation. Never clones at bind time.
    pub fn reserve(scope: &PreparedScope<E>) -> Result<Self, MemoryError> {
        Self::preflight(scope)
    }
    fn preflight(scope: &PreparedScope<E>) -> Result<Self, MemoryError> {
        Ok(Self {
            entry: scope.reserve_entry_preflight()?,
        })
    }
    pub fn bind<A, F, C>(self, recipe: ScopedRecipe<A, F, C>) -> BoundScoped<A, F, C>
    where
        A: Description<Error = E>,
        A::Value: Sync,
        C: Description<Value = (), Error = E, Env = A::Env>,
        F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, E>) -> C + Send + 'static,
    {
        BoundScoped {
            recipe,
            permit: self,
        }
    }
    pub fn try_bind<A, F, C>(
        self,
        scope: &PreparedScope<E>,
        recipe: ScopedRecipe<A, F, C>,
    ) -> CheckedBound<E, A, F, C>
    where
        A: Description<Error = E>,
        A::Value: Sync,
        C: Description<Value = (), Error = E, Env = A::Env>,
        F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, E>) -> C + Send + 'static,
    {
        if self.entry.matches(scope) {
            Ok(self.bind(recipe))
        } else {
            Err(TargetMismatch {
                recipe,
                permit: self,
            })
        }
    }
}
impl<A: Description, F, C> BoundScoped<A, F, C> {
    pub fn unbind(self) -> (ScopePermit<A::Error>, ScopedRecipe<A, F, C>) {
        (self.permit, self.recipe)
    }
}
mod construction_sealed {
    pub trait Sealed {}
}
/// Closed finite construction shape; values contain no invocation permits.
pub trait Construction: construction_sealed::Sealed + Send + Sync + 'static {
    type Permits<E: Send + 'static>: Send + 'static;
    fn count() -> Result<usize, MemoryError>;
    fn reserve<E: Send + 'static>(
        scope: &PreparedScope<E>,
    ) -> Result<Self::Permits<E>, MemoryError>;
}
#[derive(Clone, Copy, Debug)]
pub struct ScopeConstruction;
#[must_use]
pub fn construction() -> ScopeConstruction {
    ScopeConstruction
}
impl construction_sealed::Sealed for ScopeConstruction {}
impl Construction for ScopeConstruction {
    type Permits<E: Send + 'static> = ScopePermit<E>;
    fn count() -> Result<usize, MemoryError> {
        Ok(1)
    }
    fn reserve<E: Send + 'static>(
        scope: &PreparedScope<E>,
    ) -> Result<Self::Permits<E>, MemoryError> {
        ScopePermit::preflight(scope)
    }
}
impl construction_sealed::Sealed for () {}
impl Construction for () {
    type Permits<E: Send + 'static> = ();
    fn count() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn reserve<E: Send + 'static>(_scope: &PreparedScope<E>) -> Result<(), MemoryError> {
        Ok(())
    }
}
impl<L: Construction, R: Construction> construction_sealed::Sealed for (L, R) {}
impl<L: Construction, R: Construction> Construction for (L, R) {
    type Permits<E: Send + 'static> = (L::Permits<E>, R::Permits<E>);
    fn count() -> Result<usize, MemoryError> {
        L::count()?
            .checked_add(R::count()?)
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    fn reserve<E: Send + 'static>(
        scope: &PreparedScope<E>,
    ) -> Result<Self::Permits<E>, MemoryError> {
        Ok((L::reserve(scope)?, R::reserve(scope)?))
    }
}
impl<A, F, C> Description for BoundScoped<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
}
impl<A, F, C> Spec for BoundScoped<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
    type Value = super::ResourceValue<A::Value>;
    type Error = A::Error;
    type Env = A::Env;
    type Storage = <Scoped<A, F, C> as LayerSpec>::Storage;
    type Admission<V: Send + 'static> = ScopedExecution<A, F, C, V>;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        <Scoped<A, F, C> as LayerSpec>::provision::<V>(memory, region, capacity, frames, cleanup)
    }
    fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        <Scoped<A, F, C> as LayerSpec>::storage_bytes::<V>(capacity, layout)
    }
    // The descriptor owns its ACTUAL construction permit. Never reserve a
    // second independently selected target/promise in this execution admission.
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        <Scoped<A, F, C> as LayerSpec>::shape::<V>()
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            let protected = context.protected()?;
            Scoped::<A, F, C>::reserve_execution(frames, storage, environment, context, &protected)
        })();
        match admitted {
            Ok(admitted) => {
                origin
                    .attach(environment.try_clone()?)
                    .map_err(|(error, _)| error)?;
                Ok(admitted)
            }
            Err(error) => Err(error),
        }
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
        _enclosing: &PreparedScope<Self::Error>,
        enclosing_cleanup: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = Scoped::<A, F, C>::reserve_execution(
            frames,
            storage,
            environment,
            context,
            enclosing_cleanup,
        );
        match admitted {
            Ok(admitted) => {
                origin
                    .attach(environment.try_clone()?)
                    .map_err(|(error, _)| error)?;
                Ok(admitted)
            }
            Err(error) => Err(error),
        }
    }
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<Self::Value, Self::Error>,
    ) -> Transition {
        let recipe = self.recipe;
        Transition(Scoped::<A, F, C>::execute_recipe(
            recipe.acquire,
            recipe.release,
            admission,
            self.permit.entry,
            finish.0,
        ))
    }
}
