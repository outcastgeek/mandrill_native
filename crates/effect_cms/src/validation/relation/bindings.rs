//! Relation-validation binding, observation and service construction.
//!
//! The binding is constructor-bound to one source kind and relation name and stores
//! the consumer's scoped preparation observation and application validator. Both
//! callbacks are required capabilities; a nil callback is unrepresentable here.
//! `Default` still represents the unconstructed zero binding, which the service
//! constructor refuses.
//!
//! An observation records only the draft it was constructed for: the validation
//! workflow rejects a zero observation, an observation for a different draft
//! instance, or an observation that did not classify the preparation. The
//! observation constructors are the only publication path.

use std::fmt;
use std::sync::Arc;

use effect::Effect;

use super::rows::{
    RelationRowsDraft, RelationRowIssue, RelationRowsValue, RelationValidationCause,
    RelationValidationContractError,
};
use crate::definitions::{Registry, ResourceKind};

/// A scoped preparation observation returned by the bound row-check callback.
pub struct RelationRowsObservation<E> {
    value: Option<RelationRowsObservationValue<E>>,
}

/// One observation payload.
pub(super) enum RelationRowsObservationValue<E> {
    /// The bound adapter prepared the rows; membership and final bounds remain its
    /// own responsibility.
    Prepared {
        draft: Arc<RelationRowsValue>,
    },
    /// The bound adapter rejected the rows with ordered issues and an optional
    /// cause.
    Rejected {
        draft: Arc<RelationRowsValue>,
        issues: Vec<RelationRowIssue>,
        cause: Option<RelationValidationCause<E>>,
    },
}

impl<E> Default for RelationRowsObservation<E> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<E> RelationRowsObservation<E> {
    pub(super) fn into_value(self) -> Option<RelationRowsObservationValue<E>> {
        self.value
    }

    /// Whether this observation was constructed for exactly the supplied draft.
    pub(super) fn is_valid_for(&self, draft: &RelationRowsDraft) -> bool {
        let Some(observed) = self.value.as_ref() else {
            return false;
        };
        let Some(draft_value) = draft.value() else {
            return false;
        };
        let observed_draft = match observed {
            RelationRowsObservationValue::Prepared { draft }
            | RelationRowsObservationValue::Rejected { draft, .. } => draft,
        };
        Arc::ptr_eq(observed_draft, draft_value)
    }
}

/// Record that the bound preparation adapter accepted the draft.
///
/// # Errors
///
/// Returns [`RelationValidationContractError`] when the draft was not admitted.
pub fn observe_relation_rows_validated<E>(
    draft: RelationRowsDraft,
) -> Result<RelationRowsObservation<E>, RelationValidationContractError> {
    let Some(value) = draft.value() else {
        return Err(RelationValidationContractError::new(
            "relation validation observation: invalid draft",
        ));
    };
    Ok(RelationRowsObservation {
        value: Some(RelationRowsObservationValue::Prepared {
            draft: Arc::clone(value),
        }),
    })
}

/// Record that the bound preparation adapter rejected the draft.
///
/// An invalid draft yields the zero observation. An empty issue set without a cause
/// receives the owner's own stopped-without-diagnostic cause.
#[must_use]
pub fn observe_relation_rows_rejected<E>(
    draft: RelationRowsDraft,
    issues: Vec<RelationRowIssue>,
    cause: Option<E>,
) -> RelationRowsObservation<E> {
    let Some(value) = draft.value() else {
        return RelationRowsObservation::default();
    };
    let cause = match cause {
        Some(cause) => Some(RelationValidationCause::Consumer(cause)),
        None if issues.is_empty() => Some(RelationValidationCause::Message(
            "relation validation stopped without issues or cause".to_owned(),
        )),
        None => None,
    };
    RelationRowsObservation {
        value: Some(RelationRowsObservationValue::Rejected {
            draft: Arc::clone(value),
            issues,
            cause,
        }),
    }
}

/// The consumer's scoped preparation observation callback.
pub type RelationRowsCheckFunc<E, R> = Arc<
    dyn Fn(RelationRowsDraft) -> Effect<RelationRowsObservation<E>, E, R>
        + Send
        + Sync
        + 'static,
>;

/// The result returned by one application validator.
pub struct RelationRowsApplicationResult<E> {
    /// Ordered application issues.
    pub issues: Vec<RelationRowIssue>,
    /// The application failure cause, when one was recorded.
    pub cause: Option<E>,
}

impl<E> Default for RelationRowsApplicationResult<E> {
    fn default() -> Self {
        Self {
            issues: Vec::new(),
            cause: None,
        }
    }
}

impl<E> fmt::Debug for RelationRowsApplicationResult<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelationRowsApplicationResult")
            .field("issues", &self.issues)
            .field("has_cause", &self.cause.is_some())
            .finish_non_exhaustive()
    }
}

/// Validates the prepared rows against the application's own facts.
///
/// The function-callback form is accepted through the blanket implementation below,
/// so a consumer can supply either a named validator or an `Arc`-shared closure.
pub trait RelationRowsApplicationValidator<E, R>: Send + Sync + 'static
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Validate one prepared draft.
    fn validate(
        &self,
        draft: RelationRowsDraft,
    ) -> Effect<RelationRowsApplicationResult<E>, E, R>;
}

impl<E, R, F> RelationRowsApplicationValidator<E, R> for F
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(RelationRowsDraft) -> Effect<RelationRowsApplicationResult<E>, E, R>
        + Send
        + Sync
        + 'static,
{
    fn validate(
        &self,
        draft: RelationRowsDraft,
    ) -> Effect<RelationRowsApplicationResult<E>, E, R> {
        self(draft)
    }
}

/// The consumer's application validator capability.
pub type RelationRowsApplicationValidateFunc<E, R> =
    Arc<dyn RelationRowsApplicationValidator<E, R>>;

/// One constructor-bound relation-validation capability.
pub struct RelationValidationBinding<E: Send + 'static, R: Send + Sync + 'static> {
    value: Option<Arc<RelationValidationBindingValue<E, R>>>,
}

/// The admitted binding payload.
pub(super) struct RelationValidationBindingValue<E: Send + 'static, R: Send + Sync + 'static> {
    source: ResourceKind,
    relation: String,
    check: RelationRowsCheckFunc<E, R>,
    application: RelationRowsApplicationValidateFunc<E, R>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default
    for RelationValidationBinding<E, R>
{
    fn default() -> Self {
        Self { value: None }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone
    for RelationValidationBinding<E, R>
{
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

/// Bind one relation's scoped preparation check and application validator.
///
/// # Errors
///
/// Returns [`RelationValidationContractError`] when the source or relation name is
/// blank.
pub fn new_relation_validation_binding<E: Send + 'static, R: Send + Sync + 'static>(
    source: ResourceKind,
    relation: impl Into<String>,
    check: RelationRowsCheckFunc<E, R>,
    application: RelationRowsApplicationValidateFunc<E, R>,
) -> Result<RelationValidationBinding<E, R>, RelationValidationContractError> {
    let relation = relation.into();
    if source.as_str().is_empty() || relation.is_empty() {
        return Err(RelationValidationContractError::new(
            "relation validation: source, relation, scoped checks and application validator are required",
        ));
    }
    Ok(RelationValidationBinding {
        value: Some(Arc::new(RelationValidationBindingValue {
            source,
            relation,
            check,
            application,
        })),
    })
}

impl<E: Send + 'static, R: Send + Sync + 'static> RelationValidationBinding<E, R> {
    /// Whether this binding was admitted by its constructor.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value.is_some()
    }

    pub(super) fn value(&self) -> Option<&Arc<RelationValidationBindingValue<E, R>>> {
        self.value.as_ref()
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> RelationValidationBindingValue<E, R> {
    pub(super) fn source(&self) -> &ResourceKind {
        &self.source
    }

    pub(super) fn relation(&self) -> &str {
        self.relation.as_str()
    }

    pub(super) fn check(&self) -> &RelationRowsCheckFunc<E, R> {
        &self.check
    }

    pub(super) fn application(&self) -> &RelationRowsApplicationValidateFunc<E, R> {
        &self.application
    }
}

/// The relation-validation service over a required registry and admitted binding.
///
/// The default value is the unconstructed zero service: it publishes the zero
/// outcome (`None`) and invokes no callback. A production service is constructed
/// only with its required registry and admitted binding.
pub struct RelationValidationService<E: Send + 'static, R: Send + Sync + 'static> {
    resources: Option<Arc<dyn Registry + Send + Sync>>,
    binding: Option<RelationValidationBinding<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default
    for RelationValidationService<E, R>
{
    fn default() -> Self {
        Self {
            resources: None,
            binding: None,
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> RelationValidationService<E, R> {
    /// Construct the service.
    ///
    /// # Errors
    ///
    /// Returns [`RelationValidationContractError`] when the binding was not admitted.
    pub fn new(
        resources: Arc<dyn Registry + Send + Sync>,
        binding: RelationValidationBinding<E, R>,
    ) -> Result<Self, RelationValidationContractError> {
        if !binding.is_valid() {
            return Err(RelationValidationContractError::new(
                "relation validation: definitions and bound checks are required",
            ));
        }
        Ok(Self {
            resources: Some(resources),
            binding: Some(binding),
        })
    }

    pub(super) fn resources(&self) -> Option<&Arc<dyn Registry + Send + Sync>> {
        self.resources.as_ref()
    }

    pub(super) fn binding_value(
        &self,
    ) -> Option<&Arc<RelationValidationBindingValue<E, R>>> {
        self.binding.as_ref().and_then(|binding| binding.value())
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RelationValidationService<E, R> {
    fn clone(&self) -> Self {
        Self {
            resources: self.resources.clone(),
            binding: self.binding.clone(),
        }
    }
}

/// Construct the relation-validation service.
///
/// # Errors
///
/// Returns [`RelationValidationContractError`] under the same conditions as
/// [`RelationValidationService::new`].
pub fn new_relation_validation_service<E: Send + 'static, R: Send + Sync + 'static>(
    resources: Arc<dyn Registry + Send + Sync>,
    binding: RelationValidationBinding<E, R>,
) -> Result<RelationValidationService<E, R>, RelationValidationContractError> {
    RelationValidationService::new(resources, binding)
}
