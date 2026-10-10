//! Native category narrowing at one invocation boundary.
//!
//! SQL keeps its diagnostic/cleanup tree; native Identity accepts only StoreError.
//! A caller-owned invocation carrier in the Effect bridge can retain the original
//! SQL failure before using this category projection. No process-global carrier
//! or retry decision is introduced here.

use effect_sql::SqlError;
use identity_rs::identity::StoreError;

/// Select the native category without disclosing query text, bindings or keys.
/// Commit uncertainty remains uncertainty even when cleanup also failed.
pub(crate) fn sql_category(error: &SqlError) -> StoreError {
    match error {
        SqlError::CommitOutcomeUnknown(_) => StoreError::CommitOutcomeUnknown,
        SqlError::ReplicaNotAuthoritative => StoreError::UnsupportedTopology,
        SqlError::Cleanup { primary, cleanup } => primary
            .as_deref()
            .map(sql_category)
            .unwrap_or_else(|| sql_category(cleanup)),
        SqlError::Migration {
            uncertain: true, ..
        } => StoreError::CommitOutcomeUnknown,
        SqlError::Provider(error) => provider_category(error),
        SqlError::Binding(error) => {
            if let Some(error) = error.downcast_ref::<NativeCommandFailure>() {
                error.category.clone()
            } else if let Some(error) = error.downcast_ref::<ProviderFailure>() {
                provider_category(error.0.as_ref())
            } else {
                error
                    .downcast_ref::<StoreError>()
                    .cloned()
                    .unwrap_or(StoreError::CorruptStoredData)
            }
        }
        SqlError::RowAbsent => StoreError::NotFound,
        SqlError::Decode(_) => StoreError::CorruptStoredData,
        SqlError::Configuration(_)
        | SqlError::Identifier(_)
        | SqlError::InactiveSession
        | SqlError::Migration {
            uncertain: false, ..
        }
        | SqlError::MaintenanceRequired
        | SqlError::NativeInvocation(_)
        | SqlError::Consumed => StoreError::BackendUnavailable,
    }
}

/// Map only stable SQLx/Turso error codes. Message matching would risk secret
/// disclosure and would turn provider wording into an authentication policy.
pub(crate) fn provider_category(error: impl std::borrow::Borrow<sqlx::Error>) -> StoreError {
    match error.borrow() {
        sqlx::Error::RowNotFound => StoreError::NotFound,
        sqlx::Error::PoolTimedOut => StoreError::Timeout,
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::ColumnIndexOutOfBounds { .. } => StoreError::CorruptStoredData,
        sqlx::Error::Database(error) => match error.code().as_deref() {
            Some("SQLITE_CONSTRAINT") => StoreError::ConstraintViolation,
            Some("SQLITE_BUSY" | "SQLITE_BUSY_SNAPSHOT") => StoreError::Timeout,
            Some("SQLITE_CORRUPT" | "SQLITE_NOTADB") => StoreError::CorruptStoredData,
            Some("TURSO_QUERY_RETURNED_NO_ROWS") => StoreError::NotFound,
            _ => StoreError::BackendUnavailable,
        },
        sqlx::Error::Configuration(_)
        | sqlx::Error::InvalidArgument(_)
        | sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::Protocol(_)
        | sqlx::Error::TypeNotFound { .. }
        | sqlx::Error::Encode(_)
        | sqlx::Error::AnyDriverError(_)
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed
        | sqlx::Error::Migrate(_)
        | sqlx::Error::InvalidSavePointStatement
        | sqlx::Error::BeginFailed
        | sqlx::Error::ConfigFile(_) => StoreError::BackendUnavailable,
        // SQLx's public error enum is non-exhaustive; future native categories
        // remain unavailable until this boundary classifies them explicitly.
        _ => StoreError::BackendUnavailable,
    }
}

/// Shared ownership retains a provider diagnostic while a native trait returns
/// only its stable category. This contains no copied bindings or credential data.
#[derive(Clone)]
pub(crate) struct ProviderFailure(pub(crate) std::sync::Arc<sqlx::Error>);
impl std::fmt::Debug for ProviderFailure {
    /// Omit provider wording and binding values from ordinary Debug output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderFailure").finish_non_exhaustive()
    }
}
impl std::borrow::Borrow<sqlx::Error> for ProviderFailure {
    /// Borrow the original diagnostic when selecting a native category.
    fn borrow(&self) -> &sqlx::Error {
        &self.0
    }
}
impl std::fmt::Display for ProviderFailure {
    /// Keep provider wording out of public security error displays.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("identity SQL provider failed")
    }
}
impl std::error::Error for ProviderFailure {
    /// Preserve the original provider as a diagnostic source for trusted consumers.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
impl From<ProviderFailure> for SqlError {
    /// Preserve the shared source without cloning the original provider error.
    fn from(error: ProviderFailure) -> Self {
        Self::Binding(Box::new(error))
    }
}

/// Native category and its original operation-local provider diagnostic.
#[derive(Debug)]
pub(crate) struct NativeCommandFailure {
    /// Category selected by the native store's actual operation semantics.
    pub(crate) category: StoreError,
    /// Original failure, retained until the outer native invocation settles.
    pub(crate) provider: Option<ProviderFailure>,
}
impl std::fmt::Display for NativeCommandFailure {
    /// Expose only the stable native category in public error formatting.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.category, f)
    }
}
impl std::error::Error for NativeCommandFailure {
    /// Preserve diagnostics without leaking them through the category's display.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.provider.as_ref().map(|error| error as _)
    }
}

/// Preserve a native command rejection through SQL's rollback path. The boxed
/// error is invocation-local and is recovered by `sql_category`; it is neither
/// a binding error exposed to clients nor a second error/interpreter channel.
pub(crate) fn command_error(error: StoreError) -> SqlError {
    SqlError::Binding(Box::new(error))
}
