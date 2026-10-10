//! Bounded typed catalogues, request-pinned generations and scoped reload/preferences.
//!
//! Named templates are validated against message contracts before publication.
//! Requests retain one immutable generation, and failed refreshes preserve the
//! last good one. UI language and formatting culture are explicitly separate.

mod catalog;
mod message;
mod preferences;
mod reload;

pub use catalog::{
    Catalog, CatalogDefinition, CatalogLimits, CatalogResource, Contract, CultureId, CultureSet,
    MissingMessage,
};
pub use message::{
    ArgumentList, ArgumentValue, BoundMessage, Message, MessageContract, NamedArgument, Template,
};
pub use preferences::{
    LocalePreferences, PreferenceError, PreferenceJob, PreferencePolicy, PreferenceShutdown,
    PreferenceStatus, PreferenceSubmission,
};
pub use reload::{
    CatalogRuntime, CatalogSelection, CatalogStatus, IntoLocalization, Localization, PinnedCatalog,
};

/// Protected localization error categories, excluding resource contents and user values.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum LocalizationError {
    /// Malformed culture/catalogue/message/selection configuration.
    Definition(String),
    /// Caller-selected encoded byte/depth/key/value limit exceeded.
    Limit(String),
    /// Duplicate or ambiguous flattened resource key.
    Key(String),
    /// Resource syntax rejected; text/source details stay application-owned.
    Resource(String),
    /// Malformed template or contract mismatch for a named key.
    Template(String),
    /// Selected missing-message policy rejects this absent key.
    Missing(String),
    /// Owned reload/preference worker could not obtain its core resource.
    Admission,
    /// A resource loader failed; identity is safe but provider details are not public.
    Load(String),
    /// Catalogue generation counter exhausted; old generation remains retained.
    GenerationOverflow,
}
impl std::fmt::Display for LocalizationError {
    /// Render only the selected safe error category and resource/key identity.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Definition(name) => write!(f, "invalid localization definition: {name}"),
            Self::Limit(name) => write!(f, "catalogue limit: {name}"),
            Self::Key(name) => write!(f, "invalid catalogue key: {name}"),
            Self::Resource(name) => write!(f, "invalid catalogue resource: {name}"),
            Self::Template(name) => write!(f, "invalid message template: {name}"),
            Self::Missing(name) => write!(f, "missing message: {name}"),
            Self::Admission => f.write_str("localization admission failed"),
            Self::Load(name) => write!(f, "catalogue load failed: {name}"),
            Self::GenerationOverflow => f.write_str("catalogue generation exhausted"),
        }
    }
}
impl std::error::Error for LocalizationError {}
