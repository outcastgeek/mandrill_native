//! Provisioned lexical resource admission. This module does not run acquisition.
use super::*;
impl<A, F, C, B, N> Description for Using<A, F, C, B, N>
where
    A: Description,
    A::Value: Sync,
    N: Description<Error = A::Error, Env = A::Env>,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
    B: FnOnce(ResourceValue<A::Value>) -> N + Send + 'static,
{
}
impl<A, F, C, B, N> Spec for Using<A, F, C, B, N>
where
    A: Description,
    A::Value: Sync,
    N: Description<Error = A::Error, Env = A::Env>,
    C: Description<Value = (), Error = A::Error, Env = A::Env>,
    F: for<'a> FnOnce(A::Value, PrimaryOutcomeRef<'a, A::Error>) -> C + Send + 'static,
    B: FnOnce(ResourceValue<A::Value>) -> N + Send + 'static,
{
    type Value = N::Value;
    type Error = A::Error;
    type Env = A::Env;
    type Admission<V: Send + 'static> = Admission<A, F, C, B, N, V>;
    type Storage = Storage<A::Storage, N::Storage, C::Storage, A::Error>;
    fn provision<V: Send + 'static>(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: &FramePool<'static>,
        cleanup: Option<&crate::PreparedCleanupConfig<Self::Error>>,
    ) -> Result<Self::Storage, MemoryError> {
        let _ = cleanup;
        let scopes = crate::effect_scope! { prepare_pool; memory: memory,metadata: region,scopes: capacity,limits: PreparedScopeLimits { finalizers: 1,segments: 0 },callbacks: frames.try_clone()?, }?;
        let scopes = scopes.with_cleanup(
            cleanup
                .ok_or(MemoryError::UnsupportedRunContract)?
                .try_clone()?,
        );
        Ok(Storage {
            acquire: A::provision::<V>(memory, region, capacity, frames, cleanup)?,
            body: N::provision::<V>(memory, region, capacity, frames, cleanup)?,
            cleanup: C::provision::<V>(memory, region, capacity, frames, cleanup)?,
            scopes,
        })
    }
    fn storage_bytes<V: Send + 'static>(
        capacity: usize,
        layout: Layout,
    ) -> Result<Bytes, MemoryError> {
        A::storage_bytes::<V>(capacity, layout)?
            .checked_add(N::storage_bytes::<V>(capacity, layout)?)?
            .checked_add(C::storage_bytes::<V>(capacity, layout)?)?
            .checked_add(
                PreparedScopeLimits {
                    finalizers: 1,
                    segments: 0,
                }
                .pool_demand::<A::Error>(capacity)?
                .storage_payload,
            )
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> =
            (|| {
                let close = PreparedScopeClose::<(), A::Error, V>::demand(1)?;
                let close = Demand {
                    frame_layout: close.frame_layout,
                    frames_per_run: close.frames,
                    continuations: 0,
                    descriptions: 0,
                    origin_frames: 0,
                    environment_origin_frames: 0,
                    record_payload: Bytes::new(0),
                    cleanup_sites: close
                        .cleanup_sites
                        .checked_add(2)
                        .ok_or(MemoryError::ArithmeticOverflow)?,
                    panic_sites: close
                        .panic_sites
                        .checked_add(2)
                        .ok_or(MemoryError::ArithmeticOverflow)?,
                };
                A::shape::<V>()?
            .add(N::shape::<V>()?)?
            .add(C::shape::<V>()?)?
            .add(close)?
            .add(Environment::<A::Value>::demand::<V, A::Error>())?
            .add(Environment::<Mutex<Option<F>>>::demand::<V, A::Error>())?
            .add(Environment::<Mutex<Option<Exit<N::Value, N::Error>>>>::demand::<V, A::Error>())?
            .add(Demand::cell::<Entry<A, F, C, B, N, V>>(false))?
            .add(Demand::cell::<Acquired<A, B, N, V>>(true))?
            .add(Demand::cell::<StartBody<A, B, N, V>>(false))?
            .add(Demand::cell::<FinishBody<N, V>>(true))?
            .add(Demand::cell::<AfterClose<N, V>>(true))?
            .add(Demand::cell::<
                Settlement<Release<A, F, C, N, V>, ValueOwner<A::Value>>,
            >(true))?
            .add(Demand::cell::<Abandon<A::Value>>(true))?
            .add(Demand::cell::<ReleaseFinish<N, V>>(true))?
            .add(Demand::cell::<Invoke<(), A::Error>>(true))?
            .add(Demand::cell::<Invoke<N::Value, N::Error>>(true))?
            .add(Demand::cell::<Terminal<N, V>>(true))
            })();
        demand?.add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        let cleanup = C::scope_finalizers()?;
        A::scope_finalizers()?
            .checked_add(N::scope_finalizers()?)
            .and_then(|count| count.checked_add(cleanup))
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
            let owner = storage.scopes.checkout()?;
            let owner = match &context.root.panic_pool {
                Some(pool) => owner.with_panic_diagnostics(pool.try_clone()?)?,
                None => owner,
            };
            let scope = owner.handle()?;
            let protected = context.protected()?;
            let (value, view) = Environment::<A::Value>::reserve(frames, context)?;
            let factory = Slot::<F>::prepare(frames, context)?;
            let primary = Slot::<Exit<N::Value, N::Error>>::prepare(frames, context)?;
            let cleanup = C::reserve_scoped(
                frames,
                &storage.cleanup,
                environment,
                &protected,
                enclosing,
                enclosing_cleanup,
            )?;
            let operation =
                frames.reserve::<Settlement<Release<A, F, C, N, V>, ValueOwner<A::Value>>>()?;
            let abandon = frames.reserve::<Abandon<A::Value>>()?;
            let release_buffer = owner.reserve_cleanup_buffer()?;
            let cleanup_buffer = owner.reserve_cleanup_buffer()?;
            let release = Release::<A, F, C, N, V> {
                cleanup_buffer: release_buffer,
                value: value.try_clone()?,
                factory: factory.try_clone()?,
                primary: primary.try_clone()?,
                admission: cleanup,
                completed: frames.reserve()?,
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
            // Early registration has no native T and no release capture. Every
            // possible registration refusal remains pre-execution Rust MemoryError.
            scope.register_preflight(finalizer)?;
            let close = crate::effect_scope! { reserve_close; owner: owner,frames: frames,context: &protected, }?;
            Ok(Admission {
                entry: frames.reserve()?,
                controls: Controls {
                    acquire: A::reserve_scoped(
                        frames,
                        &storage.acquire,
                        environment,
                        context,
                        enclosing,
                        enclosing_cleanup,
                    )?,
                    body: N::reserve_scoped(
                        frames,
                        &storage.body,
                        environment,
                        context,
                        enclosing,
                        enclosing_cleanup,
                    )?,
                    factory,
                    primary,
                    value,
                    handle: ResourceValue { view },
                    acquired: frames.reserve()?,
                    start: frames.reserve()?,
                    failed: frames.reserve()?,
                    finished: frames.reserve()?,
                    after: frames.reserve()?,
                    terminal: frames.reserve()?,
                    close,
                    cleanup_buffer,
                    acquire_context: context.try_clone()?,
                    body_panic: context.reserve_panic()?,
                    context: context.try_clone()?,
                    marker: PhantomData,
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
            let (value, view) = Environment::<A::Value>::reserve(frames, context)?;
            let factory = Slot::<F>::prepare(frames, context)?;
            let primary = Slot::<Exit<N::Value, N::Error>>::prepare(frames, context)?;
            let cleanup = C::reserve(frames, &storage.cleanup, environment, &protected)?;
            let operation =
                frames.reserve::<Settlement<Release<A, F, C, N, V>, ValueOwner<A::Value>>>()?;
            let abandon = frames.reserve::<Abandon<A::Value>>()?;
            let release_buffer = owner.reserve_cleanup_buffer()?;
            let cleanup_buffer = owner.reserve_cleanup_buffer()?;
            let release = Release::<A, F, C, N, V> {
                cleanup_buffer: release_buffer,
                value: value.try_clone()?,
                factory: factory.try_clone()?,
                primary: primary.try_clone()?,
                admission: cleanup,
                completed: frames.reserve()?,
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
            // Early registration has no native T and no release capture. Every
            // possible registration refusal remains pre-execution Rust MemoryError.
            scope.register_preflight(finalizer)?;
            let close = crate::effect_scope! { reserve_close; owner: owner,frames: frames,context: &protected, }?;
            Ok(Admission {
                entry: frames.reserve()?,
                controls: Controls {
                    acquire: A::reserve(frames, &storage.acquire, environment, context)?,
                    body: N::reserve(frames, &storage.body, environment, context)?,
                    factory,
                    primary,
                    value,
                    handle: ResourceValue { view },
                    acquired: frames.reserve()?,
                    start: frames.reserve()?,
                    failed: frames.reserve()?,
                    finished: frames.reserve()?,
                    after: frames.reserve()?,
                    terminal: frames.reserve()?,
                    close,
                    cleanup_buffer,
                    acquire_context: context.try_clone()?,
                    body_panic: context.reserve_panic()?,
                    context: context.try_clone()?,
                    marker: PhantomData,
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
            Entry {
                description: self,
                finish: finish.0,
                controls: admission.controls,
            },
        )))
    }
}
