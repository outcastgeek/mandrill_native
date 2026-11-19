//! Concrete dynamic-description contracts; both exclusive branch shapes and all
//! producer/terminal captures are physically reserved before a producer runs.
use super::*;
pub struct Defer<F, N> {
    factory: F,
    marker: PhantomData<fn() -> N>,
}
pub fn defer<F, N>(factory: F) -> Defer<F, N>
where
    N: Description,
    F: FnOnce() -> N + Send + 'static,
{
    Defer {
        factory,
        marker: PhantomData,
    }
}
impl<F, N> Description for Defer<F, N>
where
    N: Description,
    F: FnOnce() -> N + Send + 'static,
{
}
pub struct DeferredAdmission<F, N: Description, V: Send + 'static>
where
    F: FnOnce() -> N + Send + 'static,
{
    entry: FrameReservation<'static, DeferredEntry<F, N, V>>,
    controls: DeferredControls<N, V>,
}
struct DeferredControls<N: Description, V: Send + 'static> {
    next: N::Admission<V>,
    failed: FrameReservation<'static, Invoke<N::Value, N::Error>>,
    context: PreparedRunContext<V, N::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct DeferredEntry<F, N: Description, V: Send + 'static>
where
    F: FnOnce() -> N + Send + 'static,
{
    factory: F,
    finish: Finish<N::Value, N::Error>,
    controls: DeferredControls<N, V>,
}
impl<F, N, V> Consume<(), Step> for DeferredEntry<F, N, V>
where
    N: Description,
    F: FnOnce() -> N + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let next = if self.controls.context.is_cancelled() {
            Err(Exit::Interrupted)
        } else {
            catch_unwind(AssertUnwindSafe(self.factory))
                .map_err(|panic| Exit::Die(super::super::panic_defect(self.controls.panic, panic)))
        };
        match next {
            Ok(description) => {
                description
                    .dispatch(self.controls.next, Continuation(self.finish))
                    .0
            }
            Err(outcome) => Step::More(Once::prepared(
                self.controls.failed,
                Invoke {
                    outcome,
                    finish: self.finish,
                },
            )),
        }
    }
}
impl<F, N> Spec for Defer<F, N>
where
    N: Description,
    F: FnOnce() -> N + Send + 'static,
{
    type Value = N::Value;
    type Error = N::Error;
    type Env = N::Env;
    type Admission<V: Send + 'static> = DeferredAdmission<F, N, V>;
    type Storage = N::Storage;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        N::provision::<V>(memory, region, capacity, frames, cleanup)
    }
    fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        N::storage_bytes::<V>(capacity, layout)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| {
            N::shape::<V>()?
                .add(Demand::cell::<DeferredEntry<F, N, V>>(false))?
                .add(Demand::cell::<Invoke<N::Value, N::Error>>(true))?
                .add(Demand::panic_sites(1))
        })();
        demand?.add(output_origin_shape::<Self::Env, V, Self::Error>())
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
        let admitted = (|| {
            Ok(DeferredAdmission {
                entry: frames.reserve()?,
                controls: DeferredControls {
                    next: N::reserve_scoped(
                        frames,
                        storage,
                        environment,
                        context,
                        enclosing,
                        enclosing_cleanup,
                    )?,
                    failed: frames.reserve()?,
                    context: context.try_clone()?,
                    panic: context.reserve_panic()?,
                },
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
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            Ok(DeferredAdmission {
                entry: frames.reserve()?,
                controls: DeferredControls {
                    next: N::reserve(frames, storage, environment, context)?,
                    failed: frames.reserve()?,
                    context: context.try_clone()?,
                    panic: context.reserve_panic()?,
                },
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
        finish: Continuation<Self::Value, Self::Error>,
    ) -> Transition {
        Transition(Step::More(Once::prepared(
            admission.entry,
            DeferredEntry {
                factory: self.factory,
                finish: finish.0,
                controls: admission.controls,
            },
        )))
    }
}
pub enum Branch<L, R> {
    Left(L),
    Right(R),
}
pub fn branch_left<L, R>(left: L) -> Branch<L, R> {
    Branch::Left(left)
}
pub fn branch_right<L, R>(right: R) -> Branch<L, R> {
    Branch::Right(right)
}
impl<L, R> Description for Branch<L, R>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
{
}
impl<L, R> Spec for Branch<L, R>
where
    L: Description,
    R: Description<Value = L::Value, Error = L::Error, Env = L::Env>,
{
    type Value = L::Value;
    type Error = L::Error;
    type Env = L::Env;
    type Admission<V: Send + 'static> = (L::Admission<V>, R::Admission<V>);
    type Storage = (L::Storage, R::Storage);
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        Ok((
            L::provision::<V>(memory, region, capacity, frames, cleanup)?,
            R::provision::<V>(memory, region, capacity, frames, cleanup)?,
        ))
    }
    fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        L::storage_bytes::<V>(capacity, layout)?.checked_add(R::storage_bytes::<V>(capacity, layout)?)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| L::shape::<V>()?.add(R::shape::<V>()?))();
        demand?.add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        L::scope_finalizers()?
            .checked_add(R::scope_finalizers()?)
            .ok_or(MemoryError::ArithmeticOverflow)
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
        let admitted = (|| {
            Ok((
                L::reserve_scoped(
                    frames,
                    &storage.0,
                    environment,
                    context,
                    enclosing,
                    enclosing_cleanup,
                )?,
                R::reserve_scoped(
                    frames,
                    &storage.1,
                    environment,
                    context,
                    enclosing,
                    enclosing_cleanup,
                )?,
            ))
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
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = (|| {
            Ok((
                L::reserve(frames, &storage.0, environment, context)?,
                R::reserve(frames, &storage.1, environment, context)?,
            ))
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
        finish: Continuation<Self::Value, Self::Error>,
    ) -> Transition {
        match self {
            Self::Left(description) => description.dispatch(admission.0, finish),
            Self::Right(description) => description.dispatch(admission.1, finish),
        }
    }
}
