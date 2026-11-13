//! Relation declaration validation, in exact precedence.
//!
//! Only these checks are applied: a cardinality from its
//! fixed set, exactly one of a single target or finite tagged targets (with the
//! discriminator they require, distinct tags, target identities and keys),
//! declared keys when no through path, inverse declaration or finite targets exist,
//! supported and distinct scalar key types with distinct source and target fields,
//! supported and unrepeated operations, non-blank through steps, nonnegative
//! ordered editor bounds, an explicit patch or replacement save mode, and a removal
//! operation that is actually declared.
//!
//! Nothing is broadened: arbitrary operation strings, tags, through steps and
//! policy strings are not trimmed or normalized beyond the source's own trims of
//! the source kind, relation name and cardinality.

use std::collections::BTreeSet;

use super::relations::{RelationError, ResourceRelationDefinition, ResourceRelationKeyDefinition};
use super::vocabulary::ResourceKind;

/// The declared cardinality set.
const CARDINALITIES: &[&str] = &["many_to_one", "one_to_one", "one_to_many", "many_to_many"];

/// The declared operation set.
const OPERATIONS: &[&str] = &[
    "select",
    "link",
    "replace",
    "unlink",
    "create_related",
    "edit_related",
    "delete_related",
    "reorder",
];

/// The supported scalar key types.
const KEY_TYPES: &[&str] = &["string", "int", "bool"];

/// The explicit relation save modes.
const SAVE_MODES: &[&str] = &["patch", "replacement"];

/// Validate one already-trimmed relation declaration for `source`.
pub(crate) fn validate_relation_definition(
    source: &ResourceKind,
    value: &ResourceRelationDefinition,
) -> Result<(), RelationError> {
    if source.as_str().is_empty() || value.name.is_empty() {
        return Err(RelationError::new(
            "relation definition: source and relation identity are required",
        ));
    }
    if !CARDINALITIES.contains(&value.cardinality.as_str()) {
        return Err(identity_error(
            source,
            value,
            "explicit cardinality is required",
        ));
    }
    if value.target.as_str().is_empty() && value.targets.is_empty() {
        return Err(identity_error(source, value, "target is required"));
    }
    if !value.target.as_str().is_empty() && !value.targets.is_empty() {
        return Err(identity_error(
            source,
            value,
            "use one target or finite tagged targets",
        ));
    }
    if !value.targets.is_empty() && value.discriminator.as_str().is_empty() {
        return Err(identity_error(
            source,
            value,
            "finite tagged targets require their declared discriminator",
        ));
    }
    if value.through.is_empty()
        && value.inverse_of.is_empty()
        && value.keys.is_empty()
        && value.targets.is_empty()
    {
        return Err(identity_error(source, value, "declared keys are required"));
    }
    validate_keys(&value.keys).map_err(|detail| identity_error(source, value, detail))?;

    let mut tags: BTreeSet<&str> = BTreeSet::new();
    for target in &value.targets {
        if target.tag.trim().is_empty()
            || target.target.as_str().is_empty()
            || target.keys.is_empty()
        {
            return Err(identity_error(
                source,
                value,
                "tagged targets require distinct tags, target identities and keys",
            ));
        }
        if !tags.insert(target.tag.as_str()) {
            return Err(identity_error(
                source,
                value,
                "tagged targets require distinct tags, target identities and keys",
            ));
        }
        validate_keys(&target.keys).map_err(|detail| {
            RelationError::new(format!(
                "relation {}.{} target {}: {detail}",
                source.as_str(),
                value.name,
                target.tag
            ))
        })?;
    }

    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for operation in &value.operations {
        if !OPERATIONS.contains(&operation.as_str()) || !seen.insert(operation.as_str()) {
            return Err(identity_error(
                source,
                value,
                format!("invalid or repeated operation {operation:?}"),
            ));
        }
    }

    for step in &value.through {
        if step.trim().is_empty() {
            return Err(identity_error(
                source,
                value,
                "through path has an empty step",
            ));
        }
    }

    let negative_minimum = value.editor.minimum.is_some_and(|minimum| minimum < 0);
    let negative_maximum = value.editor.maximum.is_some_and(|maximum| maximum < 0);
    if negative_minimum || negative_maximum {
        return Err(identity_error(
            source,
            value,
            "collection bounds must be nonnegative",
        ));
    }
    if let (Some(minimum), Some(maximum)) = (value.editor.minimum, value.editor.maximum)
        && minimum > maximum
    {
        return Err(identity_error(source, value, "minimum exceeds maximum"));
    }
    if !value.editor.save_mode.is_empty() && !SAVE_MODES.contains(&value.editor.save_mode.as_str())
    {
        return Err(identity_error(
            source,
            value,
            "save mode must be explicit patch or replacement",
        ));
    }
    if !value.editor.removal_operation.is_empty()
        && !seen.contains(value.editor.removal_operation.as_str())
    {
        return Err(identity_error(
            source,
            value,
            "removal operation is not declared",
        ));
    }
    Ok(())
}

/// Validate the ordered key components of one relation or tagged target.
fn validate_keys(keys: &[ResourceRelationKeyDefinition]) -> Result<(), String> {
    let mut sources: BTreeSet<&str> = BTreeSet::new();
    let mut targets: BTreeSet<&str> = BTreeSet::new();
    for key in keys {
        if key.source.as_str().trim().is_empty()
            || key.target.as_str().trim().is_empty()
            || sources.contains(key.source.as_str())
            || targets.contains(key.target.as_str())
        {
            return Err("key components require distinct source and target fields".to_owned());
        }
        if !KEY_TYPES.contains(&key.r#type.as_str()) {
            return Err(format!(
                "key {} requires a declared supported scalar type",
                key.source.as_str()
            ));
        }
        sources.insert(key.source.as_str());
        targets.insert(key.target.as_str());
    }
    Ok(())
}

/// Build one `relation {source}.{name}: {detail}` refusal.
fn identity_error(
    source: &ResourceKind,
    value: &ResourceRelationDefinition,
    detail: impl Into<String>,
) -> RelationError {
    let detail = detail.into();
    RelationError::new(format!(
        "relation {}.{}: {detail}",
        source.as_str(),
        value.name
    ))
}
