//! Consuming checked companion of ResourceValue::access; no late observer clone.
use super::*;
pub struct Access<T, F, A, E, R>
where
    T: Send + Sync + 'static,
{
    value: ResourceValue<T>,
    read: F,
    marker: PhantomData<ReadyTypes<A, E, R>>,
}
impl<T: Send + Sync + 'static> ResourceValue<T> {
    /// Simultaneously borrow two checked resource views, including aliases of
    /// one backing. No reference escapes; lexical close still invalidates views
    /// before release and a later access reports ResourceScopeEnded.
    pub fn with2<U: Send + Sync + 'static, A>(
        &self,
        other: &ResourceValue<U>,
        read: impl FnOnce(&T, &U) -> A,
    ) -> Result<A, Defect> {
        self.view
            .with_pair(&other.view, read)
            .map_err(|_| Defect::from(crate::LibraryDefect::ResourceScopeEnded))
    }
    /// Consuming, concrete multi-borrow graph operation. Both input views and
    /// the callback are part of the compiler-derived leaf capture layout; no
    /// observer owner is acquired after business execution begins.
    pub fn access2<U, F, A, E, R>(
        self,
        other: ResourceValue<U>,
        read: F,
    ) -> impl Description<Value = A, Error = E, Env = R>
    where
        U: Send + Sync + 'static,
        F: FnOnce(&T, &U) -> A + Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        ready(move |_: &R| match self.with2(&other, read) {
            Ok(value) => Exit::Success(value),
            Err(defect) => Exit::Die(defect),
        })
    }
    pub fn access<F, A, E, R>(self, read: F) -> Access<T, F, A, E, R>
    where
        F: FnOnce(&T) -> A + Send + 'static,
        A: Send + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        Access {
            value: self,
            read,
            marker: PhantomData,
        }
    }
}
impl<T, F, A, E, R> Leaf for Access<T, F, A, E, R>
where
    T: Send + Sync + 'static,
    F: FnOnce(&T) -> A + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn invoke(self, _environment: &R) -> Exit<A, E> {
        match self.value.with(self.read) {
            Ok(value) => Exit::Success(value),
            Err(defect) => Exit::Die(defect),
        }
    }
}
impl<T, F, A, E, R> Description for Access<T, F, A, E, R>
where
    T: Send + Sync + 'static,
    F: FnOnce(&T) -> A + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
}
impl<T, F, A, E, R> Spec for Access<T, F, A, E, R>
where
    T: Send + Sync + 'static,
    F: FnOnce(&T) -> A + Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    type Value = A;
    type Error = E;
    type Env = R;
    type Admission<V: Send + 'static> = LeafAdmission<Self, V>;
    type Storage = ();
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
    fn storage_bytes<V: Send + 'static>(_capacity: usize, _layout: Layout) -> Result<Bytes, MemoryError> {
        Ok(Bytes::new(0))
    }
    fn shape<V: Send + 'static>() -> Result<Demand, MemoryError> {
        let demand: Result<Demand, MemoryError> = leaf_shape::<Self, V>();
        demand?.add(output_origin_shape::<Self::Env, V, Self::Error>())
    }
    fn scope_finalizers() -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn reserve_scoped<V: Send + 'static>(
        frames: &FramePool<'static>,
        storage: &Self::Storage,
        environment: &Environment<Self::Env>,
        context: &PreparedRunContext<V, Self::Error>,
        enclosing: &crate::PreparedScope<Self::Error>,
        enclosing_cleanup: &PreparedRunContext<V, Self::Error>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let _ = (enclosing, enclosing_cleanup);
        Self::reserve(frames, storage, environment, context)
    }
    fn reserve<V: Send + 'static>(
        frames: &FramePool<'static>,
        _storage: &(),
        environment: &Environment<R>,
        context: &PreparedRunContext<V, E>,
    ) -> Result<Self::Admission<V>, MemoryError> {
        let origin =
            output_origin_reserve::<Self::Env, V, Self::Error>(frames, environment, context)?;
        let admitted = leaf_reserve(frames, environment, context);
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
        finish: Continuation<A, E>,
    ) -> Transition {
        Transition(leaf_dispatch(self, admission, finish.0))
    }
}
