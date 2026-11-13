//! List item/section state and the source item and section builders.
//!
//! The builders preserve the source's trimming, blank-key admission rule, index
//! pairing of declared metadata with supplied state, recursive child construction,
//! featured/results layout selection, caller-supplied section overrides and the
//! detached copies used when a section receives its items. Translation is the
//! caller's port; an absent port keeps the trimmed key.

use std::collections::BTreeMap;

use effect_server::Request;
use maud::Markup;

use crate::definitions::{
    RESOURCE_LIST_LAYOUT_FEATURED, RESOURCE_SECTION_KEY_FEATURED, RESOURCE_SECTION_KEY_GRID,
    RESOURCE_SECTION_KEY_RESULTS, RESOURCE_SECTION_KEY_SECONDARY, Registry,
    ResourceListItemDefinition, ResourceListDefinition,
    ResourceListSectionDefinition, ResourceSectionKey, SharedList, find_resource_definition,
};
use crate::localization::TranslateKeyFunc;
use crate::queries::first_non_blank;

use super::list_vms::{
    ResourceItemActionVM, ResourceListItemVM, ResourceListSectionVM, ResourceMetaVM,
};

/// One supplied item metadata value.
#[derive(Clone, Debug, Default)]
pub struct ResourceListItemMetaState {
    /// A prepared metadata component.
    pub component: Option<Markup>,
    /// Meta value.
    pub value: String,
    /// Meta target.
    pub href: String,
}

/// One supplied list item.
#[derive(Clone, Debug, Default)]
pub struct ResourceListItemState {
    /// Stable item key.
    pub key: String,
    /// Item title.
    pub title: crate::commands::InputBytes,
    /// Item summary.
    pub summary: crate::commands::InputBytes,
    /// Detail target.
    pub detail_href: String,
    /// External target.
    pub external_href: String,
    /// Status value.
    pub status: String,
    /// Image target.
    pub image_url: crate::commands::InputBytes,
    /// Primary action label.
    pub action_label: String,
    /// Primary action target.
    pub action_href: String,
    /// Metadata values.
    pub meta: Vec<ResourceListItemMetaState>,
    /// Actions.
    pub actions: Vec<ResourceItemActionVM>,
    /// Nested child items.
    pub children: Vec<ResourceListItemState>,
}

/// One supplied section override.
#[derive(Clone, Debug, Default)]
pub struct ResourceListSectionState {
    /// Section title override.
    pub title: String,
    /// Section description override.
    pub description: String,
    /// Section items override.
    pub items: Vec<ResourceListItemVM>,
}

/// The supplied list state.
#[derive(Clone, Debug, Default)]
pub struct ResourceListState {
    /// Items to build.
    pub items: Vec<ResourceListItemVM>,
    /// Whether the leading item is highlighted.
    pub highlight_leading: bool,
    /// Section overrides keyed by section.
    pub sections: BTreeMap<ResourceSectionKey, ResourceListSectionState>,
}

/// Construct an unlinked metadata value.
#[must_use]
pub fn resource_meta_state_value(value: &str) -> ResourceListItemMetaState {
    ResourceListItemMetaState {
        value: value.to_owned(),
        ..ResourceListItemMetaState::default()
    }
}

/// Construct a linked metadata value.
#[must_use]
pub fn resource_meta_state_link(value: &str, href: &str) -> ResourceListItemMetaState {
    ResourceListItemMetaState {
        value: value.to_owned(),
        href: href.to_owned(),
        ..ResourceListItemMetaState::default()
    }
}

/// Copy supplied metadata values in order.
#[must_use]
pub fn resource_list_meta_states(values: Vec<ResourceListItemMetaState>) -> Vec<ResourceListItemMetaState> {
    values
}

/// Build list items from one declared item definition.
#[must_use]
pub fn build_resource_list_items(
    definition: &ResourceListItemDefinition,
    states: &[ResourceListItemState],
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> Vec<ResourceListItemVM> {
    let mut items = Vec::with_capacity(states.len());
    for state in states {
        let key = state.key.trim();
        if key.is_empty() {
            continue;
        }
        items.push(ResourceListItemVM {
            key: key.to_owned(),
            title: state.title.trim().to_owned(),
            summary: state.summary.trim().to_owned(),
            detail_href: state.detail_href.trim().to_owned(),
            external_href: state.external_href.trim().to_owned(),
            status: state.status.trim().to_owned(),
            image_url: state.image_url.trim().to_owned(),
            action_label: state.action_label.trim().to_owned(),
            action_href: state.action_href.trim().to_owned(),
            actions: detached_nonempty(clone_item_actions(&state.actions)),
            children: Some(SharedList::new(build_resource_list_items(definition, &state.children, request, translate_key))),
            meta: detached_nonempty(build_resource_list_item_meta(
                definition,
                &state.meta,
                request,
                translate_key,
            )),
        });
    }
    items
}

/// Build list items through a registered definition.
#[must_use]
pub fn build_resource_list_items_for_kind<R: Registry + ?Sized>(
    resources: &R,
    kind: &crate::definitions::ResourceKind,
    states: &[ResourceListItemState],
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> Vec<ResourceListItemVM> {
    let Some(definition) = find_resource_definition(Some(resources), kind) else {
        return Vec::new();
    };
    build_resource_list_items(&definition.list.item, states, request, translate_key)
}

/// Build the declared metadata projection for one item.
fn build_resource_list_item_meta(
    definition: &ResourceListItemDefinition,
    state: &[ResourceListItemMetaState],
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> Vec<ResourceMetaVM> {
    let declared = definition.meta.to_vec();
    if declared.is_empty() {
        return Vec::new();
    }
    let mut items = Vec::with_capacity(declared.len());
    for (index, meta) in declared.iter().enumerate() {
        let supplied = state.get(index).cloned().unwrap_or_default();
        items.push(ResourceMetaVM {
            component: supplied.component,
            label: first_non_blank(&[
                meta.label.clone(),
                list_builder_text(request, translate_key, &meta.label_key),
            ]),
            value: supplied.value,
            href: supplied.href,
        });
    }
    items
}

/// Build the declared list sections.
#[must_use]
pub fn build_resource_list_sections(
    definition: &ResourceListDefinition,
    state: &ResourceListState,
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> Vec<ResourceListSectionVM> {
    if definition.layout.as_str() == RESOURCE_LIST_LAYOUT_FEATURED {
        return build_featured_resource_list_sections(definition, state, request, translate_key);
    }
    if state.items.is_empty() {
        return Vec::new();
    }
    let declared = definition.sections.to_vec();
    let section = match declared.first() {
        Some(section) => section.clone(),
        None => ResourceListSectionDefinition {
            key: ResourceSectionKey::new(RESOURCE_SECTION_KEY_RESULTS),
            ..ResourceListSectionDefinition::default()
        },
    };
    let override_state = state.sections.get(&section.key).cloned().unwrap_or_default();
    vec![build_resource_list_section(
        &section,
        &override_state,
        &state.items,
        request,
        translate_key,
    )]
}

/// Build the declared list sections through a registered definition.
#[must_use]
pub fn build_resource_list_sections_for_kind<R: Registry + ?Sized>(
    resources: &R,
    kind: &crate::definitions::ResourceKind,
    state: &ResourceListState,
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> Vec<ResourceListSectionVM> {
    let Some(definition) = find_resource_definition(Some(resources), kind) else {
        return Vec::new();
    };
    build_resource_list_sections(&definition.list, state, request, translate_key)
}

/// Build one section, preferring the supplied override value.
fn build_resource_list_section(
    definition: &ResourceListSectionDefinition,
    state: &ResourceListSectionState,
    items: &[ResourceListItemVM],
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> ResourceListSectionVM {
    let items = if state.items.is_empty() {
        items
    } else {
        state.items.as_slice()
    };
    ResourceListSectionVM {
        key: definition.key.clone(),
        title: first_non_blank(&[
            state.title.clone(),
            list_builder_text(request, translate_key, &definition.title_key),
        ]),
        description: first_non_blank(&[
            state.description.clone(),
            list_builder_text(request, translate_key, &definition.description_key),
        ]),
        items: clone_list_items(items),
    }
}

/// Build the featured/secondary/grid sections.
fn build_featured_resource_list_sections(
    definition: &ResourceListDefinition,
    state: &ResourceListState,
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> Vec<ResourceListSectionVM> {
    let base_sections = resource_split_list_sections(&state.items, state.highlight_leading);
    if base_sections.is_empty() {
        return Vec::new();
    }
    let mut declared = BTreeMap::new();
    for section in definition.sections.to_vec() {
        declared.insert(section.key.clone(), section);
    }
    let mut sections = Vec::with_capacity(base_sections.len());
    for base in base_sections {
        let mut section = declared.get(&base.key).cloned().unwrap_or_default();
        section.key = base.key.clone();
        let override_state = state.sections.get(&base.key).cloned().unwrap_or_default();
        sections.push(build_resource_list_section(
            &section,
            &override_state,
            &base.items,
            request,
            translate_key,
        ));
    }
    sections
}

/// Split list items into the source featured/secondary/grid shape.
#[must_use]
pub fn resource_split_list_sections(
    items: &[ResourceListItemVM],
    highlight_leading: bool,
) -> Vec<ResourceListSectionVM> {
    if items.is_empty() {
        return Vec::new();
    }
    if !highlight_leading {
        return vec![ResourceListSectionVM {
            key: ResourceSectionKey::new(RESOURCE_SECTION_KEY_GRID),
            items: items.to_vec(),
            ..ResourceListSectionVM::default()
        }];
    }
    let mut sections = Vec::new();
    if let Some(first) = items.first() {
        sections.push(ResourceListSectionVM {
            key: ResourceSectionKey::new(RESOURCE_SECTION_KEY_FEATURED),
            items: vec![first.clone()],
            ..ResourceListSectionVM::default()
        });
    }
    if items.len() > 1 {
        let end = items.len().min(3);
        if let Some(secondary) = items.get(1..end) {
            sections.push(ResourceListSectionVM {
                key: ResourceSectionKey::new(RESOURCE_SECTION_KEY_SECONDARY),
                items: secondary.to_vec(),
                ..ResourceListSectionVM::default()
            });
        }
    }
    if items.len() > 3
        && let Some(grid) = items.get(3..)
    {
        sections.push(ResourceListSectionVM {
            key: ResourceSectionKey::new(RESOURCE_SECTION_KEY_GRID),
            items: grid.to_vec(),
            ..ResourceListSectionVM::default()
        });
    }
    sections
}

/// Copy list items with their metadata, actions and children detached.
fn clone_list_items(items: &[ResourceListItemVM]) -> Vec<ResourceListItemVM> {
    items.iter().map(|item| {
        let mut item = item.clone();
        item.meta = item.meta.as_ref().and_then(|values| detached_nonempty(values.to_vec()));
        item.actions = item.actions.as_ref().and_then(|values| detached_nonempty(values.to_vec()));
        item.children = item.children.as_ref().and_then(|values| {
            detached_nonempty(clone_list_items(&values.to_vec()))
        });
        item
    }).collect()
}

fn detached_nonempty<T>(values: Vec<T>) -> Option<SharedList<T>> {
    if values.is_empty() { None } else { Some(SharedList::new(values)) }
}

/// Copy item actions.
fn clone_item_actions(items: &[ResourceItemActionVM]) -> Vec<ResourceItemActionVM> {
    items.to_vec()
}

/// Resolve one builder text key.
fn list_builder_text(
    request: &Request,
    translate_key: Option<&TranslateKeyFunc<Request>>,
    key: &str,
) -> String {
    match translate_key {
        Some(translate) => translate(request, key),
        None => key.trim().to_owned(),
    }
}
