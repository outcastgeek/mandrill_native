use std::collections::BTreeMap;
use std::sync::Arc;

use effect::{Effect, effect_sync};
use effect_server::Request;

use crate::definitions::{self, ResourceDashboardActionDefinition, ResourceDefinition, ResourceRouteSlot};
use crate::localization::TranslateKeyFunc;
use crate::queries::{self, ResourceListQuery};
use super::{ActionVM, DashboardSearchOptions, DashboardSearchVM, new_dashboard_search_vm};

/// Builds a dashboard route for an optional request.
pub type DashboardRoutePath = Arc<dyn Fn(Option<&Request>, &str, &[(String, String)]) -> String + Send + Sync + 'static>;

/// Select a route name for an action slot.
#[must_use]
pub fn dashboard_action_route_name(definition: &ResourceDefinition, slot: &ResourceRouteSlot) -> String {
    let m = &definition.management_routes;
    let p = &definition.public_routes;
    let values = match slot.as_str() {
        definitions::RESOURCE_ROUTE_SLOT_DASHBOARD => vec![m.dashboard.clone(), p.dashboard.clone(), m.list.clone(), p.list.clone()],
        definitions::RESOURCE_ROUTE_SLOT_LIST => vec![m.list.clone(), p.list.clone()],
        definitions::RESOURCE_ROUTE_SLOT_DETAIL => vec![m.detail.clone(), p.detail.clone()],
        definitions::RESOURCE_ROUTE_SLOT_NEW => vec![m.new.clone(), p.new.clone()],
        definitions::RESOURCE_ROUTE_SLOT_NEW_POST => vec![m.new_post.clone(), p.new_post.clone()],
        definitions::RESOURCE_ROUTE_SLOT_EDIT => vec![m.edit.clone(), p.edit.clone()],
        definitions::RESOURCE_ROUTE_SLOT_EDIT_POST => vec![m.edit_post.clone(), p.edit_post.clone()],
        definitions::RESOURCE_ROUTE_SLOT_DELETE => vec![m.delete.clone(), p.delete.clone()],
        _ => Vec::new(),
    };
    queries::first_non_blank(&values)
}

fn action_href(request: Option<&Request>, definition: &ResourceDefinition, action: &ResourceDashboardActionDefinition, route_path: Option<&DashboardRoutePath>) -> String {
    let mut route = action.route_name.trim().to_owned();
    if route.is_empty() { route = dashboard_action_route_name(definition, &action.route_slot); }
    if route.is_empty() { return String::new(); }
    if definitions::resource_uses_unified_management_routes(definition)
        && action.route_slot.as_str() == definitions::RESOURCE_ROUTE_SLOT_LIST
        && action.route_name.is_empty() && action.filters.is_empty()
        && route == dashboard_action_route_name(definition, &ResourceRouteSlot::new(definitions::RESOURCE_ROUTE_SLOT_DASHBOARD)) {
        return String::new();
    }
    let base = route_path.map_or_else(|| route.clone(), |path| path(request, &route, &[]));
    if action.filters.is_empty() { return base; }
    let filters: BTreeMap<_, _> = definition.filters.to_vec().into_iter().map(|filter| (filter.key.clone(), filter)).collect();
    let mut query: crate::queries::QueryValues = BTreeMap::new();
    for filter in action.filters.to_vec() {
        if let Some(declared) = filters.get(&filter.key) {
            let param = queries::filter_query_param(declared);
            let value = filter.value.as_str().trim();
            if value.is_empty() { query.remove(param.as_bytes()); } else { query.insert(param.into(), vec![value.into()]); }
        }
    }
    queries::append_encoded_query(&base, &query)
}

/// Resolve an action target lazily.
pub fn dashboard_action_href<E: Send + 'static, R: Send + Sync + 'static>(request: Option<Request>, definition: ResourceDefinition, action: ResourceDashboardActionDefinition, route_path: Option<DashboardRoutePath>) -> Effect<String, E, R> {
    effect_sync! { body: move || action_href(request.as_ref(), &definition, &action, route_path.as_ref()) }
}

/// Build the declared dashboard actions lazily.
pub fn build_dashboard_actions<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>(context: Arc<C>, request: Option<Request>, definition: ResourceDefinition, route_path: Option<DashboardRoutePath>, translate: Option<TranslateKeyFunc<C>>) -> Effect<Vec<ActionVM>, E, R> {
    effect_sync! { body: move || {
        let mut actions = Vec::new();
        for index in 0..definition.dashboard_actions.len() {
            let action = definition.dashboard_actions.with(|values| values.get(index).cloned());
            if let Some(action) = action {
                let href = action_href(request.as_ref(), &definition, &action, route_path.as_ref());
                if href.is_empty() { continue; }
                let label = translate.as_ref().map_or_else(|| action.label_key.trim().to_owned(), |f| f(&context, &action.label_key));
                actions.push(ActionVM { label, href, icon: action.icon.trim().to_owned(), location: action.location });
            }
        }
        actions
    } }
}

/// Read the dashboard search query.
#[must_use]
pub fn dashboard_search_query_value(request: Option<&Request>) -> crate::commands::InputBytes {
    request.map_or_else(crate::commands::InputBytes::default, |request| queries::request_query_value(request, queries::SEARCH_QUERY_PARAM))
}

fn search_action(request: Option<&Request>, route: &str, route_path: Option<&DashboardRoutePath>) -> String {
    if !route.trim().is_empty() {
        if let Some(path) = route_path {
            let href = path(request, route, &[]);
            if !href.trim().is_empty() { return href.trim().to_owned(); }
        }
    }
    request.map_or_else(String::new, |r| r.uri().path().trim().to_owned())
}

/// Resolve the dashboard search target lazily.
pub fn dashboard_search_action<E: Send + 'static, R: Send + Sync + 'static>(request: Option<Request>, route: String, route_path: Option<DashboardRoutePath>) -> Effect<String, E, R> {
    effect_sync! { body: move || search_action(request.as_ref(), &route, route_path.as_ref()) }
}

/// Construct resource-specific search state lazily.
pub fn resource_dashboard_search_vm<C: Send + Sync + 'static, E: Send + 'static, R: Send + Sync + 'static>(context: Arc<C>, request: Option<Request>, definition: ResourceDefinition, query: ResourceListQuery, route_path: Option<DashboardRoutePath>, translate: Option<TranslateKeyFunc<C>>) -> Effect<DashboardSearchVM, E, R> {
    effect_sync! { body: move || {
        let route = queries::first_non_blank(&[definition.management_routes.dashboard.clone(), definition.management_routes.list.clone()]);
        let action = search_action(request.as_ref(), &route, route_path.as_ref());
        let value = if query.search.trim().is_empty() { dashboard_search_query_value(request.as_ref()) } else { query.search.trim().to_owned() };
        let label = |key: &str| {
            let translated = translate.as_ref().map_or_else(|| key.to_owned(), |f| f(&context, key));
            if translated.trim().is_empty() { key.to_owned() } else { translated.trim().to_owned() }
        };
        new_dashboard_search_vm(DashboardSearchOptions { enabled: !action.is_empty(), id: format!("{}-dashboard-search", definition.kind.as_str()), form_id: format!("{}-dashboard-search-form", definition.kind.as_str()), name: queries::SEARCH_QUERY_PARAM.to_owned(), value, placeholder: label("common.search"), aria_label: label("common.search"), action, shortcut_label: label("common.search_shortcut") })
    } }
}

/// Resolve the dashboard search destination using the supplied route capability.
pub fn dashboard_search_action_value(request:Option<&Request>,route:&str,route_path:Option<&DashboardRoutePath>)->String {search_action(request,route,route_path)}
