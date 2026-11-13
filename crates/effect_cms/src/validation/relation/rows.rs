//! Relation row input, detached draft, private validated authority and rejection
//! evidence.
//!
//! The draft copies its rows and each row's value map on every projection, so a
//! caller cannot change the admitted value through a returned draft. The
//! validated record and its one-shot claim are private constructions: a valid
//! instance exists only after the owner's complete validation sequence, and a
//! claim exists only when the same execution owner has not been consumed before.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use effect_server::Request;

use crate::definitions::{
    RelationError, ResourceFieldKey, ResourceKind, ResourceRelationDescriptor, UnknownResourceKind,
};
use crate::queries::RelationSelection;

/// One decoded relation row.
///
/// A row grants no current membership and no permission to mutate its target. The
/// selection is a typed target identity; whether it belongs to the declared relation
/// is rechecked by the validation workflow.
#[derive(Clone, Debug, Default)]
pub struct RelationRowInput {
    /// Stable row key.
    pub key: String,
    /// Row revision, when the transport supplies one.
    pub revision: String,
    /// Explicit row intent, or empty when the operation is supplied by the request.
    pub operation: String,
    /// Typed target selection, or the zero selection for a new related record.
    pub selection: RelationSelection,
    /// Additional row values, keyed by resource field.
    pub values: BTreeMap<ResourceFieldKey, String>,
    /// Explicit order position.
    pub position: i64,
    /// Whether an explicit order position was supplied.
    pub has_position: bool,
}

/// The decoded transport input for one relation validation.
#[derive(Clone, Default)]
pub struct RelationValidationInput {
    /// The request whose cancellation source gates each validation step.
    pub request: Option<Request>,
    /// Source resource kind.
    pub source: ResourceKind,
    /// Relation declaration name.
    pub relation: String,
    /// Source record identity.
    pub source_id: String,
    /// Source revision.
    pub revision: String,
    /// Requested relation operation.
    pub operation: String,
    /// Requested relation save mode.
    pub save_mode: String,
    /// Decoded relation rows.
    pub rows: Vec<RelationRowInput>,
}

impl fmt::Debug for RelationValidationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelationValidationInput")
            .field("has_request", &self.request.is_some())
            .field("source", &self.source)
            .field("relation", &self.relation)
            .field("source_id", &self.source_id)
            .field("revision", &self.revision)
            .field("operation", &self.operation)
            .field("save_mode", &self.save_mode)
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

/// One ordered relation row issue.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RelationRowIssue {
    /// Row key, or empty for a request-level issue.
    pub row: String,
    /// Field key, or empty for a row-level issue.
    pub field: ResourceFieldKey,
    /// Issue code.
    pub code: String,
    /// Optional issue message.
    pub message: String,
}

/// Why a relation validation did not accept its rows.
///
/// A consumer cause keeps its own identity; an owner contract violation keeps its
/// source message; cancellation is distinct from both.
#[derive(Debug)]
pub enum RelationValidationCause<E> {
    /// The consumer's expected failure value.
    Consumer(E),
    /// A registered source definition was absent.
    Definition(UnknownResourceKind),
    /// A named relation declaration was refused.
    Relation(RelationError),
    /// An owner contract violation, with its message text.
    Message(String),
    /// The request's cancellation source was requested.
    ///
    /// The token is boolean: it does not establish whether the caller cancelled or a
    /// deadline expired.
    Cancelled,
}

impl<E: fmt::Display> fmt::Display for RelationValidationCause<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Consumer(error) => fmt::Display::fmt(error, formatter),
            Self::Definition(error) => fmt::Display::fmt(error, formatter),
            Self::Relation(error) => fmt::Display::fmt(error, formatter),
            Self::Message(message) => formatter.write_str(message.as_str()),
            Self::Cancelled => formatter.write_str("context canceled"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display + std::error::Error + 'static> std::error::Error
    for RelationValidationCause<E>
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Consumer(error) => Some(error),
            Self::Definition(error) => Some(error),
            Self::Relation(error) => Some(error),
            Self::Message(_) | Self::Cancelled => None,
        }
    }
}

/// A relation-validation constructor or observation contract refusal.
#[derive(Debug)]
pub struct RelationValidationContractError {
    message: String,
}

impl RelationValidationContractError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The refusal message.
    #[must_use]
    pub fn message(&self) -> &str {
        self.message.as_str()
    }
}

impl fmt::Display for RelationValidationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message.as_str())
    }
}

impl std::error::Error for RelationValidationContractError {}

/// The admitted relation rows payload.
pub(super) struct RelationRowsValue {
    pub(super) request: Request,
    pub(super) descriptor: ResourceRelationDescriptor,
    pub(super) source_id: String,
    pub(super) revision: String,
    pub(super) operation: String,
    pub(super) save_mode: String,
    pub(super) rows: Vec<RelationRowInput>,
}

impl RelationRowsValue {
    pub(super) fn new(
        request: Request,
        descriptor: ResourceRelationDescriptor,
        source_id: String,
        revision: String,
        operation: String,
        save_mode: String,
        rows: Vec<RelationRowInput>,
    ) -> Self {
        Self {
            request,
            descriptor,
            source_id,
            revision,
            operation,
            save_mode,
            rows,
        }
    }
}

/// The detached relation-rows draft supplied to the bound validators.
///
/// The draft remains user input until those validators accept it.
#[derive(Clone, Default)]
pub struct RelationRowsDraft {
    value: Option<Arc<RelationRowsValue>>,
}

impl RelationRowsDraft {
    pub(super) fn from_value(value: Arc<RelationRowsValue>) -> Self {
        Self { value: Some(value) }
    }

    pub(super) fn value(&self) -> Option<&Arc<RelationRowsValue>> {
        self.value.as_ref()
    }

    /// Whether this draft was admitted by the owner's decode stage.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value
            .as_ref()
            .is_some_and(|value| value.descriptor.is_valid())
    }

    /// The request whose cancellation and extensions scope this draft.
    #[must_use]
    pub fn request(&self) -> Option<&Request> {
        self.value.as_ref().map(|value| &value.request)
    }

    /// The checked relation descriptor, or the zero descriptor for an invalid draft.
    #[must_use]
    pub fn descriptor(&self) -> ResourceRelationDescriptor {
        self.value
            .as_ref()
            .map_or_else(ResourceRelationDescriptor::default, |value| {
                value.descriptor.clone()
            })
    }

    /// The source record identity, or empty for an invalid draft.
    #[must_use]
    pub fn source_id(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(String::new, |value| value.source_id.clone())
    }

    /// The source revision, or empty for an invalid draft.
    #[must_use]
    pub fn revision(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(String::new, |value| value.revision.clone())
    }

    /// The requested operation, or empty for an invalid draft.
    #[must_use]
    pub fn operation(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(String::new, |value| value.operation.clone())
    }

    /// The requested save mode, or empty for an invalid draft.
    #[must_use]
    pub fn save_mode(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(String::new, |value| value.save_mode.clone())
    }

    /// A detached copy of the draft rows, including each row's value map.
    #[must_use]
    pub fn rows(&self) -> Vec<RelationRowInput> {
        match self.value.as_ref() {
            Some(value) if value.descriptor.is_valid() => clone_relation_rows(&value.rows),
            _ => Vec::new(),
        }
    }
}

impl PartialEq for RelationRowsDraft {
    fn eq(&self, other: &Self) -> bool {
        match (self.value.as_ref(), other.value.as_ref()) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (Some(_), None) | (None, Some(_)) => false,
        }
    }
}

impl Eq for RelationRowsDraft {}

impl fmt::Debug for RelationRowsDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelationRowsDraft")
            .field("valid", &self.is_valid())
            .field("source_id", &self.source_id())
            .field("operation", &self.operation())
            .field("save_mode", &self.save_mode())
            .field("rows", &self.rows().len())
            .finish_non_exhaustive()
    }
}

/// One execution owner for a validated relation-rows value.
pub(super) struct RelationValidationExecution {
    owner: Arc<RelationRowsValue>,
    used: AtomicBool,
}

/// Rows accepted by the complete relation-validation sequence.
#[derive(Clone, Default)]
pub struct ValidatedRelationRows {
    value: Option<Arc<RelationRowsValue>>,
    execution: Option<Arc<RelationValidationExecution>>,
}

impl ValidatedRelationRows {
    pub(super) fn accept(value: Arc<RelationRowsValue>) -> Self {
        let execution = Arc::new(RelationValidationExecution {
            owner: Arc::clone(&value),
            used: AtomicBool::new(false),
        });
        Self {
            value: Some(value),
            execution: Some(execution),
        }
    }

    /// Whether this record carries an owner-constructed validated value.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let (Some(value), Some(execution)) = (self.value.as_ref(), self.execution.as_ref()) else {
            return false;
        };
        Arc::ptr_eq(value, &execution.owner)
            && RelationRowsDraft::from_value(Arc::clone(value)).is_valid()
    }

    /// The detached draft used by the validators.
    #[must_use]
    pub fn draft(&self) -> RelationRowsDraft {
        match self.value.as_ref() {
            Some(value) if self.is_valid() => {
                RelationRowsDraft::from_value(Arc::clone(value))
            }
            _ => RelationRowsDraft::default(),
        }
    }

    /// A detached copy of the validated rows.
    #[must_use]
    pub fn rows(&self) -> Vec<RelationRowInput> {
        self.draft().rows()
    }

    /// Consume this validation for one mutation preparation.
    ///
    /// A later write still rechecks current facts inside its constructor-bound
    /// transaction.
    #[must_use]
    pub fn claim(&self) -> Option<RelationRowsClaim> {
        if !self.is_valid() {
            return None;
        }
        let (Some(value), Some(execution)) =
            (self.value.as_ref(), self.execution.as_ref())
        else {
            return None;
        };
        if execution.used.swap(true, Ordering::SeqCst) {
            return None;
        }
        Some(RelationRowsClaim {
            value: Some(Arc::clone(value)),
            execution: Some(Arc::clone(execution)),
        })
    }
}

impl fmt::Debug for ValidatedRelationRows {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedRelationRows")
            .field("valid", &self.is_valid())
            .field("rows", &self.rows().len())
            .finish_non_exhaustive()
    }
}

/// A one-shot claim on one validated relation-rows execution owner.
#[derive(Clone, Default)]
pub struct RelationRowsClaim {
    value: Option<Arc<RelationRowsValue>>,
    execution: Option<Arc<RelationValidationExecution>>,
}

impl RelationRowsClaim {
    /// Whether this claim belongs to the consumed execution owner.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let (Some(value), Some(execution)) = (self.value.as_ref(), self.execution.as_ref()) else {
            return false;
        };
        Arc::ptr_eq(value, &execution.owner) && execution.used.load(Ordering::SeqCst)
    }

    /// The detached draft this claim was issued for.
    #[must_use]
    pub fn draft(&self) -> RelationRowsDraft {
        match self.value.as_ref() {
            Some(value) if self.is_valid() => {
                RelationRowsDraft::from_value(Arc::clone(value))
            }
            _ => RelationRowsDraft::default(),
        }
    }
}

impl fmt::Debug for RelationRowsClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelationRowsClaim")
            .field("valid", &self.is_valid())
            .finish_non_exhaustive()
    }
}

/// Rows the owner rejected.
pub struct RelationRowsRejected<E> {
    rows: Vec<RelationRowInput>,
    issues: Vec<RelationRowIssue>,
    cause: Option<RelationValidationCause<E>>,
}

impl<E> RelationRowsRejected<E> {
    pub(super) fn new(
        rows: Vec<RelationRowInput>,
        issues: Vec<RelationRowIssue>,
        cause: Option<RelationValidationCause<E>>,
    ) -> Self {
        Self {
            rows,
            issues,
            cause,
        }
    }

    /// A detached copy of the rejected rows.
    #[must_use]
    pub fn rows(&self) -> Vec<RelationRowInput> {
        clone_relation_rows(&self.rows)
    }

    /// The ordered rejection issues.
    #[must_use]
    pub fn issues(&self) -> Vec<RelationRowIssue> {
        self.issues.clone()
    }

    /// The rejection cause, when one was recorded.
    #[must_use]
    pub fn cause(&self) -> Option<&RelationValidationCause<E>> {
        self.cause.as_ref()
    }
}

impl<E> fmt::Debug for RelationRowsRejected<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelationRowsRejected")
            .field("rows", &self.rows.len())
            .field("issues", &self.issues)
            .field("has_cause", &self.cause.is_some())
            .finish_non_exhaustive()
    }
}

/// The closed relation-validation outcome.
pub enum RelationValidationOutcome<E> {
    /// The rows were accepted.
    Accepted(ValidatedRelationRows),
    /// The owner rejected the rows.
    Rejected(RelationRowsRejected<E>),
}

impl<E> RelationValidationOutcome<E> {
    /// The detached rows for either outcome.
    #[must_use]
    pub fn rows(&self) -> Vec<RelationRowInput> {
        match self {
            Self::Accepted(accepted) => accepted.rows(),
            Self::Rejected(rejected) => rejected.rows(),
        }
    }

    /// The rejection cause, absent for an accepted outcome.
    #[must_use]
    pub fn cause(&self) -> Option<&RelationValidationCause<E>> {
        match self {
            Self::Accepted(_) => None,
            Self::Rejected(rejected) => rejected.cause(),
        }
    }
}

impl<E> fmt::Debug for RelationValidationOutcome<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accepted(accepted) => formatter
                .debug_tuple("RelationValidationOutcome::Accepted")
                .field(accepted)
                .finish_non_exhaustive(),
            Self::Rejected(rejected) => formatter
                .debug_tuple("RelationValidationOutcome::Rejected")
                .field(rejected)
                .finish_non_exhaustive(),
        }
    }
}

/// Clone the rows and each row's value map.
pub(super) fn clone_relation_rows(rows: &[RelationRowInput]) -> Vec<RelationRowInput> {
    rows.to_vec()
}
