//! Owned prepared configuration source and loading descriptions.
use crate::__private::prepared_graph::Description;
use crate::{
    ConfigError, ConfigPath, PreparedCodec, PreparedConfigStore, PreparedMemoryConfigSource,
    PreparedParseContextPool, SchemaIssue, SchemaIssueKind, SchemaPolicy, SharedPreparedJsonValue,
};
use std::sync::Arc;

/// A selected source whose owned load uses the existing prepared graph.
pub trait PreparedConfigSource<R: Send + Sync + 'static>: Send + Sync + 'static {
    /// The source's expected lookup failure.
    type SourceError: Send + 'static;
    /// Expected root error of the source description.
    type Error: Send + 'static;
    /// Describe one lookup. The returned description owns everything it needs.
    fn load(
        &self,
        path: ConfigPath,
    ) -> impl Description<Value = Option<SharedPreparedJsonValue>, Error = Self::Error, Env = R>
    + 'static
    + use<Self, R>;
}
/// Select the prepared source from the original decoding environment.
pub trait HasPreparedConfigSource: Send + Sync + Sized + 'static {
    /// The source selected by this environment.
    type Source: PreparedConfigSource<Self> + ?Sized;
    /// Borrow the source while constructing its owned description.
    fn prepared_config_source(&self) -> &Self::Source;
}
impl<R: Send + Sync + 'static> PreparedConfigSource<R> for PreparedMemoryConfigSource {
    type SourceError = crate::memory::MemoryError;
    type Error = ConfigError<Self::SourceError>;
    fn load(
        &self,
        path: ConfigPath,
    ) -> impl Description<
        Value = Option<SharedPreparedJsonValue>,
        Error = ConfigError<Self::SourceError>,
        Env = R,
    >
    + 'static
    + use<R> {
        let source = self.try_clone();
        // ConfigError retains prepared issue owners; boxing would allocate.
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: #[allow(clippy::result_large_err)] move || {
            let source = source.map_err(|error| ConfigError::Source { path: path.clone(), error })?;
            source.try_get(&path).map_err(|error| ConfigError::Source { path, error })
        }, }, }
    }
}
impl HasPreparedConfigSource for PreparedMemoryConfigSource {
    type Source = Self;
    fn prepared_config_source(&self) -> &Self {
        self
    }
}
/// Underlying typed source failure wrapped with its configuration path.
pub type PreparedConfigFailure<D> =
    ConfigError<<<D as HasPreparedConfigSource>::Source as PreparedConfigSource<D>>::SourceError>;
/// Expected root error selected by the environment's prepared source.
pub type PreparedConfigLoadError<D> =
    <<D as HasPreparedConfigSource>::Source as PreparedConfigSource<D>>::Error;

/// An owned synchronous store selected directly in one prepared error family.
pub struct PreparedConfigStoreSource<
    S: PreparedConfigStore,
    X,
    F = fn(ConfigError<<S as PreparedConfigStore>::Error>) -> X,
> {
    store: Arc<S>,
    map: Arc<F>,
    marker: std::marker::PhantomData<fn() -> X>,
}
impl<
    S: PreparedConfigStore,
    X: Send + 'static,
    F: Fn(ConfigError<S::Error>) -> X + Send + Sync + 'static,
> PreparedConfigStoreSource<S, X, F>
{
    /// Retain the actual store and one startup mapper control.
    pub fn new(store: Arc<S>, map: F) -> Self {
        Self {
            store,
            map: Arc::new(map),
            marker: std::marker::PhantomData,
        }
    }
}
impl<R, S, X, F> PreparedConfigSource<R> for PreparedConfigStoreSource<S, X, F>
where
    R: Send + Sync + 'static,
    S: PreparedConfigStore,
    X: Send + 'static,
    F: Fn(ConfigError<S::Error>) -> X + Send + Sync + 'static,
{
    type SourceError = S::Error;
    type Error = X;
    fn load(
        &self,
        path: ConfigPath,
    ) -> impl Description<Value = Option<SharedPreparedJsonValue>, Error = X, Env = R>
    + 'static
    + use<R, S, X, F> {
        let store = Arc::clone(&self.store);
        let map = Arc::clone(&self.map);
        crate::effect_prepare! { description; program: effect_try_sync! { once; body: move || {
            store.try_get(&path).map_err(|error| map(ConfigError::Source { path, error }))
        }, }, }
    }
}
impl<S, X, F> HasPreparedConfigSource for PreparedConfigStoreSource<S, X, F>
where
    S: PreparedConfigStore,
    X: Send + 'static,
    F: Fn(ConfigError<S::Error>) -> X + Send + Sync + 'static,
{
    type Source = Self;
    fn prepared_config_source(&self) -> &Self {
        self
    }
}
fn from_failure<D: HasPreparedConfigSource>(
    failure: PreparedConfigFailure<D>,
) -> PreparedConfigLoadError<D>
where
    PreparedConfigLoadError<D>: From<PreparedConfigFailure<D>>,
{
    failure.into()
}
/// An inert effectful configuration description with explicitly selected context backing.
pub struct PreparedEffectConfig<
    C,
    A: Send + 'static,
    D: HasPreparedConfigSource,
    E: Send + Sync + 'static,
    M = fn(PreparedConfigFailure<D>) -> PreparedConfigLoadError<D>,
> {
    path: ConfigPath,
    codec: Arc<C>,
    contexts: Arc<PreparedParseContextPool>,
    policy: SchemaPolicy,
    map: Arc<M>,
    marker: std::marker::PhantomData<fn() -> A>,
    environments: std::marker::PhantomData<fn() -> (D, E)>,
}
impl<C, A, D, E> PreparedEffectConfig<C, A, D, E>
where
    A: Send + 'static,
    D: HasPreparedConfigSource,
    E: Send + Sync + 'static,
    C: PreparedCodec<A, SharedPreparedJsonValue, D, E, Error = PreparedConfigLoadError<D>>,
    PreparedConfigLoadError<D>: From<PreparedConfigFailure<D>>,
{
    /// Select the standard From conversion before graph construction.
    pub fn new(
        path: ConfigPath,
        codec: Arc<C>,
        contexts: Arc<PreparedParseContextPool>,
        policy: SchemaPolicy,
    ) -> Self {
        Self::new_mapped(path, codec, contexts, policy, from_failure::<D>)
    }
}
impl<C, A, D, E, M> PreparedEffectConfig<C, A, D, E, M>
where
    A: Send + 'static,
    D: HasPreparedConfigSource,
    E: Send + Sync + 'static,
    C: PreparedCodec<A, SharedPreparedJsonValue, D, E, Error = PreparedConfigLoadError<D>>,
    M: Fn(PreparedConfigFailure<D>) -> PreparedConfigLoadError<D> + Send + Sync + 'static,
{
    /// Select the metadata-error mapper before graph construction.
    pub fn new_mapped(
        path: ConfigPath,
        codec: Arc<C>,
        contexts: Arc<PreparedParseContextPool>,
        policy: SchemaPolicy,
        map: M,
    ) -> Self {
        Self {
            path,
            codec,
            contexts,
            policy,
            map: Arc::new(map),
            marker: std::marker::PhantomData,
            environments: std::marker::PhantomData,
        }
    }
    /// Select an owned prepared default factory, retained at startup.
    #[must_use]
    pub fn with_default<F, N>(
        self,
        factory: F,
    ) -> PreparedEffectConfigWithDefault<C, A, D, E, F, N, M>
    where
        F: Fn() -> N + Send + Sync + 'static,
        N: Description<Value = A, Error = PreparedConfigLoadError<D>, Env = D>,
    {
        PreparedEffectConfigWithDefault {
            config: self,
            factory: Arc::new(factory),
            marker: std::marker::PhantomData,
        }
    }
    fn load_input(
        &self,
    ) -> impl Description<
        Value = (Option<SharedPreparedJsonValue>, crate::ParseContext),
        Error = PreparedConfigLoadError<D>,
        Env = D,
    > + use<C, A, D, E, M> {
        let source_path = self.path.clone();
        let context_path = self.path.clone();
        let contexts = Arc::clone(&self.contexts);
        let policy = self.policy;
        let map = Arc::clone(&self.map);
        // ConfigError retains prepared issue owners; boxing would allocate.
        crate::effect_prepare! { description; program: effect! {
            let context = effect_try_sync! { once; body: #[allow(clippy::result_large_err)] move || contexts.try_json_context(policy).map_err(|error| map(ConfigError::Invalid { path: context_path, issue: SchemaIssue::new(SchemaIssueKind::Memory(error)) })), }?;
            let lookup = service! { env: move |env: &D| env.prepared_config_source().load(source_path), }?;
            let value = lookup?;
            (value,context)
        }, }
    }
    /// Build a lazy load using the original environment's selected source.
    #[must_use]
    pub fn load(
        &self,
    ) -> impl Description<Value = A, Error = PreparedConfigLoadError<D>, Env = D> + use<C, A, D, E, M>
    {
        let input = self.load_input();
        let path = self.path.clone();
        let codec = Arc::clone(&self.codec);
        let map = Arc::clone(&self.map);
        // ConfigError retains prepared issue owners; boxing would allocate.
        crate::effect_prepare! { description; program: effect! {
            let (value,context) = input?;
            let (value,context) = effect_try_sync! { once; body: #[allow(clippy::result_large_err)] move || {
                let value = value.ok_or_else(|| map(ConfigError::Missing { path: path.clone() }))?;
                context.check_prepared_json(&value).map_err(|issue| map(ConfigError::Invalid { path, issue }))?;
                Ok((value,context))
            }, }?;
            let value = <C as PreparedCodec<A,SharedPreparedJsonValue,D,E>>::decode(codec,value,Some(context))?;
            value
        }, }
    }
}
/// A prepared configuration with a lazy, independent default factory.
pub struct PreparedEffectConfigWithDefault<
    C,
    A: Send + 'static,
    D: HasPreparedConfigSource,
    E: Send + Sync + 'static,
    F,
    N,
    M = fn(PreparedConfigFailure<D>) -> PreparedConfigLoadError<D>,
> {
    config: PreparedEffectConfig<C, A, D, E, M>,
    factory: Arc<F>,
    marker: std::marker::PhantomData<fn() -> N>,
}
impl<C, A, D, E, F, N, M> PreparedEffectConfigWithDefault<C, A, D, E, F, N, M>
where
    A: Send + 'static,
    D: HasPreparedConfigSource,
    E: Send + Sync + 'static,
    C: PreparedCodec<A, SharedPreparedJsonValue, D, E, Error = PreparedConfigLoadError<D>>,
    F: Fn() -> N + Send + Sync + 'static,
    M: Fn(PreparedConfigFailure<D>) -> PreparedConfigLoadError<D> + Send + Sync + 'static,
    N: Description<Value = A, Error = PreparedConfigLoadError<D>, Env = D>,
{
    /// Build the lazy load; absent values alone invoke the selected factory.
    #[must_use]
    pub fn load(
        &self,
    ) -> impl Description<Value = A, Error = PreparedConfigLoadError<D>, Env = D>
    + use<C, A, D, E, F, N, M> {
        let input = self.config.load_input();
        let path = self.config.path.clone();
        let codec = Arc::clone(&self.config.codec);
        let factory = Arc::clone(&self.factory);
        let map = Arc::clone(&self.config.map);
        // Both validation failures retain ConfigError issue backing without boxing.
        crate::effect_prepare! { description; program: effect! {
            let (value,context) = input?;
            match value {
                Some(value) => {
                    let (value,context) = effect_try_sync! { once; body: #[allow(clippy::result_large_err)] move || {
                        context.check_prepared_json(&value).map_err(|issue| map(ConfigError::Invalid { path, issue }))?;
                        Ok((value,context))
                    }, }?;
                    let value = <C as PreparedCodec<A,SharedPreparedJsonValue,D,E>>::decode(codec,value,Some(context))?;
                    value
                },
                None => {
                    let value = (factory)()?;
                    let value = effect_try_sync! { once; body: #[allow(clippy::result_large_err)] move || {
                        codec.validate_default_ref(&value,Some(&context))?;
                        Ok(value)
                    }, }?;
                    value
                },
            }
        }, }
    }
}
