//! Detail page state and the source detail-section builders.
//!
//! The builders preserve the declared section order, the caller's title/description
//! override precedence, the identity section construction, the supplied body
//! section's blank rejection and the declared-field projection in declaration order
//! with exact blank values skipped.

use std::collections::BTreeMap;

use effect_server::Request;

use crate::definitions::{
    RESOURCE_SECTION_KEY_IDENTITY, Registry, ResourceDetailDefinition,
    ResourceDetailFieldDefinition, ResourceDetailSectionDefinition, ResourceSectionKey,
    find_resource_definition,
};
use crate::localization::TranslateKeyFunc;

use super::detail_vms::{FieldVM, SectionVM};
use super::list_vms::ResourceListItemVM;

/// One supplied detail section.
#[derive(Clone, Debug, Default)]
pub struct ResourceDetailSectionState {
    /// Title override.
    pub title: String,
    /// Description override.
    pub description: String,
    /// Section body.
    pub body: String,
    /// Whether the body is authored HTML.
    pub body_is_html: bool,
    /// Field values keyed by state key.
    pub fields: BTreeMap<String, String>,
    /// Section items.
    pub items: Vec<ResourceListItemVM>,
}

/// The supplied detail state keyed by section.
pub type ResourceDetailState = BTreeMap<ResourceSectionKey, ResourceDetailSectionState>;

/// Build the declared detail sections.
#[must_use]
pub fn build_resource_detail_sections(
    definition: &ResourceDetailDefinition,
    state: &ResourceDetailState,
    request: &Request,
    translate: Option<&TranslateKeyFunc<Request>>,
) -> Vec<SectionVM> {
    let declared = definition.sections.to_vec();
    if declared.is_empty() {
        return Vec::new();
    }
    let mut sections = Vec::with_capacity(declared.len());
    for section_definition in declared {
        let value = state.get(&section_definition.key);
        let section = SectionVM {
            key: section_definition.key.clone(),
            title: first_detail_text(&[
                value.map(|value| value.title.clone()).unwrap_or_default(),
                translate_detail_key(request, translate, &section_definition.title_key),
            ]),
            description: first_detail_text(&[
                value
                    .map(|value| value.description.clone())
                    .unwrap_or_default(),
                translate_detail_key(request, translate, &section_definition.description_key),
            ]),
            body: value.map(|value| value.body.clone()).unwrap_or_default(),
            body_is_html: value.map(|value| value.body_is_html).unwrap_or(false),
            items: value.map(|value| value.items.clone()).unwrap_or_default(),
            fields: detail_fields(
                request,
                &section_definition,
                value.map(|value| &value.fields),
                translate,
            ),
        };
        if value.is_some() || resource_has_detail_content(&section) {
            sections.push(section);
        }
    }
    sections
}

/// Build the detail sections through a registered definition.
#[must_use]
pub fn build_resource_detail_sections_for_kind<R: Registry + ?Sized>(
    resources: &R,
    kind: &crate::definitions::ResourceKind,
    state: &ResourceDetailState,
    request: &Request,
    translate: Option<&TranslateKeyFunc<Request>>,
) -> Vec<SectionVM> {
    let Some(definition) = find_resource_definition(Some(resources), kind) else {
        return Vec::new();
    };
    build_resource_detail_sections(&definition.detail, state, request, translate)
}

/// Build the identity section from supplied field values.
#[must_use]
pub fn resource_detail_identity_state(fields: BTreeMap<String, String>) -> ResourceDetailState {
    if fields.is_empty() {
        return ResourceDetailState::new();
    }
    let mut state = ResourceDetailState::new();
    state.insert(
        ResourceSectionKey::new(RESOURCE_SECTION_KEY_IDENTITY),
        ResourceDetailSectionState {
            fields,
            ..ResourceDetailSectionState::default()
        },
    );
    state
}

/// Add or replace one body section.
///
/// A blank body leaves the supplied state unchanged. A non-blank body replaces the
/// whole section value, as the source workflow does.
#[must_use]
pub fn resource_detail_set_body_section(
    mut state: ResourceDetailState,
    key: &ResourceSectionKey,
    body: &str,
    html: bool,
) -> ResourceDetailState {
    let body = body.trim();
    if body.is_empty() {
        return state;
    }
    state.insert(
        key.clone(),
        ResourceDetailSectionState {
            body: body.to_owned(),
            body_is_html: html,
            ..ResourceDetailSectionState::default()
        },
    );
    state
}

/// Resolve the state key for a declared detail field.
#[must_use]
pub fn resource_detail_field_state_key(field: &ResourceDetailFieldDefinition) -> String {
    if !field.name.is_empty() {
        field.name.clone()
    } else {
        field.key.as_str().to_owned()
    }
}

/// Whether a section carries renderable detail content.
#[must_use]
pub fn resource_has_detail_content(section: &SectionVM) -> bool {
    !section.fields.is_empty() || !section.body.trim().is_empty() || !section.items.is_empty()
}

/// Project the declared fields for one section.
fn detail_fields(
    request: &Request,
    section: &ResourceDetailSectionDefinition,
    values: Option<&BTreeMap<String, String>>,
    translate: Option<&TranslateKeyFunc<Request>>,
) -> Vec<FieldVM> {
    let declared = section.fields.to_vec();
    if declared.is_empty() {
        return Vec::new();
    }
    let mut fields = Vec::with_capacity(declared.len());
    for field in declared {
        let value = values
            .and_then(|values| values.get(&resource_detail_field_state_key(&field)).cloned())
            .unwrap_or_default();
        if value.is_empty() {
            continue;
        }
        fields.push(FieldVM {
            key: field.key.clone(),
            name: field.name.clone(),
            label: translate_detail_key(request, translate, &field.label_key),
            value: value.into(),
            ..FieldVM::default()
        });
    }
    fields
}

/// Resolve one detail text key.
fn translate_detail_key(
    request: &Request,
    translate: Option<&TranslateKeyFunc<Request>>,
    key: &str,
) -> String {
    match translate {
        Some(translate) => translate(request, key),
        None => key.trim().to_owned(),
    }
}

/// The first trimmed non-blank value.
fn first_detail_text(values: &[String]) -> String {
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    String::new()
}
