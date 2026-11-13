//! Relation-row decoding and admission checks.
//!
//! The decode stage clones the supplied rows, resolves the registered source
//! definition and the named relation declaration, checks the constructor-bound
//! source/relation identity, the operation declaration, the save mode, the distinct
//! raw row keys, each row's target identity versus its explicit operation, and the
//! reorder prerequisites. A refusal carries the cloned rows, one ordered row issue
//! with its source code, and the cause.
//!
//! A service always carries its required registry and an admitted binding; those
//! nil states are unrepresentable here. The absent request remains a real input
//! branch.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::bindings::RelationValidationService;
use super::rows::{
    RelationRowIssue, RelationRowsRejected, RelationRowsValue, RelationValidationCause,
    RelationValidationInput, clone_relation_rows,
};
use crate::definitions::resolve_resource_relation;

/// The decoded relation-rows stage, consumed by the validation workflow.
pub(super) struct DecodedRelationRows {
    pub(super) value: Arc<RelationRowsValue>,
}

/// Decode and admit one relation-validation input.
pub(super) fn decode_relation_rows<E, R>(
    service: &RelationValidationService<E, R>,
    input: RelationValidationInput,
) -> Result<DecodedRelationRows, RelationRowsRejected<E>>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let rows = clone_relation_rows(&input.rows);
    let reject = |code: &str, cause: RelationValidationCause<E>| {
        RelationRowsRejected::new(
            clone_relation_rows(&rows),
            vec![RelationRowIssue {
                code: code.to_owned(),
                ..RelationRowIssue::default()
            }],
            Some(cause),
        )
    };

    let Some(request) = input.request.as_ref() else {
        return Err(reject(
            "invalid_request",
            RelationValidationCause::Message(
                "relation validation requires its request and bound capabilities".to_owned(),
            ),
        ));
    };
    if request.cancellation().is_requested() {
        return Err(reject("cancelled", RelationValidationCause::Cancelled));
    }
    let Some(resources) = service.resources() else {
        return Err(reject(
            "invalid_request",
            RelationValidationCause::Message(
                "relation validation requires its request and bound capabilities".to_owned(),
            ),
        ));
    };
    let Some(binding) = service.binding_value() else {
        return Err(reject(
            "invalid_request",
            RelationValidationCause::Message(
                "relation validation requires its request and bound capabilities".to_owned(),
            ),
        ));
    };

    let resource = match resources.definition(&input.source) {
        Ok(resource) => resource,
        Err(error) => {
            return Err(reject(
                "invalid_relation",
                RelationValidationCause::Definition(error),
            ));
        }
    };
    let descriptor = match resolve_resource_relation(&resource, input.relation.as_str()) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            return Err(reject(
                "invalid_relation",
                RelationValidationCause::Relation(error),
            ));
        }
    };
    let Some(definition) = descriptor.definition() else {
        return Err(reject(
            "invalid_relation",
            RelationValidationCause::Message(
                "relation validation requires an admitted relation declaration".to_owned(),
            ),
        ));
    };
    if &input.source != binding.source() || definition.name.as_str() != binding.relation() {
        return Err(reject(
            "invalid_relation",
            RelationValidationCause::Message(
                "relation validation does not match its constructor-bound source/relation"
                    .to_owned(),
            ),
        ));
    }
    if !descriptor.allows(input.operation.as_str()) {
        return Err(reject(
            "unsupported_operation",
            RelationValidationCause::Message(format!(
                "relation operation {:?} is not declared",
                input.operation
            )),
        ));
    }
    if input.save_mode != definition.editor.save_mode {
        return Err(reject(
            "invalid_save_mode",
            RelationValidationCause::Message(
                "relation save mode does not match its declaration".to_owned(),
            ),
        ));
    }

    let mut seen: BTreeSet<String> = BTreeSet::new();
    for row in &rows {
        if row.key.trim().is_empty() || !seen.insert(row.key.clone()) {
            return Err(reject(
                "duplicate_row",
                RelationValidationCause::Message(
                    "relation rows require distinct stable row keys".to_owned(),
                ),
            ));
        }
        if !row.operation.is_empty() && !descriptor.allows(row.operation.as_str()) {
            return Err(reject(
                "unsupported_operation",
                RelationValidationCause::Message(format!(
                    "row {} uses an undeclared operation",
                    row.key
                )),
            ));
        }
        if row.selection.is_valid() {
            if !row.selection.belongs_to(&descriptor) {
                return Err(reject(
                    "invalid_target",
                    RelationValidationCause::Message(format!(
                        "row {} belongs to a different relation declaration",
                        row.key
                    )),
                ));
            }
            if row.operation == "create_related" {
                return Err(reject(
                    "invalid_row",
                    RelationValidationCause::Message(format!(
                        "new row {} already has a target identity",
                        row.key
                    )),
                ));
            }
        } else if row.operation != "create_related" {
            return Err(reject(
                "invalid_target",
                RelationValidationCause::Message(format!(
                    "row {} requires a target or explicit related-record creation",
                    row.key
                )),
            ));
        }
        if row.operation == "reorder"
            && (!row.has_position || definition.editor.order_field.as_str().is_empty())
        {
            return Err(reject(
                "invalid_order",
                RelationValidationCause::Message(format!(
                    "row {} requires a declared order field and explicit position",
                    row.key
                )),
            ));
        }
    }

    let value = Arc::new(RelationRowsValue::new(
        request.clone(),
        descriptor,
        input.source_id,
        input.revision,
        input.operation,
        input.save_mode,
        rows,
    ));
    Ok(DecodedRelationRows { value })
}
