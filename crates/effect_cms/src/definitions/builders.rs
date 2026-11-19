//! Definition builders and the owner's management-route normalization.
//!
//! Every builder copies its *outer* input collection into its own collection while
//! the elements keep the model's shared reference semantics. No extra rule, default
//! or normalization is added: the featured and results list layouts are the section
//! lists, and unified routing only maps the dashboard route onto a blank list route.

use super::model::{
    ResourceDashboardDefinition, ResourceDashboardSectionDefinition, ResourceDefinition,
    ResourceDetailDefinition, ResourceDetailSectionDefinition, ResourceFormDefinition,
    ResourceFormFieldDefinition, ResourceFormSectionDefinition, ResourceListDefinition,
    ResourceListItemDefinition, ResourceListItemMetaDefinition, ResourceListSectionDefinition,
    ResourceRouteSet,
};
use super::relations::clone_relations;
use super::sharing::SharedList;
use super::vocabulary::{
    RESOURCE_LIST_LAYOUT_FEATURED, RESOURCE_LIST_LAYOUT_RESULTS,
    RESOURCE_MANAGEMENT_ROUTE_MODE_UNIFIED, RESOURCE_SECTION_KEY_FEATURED,
    RESOURCE_SECTION_KEY_GRID, RESOURCE_SECTION_KEY_RESULTS, RESOURCE_SECTION_KEY_SECONDARY,
    ResourceListLayout, ResourceSectionKey,
};

/// Build a form definition from its sections.
#[must_use]
pub fn new_resource_form_definition(
    sections: Vec<ResourceFormSectionDefinition>,
) -> ResourceFormDefinition {
    ResourceFormDefinition {
        sections: SharedList::new(sections),
    }
}

/// Build an untitled form section.
#[must_use]
pub fn new_resource_form_section(
    key: ResourceSectionKey,
    fields: Vec<ResourceFormFieldDefinition>,
) -> ResourceFormSectionDefinition {
    ResourceFormSectionDefinition {
        key,
        title_key: String::new(),
        description_key: String::new(),
        fields: SharedList::new(fields),
    }
}

/// Build a titled form section.
#[must_use]
pub fn new_titled_resource_form_section(
    key: ResourceSectionKey,
    title_key: String,
    description_key: String,
    fields: Vec<ResourceFormFieldDefinition>,
) -> ResourceFormSectionDefinition {
    let mut section = new_resource_form_section(key, fields);
    section.title_key = title_key;
    section.description_key = description_key;
    section
}

/// Build a list definition from ordered section keys and item meta fields.
#[must_use]
pub fn new_resource_list_definition(
    layout: ResourceListLayout,
    sections: Vec<ResourceSectionKey>,
    meta: Vec<ResourceListItemMetaDefinition>,
) -> ResourceListDefinition {
    let mut definition = ResourceListDefinition {
        layout,
        sections: SharedList::default(),
        item: ResourceListItemDefinition {
            meta: SharedList::new(meta),
        },
    };
    for section in sections {
        definition.sections.push(ResourceListSectionDefinition {
            key: section,
            title_key: String::new(),
            description_key: String::new(),
        });
    }
    definition
}

/// Build the featured list layout.
#[must_use]
pub fn new_featured_resource_list_definition(
    meta: Vec<ResourceListItemMetaDefinition>,
) -> ResourceListDefinition {
    new_resource_list_definition(
        ResourceListLayout::new(RESOURCE_LIST_LAYOUT_FEATURED),
        vec![
            ResourceSectionKey::new(RESOURCE_SECTION_KEY_FEATURED),
            ResourceSectionKey::new(RESOURCE_SECTION_KEY_SECONDARY),
            ResourceSectionKey::new(RESOURCE_SECTION_KEY_GRID),
        ],
        meta,
    )
}

/// Build the results list layout.
#[must_use]
pub fn new_results_resource_list_definition(
    meta: Vec<ResourceListItemMetaDefinition>,
) -> ResourceListDefinition {
    new_resource_list_definition(
        ResourceListLayout::new(RESOURCE_LIST_LAYOUT_RESULTS),
        vec![ResourceSectionKey::new(RESOURCE_SECTION_KEY_RESULTS)],
        meta,
    )
}

/// Build a detail definition from its sections.
#[must_use]
pub fn new_resource_detail_definition(
    sections: Vec<ResourceDetailSectionDefinition>,
) -> ResourceDetailDefinition {
    ResourceDetailDefinition {
        sections: SharedList::new(sections),
    }
}

/// Build a dashboard definition from its sections.
#[must_use]
pub fn new_resource_dashboard_definition(
    sections: Vec<ResourceDashboardSectionDefinition>,
) -> ResourceDashboardDefinition {
    ResourceDashboardDefinition {
        sections: SharedList::new(sections),
    }
}

/// Apply the owner's management-route normalization.
///
/// Relation declarations are detached, and unified management routing maps the
/// dashboard route onto the list slot. Nothing else in the declaration changes.
#[must_use]
pub fn normalize_resource_definition(mut definition: ResourceDefinition) -> ResourceDefinition {
    let relations = clone_relations(&definition.relations);
    definition.relations = relations;
    normalize_owned_resource_definition(definition)
}

/// Take ownership of detached relation data during prepared registration.
pub(super) fn normalize_owned_resource_definition(mut definition: ResourceDefinition) -> ResourceDefinition {
    if definition.management_route_mode.as_str() == RESOURCE_MANAGEMENT_ROUTE_MODE_UNIFIED {
        let routes = unified_management_route_set(definition.management_routes);
        definition.management_routes = routes;
    }
    definition
}

/// Map the dashboard route onto the list slot, keeping every other route name and
/// the shared aux routes.
#[must_use]
pub fn unified_management_route_set(mut route_set: ResourceRouteSet) -> ResourceRouteSet {
    let list = first_non_blank(&[route_set.dashboard.as_str(), route_set.list.as_str()]);
    route_set.list = list;
    route_set
}

/// Whether the definition uses unified management routes.
#[must_use]
pub fn resource_uses_unified_management_routes(definition: &ResourceDefinition) -> bool {
    definition.management_route_mode.as_str() == RESOURCE_MANAGEMENT_ROUTE_MODE_UNIFIED
}

/// The first trimmed non-blank value, or the empty string.
fn first_non_blank(values: &[&str]) -> String {
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    String::new()
}
