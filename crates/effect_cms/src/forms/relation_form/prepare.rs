//! Relation-form preparation and its owner-defined refusals.
//!
//! Preparation admits only a matching admitted lookup and a complete form input:
//! the descriptor must equal the lookup's descriptor, the control identity and the
//! required localized labels must be present, client bindings must use data
//! attributes, pagination must carry localized labels, and every declared action
//! must name a declared operation with its transport name and localized label.
//!
//! Rejected selections are untrusted transport: each needs a distinct occurrence
//! key, an issue and the localized removal label. A row needs a stable distinct key;
//! an existing row selection must be available from the admitted lookup; a new row
//! must be the declared related-record creation; and rejected row selections cannot
//! carry canonical authority.
//!
//! The encoder may rewrite each option's transport; colliding transport values are
//! refused. Without an encoder the option keeps the selection's canonical identity
//! token. Row ids are derived from the row key with SHA-256.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::definitions::{
    ResourceRelationDefinition, ResourceRelationDescriptor, ResourceRelationKeyDefinition,
};
use crate::queries::{
    RelationLookupCause, RelationLookupFailed, RelationLookupOutcome, RelationLookupRecord,
    RelationLookupResult, RelationLookupUnavailable, RelationSelection,
};

use super::{
    RelationFormInput, RelationFormLabels, RelationFormOption, RelationFormRow,
    RelationFormRowValue, RelationFormState, RelationFormValue, RelationRowDraft,
    relation_form_required,
};

/// A relation-form refusal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationFormError<E> {
    /// The lookup was rejected with its issues.
    LookupRejected(Vec<String>),
    /// The lookup was reported unavailable, with its typed payload.
    LookupUnavailable(RelationLookupUnavailable<E>),
    /// The lookup failed, with its typed payload.
    LookupFailed(RelationLookupFailed<E>),
    /// No admitted lookup result was supplied.
    LookupRequired,
    /// The descriptor does not match the admitted lookup.
    DescriptorMismatch,
    /// The control identity and its required localized labels are missing.
    ControlIdentityAndLabelsRequired,
    /// The searchable control is missing its localized labels or click binding.
    SearchLabelsRequired,
    /// The selection prompt is missing for a non-related-rows editor.
    ChooseLabelRequired,
    /// A client binding uses a non-data attribute name.
    ClientBindingsMustUseDataAttributes,
    /// A pagination target is missing its localized labels.
    PaginationLabelsRequired,
    /// An action does not name a declared operation, transport name or label.
    InvalidAction,
    /// A rejected selection is missing its key, issues or removal label.
    RejectedSelectionRequiresLabels,
    /// A required relation is missing its localized required label.
    RequiredLabelRequired,
    /// Row values require the declared related-row control.
    RowsRequireRelatedRowControl,
    /// Two options encode to the same transport value.
    TransportValuesCollide,
    /// A singular relation has more than one submitted selection.
    SingularRelationMultipleSelections,
    /// Related rows require their bound field renderer.
    RowsRequireRenderer,
    /// A row does not have a stable distinct key.
    StableRowIdentitiesRequired,
    /// A row names an operation the declaration does not allow.
    RowOperationNotDeclared,
    /// A rejected row carries canonical selection authority.
    RejectedRowCarriesAuthority,
    /// A rejected row selection is missing its key or issues.
    RejectedRowSelectionsRequireKeys,
    /// An existing row selection has no admitted availability.
    RowSelectionAvailabilityMissing,
    /// A new row requires the declared related-record creation.
    NewRowRequiresCreation,
    /// A new row is missing its localized label.
    NewRowLabelRequired,
    /// The consumer's selection encoder failed.
    SelectionEncoding(E),
}

impl<E: std::fmt::Display> std::fmt::Display for RelationFormError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LookupRejected(issues) => {
                write!(formatter, "relation form: lookup rejected: {}", issues.join("; "))
            }
            Self::LookupUnavailable(unavailable) => {
                formatter.write_str("relation form: lookup unavailable: ")?;
                write_relation_cause(unavailable.cause(), formatter)
            }
            Self::LookupFailed(failed) => {
                formatter.write_str("relation form: lookup failed: ")?;
                write_relation_cause(failed.cause(), formatter)
            }
            Self::LookupRequired => {
                formatter.write_str("relation form: an admitted lookup result is required")
            }
            Self::DescriptorMismatch => {
                formatter.write_str("relation form: descriptor does not match admitted lookup")
            }
            Self::ControlIdentityAndLabelsRequired => formatter.write_str(
                "relation form: stable control identity and localized labels are required",
            ),
            Self::SearchLabelsRequired => {
                formatter.write_str("relation form: localized search labels are required")
            }
            Self::ChooseLabelRequired => {
                formatter.write_str("relation form: localized selection prompt is required")
            }
            Self::ClientBindingsMustUseDataAttributes => formatter
                .write_str("relation form: client bindings must use existing data attributes"),
            Self::PaginationLabelsRequired => {
                formatter.write_str("relation form: pagination requires localized labels")
            }
            Self::InvalidAction => formatter.write_str(
                "relation form: actions require a declared operation, transport name and localized label",
            ),
            Self::RejectedSelectionRequiresLabels => formatter.write_str(
                "relation form: rejected selections require distinct occurrence keys, issues and removal label",
            ),
            Self::RequiredLabelRequired => {
                formatter.write_str("relation form: a localized required label is needed")
            }
            Self::RowsRequireRelatedRowControl => formatter.write_str(
                "relation form: row values require the declared related-row control",
            ),
            Self::TransportValuesCollide => {
                formatter.write_str("relation form: target transport values collide")
            }
            Self::SingularRelationMultipleSelections => formatter.write_str(
                "relation form: singular relation has multiple submitted selections",
            ),
            Self::RowsRequireRenderer => formatter
                .write_str("relation form: related rows require their bound field renderer"),
            Self::StableRowIdentitiesRequired => {
                formatter.write_str("relation form: stable distinct row identities are required")
            }
            Self::RowOperationNotDeclared => {
                formatter.write_str("relation form: row operation is not declared")
            }
            Self::RejectedRowCarriesAuthority => formatter
                .write_str("relation form: rejected row selection cannot carry canonical authority"),
            Self::RejectedRowSelectionsRequireKeys => formatter.write_str(
                "relation form: rejected row selections require distinct occurrence keys and issues",
            ),
            Self::RowSelectionAvailabilityMissing => formatter.write_str(
                "relation form: existing row has no admitted selection availability",
            ),
            Self::NewRowRequiresCreation => formatter.write_str(
                "relation form: new row requires declared related-record creation",
            ),
            Self::NewRowLabelRequired => {
                formatter.write_str("relation form: a localized new-row label is required")
            }
            Self::SelectionEncoding(error) => std::fmt::Display::fmt(error, formatter),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for RelationFormError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LookupUnavailable(unavailable) => relation_cause_source(unavailable.cause()),
            Self::LookupFailed(failed) => relation_cause_source(failed.cause()),
            Self::SelectionEncoding(error) => Some(error),
            _ => None,
        }
    }
}

/// Expose the typed cause's consumer error, when the cause has one.
fn relation_cause_source<E: std::error::Error + 'static>(
    cause: &RelationLookupCause<E>,
) -> Option<&(dyn std::error::Error + 'static)> {
    match cause {
        RelationLookupCause::Consumer(error) => Some(error),
        RelationLookupCause::Decode(error) => Some(error),
        RelationLookupCause::Message(_) | RelationLookupCause::Cancelled => None,
    }
}

/// Prepare one relation-form state.
///
/// # Errors
///
/// Returns the owner's typed refusal for an unadmitted lookup, a mismatched
/// descriptor, missing labels/bindings/pagination, invalid actions, rejected
/// selection transport, invalid rows, colliding transports, a singular relation with
/// multiple submitted selections, or the consumer's encoder failure.
pub fn prepare_relation_form<E>(
    input: RelationFormInput<E>,
) -> Result<RelationFormState, RelationFormError<E>>
where
    E: std::fmt::Display,
{
    let result = relation_form_lookup_result(input.lookup)?;
    if !input.descriptor.is_valid()
        || !same_relation_form_descriptor(&input.descriptor, &result.query().descriptor())
    {
        return Err(RelationFormError::DescriptorMismatch);
    }
    if !valid_relation_control_id(input.id.as_str())
        || input.name.trim().is_empty()
        || input.labels.label.trim().is_empty()
        || input.labels.empty.trim().is_empty()
        || input.labels.unavailable.trim().is_empty()
        || input.labels.status.is_empty()
    {
        return Err(RelationFormError::ControlIdentityAndLabelsRequired);
    }
    let control = input
        .descriptor
        .definition()
        .map(|definition| definition.editor.control)
        .unwrap_or_default();
    if input.searchable
        && (input.labels.search.trim().is_empty()
            || input.labels.search_action.trim().is_empty()
            || input
                .search_action_attributes
                .get("data-on:click")
                .is_none_or(String::is_empty))
    {
        return Err(RelationFormError::SearchLabelsRequired);
    }
    if control != "related_rows" && input.labels.choose.is_empty() {
        return Err(RelationFormError::ChooseLabelRequired);
    }
    relation_form_data_attributes(&[
        &input.search_attributes,
        &input.search_action_attributes,
        &input.previous_attributes,
        &input.next_attributes,
    ])?;
    if (!input.previous_href.is_empty() && input.previous_label.is_empty())
        || (!input.next_href.is_empty() && input.next_label.is_empty())
        || ((!input.previous_href.is_empty() || !input.next_href.is_empty())
            && input.labels.pagination.is_empty())
    {
        return Err(RelationFormError::PaginationLabelsRequired);
    }
    validate_relation_form_actions(&input.descriptor, &input.actions)?;
    let mut value = RelationFormValue {
        rejected_selections: input.rejected_selections.clone(),
        descriptor: input.descriptor.clone(),
        lookup: result.clone(),
        id: input.id.clone(),
        name: input.name.clone(),
        labels: input.labels.clone(),
        options: Vec::new(),
        rows: Vec::new(),
        issues: input.issues.clone(),
        actions: input.actions.clone(),
        search_attributes: input.search_attributes.clone(),
        search_action_attributes: input.search_action_attributes.clone(),
        previous_attributes: input.previous_attributes.clone(),
        next_attributes: input.next_attributes.clone(),
        searchable: input.searchable,
        previous_href: input.previous_href.clone(),
        next_href: input.next_href.clone(),
        previous_label: input.previous_label.clone(),
        next_label: input.next_label.clone(),
        render_row: input.render_row.clone(),
    };
    let mut rejected_keys = BTreeSet::new();
    for draft in &input.rejected_selections {
        if draft.key.is_empty()
            || !rejected_keys.insert(draft.key.clone())
            || draft.issues.is_empty()
            || input.labels.remove_rejected.is_empty()
        {
            return Err(RelationFormError::RejectedSelectionRequiresLabels);
        }
        value.issues.extend(draft.issues.iter().cloned());
    }
    if relation_form_required(&input.descriptor) && input.labels.required.is_empty() {
        return Err(RelationFormError::RequiredLabelRequired);
    }
    if !input.rows.is_empty() && control != "related_rows" {
        return Err(RelationFormError::RowsRequireRelatedRowControl);
    }
    let mut selected = BTreeSet::new();
    let mut visible: BTreeMap<String, RelationLookupRecord> = BTreeMap::new();
    for record in result.selected() {
        selected.insert(record.selection().identity());
        visible.insert(record.selection().identity(), record);
    }
    for record in result.records() {
        visible.insert(record.selection().identity(), record);
    }
    let mut added = BTreeSet::new();
    let mut unavailable: BTreeMap<String, RelationSelection> = BTreeMap::new();
    let selected_records = result.selected();
    let records = result.records();
    let mut combined = selected_records;
    combined.extend(records);
    for record in combined {
        let identity = record.selection().identity();
        if !added.insert(identity.clone()) {
            continue;
        }
        value.options.push(RelationFormOption {
            identity: identity.clone(),
            value: identity.clone(),
            label: record.label(),
            selected: selected.contains(&identity),
        });
    }
    for missing in result.missing() {
        let identity = missing.identity();
        unavailable.insert(identity.clone(), missing);
        value.options.push(RelationFormOption {
            identity: identity.clone(),
            value: identity,
            label: input.labels.unavailable.clone(),
            selected: true,
        });
    }
    if let Some(encode) = input.encode_selection.as_ref() {
        let mut encoded: BTreeMap<String, String> = BTreeMap::new();
        for index in 0..value.options.len() {
            let identity = value.options[index].identity.clone();
            let selection = match unavailable.get(identity.as_str()) {
                Some(selection) => selection.clone(),
                None => match visible.get(identity.as_str()) {
                    Some(record) => record.selection(),
                    None => RelationSelection::default(),
                },
            };
            let transport = encode(&selection).map_err(RelationFormError::SelectionEncoding)?;
            if let Some(previous) = encoded.get(transport.as_str())
                && previous != &identity
            {
                return Err(RelationFormError::TransportValuesCollide);
            }
            encoded.insert(transport.clone(), identity);
            value.options[index].value = transport;
        }
    }
    let definition = input.descriptor.definition().unwrap_or_default();
    if (definition.cardinality == "many_to_one" || definition.cardinality == "one_to_one")
        && result.query().selected().len() > 1
    {
        return Err(RelationFormError::SingularRelationMultipleSelections);
    }
    if !input.rows.is_empty() && input.render_row.is_none() {
        return Err(RelationFormError::RowsRequireRenderer);
    }
    let mut seen = BTreeSet::new();
    for draft in &input.rows {
        if draft.key.is_empty() || !seen.insert(draft.key.clone()) {
            return Err(RelationFormError::StableRowIdentitiesRequired);
        }
        if !draft.operation.is_empty() && !input.descriptor.allows(draft.operation.as_str()) {
            return Err(RelationFormError::RowOperationNotDeclared);
        }
        let (label, available) = row_availability(
            &input.labels,
            &input.descriptor,
            &visible,
            &unavailable,
            draft,
        )?;
        validate_relation_form_actions(&input.descriptor, &draft.actions)?;
        let mut row = RelationFormRowValue {
            rejected_selections: draft.rejected_selections.clone(),
            key: draft.key.clone(),
            id: format!(
                "{}-row-{}",
                input.id,
                Sha256::digest(draft.key.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            label,
            operation: draft.operation.clone(),
            selection: draft.selection.clone(),
            available,
            values: draft.values.clone(),
            actions: draft.actions.clone(),
            issues: Vec::new(),
        };
        for rejected in &draft.rejected_selections {
            row.issues.extend(rejected.issues.iter().cloned());
        }
        for issue in &input.issues {
            if issue.row == draft.key {
                row.issues.push(issue.clone());
            }
        }
        value.rows.push(RelationFormRow {
            value: Some(Arc::new(row)),
        });
    }
    Ok(RelationFormState {
        value: Some(Arc::new(value)),
    })
}

fn row_availability<E>(
    labels: &RelationFormLabels,
    descriptor: &ResourceRelationDescriptor,
    visible: &BTreeMap<String, RelationLookupRecord>,
    unavailable: &BTreeMap<String, RelationSelection>,
    draft: &RelationRowDraft,
) -> Result<(String, bool), RelationFormError<E>> {
    if !draft.rejected_selections.is_empty() {
        if draft.selection.is_valid() {
            return Err(RelationFormError::RejectedRowCarriesAuthority);
        }
        let mut keys = BTreeSet::new();
        for rejected in &draft.rejected_selections {
            if rejected.key.is_empty() || !keys.insert(rejected.key.clone()) || rejected.issues.is_empty() {
                return Err(RelationFormError::RejectedRowSelectionsRequireKeys);
            }
        }
        return Ok((labels.unavailable.clone(), false));
    }
    if draft.selection.is_valid() {
        if let Some(record) = visible.get(draft.selection.identity().as_str())
            && same_relation_selection(&draft.selection, &record.selection(), descriptor)
        {
            return Ok((record.label(), true));
        }
        if let Some(missing) = unavailable.get(draft.selection.identity().as_str())
            && same_relation_selection(&draft.selection, missing, descriptor)
        {
            return Ok((labels.unavailable.clone(), false));
        }
        return Err(RelationFormError::RowSelectionAvailabilityMissing);
    }
    if draft.operation != "create_related" || !descriptor.allows("create_related") {
        return Err(RelationFormError::NewRowRequiresCreation);
    }
    if labels.new_row.is_empty() {
        return Err(RelationFormError::NewRowLabelRequired);
    }
    Ok((labels.new_row.clone(), false))
}

fn relation_form_lookup_result<E>(
    outcome: Option<RelationLookupOutcome<E>>,
) -> Result<RelationLookupResult, RelationFormError<E>>
where
    E: std::fmt::Display,
{
    let Some(outcome)=outcome else{return Err(RelationFormError::LookupRequired)};
    match outcome {
        RelationLookupOutcome::Found(found) => {
            if found.result().is_valid() {
                Ok(found.result().clone())
            } else {
                Err(RelationFormError::LookupRequired)
            }
        }
        RelationLookupOutcome::Empty(empty) => {
            if empty.result().is_valid() {
                Ok(empty.result().clone())
            } else {
                Err(RelationFormError::LookupRequired)
            }
        }
        RelationLookupOutcome::Rejected(rejected) => {
            Err(RelationFormError::LookupRejected(rejected.issues()))
        }
        RelationLookupOutcome::Unavailable(unavailable) => {
            Err(RelationFormError::LookupUnavailable(unavailable))
        }
        RelationLookupOutcome::Failed(failed) => Err(RelationFormError::LookupFailed(failed)),
    }
}

/// Write the typed cause's display text without changing the retained payload.
fn write_relation_cause<E: std::fmt::Display>(
    cause: &RelationLookupCause<E>,
    formatter: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    match cause {
        RelationLookupCause::Consumer(error) => std::fmt::Display::fmt(error, formatter),
        RelationLookupCause::Decode(error) => std::fmt::Display::fmt(error, formatter),
        RelationLookupCause::Message(message) => formatter.write_str(message),
        RelationLookupCause::Cancelled => formatter.write_str("context canceled"),
    }
}

fn same_relation_form_descriptor(
    left: &crate::definitions::ResourceRelationDescriptor,
    right: &crate::definitions::ResourceRelationDescriptor,
) -> bool {
    if !left.is_valid() || !right.is_valid() || left.source_kind() != right.source_kind() {
        return false;
    }
    match (left.definition(), right.definition()) {
        (Some(left), Some(right)) => same_relation_definition(&left, &right),
        _ => false,
    }
}

fn same_relation_definition(
    left: &ResourceRelationDefinition,
    right: &ResourceRelationDefinition,
) -> bool {
    left.name == right.name
        && left.cardinality == right.cardinality
        && left.inverse_of == right.inverse_of
        && left.on_target_delete == right.on_target_delete
        && left.query_binding == right.query_binding
        && left.scope == right.scope
        && left.target == right.target
        && left.discriminator == right.discriminator
        && left.optional == right.optional
        && left.owned == right.owned
        && left.through == right.through
        && left.operations == right.operations
        && same_keys(&left.keys, &right.keys)
        && left.targets.len() == right.targets.len()
        && left.targets.iter().zip(&right.targets).all(|(left, right)| {
            left.scope == right.scope
                && left.tag == right.tag
                && left.target == right.target
                && same_keys(&left.keys, &right.keys)
        })
        && left.editor.control == right.editor.control
        && left.editor.label_query == right.editor.label_query
        && left.editor.locale_key == right.editor.locale_key
        && left.editor.save_mode == right.editor.save_mode
        && left.editor.removal_operation == right.editor.removal_operation
        && left.editor.label_field == right.editor.label_field
        && left.editor.order_field == right.editor.order_field
        && left.editor.columns == right.editor.columns
        && left.editor.minimum == right.editor.minimum
        && left.editor.maximum == right.editor.maximum
}

fn same_keys(left: &[ResourceRelationKeyDefinition], right: &[ResourceRelationKeyDefinition]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.source == right.source && left.target == right.target && left.r#type == right.r#type
        })
}

fn same_relation_selection(
    left: &RelationSelection,
    right: &RelationSelection,
    descriptor: &crate::definitions::ResourceRelationDescriptor,
) -> bool {
    left.belongs_to(descriptor)
        && right.belongs_to(descriptor)
        && left.identity() == right.identity()
}

fn valid_relation_control_id(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    value
        .chars()
        .all(|character| character.is_alphanumeric() || character == '-' || character == '_')
}

fn relation_form_data_attributes<E>(
    values: &[&BTreeMap<String, String>],
) -> Result<(), RelationFormError<E>> {
    for attributes in values {
        for name in attributes.keys() {
            if !name.starts_with("data-") {
                return Err(RelationFormError::ClientBindingsMustUseDataAttributes);
            }
        }
    }
    Ok(())
}

fn validate_relation_form_actions<E>(
    descriptor: &crate::definitions::ResourceRelationDescriptor,
    actions: &[super::RelationFormActionInput],
) -> Result<(), RelationFormError<E>> {
    for action in actions {
        if !descriptor.allows(action.operation.as_str())
            || action.name.trim().is_empty()
            || action.label.trim().is_empty()
        {
            return Err(RelationFormError::InvalidAction);
        }
    }
    Ok(())
}

