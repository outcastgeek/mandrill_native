use crate::internal::runtime;
use crate::{Defect, Effect, Exit, JsonValue, PureCodec, SchemaIssue};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, RwLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigPathError {
    EmptyPath,
    EmptySegment,
}

impl std::fmt::Display for ConfigPathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for ConfigPathError {}

/// A typed path assembled from consumer-owned segments. No delimiter parsing
/// or environment-variable naming convention is built into core.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigPath(Arc<[Arc<str>]>);

impl ConfigPath {
    pub fn new(segments: impl IntoIterator<Item = Arc<str>>) -> Result<Self, ConfigPathError> {
        let segments: Vec<_> = segments.into_iter().collect();
        if segments.is_empty() {
            return Err(ConfigPathError::EmptyPath);
        }
        if segments.iter().any(|segment| segment.trim().is_empty()) {
            return Err(ConfigPathError::EmptySegment);
        }
        Ok(Self(segments.into()))
    }

    #[must_use]
    pub fn segments(&self) -> &[Arc<str>] {
        &self.0
    }
}

/// Absence, invalid encoded data and source failure are separate outcomes.
/// Debug/Display intentionally omit raw values and provider error text;
/// callers retain typed access to the underlying issue or error.
pub enum ConfigError<SourceError> {
    Missing {
        path: ConfigPath,
    },
    Invalid {
        path: ConfigPath,
        issue: SchemaIssue,
    },
    Source {
        path: ConfigPath,
        error: SourceError,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigErrorKind {
    Missing,
    Invalid,
    Source,
}

impl<SourceError> ConfigError<SourceError> {
    #[must_use]
    pub const fn kind(&self) -> ConfigErrorKind {
        match self {
            Self::Missing { .. } => ConfigErrorKind::Missing,
            Self::Invalid { .. } => ConfigErrorKind::Invalid,
            Self::Source { .. } => ConfigErrorKind::Source,
        }
    }

    #[must_use]
    pub const fn path(&self) -> &ConfigPath {
        match self {
            Self::Missing { path } | Self::Invalid { path, .. } | Self::Source { path, .. } => path,
        }
    }

    #[must_use]
    pub const fn issue(&self) -> Option<&SchemaIssue> {
        match self {
            Self::Invalid { issue, .. } => Some(issue),
            Self::Missing { .. } | Self::Source { .. } => None,
        }
    }

    #[must_use]
    pub const fn source_error(&self) -> Option<&SourceError> {
        match self {
            Self::Source { error, .. } => Some(error),
            Self::Missing { .. } | Self::Invalid { .. } => None,
        }
    }
}

impl<SourceError> std::fmt::Debug for ConfigError<SourceError> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.kind(), formatter)
    }
}

impl<SourceError> std::fmt::Display for ConfigError<SourceError> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.kind(), formatter)
    }
}

impl<SourceError: Send + Sync + 'static> std::error::Error for ConfigError<SourceError> {}

/// A source may perform effectful provider work. Its handle is obtained only
/// through R; this contract does not prescribe files, variables or services.
pub trait ConfigSource: Send + Sync + 'static {
    type Error: Send + 'static;

    fn load(&self, path: ConfigPath) -> Effect<Option<Arc<JsonValue>>, Self::Error>;
}

pub trait HasConfigSource {
    type Source: ConfigSource + ?Sized;

    fn config_source(&self) -> &Self::Source;
}

type DefaultValue<A> = Arc<dyn Fn() -> A + Send + Sync>;

/// Pure schema-backed configuration. Definitions are inert; every load reads
/// the current R-provided source, then decodes once. A default applies only to
/// a missing value, never to malformed data or source failure.
pub struct Config<A> {
    path: ConfigPath,
    schema: PureCodec<A, JsonValue>,
    default: Option<DefaultValue<A>>,
}

impl<A> Clone for Config<A> {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            schema: self.schema.clone(),
            default: self.default.as_ref().map(Arc::clone),
        }
    }
}

impl<A: Send + 'static> Config<A> {
    #[must_use]
    pub fn new(path: ConfigPath, schema: PureCodec<A, JsonValue>) -> Self {
        Self {
            path,
            schema,
            default: None,
        }
    }

    #[must_use]
    pub fn with_default(mut self, make: impl Fn() -> A + Send + Sync + 'static) -> Self {
        self.default = Some(Arc::new(make));
        self
    }

    #[must_use]
    pub const fn path(&self) -> &ConfigPath {
        &self.path
    }

    pub fn load<R>(&self) -> Effect<A, ConfigError<<R::Source as ConfigSource>::Error>, R>
    where
        R: HasConfigSource + Send + Sync + 'static,
    {
        let config = self.clone();
        Effect::new(move |env: Arc<R>, context, finish| {
            let source = catch_unwind(AssertUnwindSafe(|| {
                env.config_source().load(config.path.clone())
            }));
            let source = match source {
                Ok(source) => source,
                Err(panic) => return runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
            };
            let config = config.clone();
            source.execute(
                Arc::new(()),
                context,
                runtime::continuation(
                    move |outcome: Exit<
                        Option<Arc<JsonValue>>,
                        <R::Source as ConfigSource>::Error,
                    >| {
                        let converted = catch_unwind(AssertUnwindSafe(|| {
                            let outcome = outcome.map_error(|error| ConfigError::Source {
                                path: config.path.clone(),
                                error,
                            });
                            match outcome.into_failure::<A>() {
                                Ok(Some(encoded)) => match config.schema.decode_ref(&encoded) {
                                    Ok(value) => Exit::Success(value),
                                    Err(issue) => Exit::Fail(ConfigError::Invalid {
                                        path: config.path.clone(),
                                        issue,
                                    }),
                                },
                                Ok(None) => match &config.default {
                                    Some(make) => Exit::Success(make()),
                                    None => Exit::Fail(ConfigError::Missing {
                                        path: config.path.clone(),
                                    }),
                                },
                                Err(failure) => failure,
                            }
                        }))
                        .unwrap_or_else(|panic| Exit::Die(Defect::from_panic(panic)));
                        runtime::finish(finish, converted)
                    },
                ),
            )
        })
    }
}

/// Volatile source for composition and deterministic applications. Returned
/// encoded values are shared; reads never clone an entire configuration tree.
#[derive(Clone, Default)]
pub struct MemoryConfigSource {
    values: Arc<RwLock<BTreeMap<ConfigPath, Arc<JsonValue>>>>,
}

impl MemoryConfigSource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, path: ConfigPath, value: JsonValue) -> Option<Arc<JsonValue>> {
        self.values
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(path, Arc::new(value))
    }

    pub fn remove(&self, path: &ConfigPath) -> Option<Arc<JsonValue>> {
        self.values
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(path)
    }
}

impl ConfigSource for MemoryConfigSource {
    type Error = std::convert::Infallible;

    fn load(&self, path: ConfigPath) -> Effect<Option<Arc<JsonValue>>, Self::Error> {
        let values = Arc::clone(&self.values);
        Effect::sync(move || {
            values
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&path)
                .map(Arc::clone)
        })
    }
}

impl HasConfigSource for MemoryConfigSource {
    type Source = Self;

    fn config_source(&self) -> &Self::Source {
        self
    }
}
