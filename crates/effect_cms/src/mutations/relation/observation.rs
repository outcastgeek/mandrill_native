//! Relation-mutation requests, consumer observations and classified outcomes.
//!
//! The before hook and executor return observations tied to the exact private
//! readiness/request they were issued for. The observation constructors preserve
//! the zero-observation branch, the optional rejection cause and the synthesized
//! stopped-without-cause diagnostic. The applied result keeps its publication
//! private and is valid only for the executed request.

use std::fmt;
use std::sync::Arc;

use crate::validation::{RelationRowIssue, RelationRowsDraft};

use super::execution::RelationMutationReadiness;

/// Why a relation mutation did not apply.
#[derive(Debug)]
pub enum RelationMutationCause<E> {
    /// The consumer's expected failure value.
    Consumer(E),
    /// An owner contract violation, with its message text.
    Message(String),
    /// The request's cancellation source was requested.
    ///
    /// The token is boolean: it does not establish whether the caller cancelled or a
    /// deadline expired.
    Cancelled,
}

impl<E: fmt::Display> fmt::Display for RelationMutationCause<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Consumer(error) => fmt::Display::fmt(error, formatter),
            Self::Message(message) => formatter.write_str(message.as_str()),
            Self::Cancelled => formatter.write_str("context canceled"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display + std::error::Error + 'static> std::error::Error
    for RelationMutationCause<E>
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Consumer(error) => Some(error),
            Self::Message(_) | Self::Cancelled => None,
        }
    }
}

/// A relation-mutation constructor refusal.
#[derive(Debug)]
pub struct RelationMutationContractError {
    message: String,
}

impl RelationMutationContractError {
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

impl fmt::Display for RelationMutationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message.as_str())
    }
}

impl std::error::Error for RelationMutationContractError {}

/// One before-hook observation.
pub struct RelationBeforeObservation<E> {
    value: Option<RelationBeforeObservationValue<E>>,
}

/// One before-hook observation payload.
pub(super) enum RelationBeforeObservationValue<E> {
    /// The hook accepted the draft.
    Accepted { draft: RelationRowsDraft },
    /// The hook rejected the draft with ordered issues.
    Rejected {
        draft: RelationRowsDraft,
        issues: Vec<RelationRowIssue>,
        cause: Option<RelationMutationCause<E>>,
    },
    /// The hook failed.
    Failed {
        draft: RelationRowsDraft,
        cause: RelationMutationCause<E>,
    },
}

impl<E> Default for RelationBeforeObservation<E> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<E> RelationBeforeObservation<E> {
    pub(super) fn into_value(self) -> Option<RelationBeforeObservationValue<E>> {
        self.value
    }

    /// Whether this observation belongs to exactly the supplied draft.
    pub(super) fn is_matching(&self, draft: &RelationRowsDraft) -> bool {
        match self.value.as_ref() {
            Some(RelationBeforeObservationValue::Accepted { draft: observed })
            | Some(RelationBeforeObservationValue::Rejected { draft: observed, .. })
            | Some(RelationBeforeObservationValue::Failed { draft: observed, .. }) => {
                observed == draft
            }
            None => false,
        }
    }
}

/// Record acceptance by the before hook.
#[must_use]
pub fn observe_relation_before_accepted<E>(
    draft: RelationRowsDraft,
) -> RelationBeforeObservation<E> {
    if !draft.is_valid() {
        return RelationBeforeObservation::default();
    }
    RelationBeforeObservation {
        value: Some(RelationBeforeObservationValue::Accepted { draft }),
    }
}

/// Record rejection by the before hook.
#[must_use]
pub fn observe_relation_before_rejected<E>(
    draft: RelationRowsDraft,
    issues: &[RelationRowIssue],
) -> RelationBeforeObservation<E> {
    if !draft.is_valid() {
        return RelationBeforeObservation::default();
    }
    RelationBeforeObservation {
        value: Some(RelationBeforeObservationValue::Rejected {
            draft,
            issues: issues.to_vec(),
            cause: None,
        }),
    }
}

/// Record failure by the before hook.
///
/// An absent cause receives the owner's stopped-without-cause diagnostic.
#[must_use]
pub fn observe_relation_before_failed<E>(
    draft: RelationRowsDraft,
    cause: Option<E>,
) -> RelationBeforeObservation<E> {
    if !draft.is_valid() {
        return RelationBeforeObservation::default();
    }
    let cause = match cause {
        Some(cause) => RelationMutationCause::Consumer(cause),
        None => RelationMutationCause::Message(
            "relation before hook stopped without cause".to_owned(),
        ),
    };
    RelationBeforeObservation {
        value: Some(RelationBeforeObservationValue::Failed { draft, cause }),
    }
}

/// The private request issued to one bound relation executor.
///
/// A request is not permanent write authority: the consumer's adapter rechecks
/// current facts in its own transaction.
#[derive(Clone, Default)]
pub struct RelationMutationRequest {
    readiness: Option<Arc<RelationMutationReadiness>>,
}

impl RelationMutationRequest {
    pub(super) fn from_readiness(readiness: Arc<RelationMutationReadiness>) -> Self {
        Self {
            readiness: Some(readiness),
        }
    }

    pub(super) fn readiness(&self) -> Option<&Arc<RelationMutationReadiness>> {
        self.readiness.as_ref()
    }

    pub(super) fn valid_readiness(&self) -> Option<&Arc<RelationMutationReadiness>> {
        if self.is_valid() {
            self.readiness.as_ref()
        } else {
            None
        }
    }

    /// Whether this request belongs to the consumed validated execution.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let Some(readiness) = self.readiness.as_ref() else {
            return false;
        };
        readiness.claim.is_valid() && readiness.execution.before_started.load(
            std::sync::atomic::Ordering::SeqCst,
        )
    }

    /// The detached draft for this request.
    #[must_use]
    pub fn draft(&self) -> RelationRowsDraft {
        match self.readiness.as_ref() {
            Some(readiness) if self.is_valid() => readiness.claim.draft(),
            _ => RelationRowsDraft::default(),
        }
    }
}

impl fmt::Debug for RelationMutationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelationMutationRequest")
            .field("valid", &self.is_valid())
            .finish_non_exhaustive()
    }
}

/// One executor observation.
pub struct RelationMutationObservation<E> {
    value: Option<RelationMutationObservationValue<E>>,
}

/// One executor observation payload.
pub(super) enum RelationMutationObservationValue<E> {
    /// The executor applied the mutation and established its resulting revision.
    Applied {
        readiness: Arc<RelationMutationReadiness>,
        revision: String,
    },
    /// The executor rejected the rows with ordered issues.
    Rejected {
        readiness: Arc<RelationMutationReadiness>,
        issues: Vec<RelationRowIssue>,
        cause: Option<RelationMutationCause<E>>,
    },
    /// The executor failed.
    Failed {
        readiness: Arc<RelationMutationReadiness>,
        cause: RelationMutationCause<E>,
    },
    /// The executor could not resolve the mutation outcome.
    Unresolved {
        readiness: Arc<RelationMutationReadiness>,
        cause: RelationMutationCause<E>,
    },
}

impl<E> Default for RelationMutationObservation<E> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<E> RelationMutationObservation<E> {
    pub(super) fn into_value(self) -> Option<RelationMutationObservationValue<E>> {
        self.value
    }
}

/// Record an applied mutation after the executor established commit/application.
#[must_use]
pub fn observe_relation_mutation_applied<E>(
    request: &RelationMutationRequest,
    revision: impl Into<String>,
) -> RelationMutationObservation<E> {
    let Some(readiness) = request.valid_readiness() else {
        return RelationMutationObservation::default();
    };
    RelationMutationObservation {
        value: Some(RelationMutationObservationValue::Applied {
            readiness: Arc::clone(readiness),
            revision: revision.into(),
        }),
    }
}

/// Record a rejected mutation.
#[must_use]
pub fn observe_relation_mutation_rejected<E>(
    request: &RelationMutationRequest,
    issues: &[RelationRowIssue],
    cause: Option<E>,
) -> RelationMutationObservation<E> {
    let Some(readiness) = request.valid_readiness() else {
        return RelationMutationObservation::default();
    };
    RelationMutationObservation {
        value: Some(RelationMutationObservationValue::Rejected {
            readiness: Arc::clone(readiness),
            issues: issues.to_vec(),
            cause: cause.map(RelationMutationCause::Consumer),
        }),
    }
}

/// Record a failed mutation.
#[must_use]
pub fn observe_relation_mutation_failed<E>(
    request: &RelationMutationRequest,
    cause: Option<E>,
) -> RelationMutationObservation<E> {
    relation_mutation_stop(request, cause, false)
}

/// Record an unresolved mutation.
#[must_use]
pub fn observe_relation_mutation_unresolved<E>(
    request: &RelationMutationRequest,
    cause: Option<E>,
) -> RelationMutationObservation<E> {
    relation_mutation_stop(request, cause, true)
}

fn relation_mutation_stop<E>(
    request: &RelationMutationRequest,
    cause: Option<E>,
    unresolved: bool,
) -> RelationMutationObservation<E> {
    let Some(readiness) = request.valid_readiness() else {
        return RelationMutationObservation::default();
    };
    let cause = match cause {
        Some(cause) => RelationMutationCause::Consumer(cause),
        None => RelationMutationCause::Message(
            "relation mutation stopped without a diagnostic cause".to_owned(),
        ),
    };
    let value = if unresolved {
        RelationMutationObservationValue::Unresolved {
            readiness: Arc::clone(readiness),
            cause,
        }
    } else {
        RelationMutationObservationValue::Failed {
            readiness: Arc::clone(readiness),
            cause,
        }
    };
    RelationMutationObservation { value: Some(value) }
}

/// An applied relation mutation.
#[derive(Clone, Default)]
pub struct RelationMutationApplied {
    publication: Option<Arc<RelationMutationPublication>>,
}

#[derive(Clone)]
struct RelationMutationPublication {
    request: RelationMutationRequest,
    revision: String,
}

impl RelationMutationApplied {
    pub(super) fn new(request: RelationMutationRequest, revision: String) -> Self {
        Self {
            publication: Some(Arc::new(RelationMutationPublication { request, revision })),
        }
    }

    /// Whether this result belongs to the executed validated request.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let Some(publication) = self.publication.as_ref() else {
            return false;
        };
        if !publication.request.is_valid() {
            return false;
        }
        publication
            .request
            .readiness()
            .is_some_and(|readiness| {
                readiness
                    .execution
                    .execute_started
                    .load(std::sync::atomic::Ordering::SeqCst)
            })
    }

    /// The applied request's detached draft.
    #[must_use]
    pub fn draft(&self) -> RelationRowsDraft {
        match self.publication.as_ref() {
            Some(publication) if self.is_valid() => publication.request.draft(),
            _ => RelationRowsDraft::default(),
        }
    }

    /// The executor's resulting revision, empty for an invalid result.
    #[must_use]
    pub fn revision(&self) -> String {
        self.publication
            .as_ref()
            .filter(|_| self.is_valid())
            .map_or_else(String::new, |publication| publication.revision.clone())
    }
}

impl fmt::Debug for RelationMutationApplied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelationMutationApplied")
            .field("valid", &self.is_valid())
            .field("revision", &self.revision())
            .finish_non_exhaustive()
    }
}

/// A rejected relation mutation.
pub struct RelationMutationRejected<E> {
    draft: RelationRowsDraft,
    issues: Vec<RelationRowIssue>,
    cause: Option<RelationMutationCause<E>>,
}

impl<E> RelationMutationRejected<E> {
    pub(super) fn new(
        draft: RelationRowsDraft,
        issues: Vec<RelationRowIssue>,
        cause: Option<RelationMutationCause<E>>,
    ) -> Self {
        Self {
            draft,
            issues,
            cause,
        }
    }

    /// The rejected draft.
    #[must_use]
    pub fn draft(&self) -> RelationRowsDraft {
        self.draft.clone()
    }

    /// The ordered rejection issues.
    #[must_use]
    pub fn issues(&self) -> Vec<RelationRowIssue> {
        self.issues.clone()
    }

    /// The rejection cause, when one was recorded.
    #[must_use]
    pub fn cause(&self) -> Option<&RelationMutationCause<E>> {
        self.cause.as_ref()
    }
}

/// A failed relation mutation.
pub struct RelationMutationFailed<E> {
    draft: RelationRowsDraft,
    cause: RelationMutationCause<E>,
}

impl<E> RelationMutationFailed<E> {
    pub(super) fn new(draft: RelationRowsDraft, cause: RelationMutationCause<E>) -> Self {
        Self { draft, cause }
    }

    /// The detached draft observed at failure.
    #[must_use]
    pub fn draft(&self) -> RelationRowsDraft {
        self.draft.clone()
    }

    /// The failure cause.
    #[must_use]
    pub fn cause(&self) -> &RelationMutationCause<E> {
        &self.cause
    }
}

/// An unresolved relation mutation.
pub struct RelationMutationUnresolved<E> {
    draft: RelationRowsDraft,
    cause: RelationMutationCause<E>,
}

impl<E> RelationMutationUnresolved<E> {
    pub(super) fn new(draft: RelationRowsDraft, cause: RelationMutationCause<E>) -> Self {
        Self { draft, cause }
    }

    /// The detached draft observed as unresolved.
    #[must_use]
    pub fn draft(&self) -> RelationRowsDraft {
        self.draft.clone()
    }

    /// The unresolved cause.
    #[must_use]
    pub fn cause(&self) -> &RelationMutationCause<E> {
        &self.cause
    }
}

/// The closed relation-mutation outcome.
pub enum RelationMutationOutcome<E> {
    /// The mutation was applied.
    Applied(RelationMutationApplied),
    /// The mutation was rejected.
    Rejected(RelationMutationRejected<E>),
    /// The mutation failed.
    Failed(RelationMutationFailed<E>),
    /// The mutation outcome could not be resolved.
    Unresolved(RelationMutationUnresolved<E>),
}

impl<E> RelationMutationOutcome<E> {
    /// The detached draft for any outcome.
    #[must_use]
    pub fn draft(&self) -> RelationRowsDraft {
        match self {
            Self::Applied(applied) => applied.draft(),
            Self::Rejected(rejected) => rejected.draft(),
            Self::Failed(failed) => failed.draft(),
            Self::Unresolved(unresolved) => unresolved.draft(),
        }
    }

    /// The failure/rejection cause, absent for an applied outcome.
    #[must_use]
    pub fn cause(&self) -> Option<&RelationMutationCause<E>> {
        match self {
            Self::Applied(_) => None,
            Self::Rejected(rejected) => rejected.cause(),
            Self::Failed(failed) => Some(failed.cause()),
            Self::Unresolved(unresolved) => Some(unresolved.cause()),
        }
    }
}
