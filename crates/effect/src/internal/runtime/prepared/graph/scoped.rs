//! Generated scope bodies use the canonical prepared Scope/Closing protocol.
//! Construction promises and the whole body/close tree precede user factories.
use super::resource::Closing;
use super::scope_binding::Construction;
use super::*;
use crate::{PreparedScope, PreparedScopeLimits, PreparedScopePool};

mod sealed {
    pub trait Sealed {}
}
pub trait ScopeBody<N: Description, P: Construction>: sealed::Sealed + Send + 'static {
    fn build(self, scope: PreparedScope<N::Error>, permits: P::Permits<N::Error>) -> N;
}
pub struct ScopeFactory<F>(F);
pub struct ScopeConstructor<F>(F);
pub struct ProgramBody<N>(N);
impl<F> sealed::Sealed for ScopeFactory<F> {}
impl<F> sealed::Sealed for ScopeConstructor<F> {}
impl<N> sealed::Sealed for ProgramBody<N> {}
impl<F, N> ScopeBody<N, ()> for ScopeFactory<F>
where
    N: Description,
    F: FnOnce(PreparedScope<N::Error>) -> N + Send + 'static,
{
    fn build(self, scope: PreparedScope<N::Error>, (): ()) -> N {
        (self.0)(scope)
    }
}
impl<F, N, P> ScopeBody<N, P> for ScopeConstructor<F>
where
    N: Description,
    P: Construction,
    F: FnOnce(PreparedScope<N::Error>, P::Permits<N::Error>) -> N + Send + 'static,
{
    fn build(self, scope: PreparedScope<N::Error>, permits: P::Permits<N::Error>) -> N {
        (self.0)(scope, permits)
    }
}
impl<N: Description> ScopeBody<N, ()> for ProgramBody<N> {
    fn build(self, _scope: PreparedScope<N::Error>, (): ()) -> N {
        self.0
    }
}
pub struct Scoped<B, N: Description, P: Construction> {
    body: B,
    marker: PhantomData<fn() -> (N, P)>,
}
pub type ScopedProgram<N> = Scoped<ProgramBody<N>, N, ()>;
#[must_use]
pub fn scope<F, N>(body: F) -> Scoped<ScopeFactory<F>, N, ()>
where
    N: Description,
    F: FnOnce(PreparedScope<N::Error>) -> N + Send + 'static,
{
    Scoped {
        body: ScopeFactory(body),
        marker: PhantomData,
    }
}
#[must_use]
pub fn scope_construct<F, N, P>(body: F, _construction: P) -> Scoped<ScopeConstructor<F>, N, P>
where
    N: Description,
    P: Construction,
    F: FnOnce(PreparedScope<N::Error>, P::Permits<N::Error>) -> N + Send + 'static,
{
    Scoped {
        body: ScopeConstructor(body),
        marker: PhantomData,
    }
}
#[must_use]
pub(super) fn scope_program<N: Description>(program: N) -> ScopedProgram<N> {
    Scoped {
        body: ProgramBody(program),
        marker: PhantomData,
    }
}
pub struct Storage<S, E: Send + 'static> {
    body: S,
    scopes: PreparedScopePool<E>,
}
pub struct Admission<B: ScopeBody<N, P>, N: Description, P: Construction, V: Send + 'static> {
    entry: FrameReservation<'static, Entry<B, N, P, V>>,
    body: N::Admission<V>,
    scope: PreparedScope<N::Error>,
    permits: P::Permits<N::Error>,
    closing: Closing<N, V>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct Entry<B: ScopeBody<N, P>, N: Description, P: Construction, V: Send + 'static> {
    body: B,
    admission: N::Admission<V>,
    scope: PreparedScope<N::Error>,
    permits: P::Permits<N::Error>,
    closing: Closing<N, V>,
    finish: Finish<N::Value, N::Error>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<B, N, P, V> Consume<(), Step> for Entry<B, N, P, V>
where
    B: ScopeBody<N, P>,
    N: Description,
    P: Construction,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let finish = self.closing.continuation(self.finish);
        if self.context.is_cancelled() {
            return finish.call(Exit::Interrupted).unwrap_or(Step::Done);
        }
        match catch_unwind(AssertUnwindSafe(|| {
            self.body.build(self.scope, self.permits)
        })) {
            Ok(body) => body.dispatch(self.admission, Continuation(finish)).0,
            Err(payload) => finish
                .call(Exit::Die(super::super::panic_defect(self.panic, payload)))
                .unwrap_or(Step::Done),
        }
    }
}
fn finalizers<N: Description, P: Construction>() -> Result<usize, MemoryError> {
    N::scope_finalizers()?
        .checked_add(P::count()?)
        .ok_or(MemoryError::ArithmeticOverflow)
}
impl<B, N, P> Description for Scoped<B, N, P>
where
    B: ScopeBody<N, P>,
    N: Description,
    P: Construction,
{
}
impl<B, N, P> Spec for Scoped<B, N, P>
where
    B: ScopeBody<N, P>,
    N: Description,
    P: Construction,
{
    type Value = N::Value;
    type Error = N::Error;
    type Env = N::Env;
    type Storage = Storage<N::Storage, N::Error>;
    type Admission<V: Send + 'static> = Admission<B, N, P, V>;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        let limits = PreparedScopeLimits {
            finalizers: finalizers::<N, P>()?,
            segments: 0,
        };
        let scopes = crate::effect_scope! {prepare_pool;memory:memory,metadata:region,scopes:capacity,limits:limits,callbacks:frames.try_clone()?,}?;
        let scopes = scopes.with_cleanup(
            cleanup
                .ok_or(MemoryError::UnsupportedRunContract)?
                .try_clone()?,
        );
        Ok(Storage {
            body: N::provision::<V>(memory, region, capacity, frames, cleanup)?,
            scopes,
        })
    }
    fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        N::storage_bytes::<V>(capacity, layout)?.checked_add(
            PreparedScopeLimits {
                finalizers: finalizers::<N, P>()?,
                segments: 0,
            }
            .pool_demand::<N::Error>(capacity)?
            .storage_payload,
        )
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        N::shape::<V>()?
            .add(Closing::<N, V>::shape(finalizers::<N, P>()?)?)?
            .add(Demand::cell::<Entry<B, N, P, V>>(false))?
            .add(Demand::panic_sites(1))?
            .add(output_origin_shape::<N::Env, V, N::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<N::Env>,
        context: &PreparedRunContext<V, N::Error>,
        _scope: &PreparedScope<N::Error>,
        _cleanup: &PreparedRunContext<V, N::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<N::Env>,
        context: &PreparedRunContext<V, N::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin = output_origin_reserve::<N::Env, V, N::Error>(frames, environment, context)?;
        let owner = storage.scopes.checkout()?;
        let owner = match &context.root.panic_pool {
            Some(pool) => owner.with_panic_diagnostics(pool.try_clone()?)?,
            None => owner,
        };
        let scope = owner.handle()?;
        let protected = context.protected()?;
        let permits = P::reserve(&scope)?;
        let admission = Admission {
            entry: frames.reserve()?,
            body: N::reserve_scoped(
                frames,
                &storage.body,
                environment,
                context,
                &scope,
                &protected,
            )?,
            scope,
            permits,
            closing: Closing::reserve(owner, frames, context, &protected)?,
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
        finish: Continuation<N::Value, N::Error>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            Entry {
                body: self.body,
                admission: admission.body,
                scope: admission.scope,
                permits: admission.permits,
                closing: admission.closing,
                finish: finish.0,
                context: admission.context,
                panic: admission.panic,
            },
        )))
    }
}
