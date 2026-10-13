//! Typed Identity configuration, authority and native operation failures.

#[cfg(feature = "sqlx-turso")]
use effect_sql::SqlError;
use identity_rs::identity::{IdentityResult, StoreError};

/// Optional selections retained by D17; absence never creates a fallback provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityName {
    /// Atomic account-snapshot service.
    Accounts,
    /// Stored native sessions/tickets.
    Sessions,
    /// Durable native key documents.
    Keys,
    /// Native stable role service.
    Roles,
    /// Application-owned permission decisions.
    Permissions,
    /// Generic expiring protocol challenges.
    Challenges,
    /// Incremental authorized account views.
    Queries,
    /// Typed audit acknowledgment.
    Audit,
    /// Consent/preference persistence.
    Consent,
    /// Durable delivery acceptance and selected transport.
    Delivery,
    /// Durable notification acceptance and selected callback.
    Notifications,
    /// Acknowledged export/delete job requests.
    Jobs,
    /// Application-owned personal-data inventory/hooks.
    PersonalData,
    /// Native bounded user query integration.
    NativeUserQueries,
    /// Native bounded role query integration.
    NativeRoleQueries,
}
/// Safe public failure category with original native/SQL diagnostics retained.
#[derive(Debug)]
pub enum IdentityError {
    /// Inert definition or selected native policy was invalid.
    Configuration(&'static str),
    /// An explicitly optional capability was not installed.
    MissingCapability(CapabilityName),
    /// A native replacement point was deliberately left unselected.
    MissingPolicy(&'static str),
    /// A selected handle/grant belongs to another storage/definition owner.
    WrongConfiguration,
    /// No trusted current authentication/permission proof was established.
    Unauthorized,
    /// An owned grant expired or current persisted security state changed.
    StaleGrant,
    /// Native validation/manager rejection, preserved without replacing its vocabulary.
    Native(IdentityResult),
    /// Narrow native store category outside an invocation diagnostic scope.
    Store(StoreError),
    /// Retained owned SQL primary/cleanup/uncertain acknowledgment diagnostic tree.
    #[cfg(feature = "sqlx-turso")]
    Sql(Box<SqlError>),
    /// Native browser/protocol policy rejected the request before application work.
    Policy(http::StatusCode),
    /// Retained owned auxiliary persistence/codec failure with its original diagnostics.
    #[cfg(feature = "sqlx-turso")]
    Record(Box<crate::store::records::RecordError>),
    /// Native Server input/body ownership failure, preserving its actual diagnostics.
    Input(effect_server::ServerError),
    /// Native token verification/provider failure; outages remain distinct from rejection.
    Bearer(identity_rs::identity::tokens::BearerTokenError),
    /// Native ticket protection, transport or current-user validation failure.
    Ticket(identity_rs::identity::ticket::TicketError),
    /// Selected native factor/provider execution failed; distinct from false proof.
    Token(identity_rs::identity::tokens::TokenProviderError),
}
impl std::fmt::Display for IdentityError {
    /// Keep secrets/provider wording out of public error rendering.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Configuration(_) => "invalid identity configuration",
            Self::MissingCapability(_) | Self::MissingPolicy(_) => {
                "identity capability not selected"
            }
            Self::WrongConfiguration => "identity owner mismatch",
            Self::Unauthorized => "identity authorization rejected",
            Self::StaleGrant => "identity authority expired or changed",
            Self::Native(_) => "identity operation rejected",
            Self::Store(_) => "identity persistence failed",
            #[cfg(feature = "sqlx-turso")]
            Self::Sql(_) | Self::Record(_) => "identity persistence failed",
            Self::Policy(_) => "identity request policy rejected",
            Self::Input(_) => "identity request input failed",
            Self::Bearer(_) => "identity token verification failed",
            Self::Ticket(_) => "identity session verification failed",
            Self::Token(_) => "identity provider execution failed",
        })
    }
}
impl std::error::Error for IdentityError {
    /// Preserve actual backend/codec sources for explicit trusted diagnostic consumers.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            #[cfg(feature = "sqlx-turso")]
            Self::Sql(error) => Some(error.as_ref()),
            #[cfg(feature = "sqlx-turso")]
            Self::Record(error) => Some(error.as_ref()),
            Self::Input(error) => Some(error),
            Self::Bearer(error) => Some(error),
            Self::Ticket(error) => Some(error),
            Self::Token(error) => Some(error),
            _ => None,
        }
    }
}

/// Rejoin the original settled SQL diagnostic after a narrow native operation.
#[cfg(feature = "sqlx-turso")]
pub(crate) fn restore_sql_result<A>(
    failure: Option<SqlError>,
    result: Result<A, IdentityError>,
) -> Result<A, IdentityError> {
    match failure {
        Some(error) => Err(IdentityError::Sql(Box::new(error))),
        None => result,
    }
}
impl IdentityError {
    /// Restore native defects/interruption/finalization at the outer Effect boundary.
    #[must_use]
    pub fn into_exit<A>(self) -> effect::Exit<A, Self> {
        match self {
            #[cfg(feature = "sqlx-turso")]
            Self::Sql(error) => (*error)
                .into_exit()
                .map_error(|error| Self::Sql(Box::new(error))),
            #[cfg(feature = "sqlx-turso")]
            Self::Record(error) => (*error)
                .into_exit()
                .map_error(|error| Self::Record(Box::new(error))),
            error => effect::Exit::Fail(error),
        }
    }
}
impl From<StoreError> for IdentityError {
    /// Preserve a native store category; scoped workflows also retain its original SQL exit.
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}
#[cfg(feature = "sqlx-turso")]
impl From<crate::store::components::SessionError> for IdentityError {
    /// Restore stored-ticket provider diagnostics instead of narrowing them into transport rejection.
    fn from(error: crate::store::components::SessionError) -> Self {
        match error {
            crate::store::components::SessionError::Ticket(error) => Self::Ticket(error),
            crate::store::components::SessionError::Storage(error) => Self::Record(Box::new(error)),
        }
    }
}
#[cfg(feature = "sqlx-turso")]
impl From<crate::store::records::RecordError> for IdentityError {
    /// Retain the selected codec/provider error, including acknowledgment uncertainty.
    fn from(error: crate::store::records::RecordError) -> Self {
        Self::Record(Box::new(error))
    }
}
