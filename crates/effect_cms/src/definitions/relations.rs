//! Relation declarations and their checked descriptor.
//!
//! A declared relation is caller-owned data and is deliberately *detached*: the
//! descriptor constructor, definition normalization and every registry lookup copy
//! the declaration, its key components, its finite targets and their keys, its
//! through steps, its operations, its editor columns and its bounds. Mutating the
//! caller's declaration afterwards therefore cannot alter an admitted descriptor,
//! and mutating a returned declaration cannot alter the resource.
//!
//! The descriptor itself is a private checked representation. Only
//! [`new_resource_relation_descriptor`] and [`resolve_resource_relation`] admit
//! one, there is no public field, and the zero descriptor grants no operation.

use std::fmt;
use std::sync::Arc;

use super::model::ResourceDefinition;
use super::relation_validation::validate_relation_definition;
use super::vocabulary::{ResourceFieldKey, ResourceKind};

/// One ordered scalar key component.
#[derive(Clone, Debug, Default)]
pub struct ResourceRelationKeyDefinition {
    /// Field on the relation's source resource.
    pub source: ResourceFieldKey,
    /// Field on the relation's target resource.
    pub target: ResourceFieldKey,
    /// Declared scalar type, which the owner admits only for `string`, `int` and
    /// `bool`.
    pub r#type: String,
}

/// One finite tagged target mapping.
#[derive(Clone, Debug, Default)]
pub struct ResourceRelationTargetDefinition {
    /// Declared tag value.
    pub scope: String,
    /// Tag that selects this target.
    pub tag: String,
    /// Target resource kind.
    pub target: ResourceKind,
    /// Key components joining the source to this target.
    pub keys: Vec<ResourceRelationKeyDefinition>,
}

/// Presentation and explicit row intent for a relation editor.
#[derive(Clone, Debug, Default)]
pub struct ResourceRelationEditorDefinition {
    /// Editor control.
    pub control: String,
    /// Label query.
    pub label_query: String,
    /// Locale key.
    pub locale_key: String,
    /// Save mode, which the owner admits only as `patch` or `replacement`.
    pub save_mode: String,
    /// Explicit removal operation, which must be one of the declared operations.
    pub removal_operation: String,
    /// Field used as the editor label.
    pub label_field: ResourceFieldKey,
    /// Field used to order rows.
    pub order_field: ResourceFieldKey,
    /// Editor columns, in order.
    pub columns: Vec<ResourceFieldKey>,
    /// Optional minimum number of rows.
    pub minimum: Option<i64>,
    /// Optional maximum number of rows.
    pub maximum: Option<i64>,
}

/// The caller-owned declaration of one relation.
#[derive(Clone, Debug, Default)]
pub struct ResourceRelationDefinition {
    /// Relation name, matched after trimming.
    pub name: String,
    /// Declared cardinality, admitted only from the declared cardinality set.
    pub cardinality: String,
    /// Inverse relation name.
    pub inverse_of: String,
    /// Target-delete policy.
    pub on_target_delete: String,
    /// Query binding.
    pub query_binding: String,
    /// Relation scope.
    pub scope: String,
    /// Single target resource kind.
    pub target: ResourceKind,
    /// Discriminator field for finite tagged targets.
    pub discriminator: ResourceFieldKey,
    /// Finite tagged targets.
    pub targets: Vec<ResourceRelationTargetDefinition>,
    /// Whether the relation is optional.
    pub optional: bool,
    /// Whether the relation owns its targets.
    pub owned: bool,
    /// Key components.
    pub keys: Vec<ResourceRelationKeyDefinition>,
    /// Through path steps.
    pub through: Vec<String>,
    /// Declared operations, admitted only from the declared operation set.
    pub operations: Vec<String>,
    /// Editor declaration.
    pub editor: ResourceRelationEditorDefinition,
}

/// A checked, immutable relation declaration.
///
/// The representation is private. A descriptor exists only because its checked
/// constructor admitted it, and an invalid (zero) descriptor grants no operation.
#[derive(Clone, Debug, Default)]
pub struct ResourceRelationDescriptor {
    value: Option<Arc<ResourceRelationValue>>,
}

/// The admitted declaration and the source it is bound to.
#[derive(Debug)]
struct ResourceRelationValue {
    source: ResourceKind,
    definition: ResourceRelationDefinition,
}

impl ResourceRelationDescriptor {
    /// Whether a checked constructor admitted this descriptor.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value.is_some()
    }

    /// The bound source resource identity, absent for the zero descriptor.
    #[must_use]
    pub fn source_kind(&self) -> Option<&ResourceKind> {
        self.value.as_ref().map(|value| &value.source)
    }

    /// A detached copy of the admitted declaration, absent for the zero
    /// descriptor.
    #[must_use]
    pub fn definition(&self) -> Option<ResourceRelationDefinition> {
        self.value
            .as_ref()
            .map(|value| clone_relation_definition(&value.definition))
    }

    pub(crate) fn definition_ref(&self) -> Option<&ResourceRelationDefinition> {
        self.value.as_ref().map(|value| &value.definition)
    }

    /// Whether the declaration explicitly allows `operation`.
    ///
    /// The comparison is exact: the value is not trimmed or otherwise
    /// normalize a declared operation.
    #[must_use]
    pub fn allows(&self, operation: &str) -> bool {
        self.value.as_ref().is_some_and(|value| {
            value
                .definition
                .operations
                .iter()
                .any(|allowed| allowed.as_str() == operation)
        })
    }
}

/// Admit one detached relation declaration for its source resource.
///
/// # Errors
///
/// Reports validation failures in their exact precedence, with the refusal message.
pub fn new_resource_relation_descriptor(
    source: ResourceKind,
    definition: ResourceRelationDefinition,
) -> Result<ResourceRelationDescriptor, RelationError> {
    let source = ResourceKind::new(source.as_str().trim());
    let mut definition = clone_relation_definition(&definition);
    definition.name = definition.name.trim().to_owned();
    definition.cardinality = definition.cardinality.trim().to_owned();
    validate_relation_definition(&source, &definition)?;
    Ok(ResourceRelationDescriptor {
        value: Some(Arc::new(ResourceRelationValue { source, definition })),
    })
}

/// Admit the named relation declared by `resource`.
///
/// Names are matched after trimming. A duplicated declared name is rejected
/// before the matched declaration is validated.
///
/// # Errors
///
/// Reports a duplicated or unknown relation, or the validation failure of the
/// matched declaration.
pub fn resolve_resource_relation(
    resource: &ResourceDefinition,
    name: &str,
) -> Result<ResourceRelationDescriptor, RelationError> {
    let wanted = name.trim();
    let mut found: Option<&ResourceRelationDefinition> = None;
    for definition in &resource.relations {
        if definition.name.trim() == wanted {
            if found.is_some() {
                return Err(RelationError::new(format!(
                    "resource {}: relation {} is duplicated",
                    resource.kind.as_str(),
                    name
                )));
            }
            found = Some(definition);
        }
    }
    let Some(found) = found else {
        return Err(RelationError::new(format!(
            "resource {}: unknown relation {}",
            resource.kind.as_str(),
            name
        )));
    };
    new_resource_relation_descriptor(resource.kind.clone(), found.clone())
}

/// A relation declaration the owner refused, with the refusal message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationError {
    message: String,
}

impl RelationError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The message text for this refusal.
    #[must_use]
    pub fn message(&self) -> &str {
        self.message.as_str()
    }
}

impl fmt::Display for RelationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message.as_str())
    }
}

impl std::error::Error for RelationError {}

/// Detached copy of one relation declaration.
///
/// A Rust value copy copies the declaration's own `Vec`/`String`/`Option` fields,
/// so the copy is detached from the original; the shared collection handles used
/// elsewhere in the definition model are not part of a relation declaration.
pub(crate) fn clone_relation_definition(
    value: &ResourceRelationDefinition,
) -> ResourceRelationDefinition {
    value.clone()
}

/// Detached copy of a declared relation collection.
pub(crate) fn clone_relations(
    values: &[ResourceRelationDefinition],
) -> Vec<ResourceRelationDefinition> {
    values.to_vec()
}
