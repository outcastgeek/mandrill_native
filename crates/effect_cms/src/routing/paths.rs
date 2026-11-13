//! Route paths, titles, breadcrumbs, page metadata and form targets.
//!
//! Every route name here is an open declared value: the owner trims for *deciding*
//! presence, then passes the declared value to the caller's path builder unchanged, so
//! the builder owns URL construction and encoding. An absent builder yields the empty
//! target rather than a guessed one.
//!
//! # Selection order
//!
//! Titles prefer the surface key and fall back to the list key. A management
//! breadcrumb prefers the management dashboard route, then the management list route,
//! then the public list route. Form submission targets prefer the declared edit-post
//! route (only for an edit with parameters), then the declared new-post route, then the
//! fallback edit route (again only for an edit with parameters), then the fallback new
//! route. A cancelled form returns to the dashboard, else the list.
//!
//! # Translation
//!
//! The translation port is the localization owner's [`TranslateKeyFunc`] over the
//! request. The key is trimmed; a blank key resolves to the empty string and an absent
//! port resolves the trimmed key itself. Translated values are trimmed where they are
//! combined.

use std::collections::BTreeMap;
use std::sync::Arc;

use effect_server::Request;

use crate::definitions::{
    RESOURCE_VIEW_MODE_DASHBOARD, RESOURCE_VIEW_MODE_DETAIL, RESOURCE_VIEW_MODE_EDIT,
    RESOURCE_VIEW_MODE_NEW, Registry, ResourceDefinition, ResourceKind, ResourceRouteSet,
    ResourceViewMode, find_resource_definition,
};
use crate::localization::TranslateKeyFunc;
use crate::queries::first_non_blank;

use super::route_sets::has_route_set;

/// Builds one route target for the request.
///
/// The route name is passed as declared and the parameters are ordered key/value
/// pairs; the builder owns URL encoding.
pub type RoutePathBuilder =
    Arc<dyn Fn(&Request, &str, &[(String, String)]) -> String + Send + Sync + 'static>;

/// Builds a localized route target from a route name and ordered parameters.
pub type LocalizedRoutePathFunc =
    Arc<dyn Fn(&str, &[(String, String)]) -> String + Send + Sync + 'static>;

/// Builds the breadcrumb for one definition and surface.
pub type ManagementBreadcrumbBuilder = Arc<
    dyn Fn(&Request, &ResourceDefinition, &ResourceViewMode) -> ManagementBreadcrumbVM
        + Send
        + Sync
        + 'static,
>;

/// Resolves the route set for one request and definition.
pub type RouteSetForRequestResolver =
    Arc<dyn Fn(&Request, &ResourceDefinition) -> ResourceRouteSet + Send + Sync + 'static>;

/// Resolves the route set for one definition and surface.
pub type RouteSetForModeResolver =
    Arc<dyn Fn(&ResourceDefinition, &ResourceViewMode) -> ResourceRouteSet + Send + Sync + 'static>;

/// Builds the canonical path for one definition and surface.
pub type CanonicalRoutePathBuilder =
    Arc<dyn Fn(&Request, &ResourceDefinition, &ResourceViewMode) -> String + Send + Sync + 'static>;

/// The resolved breadcrumb of a management page.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManagementBreadcrumbVM {
    /// The dashboard route name.
    pub dashboard_route: String,
    /// The localized dashboard label.
    pub dashboard_label: String,
    /// The localized current-surface label.
    pub current_label: String,
    /// The localized current-surface subtitle.
    pub current_subtitle: String,
}

/// The resolved metadata of a management page.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManagementPageMeta {
    /// The dashboard route name.
    pub dashboard_route: String,
    /// The localized dashboard label.
    pub dashboard_label: String,
    /// The localized page title.
    pub title: String,
    /// The localized page subtitle.
    pub subtitle: String,
}

/// Resolve one translation key over the request.
///
/// The key is trimmed; a blank key yields the empty string, and an absent port yields
/// the trimmed key.
#[must_use]
pub fn translate(
    translate_key: Option<&TranslateKeyFunc<Request>>,
    request: &Request,
    key: &str,
) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    match translate_key {
        Some(translate_key) => translate_key(request, key),
        None => key.to_owned(),
    }
}

/// Resolve the translated title for a surface.
#[must_use]
pub fn resource_mode_title(
    request: &Request,
    definition: &ResourceDefinition,
    mode: &ResourceViewMode,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> String {
    let display = &definition.display;
    let translate = |key: &str| translate(translate_key, request, key);
    match mode.as_str() {
        RESOURCE_VIEW_MODE_NEW => first_non_blank(&[
            translate(display.new_title_key.as_str()),
            translate(display.list_title_key.as_str()),
        ]),
        RESOURCE_VIEW_MODE_EDIT => first_non_blank(&[
            translate(display.edit_title_key.as_str()),
            translate(display.list_title_key.as_str()),
        ]),
        RESOURCE_VIEW_MODE_DASHBOARD => first_non_blank(&[
            translate(display.dashboard_title_key.as_str()),
            translate(display.list_title_key.as_str()),
        ]),
        RESOURCE_VIEW_MODE_DETAIL => first_non_blank(&[
            translate(display.detail_title_key.as_str()),
            translate(display.list_title_key.as_str()),
        ]),
        _ => translate(display.list_title_key.as_str()),
    }
}

/// Build the breadcrumb for a definition and surface.
#[must_use]
pub fn management_breadcrumb(
    request: &Request,
    definition: &ResourceDefinition,
    mode: &ResourceViewMode,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> ManagementBreadcrumbVM {
    let display = &definition.display;
    let subtitle_key = if mode.as_str() == RESOURCE_VIEW_MODE_DASHBOARD {
        first_non_blank(&[
            display.dashboard_subtitle_key.clone(),
            display.list_subtitle_key.clone(),
        ])
    } else {
        display.list_subtitle_key.clone()
    };
    ManagementBreadcrumbVM {
        dashboard_route: first_non_blank(&[
            definition.management_routes.dashboard.clone(),
            definition.management_routes.list.clone(),
            definition.public_routes.list.clone(),
        ]),
        dashboard_label: first_non_blank(&[
            translate(translate_key, request, display.dashboard_title_key.as_str()),
            translate(translate_key, request, display.list_title_key.as_str()),
        ]),
        current_label: resource_mode_title(request, definition, mode, translate_key),
        current_subtitle: translate(translate_key, request, subtitle_key.as_str()),
    }
}

/// Build the page metadata for a definition and surface.
///
/// An unregistered kind yields empty metadata.
#[must_use]
pub fn resource_page_meta<R: Registry + ?Sized>(
    request: &Request,
    resources: Option<&R>,
    kind: &ResourceKind,
    mode: &ResourceViewMode,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> ManagementPageMeta {
    let Some(definition) = find_resource_definition(resources, kind) else {
        return ManagementPageMeta::default();
    };
    let breadcrumb = management_breadcrumb(request, &definition, mode, translate_key);
    ManagementPageMeta {
        dashboard_route: breadcrumb.dashboard_route,
        dashboard_label: breadcrumb.dashboard_label,
        title: breadcrumb.current_label,
        subtitle: breadcrumb.current_subtitle,
    }
}

/// Build the list-surface page metadata for a registered kind.
///
/// A management request for a definition that declares management routes resolves the
/// management surface; every other request resolves the list surface.
#[must_use]
pub fn resource_list_page_meta<R: Registry + ?Sized>(
    request: &Request,
    resources: Option<&R>,
    kind: &ResourceKind,
    is_management_request: Option<&super::composition_helpers::ManagementRequestDetector>,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> ManagementPageMeta {
    let Some(definition) = find_resource_definition(resources, kind) else {
        return ManagementPageMeta::default();
    };
    let manage = is_management_request.is_some_and(|detector| detector(request))
        && has_route_set(&definition.management_routes);
    let mode = if manage {
        ResourceViewMode::new(crate::definitions::RESOURCE_VIEW_MODE_MANAGE)
    } else {
        ResourceViewMode::new(crate::definitions::RESOURCE_VIEW_MODE_LIST)
    };
    resource_page_meta(request, resources, kind, &mode, translate_key)
}

/// Build the detail-surface page metadata for a registered kind.
///
/// A management request for a definition that declares management routes resolves the
/// edit surface; every other request resolves the detail surface. A non-blank current
/// title replaces the resolved title.
#[must_use]
pub fn resource_detail_page_meta<R: Registry + ?Sized>(
    request: &Request,
    resources: Option<&R>,
    kind: &ResourceKind,
    current_title: &str,
    is_management_request: Option<&super::composition_helpers::ManagementRequestDetector>,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> ManagementPageMeta {
    let Some(definition) = find_resource_definition(resources, kind) else {
        return ManagementPageMeta::default();
    };
    let manage = is_management_request.is_some_and(|detector| detector(request))
        && has_route_set(&definition.management_routes);
    let mode = if manage {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_EDIT)
    } else {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_DETAIL)
    };
    let mut meta = resource_page_meta(request, resources, kind, &mode, translate_key);
    let current_title = current_title.trim();
    if !current_title.is_empty() {
        current_title.clone_into(&mut meta.title);
    }
    meta
}

/// Build the form-surface page metadata for a registered kind.
#[must_use]
pub fn resource_form_page_meta<R: Registry + ?Sized>(
    request: &Request,
    resources: Option<&R>,
    kind: &ResourceKind,
    mode: &ResourceViewMode,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> ManagementPageMeta {
    resource_page_meta(request, resources, kind, mode, translate_key)
}

/// Resolve the form submission target.
///
/// See the module documentation for the declared/fallback order. An edit with
/// parameters prefers the edit-post slots; every submission falls back to the
/// new-post slots. An absent builder yields the empty target.
#[must_use]
pub fn resource_form_action_href(
    request: &Request,
    route_set: &ResourceRouteSet,
    is_edit: bool,
    route_path: Option<&RoutePathBuilder>,
    edit_params: &[(String, String)],
    fallback_edit_route: &str,
    fallback_new_route: &str,
) -> String {
    let Some(route_path) = route_path else {
        return String::new();
    };
    let edit_with_params = is_edit && !edit_params.is_empty();
    if edit_with_params && !route_set.edit_post.trim().is_empty() {
        return route_path(request, route_set.edit_post.as_str(), edit_params);
    }
    if !route_set.new_post.trim().is_empty() {
        return route_path(request, route_set.new_post.as_str(), &[]);
    }
    if edit_with_params && !fallback_edit_route.trim().is_empty() {
        return route_path(request, fallback_edit_route, edit_params);
    }
    if !fallback_new_route.trim().is_empty() {
        return route_path(request, fallback_new_route, &[]);
    }
    String::new()
}

/// Resolve the cancelled-form target: the dashboard route, else the list route.
#[must_use]
pub fn resource_form_cancel_href(
    request: &Request,
    route_set: &ResourceRouteSet,
    route_path: Option<&RoutePathBuilder>,
) -> String {
    let route_name = first_non_blank(&[route_set.dashboard.clone(), route_set.list.clone()]);
    if route_name.is_empty() {
        return String::new();
    }
    let Some(route_path) = route_path else {
        return String::new();
    };
    route_path(request, route_name.as_str(), &[])
}

/// Build the localized path map for a route.
///
/// The same builder and parameters are applied once per language key; the language is
/// the map key and is not passed to the builder. An absent route, an empty language
/// list or an absent builder yields no map at all.
#[must_use]
pub fn resource_localized_paths(
    route: &str,
    langs: &[String],
    route_path: Option<&LocalizedRoutePathFunc>,
    params: &[(String, String)],
) -> Option<BTreeMap<String, String>> {
    if route.trim().is_empty() || langs.is_empty() {
        return None;
    }
    let route_path = route_path?;
    let mut paths = BTreeMap::new();
    for lang in langs {
        paths.insert(lang.clone(), route_path(route, params));
    }
    Some(paths)
}

/// Resolve an optional route target.
///
/// A blank route name or an absent builder yields the empty target.
#[must_use]
pub fn optional_route_path(
    request: &Request,
    route: &str,
    route_path: Option<&RoutePathBuilder>,
    params: &[(String, String)],
) -> String {
    if route.trim().is_empty() {
        return String::new();
    }
    let Some(route_path) = route_path else {
        return String::new();
    };
    route_path(request, route, params)
}
