//! Typed existing Layer grammar over one canonical resource Scope.
use super::environment::Publisher;
use super::resource::{Closing, ResourceValue};
use super::*;
use crate::{PreparedScope, PreparedScopeLimits, PreparedScopePool};
mod composition;
mod construct;
pub use construct::{Checked, Construct, Infallible, construct, try_construct};
mod scoped;
pub use composition::{Merge, Provide, merge, provide};
pub(super) use scoped::Execution as ScopedExecution;
pub use scoped::{Scoped, scoped};
mod sealed {
    use super::*;
    pub trait LayerSpec: Send + Sized + 'static {
        type Input: Send + Sync + 'static;
        type Error: Send + 'static;
        type Output: Send + 'static;
        type Storage: Send + Sync + 'static;
        type Admission<V: Send + 'static>: Send + 'static;
        fn finalizers() -> Result<usize, MemoryError>;
        fn provision<V: Send + 'static>(
            memory: &PreparedMemory<'static>,
            region: RegionId,
            capacity: usize,
            frames: &FramePool<'static>,
            cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
        ) -> Result<Self::Storage, MemoryError>;
        fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError>;
        fn shape<V: Send + 'static>() -> Result<Demand, MemoryError>;
        fn reserve<V: Send + 'static>(
            frames: &FramePool<'static>,
            storage: &Self::Storage,
            environment: &Environment<Self::Input>,
            context: &PreparedRunContext<V, Self::Error>,
            scope: &PreparedScope<Self::Error>,
            protected: &PreparedRunContext<V, Self::Error>,
        ) -> Result<Self::Admission<V>, MemoryError>;
        fn dispatch<V: Send + 'static>(
            self,
            admission: Self::Admission<V>,
            finish: Continuation<Self::Output, Self::Error>,
        ) -> Transition;
    }
}
pub(super) use sealed::LayerSpec;
pub trait LayerDescription: LayerSpec {}
fn lower<L: LayerDescription, V: Send + 'static>(
    layer: L,
    admission: L::Admission<V>,
    finish: Finish<L::Output, L::Error>,
) -> Step {
    layer.dispatch(admission, Continuation(finish)).0
}
struct Guarded<A: Send + 'static, E: Send + 'static, V: Send + 'static> {
    outcome: Exit<A, E>,
    finish: Finish<A, E>,
    _context: PreparedRunContext<V, E>,
}
impl<A: Send + 'static, E: Send + 'static, V: Send + 'static> Consume<(), Step>
    for Guarded<A, E, V>
{
    fn consume(self, (): ()) -> Step {
        self.finish.call(self.outcome).unwrap_or(Step::Done)
    }
}
fn failure<A, B, E>(value: Exit<A, E>) -> Result<A, Exit<B, E>> {
    match value {
        Exit::Success(value) => Ok(value),
        Exit::Fail(error) => Err(Exit::Fail(error)),
        Exit::Die(defect) => Err(Exit::Die(defect)),
        Exit::Interrupted => Err(Exit::Interrupted),
        Exit::FinalizationFailed(details) => Err(Exit::FinalizationFailed(details)),
    }
}
pub struct Build<B, D> {
    build: B,
    marker: PhantomData<fn() -> D>,
}
pub fn build<B, D>(build: B) -> Build<B, D>
where
    D: Description,
    B: FnOnce(PreparedScope<D::Error>) -> D + Send + 'static,
{
    Build {
        build,
        marker: PhantomData,
    }
}
pub struct BuildAdmission<B: Send + 'static, D: Description, V: Send + 'static> {
    entry: FrameReservation<'static, BuildEntry<B, D, V>>,
    admission: D::Admission<V>,
    scope: PreparedScope<D::Error>,
    failed: FrameReservation<'static, Guarded<D::Value, D::Error, V>>,
    context: PreparedRunContext<V, D::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
struct BuildEntry<B, D: Description, V: Send + 'static> {
    build: B,
    admission: D::Admission<V>,
    scope: PreparedScope<D::Error>,
    failed: FrameReservation<'static, Guarded<D::Value, D::Error, V>>,
    finish: Finish<D::Value, D::Error>,
    context: PreparedRunContext<V, D::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<B, D, V> Consume<(), Step> for BuildEntry<B, D, V>
where
    D: Description,
    B: FnOnce(PreparedScope<D::Error>) -> D + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let produced = if self.context.is_cancelled() {
            Err(Exit::Interrupted)
        } else {
            catch_unwind(AssertUnwindSafe(|| (self.build)(self.scope)))
                .map_err(|panic| Exit::Die(super::super::panic_defect(self.panic, panic)))
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
impl<B, D> LayerDescription for Build<B, D>
where
    D: Description,
    B: FnOnce(PreparedScope<D::Error>) -> D + Send + 'static,
{
}
impl<B, D> LayerSpec for Build<B, D>
where
    D: Description,
    B: FnOnce(PreparedScope<D::Error>) -> D + Send + 'static,
{
    type Input = D::Env;
    type Error = D::Error;
    type Output = D::Value;
    type Storage = D::Storage;
    type Admission<V: Send + 'static> = BuildAdmission<B, D, V>;
    fn finalizers() -> Result<usize, MemoryError> {
        D::scope_finalizers()
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
                .add(Demand::cell::<BuildEntry<B, D, V>>(false))?
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
            Ok(BuildAdmission {
                entry: frames.reserve()?,
                admission: D::reserve_scoped(
                    frames,
                    storage,
                    environment,
                    context,
                    scope,
                    protected,
                )?,
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
            BuildEntry {
                build: self.build,
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
pub struct WithLayer<L, D> {
    layer: L,
    program: D,
}
pub fn with_layer<L, D>(layer: L, program: D) -> WithLayer<L, D>
where
    L: LayerDescription,
    L::Output: Sync,
    D: Description<Env = L::Output, Error = L::Error>,
{
    WithLayer { layer, program }
}
pub struct Storage<LS, DS, E: Send + 'static> {
    layer: LS,
    program: DS,
    scopes: PreparedScopePool<E>,
}
pub struct Admission<
    L: LayerDescription,
    D: Description<Env = L::Output, Error = L::Error>,
    V: Send + 'static,
> where
    L::Output: Sync,
{
    entry: FrameReservation<'static, Entry<L, D, V>>,
    ready: Ready<L, D, V>,
}
struct Ready<
    L: LayerDescription,
    D: Description<Env = L::Output, Error = L::Error>,
    V: Send + 'static,
> where
    L::Output: Sync,
{
    layer: L::Admission<V>,
    program: D::Admission<V>,
    publisher: Publisher<L::Output>,
    built: FrameReservation<'static, Built<L, D, V>>,
    failed: FrameReservation<'static, Guarded<D::Value, D::Error, V>>,
    closing: Closing<D, V>,
    context: PreparedRunContext<V, D::Error>,
}
struct Entry<
    L: LayerDescription,
    D: Description<Env = L::Output, Error = L::Error>,
    V: Send + 'static,
> where
    L::Output: Sync,
{
    description: WithLayer<L, D>,
    finish: Finish<D::Value, D::Error>,
    admission: Ready<L, D, V>,
}
struct Built<
    L: LayerDescription,
    D: Description<Env = L::Output, Error = L::Error>,
    V: Send + 'static,
> where
    L::Output: Sync,
{
    program: D,
    admission: D::Admission<V>,
    publisher: Publisher<L::Output>,
    failed: FrameReservation<'static, Guarded<D::Value, D::Error, V>>,
    finish: Finish<D::Value, D::Error>,
    context: PreparedRunContext<V, D::Error>,
}
impl<L, D, V> Consume<(), Step> for Entry<L, D, V>
where
    L: LayerDescription,
    L::Output: Sync,
    D: Description<Env = L::Output, Error = L::Error>,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let admission = self.admission;
        let finish = admission.closing.continuation(self.finish);
        let built = Once::prepared(
            admission.built,
            Built::<L, D, V> {
                program: self.description.program,
                admission: admission.program,
                publisher: admission.publisher,
                failed: admission.failed,
                finish,
                context: admission.context,
            },
        );
        lower(self.description.layer, admission.layer, built)
    }
}
impl<L, D, V> Consume<Exit<L::Output, L::Error>, Step> for Built<L, D, V>
where
    L: LayerDescription,
    L::Output: Sync,
    D: Description<Env = L::Output, Error = L::Error>,
    V: Send + 'static,
{
    fn consume(self, result: Exit<L::Output, L::Error>) -> Step {
        match failure(result) {
            Ok(output) => {
                if self.context.is_cancelled() {
                    return Step::More(Once::prepared(
                        self.failed,
                        Guarded {
                            outcome: Exit::Interrupted,
                            finish: self.finish,
                            _context: self.context,
                        },
                    ));
                }
                self.publisher.install(output);
                self.program
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
impl<L, D> Description for WithLayer<L, D>
where
    L: LayerDescription,
    L::Output: Sync,
    D: Description<Env = L::Output, Error = L::Error>,
{
}
impl<L, D> Spec for WithLayer<L, D>
where
    L: LayerDescription,
    L::Output: Sync,
    D: Description<Env = L::Output, Error = L::Error>,
{
    type Value = D::Value;
    type Error = D::Error;
    type Env = L::Input;
    type Storage = Storage<L::Storage, D::Storage, D::Error>;
    type Admission<V: Send + 'static> = Admission<L, D, V>;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        let limits = PreparedScopeLimits {
            finalizers: L::finalizers()?,
            segments: 0,
        };
        let scopes = crate::effect_scope! { prepare_pool; memory: memory,metadata: region,scopes: capacity,limits: limits,callbacks: frames.try_clone()?, }?;
        let scopes = scopes.with_cleanup(
            cleanup
                .ok_or(MemoryError::UnsupportedRunContract)?
                .try_clone()?,
        );
        Ok(Storage {
            layer: L::provision::<V>(memory, region, capacity, frames, cleanup)?,
            program: D::provision::<V>(memory, region, capacity, frames, cleanup)?,
            scopes,
        })
    }
    fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        L::storage_bytes::<V>(capacity, layout)?
            .checked_add(D::storage_bytes::<V>(capacity, layout)?)?
            .checked_add(
                PreparedScopeLimits {
                    finalizers: L::finalizers()?,
                    segments: 0,
                }
                .pool_demand::<D::Error>(capacity)?
                .storage_payload,
            )
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| {
            L::shape::<V>()?
                .add(D::shape::<V>()?)?
                .add(Environment::<L::Output>::demand::<V, D::Error>())?
                .add(Closing::<D, V>::shape(L::finalizers()?)?)?
                .add(Demand::cell::<Entry<L, D, V>>(false))?
                .add(Demand::cell::<Built<L, D, V>>(true))?
                .add(Demand::cell::<Guarded<D::Value, D::Error, V>>(true))
        })();
        demand?.add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        D::scope_finalizers()
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
            let owner = storage.scopes.checkout()?;
            let owner = match &context.root.panic_pool {
                Some(pool) => owner.with_panic_diagnostics(pool.try_clone()?)?,
                None => owner,
            };
            let scope = owner.handle()?;
            let protected = context.protected()?;
            let (publisher, projected) = Environment::<L::Output>::reserve(frames, context)?;
            let layer = L::reserve(
                frames,
                &storage.layer,
                environment,
                context,
                &scope,
                &protected,
            )?;
            let program = D::reserve_scoped(
                frames,
                &storage.program,
                &projected,
                context,
                enclosing,
                enclosing_cleanup,
            )?;
            let closing = Closing::reserve(owner, frames, context, &protected)?;
            Ok(Admission {
                entry: frames.reserve()?,
                ready: Ready {
                    layer,
                    program,
                    publisher,
                    built: frames.reserve()?,
                    failed: frames.reserve()?,
                    closing,
                    context: context.try_clone()?,
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
            let owner = storage.scopes.checkout()?;
            let owner = match &context.root.panic_pool {
                Some(pool) => owner.with_panic_diagnostics(pool.try_clone()?)?,
                None => owner,
            };
            let scope = owner.handle()?;
            let protected = context.protected()?;
            let (publisher, projected) = Environment::<L::Output>::reserve(frames, context)?;
            let layer = L::reserve(
                frames,
                &storage.layer,
                environment,
                context,
                &scope,
                &protected,
            )?;
            let program = D::reserve(frames, &storage.program, &projected, context)?;
            let closing = Closing::reserve(owner, frames, context, &protected)?;
            Ok(Admission {
                entry: frames.reserve()?,
                ready: Ready {
                    layer,
                    program,
                    publisher,
                    built: frames.reserve()?,
                    failed: frames.reserve()?,
                    closing,
                    context: context.try_clone()?,
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
        // Move the pre-held entry separately; no post-factory reservation.
        Transition(Step::More(Once::prepared(
            admission.entry,
            Entry {
                description: self,
                admission: admission.ready,
                finish: finish.0,
            },
        )))
    }
}
