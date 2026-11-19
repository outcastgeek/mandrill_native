//! Additive finite construction contract. P is independent of executed D.
use super::*;
use crate::internal::runtime::prepared::graph::scope_binding::Construction;
mod factory_sealed {
    pub trait Sealed {}
}
pub trait Factory<P: Construction, D: Description>:
    factory_sealed::Sealed + Send + 'static
{
    fn invoke(
        self,
        scope: PreparedScope<D::Error>,
        permits: P::Permits<D::Error>,
    ) -> Result<D, D::Error>;
}
pub struct Infallible<B>(B);
pub struct Checked<B>(B);
impl<B> factory_sealed::Sealed for Infallible<B> {}
impl<B> factory_sealed::Sealed for Checked<B> {}
impl<B, P, D> Factory<P, D> for Infallible<B>
where
    P: Construction,
    D: Description,
    B: FnOnce(PreparedScope<D::Error>, P::Permits<D::Error>) -> D + Send + 'static,
{
    fn invoke(
        self,
        scope: PreparedScope<D::Error>,
        permits: P::Permits<D::Error>,
    ) -> Result<D, D::Error> {
        Ok((self.0)(scope, permits))
    }
}
impl<B, P, D> Factory<P, D> for Checked<B>
where
    P: Construction,
    D: Description,
    B: FnOnce(PreparedScope<D::Error>, P::Permits<D::Error>) -> Result<D, D::Error>
        + Send
        + 'static,
{
    fn invoke(
        self,
        scope: PreparedScope<D::Error>,
        permits: P::Permits<D::Error>,
    ) -> Result<D, D::Error> {
        (self.0)(scope, permits)
    }
}
pub struct Construct<B, D, P> {
    build: B,
    _construction: P,
    marker: PhantomData<fn() -> D>,
}
pub fn construct<B, D, P>(build: B, construction: P) -> Construct<Infallible<B>, D, P>
where
    P: Construction,
    D: Description,
    B: FnOnce(PreparedScope<D::Error>, P::Permits<D::Error>) -> D + Send + 'static,
{
    Construct {
        build: Infallible(build),
        _construction: construction,
        marker: PhantomData,
    }
}
pub fn try_construct<B, D, P>(build: B, construction: P) -> Construct<Checked<B>, D, P>
where
    P: Construction,
    D: Description,
    B: FnOnce(PreparedScope<D::Error>, P::Permits<D::Error>) -> Result<D, D::Error>
        + Send
        + 'static,
{
    Construct {
        build: Checked(build),
        _construction: construction,
        marker: PhantomData,
    }
}
pub struct Admission<B: Factory<P, D>, D: Description, P: Construction, V: Send + 'static> {
    entry: FrameReservation<'static, Entry<B, D, P, V>>,
    permits: P::Permits<D::Error>,
    admission: D::Admission<V>,
    scope: PreparedScope<D::Error>,
    failed: FrameReservation<'static, Guarded<D::Value, D::Error, V>>,
    context: PreparedRunContext<V, D::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct Entry<B, D: Description, P: Construction, V: Send + 'static> {
    build: B,
    permits: P::Permits<D::Error>,
    admission: D::Admission<V>,
    scope: PreparedScope<D::Error>,
    failed: FrameReservation<'static, Guarded<D::Value, D::Error, V>>,
    finish: Finish<D::Value, D::Error>,
    context: PreparedRunContext<V, D::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<B, D, P, V> Consume<(), Step> for Entry<B, D, P, V>
where
    P: Construction,
    D: Description,
    B: Factory<P, D>,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let produced = if self.context.is_cancelled() {
            Err(Exit::Interrupted)
        } else {
            match catch_unwind(AssertUnwindSafe(|| {
                self.build.invoke(self.scope, self.permits)
            })) {
                Ok(Ok(description)) => Ok(description),
                Ok(Err(error)) => Err(Exit::Fail(error)),
                Err(panic) => Err(Exit::Die(super::super::super::panic_defect(
                    self.panic, panic,
                ))),
            }
        };
        match produced {
            Ok(description) => {
                description
                    .dispatch(self.admission, Continuation(self.finish))
                    .0
            }
            Err(outcome) => Step::More(Once::prepared(
                self.failed,
                Guarded {
                    outcome,
                    finish: self.finish,
                    _context: self.context,
                },
            )),
        }
    }
}
impl<B, D, P> LayerDescription for Construct<B, D, P>
where
    P: Construction,
    D: Description,
    B: Factory<P, D>,
{
}
impl<B, D, P> LayerSpec for Construct<B, D, P>
where
    P: Construction,
    D: Description,
    B: Factory<P, D>,
{
    type Input = D::Env;
    type Error = D::Error;
    type Output = D::Value;
    type Storage = D::Storage;
    type Admission<V: Send + 'static> = Admission<B, D, P, V>;
    fn finalizers() -> Result<usize, MemoryError> {
        P::count()?
            .checked_add(D::scope_finalizers()?)
            .ok_or(MemoryError::ArithmeticOverflow)
    }
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        D::provision::<V>(memory, region, capacity, frames, cleanup)
    }
    fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        D::storage_bytes::<V>(capacity, layout)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| {
            D::shape::<V>()?
                .add(Demand::cell::<Entry<B, D, P, V>>(false))?
                .add(Demand::cell::<Guarded<D::Value, D::Error, V>>(true))?
                .add(Demand::panic_sites(1))
        })();
        demand?.add(output_origin_shape::<Self::Input, V, Self::Error>())
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Input>,
        context: &PreparedRunContext<V, Self::Error>,
        scope: &PreparedScope<Self::Error>,
        protected: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Input, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            let permits = P::reserve(scope)?;
            let admission =
                D::reserve_scoped(frames, storage, environment, context, scope, protected)?;
            Ok(Admission {
                entry: frames.reserve()?,
                permits,
                admission,
                scope: scope.try_clone()?,
                failed: frames.reserve()?,
                context: context.try_clone()?,
                panic: context.reserve_panic()?,
            })
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
    fn dispatch<V: Send + 'static>(
        self,
        admission: Self::Admission<V>,
        finish: Continuation<Self::Output, Self::Error>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            Entry {
                build: self.build,
                permits: admission.permits,
                admission: admission.admission,
                scope: admission.scope,
                failed: admission.failed,
                finish: finish.0,
                context: admission.context,
                panic: admission.panic,
            },
        )))
    }
}
