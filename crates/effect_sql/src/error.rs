//! Typed SQL failures keep provider diagnostics separate from safe display text.

use std::fmt;

/// A native cell failure with the original column identity and schema issue.
#[derive(Debug)]
pub enum RowError {
    /// A decoder/choice description is empty or structurally invalid.
    InvalidDecoder,
    /// A requested name or ordinal is absent.
    MissingColumn(String),
    /// A required cell is SQL NULL; this is distinct from no row.
    NullColumn(String),
    /// SQLx rejected the native representation for the requested Rust type.
    Native {
        /// Requested column identity.
        column: String,
        /// Original SQLx diagnostic; do not expose to HTTP clients.
        source: sqlx::Error,
    },
    /// A native-cell codec or refinement rejected the value.
    Schema {
        /// Requested column identity.
        column: String,
        /// Structured validation issue including its path.
        issue: effect::SchemaIssue,
    },
    /// A caller's projection failed with an explicitly mapped reason.
    Validation(String),
}

impl fmt::Display for RowError {
    /// Redact cell contents and application validation messages from public error text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidDecoder => "invalid SQL row decoder",
            Self::MissingColumn(_) => "SQL column is absent",
            Self::NullColumn(_) => "required SQL cell is NULL",
            Self::Native { .. } => "SQL cell representation is invalid",
            Self::Schema { .. } => "SQL cell validation failed",
            Self::Validation(_) => "SQL row projection failed",
        })
    }
}
impl std::error::Error for RowError {
    /// Preserve the native representation error for trusted diagnostic consumers.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Native { source, .. } => Some(source),
            Self::InvalidDecoder
            | Self::MissingColumn(_)
            | Self::NullColumn(_)
            | Self::Schema { .. }
            | Self::Validation(_) => None,
        }
    }
}

/// Which acknowledgment boundary failed during migration execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationPhase {
    /// Catalog or history did not satisfy its immutable contract.
    Validate,
    /// Database write authority could not be acquired.
    Lock,
    /// History differs from the catalog or a captured down-plan.
    History,
    /// An unresolved dirty step needs an explicit schema assessment.
    Recovery,
    /// Provider execution of the schema step failed.
    Apply,
    /// Journal publication failed.
    Journal,
    /// Commit acknowledgment was lost.
    Commit,
    /// Rollback or resource cleanup failed.
    Cleanup,
}

/// SQL expected errors; defects and interruption remain Effect exit channels.
#[derive(Debug)]
pub enum SqlError {
    /// An inert description or selected provider configuration is invalid.
    Configuration(String),
    /// Identifier syntax cannot represent a native identifier.
    Identifier(String),
    /// SQLx could not encode a bound native value.
    Binding(Box<dyn std::error::Error + Send + Sync>),
    /// Native provider failure before a durable acknowledgment boundary.
    Provider(sqlx::Error),
    /// No row exists for an operation requiring one.
    RowAbsent,
    /// A present native row failed typed decoding.
    Decode(RowError),
    /// A scoped handle was released, outside its frame, or reentered while its cursor owns native access.
    InactiveSession,
    /// Native commit was attempted but its durable result is unknown. Never retry blindly.
    CommitOutcomeUnknown(sqlx::Error),
    /// Cleanup failed separately from the primary native operation.
    Cleanup {
        /// Original failure, if the body failed before cleanup.
        primary: Option<Box<SqlError>>,
        /// Failure observed while resolving the connection.
        cleanup: Box<SqlError>,
    },
    /// Migration failure retains phase, identity and uncertainty.
    Migration {
        /// Failed acknowledgment boundary.
        phase: MigrationPhase,
        /// Stable SQLx version when one step is implicated.
        version: Option<i64>,
        /// Whether the schema/history may already have committed.
        uncertain: bool,
        /// Underlying diagnostic without public payload serialization.
        source: Box<SqlError>,
    },
    /// A nontransactional schema step requires explicit exclusive maintenance.
    MaintenanceRequired,
    /// Synchronized replicas cannot claim authoritative schema/security writes.
    ReplicaNotAuthoritative,
    /// Invocation-local complete failure retained when a native trait narrows its error type.
    /// Outer Effect adapters restore these channels; no process-global side channel is used.
    NativeInvocation(Box<effect::Exit<(), SqlError>>),
    /// An owned one-shot payload has already been consumed.
    Consumed,
}
impl fmt::Display for SqlError {
    /// Produce safe category text without SQL, bindings or connection strings.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration(_) => "invalid SQL configuration",
            Self::Identifier(_) => "invalid SQL identifier",
            Self::Binding(_) => "SQL binding failed",
            Self::Provider(_) => "SQL provider failed",
            Self::RowAbsent => "required SQL row is absent",
            Self::Decode(_) => "SQL row decoding failed",
            Self::InactiveSession => "SQL session is inactive",
            Self::CommitOutcomeUnknown(_) => "SQL commit outcome is unknown",
            Self::Cleanup { .. } => "SQL cleanup failed",
            Self::Migration { .. } => "SQL migration failed",
            Self::MaintenanceRequired => "exclusive maintenance authority is required",
            Self::ReplicaNotAuthoritative => "replica lacks write authority",
            Self::NativeInvocation(_) => "native SQL invocation failed",
            Self::Consumed => "SQL payload already consumed",
        })
    }
}
impl std::error::Error for SqlError {
    /// Retain native diagnostics for trusted callers while safe Display stays redacted.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Binding(source) => Some(source.as_ref()),
            Self::Provider(source) | Self::CommitOutcomeUnknown(source) => Some(source),
            Self::Migration { source, .. } => Some(source.as_ref()),
            Self::Cleanup { cleanup, .. } => Some(cleanup.as_ref()),
            Self::Decode(source) => Some(source),
            Self::Configuration(_)
            | Self::Identifier(_)
            | Self::RowAbsent
            | Self::InactiveSession
            | Self::MaintenanceRequired
            | Self::ReplicaNotAuthoritative
            | Self::NativeInvocation(_)
            | Self::Consumed => None,
        }
    }
}
impl From<sqlx::Error> for SqlError {
    /// Lift an ordinary provider failure without claiming a commit outcome.
    fn from(error: sqlx::Error) -> Self {
        Self::Provider(error)
    }
}
impl From<RowError> for SqlError {
    /// Preserve the structured row error rather than returning absence.
    fn from(error: RowError) -> Self {
        Self::Decode(error)
    }
}

impl SqlError {
    /// Restore a narrowed native invocation's defect/interruption/ordered cleanup at its outer Effect boundary.
    /// Ordinary SQL and durable uncertainty remain in the declared expected-error channel.
    #[must_use = "restoring a native failure produces the complete outcome for its outer Effect boundary"]
    pub fn into_exit<A>(self) -> effect::Exit<A, Self> {
        match self {
            Self::NativeInvocation(outcome) => match *outcome {
                effect::Exit::Success(()) => effect::Exit::Fail(Self::Configuration(
                    "native failure carrier contains success".into(),
                )),
                effect::Exit::Fail(error) => error.into_exit(),
                effect::Exit::Die(defect) => effect::Exit::Die(defect),
                effect::Exit::Interrupted => effect::Exit::Interrupted,
                effect::Exit::FinalizationFailed(details) => {
                    effect::Exit::FinalizationFailed(details)
                }
            },
            Self::Migration {
                phase,
                version,
                uncertain,
                source,
            } => source.into_exit().map_error(move |source| Self::Migration {
                phase,
                version,
                uncertain,
                source: Box::new(source),
            }),
            Self::Cleanup { primary, cleanup } => {
                let outcome = match primary {
                    Some(primary) => primary.into_exit(),
                    None => effect::Exit::Success(()),
                };
                match crate::client::append_cleanup(outcome, Err(*cleanup)) {
                    effect::Exit::FinalizationFailed(details) => {
                        effect::Exit::FinalizationFailed(details)
                    }
                    effect::Exit::Fail(error) => effect::Exit::Fail(error),
                    effect::Exit::Die(defect) => effect::Exit::Die(defect),
                    effect::Exit::Interrupted => effect::Exit::Interrupted,
                    effect::Exit::Success(()) => effect::Exit::Fail(Self::Configuration(
                        "cleanup failure carrier contains success".into(),
                    )),
                }
            }
            error @ (Self::Configuration(_)
            | Self::Identifier(_)
            | Self::Binding(_)
            | Self::Provider(_)
            | Self::RowAbsent
            | Self::Decode(_)
            | Self::InactiveSession
            | Self::CommitOutcomeUnknown(_)
            | Self::MaintenanceRequired
            | Self::ReplicaNotAuthoritative
            | Self::Consumed) => effect::Exit::Fail(error),
        }
    }
    /// Whether any retained primary/cleanup diagnostic includes unknown commit acknowledgment.
    /// A rollback attempt following such a failure cannot erase this durable uncertainty.
    #[must_use]
    pub fn outcome_uncertain(&self) -> bool {
        match self {
            Self::CommitOutcomeUnknown(_) => true,
            Self::Migration { uncertain, source, .. } => *uncertain || source.outcome_uncertain(),
            Self::Cleanup { primary, cleanup } => primary.as_ref().is_some_and(|primary| primary.outcome_uncertain()) || cleanup.outcome_uncertain(),
            Self::NativeInvocation(outcome) => match outcome.as_ref() { effect::Exit::Fail(error) => error.outcome_uncertain(), effect::Exit::FinalizationFailed(details) => matches!(&details.primary, effect::PrimaryOutcome::Failed(error) if error.outcome_uncertain()) || std::iter::once(&details.first).chain(details.rest.iter()).any(|failure| matches!(failure, effect::CleanupFailure::ReleaseFailure(error) if error.outcome_uncertain())), effect::Exit::Success(()) | effect::Exit::Die(_) | effect::Exit::Interrupted => false },
            Self::Configuration(_) | Self::Identifier(_) | Self::Binding(_) | Self::Provider(_) | Self::RowAbsent | Self::Decode(_) | Self::InactiveSession | Self::MaintenanceRequired | Self::ReplicaNotAuthoritative | Self::Consumed => false,
        }
    }
}
