//! Route-set selection.
//!
//! A definition declares two complete route sets, management and public. Selection
//! returns one whole set; no slot is merged with, or inherited from, the other set.
//!
//! # Presence
//!
//! A set declares routes when at least one slot holds a non-empty value or when the
//! auxiliary map holds an entry. Presence is tested on the declared values, so a slot
//! holding only whitespace still counts as declared and its value is returned
//! unchanged.
//!
//! # Order
//!
//! The management set is selected when management is requested (or when the request is
//! a management request) **and** the management set declares routes; otherwise the
//! public set when it declares routes; otherwise the management set. An unregistered
//! kind yields an empty set.

use effect_server::Request;

use crate::definitions::{
    RESOURCE_VIEW_MODE_DASHBOARD, RESOURCE_VIEW_MODE_EDIT, RESOURCE_VIEW_MODE_MANAGE,
    RESOURCE_VIEW_MODE_NEW, Registry, ResourceDefinition, ResourceKind, ResourceRouteSet,
    ResourceViewMode, find_resource_definition,
};

use super::composition_helpers::ManagementRequestDetector;

/// Whether the route set declares at least one slot.
#[must_use]
pub fn has_route_set(route_set: &ResourceRouteSet) -> bool {
    !route_set.dashboard.is_empty()
        || !route_set.list.is_empty()
        || !route_set.detail.is_empty()
        || !route_set.new.is_empty()
        || !route_set.new_post.is_empty()
        || !route_set.edit.is_empty()
        || !route_set.edit_post.is_empty()
        || !route_set.delete.is_empty()
        || !route_set.aux.with(|aux| aux.is_empty())
}

/// Resolve the route set for a surface.
#[must_use]
pub fn route_set_for_mode(
    definition: &ResourceDefinition,
    mode: &ResourceViewMode,
) -> ResourceRouteSet {
    let management_surface = matches!(
        mode.as_str(),
        RESOURCE_VIEW_MODE_MANAGE
            | RESOURCE_VIEW_MODE_DASHBOARD
            | RESOURCE_VIEW_MODE_NEW
            | RESOURCE_VIEW_MODE_EDIT
    );
    if management_surface && has_route_set(&definition.management_routes) {
        return definition.management_routes.clone();
    }
    if has_route_set(&definition.public_routes) {
        return definition.public_routes.clone();
    }
    definition.management_routes.clone()
}

/// Map the edit state onto a form surface.
#[must_use]
pub fn resource_form_mode(is_edit: bool) -> ResourceViewMode {
    if is_edit {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_EDIT)
    } else {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_NEW)
    }
}

/// Resolve the route set for a request.
#[must_use]
pub fn route_set_for_request(
    request: &Request,
    definition: &ResourceDefinition,
    is_management_request: Option<&ManagementRequestDetector>,
) -> ResourceRouteSet {
    let management_request = is_management_request.is_some_and(|detector| detector(request));
    if management_request && has_route_set(&definition.management_routes) {
        return definition.management_routes.clone();
    }
    if has_route_set(&definition.public_routes) {
        return definition.public_routes.clone();
    }
    definition.management_routes.clone()
}

/// Resolve the route set for a definition.
#[must_use]
pub fn resource_route_set_for_definition(
    definition: &ResourceDefinition,
    manage: bool,
) -> ResourceRouteSet {
    if manage && has_route_set(&definition.management_routes) {
        return definition.management_routes.clone();
    }
    if has_route_set(&definition.public_routes) {
        return definition.public_routes.clone();
    }
    definition.management_routes.clone()
}

/// Resolve the route set for a registered kind.
///
/// An unregistered kind yields an empty set.
#[must_use]
pub fn resource_route_set_for_kind<R: Registry + ?Sized>(
    resources: Option<&R>,
    kind: &ResourceKind,
    manage: bool,
) -> ResourceRouteSet {
    match find_resource_definition(resources, kind) {
        Some(definition) => resource_route_set_for_definition(&definition, manage),
        None => ResourceRouteSet::default(),
    }
}

/// Select the detail route name for a surface.
///
/// The value is returned exactly as declared for the selected side.
#[must_use]
pub fn resource_detail_route(manage: bool, public_route: &str, manage_route: &str) -> String {
    if manage {
        manage_route.to_owned()
    } else {
        public_route.to_owned()
    }
}
