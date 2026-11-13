//! Relation-mutation input, consumer capabilities and service construction.
//!
//! The binding is constructor-bound to one source kind and relation name and stores
//! the required before hook and executor. The default service is the unconstructed
//! zero service: it publishes the zero outcome (`None`) and invokes no callback.
//! A production service is constructed only with an admitted binding.

use std::fmt;
use std::sync::Arc;

use effect::Effect;

use crate::definitions::ResourceKind;
use crate::validation::ValidatedRelationRows;

use super::observation::{
    RelationBeforeObservation, RelationMutationContractError,
    RelationMutationObservation, RelationMutationRequest,
};

/// The decoded input for one relation mutation.
#[derive(Clone, Default)]
pub struct RelationMutationInput {
    /// Source resource kind.
    pub source: ResourceKind,
    /// Relation declaration name.
    pub relation: String,
    /// Source record identity.
    pub source_id: String,
    /// Source revision.
    pub revision: String,
    /// The owner-issued validated rows.
    pub rows: ValidatedRelationRows,
}

impl fmt::Debug for RelationMutationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelationMutationInput")
            .field("source", &self.source)
            .field("relation", &self.relation)
            .field("source_id", &self.source_id)
            .field("revision", &self.revision)
            .field("rows_valid", &self.rows.is_valid())
            .finish_non_exhaustive()
    }
}

/// The consumer's relation before hook.
pub trait RelationBefore<E, R>: Send + Sync + 'static
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Observe one validated draft before execution.
    fn before(&self, draft: crate::validation::RelationRowsDraft)
    -> Effect<RelationBeforeObservation<E>, E, R>;
}

impl<E, R, F> RelationBefore<E, R> for F
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Fn(crate::validation::RelationRowsDraft) -> Effect<RelationBeforeObservation<E>, E, R>
        + Send
        + Sync
        + 'static,
{
    fn before(
        &self,
        draft: crate::validation::RelationRowsDraft,
    ) -> Effect<RelationBeforeObservation<E>, E, R> {
        self(draft)
    }
}

/// The consumer's before-hook capability.
pub type RelationBeforeFunc<E, R> = Arc<dyn RelationBefore<E, R>>;

/// The consumer's relation executor capability.
pub type RelationExecuteFunc<E, R> = Arc<
    dyn Fn(RelationMutationRequest) -> Effect<RelationMutationObservation<E>, E, R>
        + Send
        + Sync
        + 'static,
>;

/// One constructor-bound relation-mutation capability.
pub struct RelationMutationBinding<E: Send + 'static, R: Send + Sync + 'static> {
    value: Option<Arc<RelationMutationBindingValue<E, R>>>,
}

/// The admitted binding payload.
pub(super) struct RelationMutationBindingValue<E: Send + 'static, R: Send + Sync + 'static> {
    source: ResourceKind,
    relation: String,
    before: RelationBeforeFunc<E, R>,
    execute: RelationExecuteFunc<E, R>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default
    for RelationMutationBinding<E, R>
{
    fn default() -> Self {
        Self { value: None }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone
    for RelationMutationBinding<E, R>
{
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

/// Bind one relation's before hook and executor.
///
/// # Errors
///
/// Returns [`RelationMutationContractError`] when the source or relation name is
/// blank.
pub fn new_relation_mutation_binding<E: Send + 'static, R: Send + Sync + 'static>(
    source: ResourceKind,
    relation: impl Into<String>,
    before: RelationBeforeFunc<E, R>,
    execute: RelationExecuteFunc<E, R>,
) -> Result<RelationMutationBinding<E, R>, RelationMutationContractError> {
    let relation = relation.into();
    if source.as_str().is_empty() || relation.is_empty() {
        return Err(RelationMutationContractError::new(
            "relation mutation: source, relation, before hook and executor are required",
        ));
    }
    Ok(RelationMutationBinding {
        value: Some(Arc::new(RelationMutationBindingValue {
            source,
            relation,
            before,
            execute,
        })),
    })
}

impl<E: Send + 'static, R: Send + Sync + 'static> RelationMutationBinding<E, R> {
    /// Whether this binding was admitted by its constructor.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value.is_some()
    }

    pub(super) fn value(&self) -> Option<&Arc<RelationMutationBindingValue<E, R>>> {
        self.value.as_ref()
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> RelationMutationBindingValue<E, R> {
    pub(super) fn source(&self) -> &ResourceKind {
        &self.source
    }

    pub(super) fn relation(&self) -> &str {
        self.relation.as_str()
    }

    pub(super) fn before(&self) -> &RelationBeforeFunc<E, R> {
        &self.before
    }

    pub(super) fn execute(&self) -> &RelationExecuteFunc<E, R> {
        &self.execute
    }
}

/// The relation-mutation service over an admitted binding.
///
/// The default value is the unconstructed zero service: it publishes the zero
/// outcome (`None`) and invokes no callback.
pub struct RelationMutationService<E: Send + 'static, R: Send + Sync + 'static> {
    binding: Option<RelationMutationBinding<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default
    for RelationMutationService<E, R>
{
    fn default() -> Self {
        Self { binding: None }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> RelationMutationService<E, R> {
    /// Construct the service.
    ///
    /// # Errors
    ///
    /// Returns [`RelationMutationContractError`] when the binding was not admitted.
    pub fn new(
        binding: RelationMutationBinding<E, R>,
    ) -> Result<Self, RelationMutationContractError> {
        if !binding.is_valid() {
            return Err(RelationMutationContractError::new(
                "relation mutation: constructor-bound capabilities are required",
            ));
        }
        Ok(Self {
            binding: Some(binding),
        })
    }

    pub(super) fn binding_value(
        &self,
    ) -> Option<&Arc<RelationMutationBindingValue<E, R>>> {
        self.binding.as_ref().and_then(|binding| binding.value())
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RelationMutationService<E, R> {
    fn clone(&self) -> Self {
        Self {
            binding: self.binding.clone(),
        }
    }
}

/// Construct the relation-mutation service.
///
/// # Errors
///
/// Returns [`RelationMutationContractError`] under the same conditions as
/// [`RelationMutationService::new`].
pub fn new_relation_mutation_service<E: Send + 'static, R: Send + Sync + 'static>(
    binding: RelationMutationBinding<E, R>,
) -> Result<RelationMutationService<E, R>, RelationMutationContractError> {
    RelationMutationService::new(binding)
}
