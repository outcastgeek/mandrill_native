//! Layer::scoped retains acquisition in the enclosing Layer Scope, not an
//! already-closed lexical Using wrapper. Canonical entry promises precede the
//! body; raw success commits under the phase gate or awaits pre-held refusal cleanup.
use super::*;
use crate::PrimaryOutcomeRef;
use crate::internal::runtime::prepared::graph::resource::{Abandon, Slot, ValueOwner};
use crate::internal::runtime_storage::Settlement;
use crate::memory::RuntimeFrame;
use crate::scope::{PreparedScopeEntry, Registered};
pub struct Scoped<A, F, C> {
    acquire: A,
    release: F,
    marker: PhantomData<fn() -> C>,
}
pub fn scoped<A, F, C>(acquire: A, release: F) -> Scoped<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
    Scoped {
        acquire,
        release,
        marker: PhantomData,
    }
}
pub struct Storage<AS, CS, E: Send + 'static> {
    acquire: AS,
    cleanup: CS,
    cleanup_config: crate::PreparedCleanupConfig<E>,
}
pub struct Execution<
    A: Description,
    F: Send + 'static,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    entry: FrameReservation<'static, Entry<A, F, C, V>>,
    ready: Ready<A, F, V>,
    marker: PhantomData<fn() -> C>,
}
pub struct Admission<
    A: Description,
    F: Send + 'static,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    promise: PreparedScopeEntry<A::Error>,
    execution: Execution<A, F, C, V>,
}
struct Ready<A: Description, F: Send + 'static, V: Send + 'static>
where
    A::Value: Sync,
{
    acquire: A::Admission<V>,
    factory: Slot<F>,
    value: Publisher<A::Value>,
    handle: ResourceValue<A::Value>,
    acquired: FrameReservation<'static, Acquired<A, V>>,
    terminal: FrameReservation<'static, Guarded<ResourceValue<A::Value>, A::Error, V>>,
    refused: FrameReservation<'static, Refused<A, V>>,
    finalizer: Registered<A::Error>,
    cleanup_buffer: crate::CleanupFailures<A::Error>,
    context: PreparedRunContext<V, A::Error>,
}
struct Entry<
    A: Description,
    F: Send + 'static,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    description: Scoped<A, F, C>,
    finish: Finish<ResourceValue<A::Value>, A::Error>,
    promise: PreparedScopeEntry<A::Error>,
    ready: Ready<A, F, V>,
}
struct Acquired<A: Description, V: Send + 'static>
where
    A::Value: Sync,
{
    value: Publisher<A::Value>,
    handle: ResourceValue<A::Value>,
    terminal: FrameReservation<'static, Guarded<ResourceValue<A::Value>, A::Error, V>>,
    refused: FrameReservation<'static, Refused<A, V>>,
    promise: PreparedScopeEntry<A::Error>,
    finalizer: Registered<A::Error>,
    finish: Finish<ResourceValue<A::Value>, A::Error>,
    context: PreparedRunContext<V, A::Error>,
    cleanup_buffer: crate::CleanupFailures<A::Error>,
}
impl<A: Description, V: Send + 'static> Consume<Exit<A::Value, A::Error>, Step> for Acquired<A, V>
where
    A::Value: Sync,
{
    fn consume(self, result: Exit<A::Value, A::Error>) -> Step {
        let outcome = match failure(result) {
            Ok(value) => {
                self.value.install(value);
                match self.promise.commit_prepared(self.finalizer) {
                    Ok(()) => self.context.publish(Exit::Success(self.handle)),
                    Err(finalizer) => {
                        return finalizer.execute_prepared_with(move |retained| {
                            Once::prepared(
                                self.refused,
                                Refused::<A, V> {
                                    handle: self.handle,
                                    terminal: self.terminal,
                                    finish: self.finish,
                                    context: self.context,
                                    cleanup_buffer: self.cleanup_buffer,
                                    _retained: retained,
                                },
                            )
                        });
                    }
                }
            }
            Err(outcome) => outcome,
        };
        Step::More(Once::prepared(
            self.terminal,
            Guarded {
                outcome,
                finish: self.finish,
                _context: self.context,
            },
        ))
    }
}
struct Refused<A: Description, V: Send + 'static>
where
    A::Value: Sync,
{
    handle: ResourceValue<A::Value>,
    terminal: FrameReservation<'static, Guarded<ResourceValue<A::Value>, A::Error, V>>,
    finish: Finish<ResourceValue<A::Value>, A::Error>,
    context: PreparedRunContext<V, A::Error>,
    cleanup_buffer: crate::CleanupFailures<A::Error>,
    _retained: Option<RuntimeFrame>,
}
impl<A: Description, V: Send + 'static> Consume<Exit<(), A::Error>, Step> for Refused<A, V>
where
    A::Value: Sync,
{
    fn consume(self, cleanup: Exit<(), A::Error>) -> Step {
        drop(self.handle);
        let mut accumulator = crate::PreparedCleanupAccumulator::from_reserved(
            Exit::Die(Defect::from(crate::LibraryDefect::ScopeRegistrationClosed)),
            self.cleanup_buffer,
        );
        accumulator.append_cleanup(cleanup);
        let outcome = accumulator.finish();
        Step::More(Once::prepared(
            self.terminal,
            Guarded {
                outcome,
                finish: self.finish,
                _context: self.context,
            },
        ))
    }
}
struct Release<
    A: Description,
    F: Send + 'static,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    V: Send + 'static,
> where
    A::Value: Sync,
{
    value: Publisher<A::Value>,
    factory: Slot<F>,
    cleanup: C::Admission<V>,
    failed: FrameReservation<'static, Guarded<(), A::Error, V>>,
    context: PreparedRunContext<V, A::Error>,
    panic: Option<crate::PreparedPanicReservation>,
}
impl<A, F, C, V> Consume<Finish<(), A::Error>, Step> for Release<A, F, C, V>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, finish: Finish<(), A::Error>) -> Step {
        let Some(value) = self.value.take() else {
            return Step::More(Once::prepared(
                self.failed,
                Guarded {
                    outcome: Exit::Success(()),
                    finish,
                    _context: self.context,
                },
            ));
        };
        let produced = match self.factory.take() {
            Ok(Some(factory)) => catch_unwind(AssertUnwindSafe(|| {
                factory(value, PrimaryOutcomeRef::Succeeded)
            }))
            .map_err(|panic| super::super::super::panic_defect(self.panic, panic)),
            Ok(None) => Err(Defect::from(crate::LibraryDefect::FutureFactoryConsumed)),
            Err(defect) => Err(defect),
        };
        match produced {
            Ok(cleanup) => cleanup.dispatch(self.cleanup, Continuation(finish)).0,
            Err(defect) => Step::More(Once::prepared(
                self.failed,
                Guarded {
                    outcome: Exit::Die(defect),
                    finish,
                    _context: self.context,
                },
            )),
        }
    }
}
impl<A, F, C, V> Consume<(), Step> for Entry<A, F, C, V>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
    V: Send + 'static,
{
    fn consume(self, (): ()) -> Step {
        let ready = self.ready;
        if ready.context.is_cancelled() {
            return Step::More(Once::prepared(
                ready.terminal,
                Guarded {
                    outcome: Exit::Interrupted,
                    finish: self.finish,
                    _context: ready.context,
                },
            ));
        }
        if let Err(defect) = ready.factory.put(self.description.release) {
            return Step::More(Once::prepared(
                ready.terminal,
                Guarded {
                    outcome: Exit::Die(defect),
                    finish: self.finish,
                    _context: ready.context,
                },
            ));
        }
        let acquired = Once::prepared(
            ready.acquired,
            Acquired::<A, V> {
                value: ready.value,
                handle: ready.handle,
                terminal: ready.terminal,
                refused: ready.refused,
                promise: self.promise,
                finalizer: ready.finalizer,
                finish: self.finish,
                context: ready.context,
                cleanup_buffer: ready.cleanup_buffer,
            },
        );
        self.description
            .acquire
            .dispatch(ready.acquire, Continuation(acquired))
            .0
    }
}
impl<A, F, C> LayerDescription for Scoped<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
}
impl<A, F, C> LayerSpec for Scoped<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
    type Input = A::Env;
    type Error = A::Error;
    type Output = ResourceValue<A::Value>;
    type Storage = Storage<A::Storage, C::Storage, A::Error>;
    type Admission<V: Send + 'static> = Admission<A, F, C, V>;
    fn finalizers() -> Result<usize, MemoryError> {
        Ok(1)
    }
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        Ok(Storage {
            acquire: A::provision::<V>(memory, region, capacity, frames, cleanup)?,
            cleanup: C::provision::<V>(memory, region, capacity, frames, cleanup)?,
            cleanup_config: cleanup
                .ok_or(MemoryError::UnsupportedRunContract)?
                .try_clone()?,
        })
    }
    fn storage_bytes<V: Send + 'static>(capacity: usize, layout: Layout) -> Result<Bytes, MemoryError> {
        A::storage_bytes::<V>(capacity, layout)?.checked_add(C::storage_bytes::<V>(capacity, layout)?)
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = (|| {
            A::shape::<V>()?
                .add(C::shape::<V>()?)?
                .add(Environment::<A::Value>::demand::<V, A::Error>())?
                .add(Environment::<std::sync::Mutex<Option<F>>>::demand::<
                    V,
                    A::Error,
                >())?
                .add(Demand::cell::<Entry<A, F, C, V>>(false))?
                .add(Demand::cell::<Acquired<A, V>>(true))?
                .add(Demand::cell::<Refused<A, V>>(true))?
                .add(Demand::cell::<Guarded<ResourceValue<A::Value>, A::Error, V>>(true))?
                .add(Demand::cell::<
                    Settlement<Release<A, F, C, V>, ValueOwner<A::Value>>,
                >(true))?
                .add(Demand::cell::<Abandon<A::Value>>(true))?
                .add(Demand::cell::<Guarded<(), A::Error, V>>(true))
        })();
        demand?
            .add(Demand::cleanup(1))?
            .add(Demand::panic_sites(1))?
            .add(output_origin_shape::<Self::Input, V, Self::Error>())
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
            let promise = scope.reserve_entry_preflight()?;
            Ok(Admission {
                promise,
                execution: Self::reserve_execution(
                    frames,
                    storage,
                    environment,
                    context,
                    protected,
                )?,
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
        Transition(self.execute_bound(admission.execution, admission.promise, finish.0))
    }
}

impl<A, F, C> Scoped<A, F, C>
where
    A: Description,
    A::Value: Sync,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
{
    pub(in crate::internal::runtime::prepared::graph) fn reserve_execution<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Storage<A::Storage, C::Storage, A::Error>,
        environment: &Environment<A::Env>,
        context: &PreparedRunContext<V, A::Error>,
        protected: &PreparedRunContext<V, A::Error>,
    ) -> Result<Execution<A, F, C, V>, MemoryError> {
        let (value, view) = Environment::<A::Value>::reserve(frames, context)?;
        let factory = Slot::<F>::prepare(frames, context)?;
        let operation =
            frames.reserve::<Settlement<Release<A, F, C, V>, ValueOwner<A::Value>>>()?;
        let abandon = frames.reserve::<Abandon<A::Value>>()?;
        let release = Release::<A, F, C, V> {
            value: value.try_clone()?,
            factory: factory.try_clone()?,
            cleanup: C::reserve(frames, &storage.cleanup, environment, protected)?,
            failed: frames.reserve()?,
            context: protected.try_clone()?,
            panic: protected.reserve_panic()?,
        };
        let finalizer = Registered::from_prepared_settlement(
            operation,
            Settlement {
                operation: release,
                guard: ValueOwner {
                    value: value.try_clone()?,
                },
            },
            abandon,
            Abandon {
                value: value.try_clone()?,
            },
        );
        Ok(Execution {
            entry: frames.reserve()?,
            ready: Ready {
                acquire: A::reserve(frames, &storage.acquire, environment, context)?,
                factory,
                value,
                handle: ResourceValue { view },
                acquired: frames.reserve()?,
                terminal: frames.reserve()?,
                refused: frames.reserve()?,
                cleanup_buffer: storage
                    .cleanup_config
                    .pool
                    .reserve(&storage.cleanup_config.frames)?,
                finalizer,
                context: context.try_clone()?,
            },
            marker: PhantomData,
        })
    }
    pub(in crate::internal::runtime::prepared::graph) fn execute_recipe<V: Send + 'static>(
        acquire: A,
        release: F,
        execution: Execution<A, F, C, V>,
        promise: PreparedScopeEntry<A::Error>,
        finish: Finish<ResourceValue<A::Value>, A::Error>,
    ) -> Step {
        Self {
            acquire,
            release,
            marker: PhantomData,
        }
        .execute_bound(execution, promise, finish)
    }
    pub(in crate::internal::runtime::prepared::graph) fn execute_bound<V: Send + 'static>(
        self,
        execution: Execution<A, F, C, V>,
        promise: PreparedScopeEntry<A::Error>,
        finish: Finish<ResourceValue<A::Value>, A::Error>,
    ) -> Step {
        Step::More(Once::prepared(
            execution.entry,
            Entry {
                description: self,
                finish,
                promise,
                ready: execution.ready,
            },
        ))
    }
}
