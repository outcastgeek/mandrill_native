//! Schema-backed configuration lookup and typed source failures.

use crate::internal::runtime;
use crate::{Defect, Effect, Exit, JsonValue, PureCodec, SchemaIssue};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, RwLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// An invalid segment or structure in a configuration path.
pub enum ConfigPathError {
    /// The required path is empty.
    EmptyPath,
    /// The required segment is empty.
    EmptySegment,
}

impl std::fmt::Display for ConfigPathError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
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
    /// Construct a `ConfigPath` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `segments` (`impl IntoIterator<Item = Arc<str>>`): The segments supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, ConfigPathError>`.
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
    /// Borrow the ordered segments of this path or identity.
    ///
    /// # Returns
    /// Type: `&[Arc<str>]`.
    pub fn segments(&self) -> &[Arc<str>] {
        &self.0
    }
}

/// Absence, invalid encoded data and source failure are separate outcomes.
/// Debug/Display intentionally omit raw values and provider error text;
/// callers retain typed access to the underlying issue or error.
///
/// # Type parameters
/// - `SourceError`: Error returned by the underlying source.
pub enum ConfigError<SourceError> {
    /// No matching value exists.
    Missing {
        /// Path to the field or value involved in this operation. Type: `ConfigPath`.
        path: ConfigPath,
    },
    /// The  failed validation.
    Invalid {
        /// Path to the field or value involved in this operation. Type: `ConfigPath`.
        path: ConfigPath,
        /// Structured validation issue explaining this failure. Type: `SchemaIssue`.
        issue: SchemaIssue,
    },
    /// The underlying source failed before the operation could settle.
    Source {
        /// Path to the field or value involved in this operation. Type: `ConfigPath`.
        path: ConfigPath,
        /// The typed error carried by this result. Type: `SourceError`.
        error: SourceError,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The typed cause of a configuration lookup or decoding failure.
pub enum ConfigErrorKind {
    /// No matching value exists.
    Missing,
    /// The  failed validation.
    Invalid,
    /// The error originated in the underlying source.
    Source,
}

impl<SourceError> ConfigError<SourceError> {
    #[must_use]
    /// # Type parameters
    /// - `SourceError`: Error returned by the underlying source.
    ///
    /// # Returns
    /// Type: `ConfigErrorKind`.
    pub const fn kind(&self) -> ConfigErrorKind {
        match self {
            Self::Missing { .. } => ConfigErrorKind::Missing,
            Self::Invalid { .. } => ConfigErrorKind::Invalid,
            Self::Source { .. } => ConfigErrorKind::Source,
        }
    }

    #[must_use]
    /// # Type parameters
    /// - `SourceError`: Error returned by the underlying source.
    ///
    /// # Returns
    /// Type: `&ConfigPath`.
    pub const fn path(&self) -> &ConfigPath {
        match self {
            Self::Missing { path } | Self::Invalid { path, .. } | Self::Source { path, .. } => path,
        }
    }

    #[must_use]
    /// # Type parameters
    /// - `SourceError`: Error returned by the underlying source.
    ///
    /// # Returns
    /// Type: `Option<&SchemaIssue>`.
    /// Returns the value when present, or `None`.
    pub const fn issue(&self) -> Option<&SchemaIssue> {
        match self {
            Self::Invalid { issue, .. } => Some(issue),
            Self::Missing { .. } | Self::Source { .. } => None,
        }
    }

    #[must_use]
    /// # Type parameters
    /// - `SourceError`: Error returned by the underlying source.
    ///
    /// # Returns
    /// Type: `Option<&SourceError>`.
    /// Returns the value when present, or `None`.
    pub const fn source_error(&self) -> Option<&SourceError> {
        match self {
            Self::Source { error, .. } => Some(error),
            Self::Missing { .. } | Self::Invalid { .. } => None,
        }
    }
}

impl<SourceError> std::fmt::Debug for ConfigError<SourceError> {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.kind(), formatter)
    }
}

impl<SourceError> std::fmt::Display for ConfigError<SourceError> {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.kind(), formatter)
    }
}

impl<SourceError: Send + Sync + 'static> std::error::Error for ConfigError<SourceError> {}

/// A source may perform effectful provider work. Its handle is obtained only
/// through R; this contract does not prescribe files, variables or services.
pub trait ConfigSource: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;

    /// # Parameters
    /// - `path` (`ConfigPath`): Location of the field, file, or configuration value being processed.
    ///
    /// # Returns
    /// Type: `Effect<Option<Arc<JsonValue>>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn load(&self, path: ConfigPath) -> Effect<Option<Arc<JsonValue>>, Self::Error>;
}

/// Selects the source used for typed configuration lookup.
pub trait HasConfigSource {
    /// Configuration source selected by this environment.
    type Source: ConfigSource + ?Sized;

    /// # Returns
    /// Type: `&Self::Source`.
    fn config_source(&self) -> &Self::Source;
}

/// Type alias for default value in config.
///
/// # Type parameters
/// - `A`: Application value type.
type DefaultValue<A> = Arc<dyn Fn() -> A + Send + Sync>;

/// Pure schema-backed configuration. Definitions are inert; every load reads
/// the current R-provided source, then decodes once. A default applies only to
/// a missing value, never to malformed data or source failure.
///
/// # Type parameters
/// - `A`: Application value type.
pub struct Config<A> {
    path: ConfigPath,
    schema: PureCodec<A, JsonValue>,
    default: Option<DefaultValue<A>>,
}

impl<A> Clone for Config<A> {
    ///
    /// # Returns
    /// Type: `Self`.
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
    /// Construct a `Config` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `path` (`ConfigPath`): Location of the field, file, or configuration value being processed.
    /// - `schema` (`PureCodec<A, JsonValue>`): Codec and validation rules for the application value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(path: ConfigPath, schema: PureCodec<A, JsonValue>) -> Self {
        Self {
            path,
            schema,
            default: None,
        }
    }

    #[must_use]
    /// Use the supplied fallback only when the requested configuration is absent.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    ///
    /// # Parameters
    /// - `make` (`impl Fn() -> A + Send + Sync + 'static`): The make supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn with_default(mut self, make: impl Fn() -> A + Send + Sync + 'static) -> Self {
        self.default = Some(Arc::new(make));
        self
    }

    #[must_use]
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `&ConfigPath`.
    pub const fn path(&self) -> &ConfigPath {
        &self.path
    }

    /// Read and decode the selected value from its backing store.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasConfigSource + Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    ///
    /// # Returns
    /// Type: `Effect<A, ConfigError<<R::Source as ConfigSource>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    /// Construct a `MemoryConfigSource` from its required inputs and explicit policies.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the current value through the declared state capability.
    ///
    /// # Parameters
    /// - `path` (`ConfigPath`): Location of the field, file, or configuration value being processed.
    /// - `value` (`JsonValue`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Option<Arc<JsonValue>>`.
    /// Returns the value when present, or `None`.
    pub fn set(&self, path: ConfigPath, value: JsonValue) -> Option<Arc<JsonValue>> {
        self.values
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(path, Arc::new(value))
    }

    /// Remove the selected entry through this collection or store contract.
    ///
    /// # Parameters
    /// - `path` (`&ConfigPath`): Location of the field, file, or configuration value being processed.
    ///
    /// # Returns
    /// Type: `Option<Arc<JsonValue>>`.
    /// Returns the value when present, or `None`.
    pub fn remove(&self, path: &ConfigPath) -> Option<Arc<JsonValue>> {
        self.values
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(path)
    }
}

impl ConfigSource for MemoryConfigSource {
    /// Type alias for error in config.
    type Error = std::convert::Infallible;

    ///
    /// # Parameters
    /// - `path` (`ConfigPath`): Location of the field, file, or configuration value being processed.
    ///
    /// # Returns
    /// Type: `Effect<Option<Arc<JsonValue>>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    /// Type alias for source in config.
    type Source = Self;

    ///
    /// # Returns
    /// Type: `&Self::Source`.
    fn config_source(&self) -> &Self::Source {
        self
    }
}
