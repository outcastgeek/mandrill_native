use std::{borrow::Cow, error::Error, fmt};

use sqlx_core::error::{DatabaseError, Error as SqlxError, ErrorKind};

/// Error type for adapter-owned setup and unsupported surfaces
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TursoAdapterError {
    message: String,
}

impl TursoAdapterError {
    /// Creates an error for an unsupported SQLx/Turso configuration.
    pub fn unsupported(surface: impl Into<String>) -> Self {
        let surface = surface.into();
        Self {
            message: format!("{surface} is not supported by sqlx-turso"),
        }
    }

    /// Returns the error message
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TursoAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for TursoAdapterError {}

pub(crate) fn unsupported_sqlx(surface: impl Into<String>) -> sqlx_core::error::Error {
    let error = TursoAdapterError::unsupported(surface);
    sqlx_core::error::Error::Configuration(Box::new(error))
}

/// Database error returned by the Turso engine
pub struct TursoDatabaseError {
    code: &'static str,
    message: String,
    source: turso::Error,
}

impl TursoDatabaseError {
    pub(crate) fn from_turso(error: turso::Error) -> SqlxError {
        Self::from_turso_with_redactions(error, std::iter::empty::<&str>())
    }

    pub(crate) fn from_turso_with_redactions<'a>(
        error: turso::Error,
        redactions: impl IntoIterator<Item = &'a str>,
    ) -> SqlxError {
        let code = turso_error_code(&error);
        let mut message = error.to_string();
        for secret in redactions {
            if !secret.is_empty() {
                message = message.replace(secret, "<redacted>");
            }
        }
        SqlxError::database(Self {
            code,
            message,
            source: error,
        })
    }

    /// Returns the original structured error produced by Turso.
    #[must_use]
    pub fn native_error(&self) -> &turso::Error {
        &self.source
    }
}

impl fmt::Debug for TursoDatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TursoDatabaseError")
            .field("code", &self.code)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TursoDatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for TursoDatabaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

impl DatabaseError for TursoDatabaseError {
    fn message(&self) -> &str {
        &self.message
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.code))
    }

    fn as_error(&self) -> &(dyn Error + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn Error + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn Error + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> ErrorKind {
        sqlx_error_kind(self.code, &self.message)
    }
}

fn turso_error_code(error: &turso::Error) -> &'static str {
    match error {
        turso::Error::ToSqlConversionFailure(_) => "TURSO_TO_SQL_CONVERSION_FAILURE",
        turso::Error::QueryReturnedNoRows => "TURSO_QUERY_RETURNED_NO_ROWS",
        turso::Error::ConversionFailure(_) => "TURSO_CONVERSION_FAILURE",
        turso::Error::Busy(_) => "SQLITE_BUSY",
        turso::Error::BusySnapshot(_) => "SQLITE_BUSY_SNAPSHOT",
        turso::Error::Interrupt(_) => "SQLITE_INTERRUPT",
        turso::Error::Error(_) => "SQLITE_ERROR",
        turso::Error::Misuse(_) => "SQLITE_MISUSE",
        turso::Error::Constraint(_) => "SQLITE_CONSTRAINT",
        turso::Error::Readonly(_) => "SQLITE_READONLY",
        turso::Error::DatabaseFull(_) => "SQLITE_FULL",
        turso::Error::NotAdb(_) => "SQLITE_NOTADB",
        turso::Error::Corrupt(_) => "SQLITE_CORRUPT",
        turso::Error::IoError(_, _) => "SQLITE_IOERR",
        turso::Error::BatchStatementFailed { .. } => "TURSO_BATCH_STATEMENT_FAILED",
        turso::Error::BatchRollbackFailed { .. } => "TURSO_BATCH_ROLLBACK_FAILED",
    }
}

fn sqlx_error_kind(code: &str, message: &str) -> ErrorKind {
    match code {
        "SQLITE_CONSTRAINT" if constraint_message_contains(message, "unique") => {
            ErrorKind::UniqueViolation
        }
        "SQLITE_CONSTRAINT" if constraint_message_contains(message, "primary") => {
            ErrorKind::UniqueViolation
        }
        "SQLITE_CONSTRAINT" if constraint_message_contains(message, "foreign") => {
            ErrorKind::ForeignKeyViolation
        }
        "SQLITE_CONSTRAINT" if constraint_message_contains(message, "not null") => {
            ErrorKind::NotNullViolation
        }
        "SQLITE_CONSTRAINT" if constraint_message_contains(message, "check") => {
            ErrorKind::CheckViolation
        }
        _ => ErrorKind::Other,
    }
}

fn constraint_message_contains(message: &str, needle: &str) -> bool {
    message.to_ascii_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
    use super::TursoDatabaseError;

    #[test]
    fn sqlx_error_output_is_redacted_while_native_error_is_preserved() -> sqlx_core::Result<()> {
        let secret = "native-error-secret";
        let error = TursoDatabaseError::from_turso_with_redactions(
            turso::Error::Error(format!("connection rejected token {secret}")),
            [secret],
        );

        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
        let database_error = error
            .as_database_error()
            .ok_or_else(|| sqlx_core::Error::Protocol("expected database error".to_owned()))?;
        let turso_error = database_error
            .downcast_ref::<TursoDatabaseError>()
            .native_error();
        assert!(turso_error.to_string().contains(secret));
        Ok(())
    }
}
