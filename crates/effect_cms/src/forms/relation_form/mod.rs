//! Relation form state: the typed relation editor contract and its rendering.
//!
//! The child owner provides:
//!
//! - the localized labels, action transport inputs, row issues, rejected untrusted
//!   selection drafts, row drafts and the render callback contract;
//! - the private validated state returned only by [`prepare_relation_form`], its
//!   typed rows/options, the canonical selection encoder and the owner's
//!   descriptor/lookup admission rules;
//! - the relation control rendering, its search/pagination/selection/action markup
//!   and the preserved request action.
//!
//! A rejected selection draft is untrusted render transport: it is never a
//! [`RelationSelection`] and is not accepted by mutation validation or typed stores.
//! A valid state is issued only from a matching admitted lookup and form input, so a
//! forged or replayed state is unrepresentable outside this owner.
//!
//! The owner opens no store and performs no lookup itself: the caller supplies the
//! admitted lookup outcome and the encoder/callbacks. The request action and the
//! control markup are presentation contracts for the existing transport.

use std::collections::BTreeMap;
use std::sync::Arc;

use maud::Markup;

use crate::definitions::{ResourceFieldKey, ResourceRelationDescriptor, ResourceRelationDefinition};
use crate::queries::{RelationLookupOutcome, RelationLookupRecord, RelationLookupResult, RelationQuery, RelationSelection};

mod prepare;
mod render;
mod request;

pub use self::prepare::{RelationFormError, prepare_relation_form};
pub use self::render::relation_control;
pub use self::request::relation_request_action;

/// Localized relation-form labels.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RelationFormLabels {
    /// The field label.
    pub label: String,
    /// The help text.
    pub help: String,
    /// The search field label.
    pub search: String,
    /// The search action label.
    pub search_action: String,
    /// The empty-selection text.
    pub empty: String,
    /// The unavailable-target text.
    pub unavailable: String,
    /// The required marker.
    pub required: String,
    /// The selection prompt.
    pub choose: String,
    /// The new-row label.
    pub new_row: String,
    /// The non-empty status text.
    pub status: String,
    /// The pagination label.
    pub pagination: String,
    /// The rejected-removal label.
    pub remove_rejected: String,
}

/// One declared operation's transport.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RelationFormActionInput {
    /// The declared operation.
    pub operation: String,
    /// The submitted name.
    pub name: String,
    /// The submitted value.
    pub value: String,
    /// The localized label.
    pub label: String,
}

/// One row/field issue.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RelationFormIssue {
    /// The row key, empty for a form-level issue.
    pub row: String,
    /// The field key.
    pub field: ResourceFieldKey,
    /// The message.
    pub message: String,
}

/// Untrusted rejected-selection transport.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RelationRejectedSelectionDraft {
    /// The occurrence key.
    pub key: String,
    /// The raw transport value.
    pub raw: String,
    /// The rejection issues.
    pub issues: Vec<RelationFormIssue>,
}

/// One supplied row draft.
#[derive(Clone, Debug)]
pub struct RelationRowDraft {
    /// The rejected selection transport.
    pub rejected_selections: Vec<RelationRejectedSelectionDraft>,
    /// The stable row key.
    pub key: String,
    /// The existing canonical selection.
    pub selection: RelationSelection,
    /// The declared row operation.
    pub operation: String,
    /// The row's field values.
    pub values: BTreeMap<ResourceFieldKey, String>,
    /// The row's declared actions.
    pub actions: Vec<RelationFormActionInput>,
}

impl Default for RelationRowDraft {
    fn default() -> Self {
        Self {
            rejected_selections: Vec::new(),
            key: String::new(),
            selection: RelationSelection::default(),
            operation: String::new(),
            values: BTreeMap::new(),
            actions: Vec::new(),
        }
    }
}

/// Renders one related row.
pub type RelationRowRenderFunc =
    Arc<dyn Fn(&RelationFormRow) -> Markup + Send + Sync + 'static>;

/// Encodes one canonical selection as its existing field transport.
pub type RelationSelectionValueFunc<E> =
    Arc<dyn Fn(&RelationSelection) -> Result<String, E> + Send + Sync + 'static>;

/// The relation-form preparation input.
pub struct RelationFormInput<E> {
    /// The rejected selection transport.
    pub rejected_selections: Vec<RelationRejectedSelectionDraft>,
    /// The canonical selection encoder; absent uses the identity token.
    pub encode_selection: Option<RelationSelectionValueFunc<E>>,
    /// The admitted relation descriptor.
    pub descriptor: ResourceRelationDescriptor,
    /// The supplied lookup outcome.
    pub lookup: Option<RelationLookupOutcome<E>>,
    /// The stable control id.
    pub id: String,
    /// The submitted control name.
    pub name: String,
    /// The localized labels.
    pub labels: RelationFormLabels,
    /// The row drafts.
    pub rows: Vec<RelationRowDraft>,
    /// The form/row issues.
    pub issues: Vec<RelationFormIssue>,
    /// The form-level declared actions.
    pub actions: Vec<RelationFormActionInput>,
    /// The search input's client attributes.
    pub search_attributes: BTreeMap<String, String>,
    /// The search action's client attributes.
    pub search_action_attributes: BTreeMap<String, String>,
    /// The previous-page client attributes.
    pub previous_attributes: BTreeMap<String, String>,
    /// The next-page client attributes.
    pub next_attributes: BTreeMap<String, String>,
    /// Whether the search field is shown.
    pub searchable: bool,
    /// The previous-page address.
    pub previous_href: String,
    /// The next-page address.
    pub next_href: String,
    /// The previous-page label.
    pub previous_label: String,
    /// The next-page label.
    pub next_label: String,
    /// Renders one related row.
    pub render_row: Option<RelationRowRenderFunc>,
}

/// A validated relation-form state.
#[derive(Clone, Default)]
pub struct RelationFormState {
    pub(crate) value: Option<Arc<RelationFormValue>>,
}

impl std::fmt::Debug for RelationFormState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RelationFormState")
            .field("valid", &self.valid())
            .finish_non_exhaustive()
    }
}

/// The admitted relation-form payload.
pub(crate) struct RelationFormValue {
    pub(crate) rejected_selections: Vec<RelationRejectedSelectionDraft>,
    pub(crate) descriptor: ResourceRelationDescriptor,
    pub(crate) lookup: RelationLookupResult,
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) labels: RelationFormLabels,
    pub(crate) options: Vec<RelationFormOption>,
    pub(crate) rows: Vec<RelationFormRow>,
    pub(crate) issues: Vec<RelationFormIssue>,
    pub(crate) actions: Vec<RelationFormActionInput>,
    pub(crate) search_attributes: BTreeMap<String, String>,
    pub(crate) search_action_attributes: BTreeMap<String, String>,
    pub(crate) previous_attributes: BTreeMap<String, String>,
    pub(crate) next_attributes: BTreeMap<String, String>,
    pub(crate) searchable: bool,
    pub(crate) previous_href: String,
    pub(crate) next_href: String,
    pub(crate) previous_label: String,
    pub(crate) next_label: String,
    pub(crate) render_row: Option<RelationRowRenderFunc>,
}

/// One selectable option.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RelationFormOption {
    pub(crate) identity: String,
    pub(crate) value: String,
    pub(crate) label: String,
    pub(crate) selected: bool,
}

/// One validated related row.
#[derive(Clone, Default)]
pub struct RelationFormRow {
    pub(crate) value: Option<Arc<RelationFormRowValue>>,
}

impl std::fmt::Debug for RelationFormRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RelationFormRow")
            .field("valid", &self.valid())
            .field("key", &self.key())
            .finish_non_exhaustive()
    }
}

/// The validated row payload.
pub(crate) struct RelationFormRowValue {
    pub(crate) rejected_selections: Vec<RelationRejectedSelectionDraft>,
    pub(crate) key: String,
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) operation: String,
    pub(crate) selection: RelationSelection,
    pub(crate) available: bool,
    pub(crate) values: BTreeMap<ResourceFieldKey, String>,
    pub(crate) actions: Vec<RelationFormActionInput>,
    pub(crate) issues: Vec<RelationFormIssue>,
}

impl RelationFormState {
    /// Whether the state was admitted from a matching lookup and form input.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.value.as_ref().is_some_and(|value| {
            value.descriptor.is_valid() && value.lookup.is_valid()
        })
    }

    /// The admitted descriptor.
    #[must_use]
    pub fn descriptor(&self) -> ResourceRelationDescriptor {
        match self.value.as_ref() {
            Some(value) if self.valid() => value.descriptor.clone(),
            _ => ResourceRelationDescriptor::default(),
        }
    }

    /// The admitted lookup query.
    #[must_use]
    pub fn query(&self) -> RelationQuery {
        match self.value.as_ref() {
            Some(value) if self.valid() => value.lookup.query(),
            _ => RelationQuery::default(),
        }
    }

    /// The validated rows.
    #[must_use]
    pub fn rows(&self) -> Vec<RelationFormRow> {
        match self.value.as_ref() {
            Some(value) if self.valid() => value.rows.clone(),
            _ => Vec::new(),
        }
    }

    /// The form issues.
    #[must_use]
    pub fn issues(&self) -> Vec<RelationFormIssue> {
        match self.value.as_ref() {
            Some(value) if self.valid() => value.issues.clone(),
            _ => Vec::new(),
        }
    }

    /// The rejected selection transport.
    #[must_use]
    pub fn rejected_selections(&self) -> Vec<RelationRejectedSelectionDraft> {
        match self.value.as_ref() {
            Some(value) if self.valid() => clone_rejected_selections(&value.rejected_selections),
            _ => Vec::new(),
        }
    }

    pub(crate) fn lookup_selected(&self) -> Vec<RelationLookupRecord> {
        match self.value.as_ref() {
            Some(value) if self.valid() => value.lookup.selected(),
            _ => Vec::new(),
        }
    }
}

impl RelationFormRow {
    /// Whether the row was admitted.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.value.is_some()
    }

    /// The stable row key.
    #[must_use]
    pub fn key(&self) -> String {
        self.value.as_ref().map_or_else(String::new, |value| value.key.clone())
    }

    /// The row element id.
    #[must_use]
    pub fn id(&self) -> String {
        self.value.as_ref().map_or_else(String::new, |value| value.id.clone())
    }

    /// The row label.
    #[must_use]
    pub fn label(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(String::new, |value| value.label.clone())
    }

    /// The declared row operation.
    #[must_use]
    pub fn operation(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(String::new, |value| value.operation.clone())
    }

    /// The row's canonical selection.
    #[must_use]
    pub fn selection(&self) -> RelationSelection {
        self.value
            .as_ref()
            .map_or_else(RelationSelection::default, |value| value.selection.clone())
    }

    /// Chooser-time availability, never mutation authority.
    #[must_use]
    pub fn available(&self) -> bool {
        self.value.as_ref().is_some_and(|value| value.available)
    }

    /// The row's field values.
    #[must_use]
    pub fn values(&self) -> BTreeMap<ResourceFieldKey, String> {
        self.value
            .as_ref()
            .map_or_else(BTreeMap::new, |value| value.values.clone())
    }

    /// The row issues.
    #[must_use]
    pub fn issues(&self) -> Vec<RelationFormIssue> {
        self.value
            .as_ref()
            .map_or_else(Vec::new, |value| value.issues.clone())
    }

    /// The row's declared actions.
    #[must_use]
    pub fn actions(&self) -> Vec<RelationFormActionInput> {
        self.value
            .as_ref()
            .map_or_else(Vec::new, |value| value.actions.clone())
    }

    /// The rejected row selection transport.
    #[must_use]
    pub fn rejected_selections(&self) -> Vec<RelationRejectedSelectionDraft> {
        self.value.as_ref().map_or_else(Vec::new, |value| {
            clone_rejected_selections(&value.rejected_selections)
        })
    }
}

fn clone_rejected_selections(
    source: &[RelationRejectedSelectionDraft],
) -> Vec<RelationRejectedSelectionDraft> {
    source.to_vec()
}

/// The target of one related-row editor's canonical selections.
pub(crate) fn relation_form_required(descriptor: &ResourceRelationDescriptor) -> bool {
    let Some(definition) = descriptor.definition() else {
        return false;
    };
    relation_definition_required(&definition)
}

fn relation_definition_required(definition: &ResourceRelationDefinition) -> bool {
    if definition.cardinality == "one_to_many" || definition.cardinality == "many_to_many" {
        return definition.editor.minimum.is_some_and(|minimum| minimum > 0);
    }
    !definition.optional
}

pub(crate) fn relation_form_description_ids(state: &RelationFormState) -> String {
    let Some(value) = state.value.as_ref() else {
        return String::new();
    };
    let mut ids = vec![format!("{}-status", value.id)];
    if !value.labels.help.is_empty() {
        ids.push(format!("{}-help", value.id));
    }
    if !value.issues.is_empty() {
        ids.push(format!("{}-errors", value.id));
    }
    ids.join(" ")
}
