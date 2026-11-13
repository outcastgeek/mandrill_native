//! Redirect behavior: query-preserving route redirects, legacy redirect routes, SSE
//! URLs and mutation-result redirect targets.
//!
//! # Redirect responses
//!
//! A redirect target is resolved and rendered by the access owner's redirect response:
//! a relative target is resolved against the request path, the location value is escaped
//! and the read-method body is Maud-authored. This owner decides only *which* target a
//! redirect uses.
//!
//! # Query-preserving redirects
//!
//! The resolved target is trimmed; an absent builder, a blank route name or a blank
//! resolved target produces the not-found response. Any redirection status is retained and
//! any other status becomes the permanent redirection status. The request's query values
//! are appended to the resolved target, so repeated values and their order are retained and
//! the fragment of a supplied target stays where the target put it. Every redirection status
//! shares the same request-aware target resolution: a body-bearing status is rendered by the
//! access owner's redirect response, while a body-forbidden status (not modified) carries the
//! same resolved and escaped location without a body, because the local response contract
//! forbids a body there.
//!
//! # Mutation-result redirects
//!
//! An absent result, or a result whose redirect route is blank, resolves no target; an
//! absent builder then keeps that empty target. The redirect language is the result's own
//! language unless it is blank, in which case the caller's fallback language is used.
//! Redirect parameters are the result's shared parameter map read when the response runs,
//! flattened into lexically sorted key/value pairs with the values unescaped: encoding
//! belongs to the supplied path builder. The response is a see-other redirect, and an
//! empty target resolves to the request's own directory rather than to a status-text
//! response.
//!
//! # SSE URLs
//!
//! The page replaces the request's own page value while every other query value is
//! retained. The builder is invoked for the declared auxiliary route even when that route
//! is absent.

use std::collections::BTreeMap;
use std::sync::Arc;

use effect::{effect, effect_sync};
use effect_server::{BodyStatus, Reply, Request, native::axum::body::Body, reply};
use http::{HeaderName, HeaderValue, Method, StatusCode};

use crate::access::{hex_escape_non_ascii, new_redirect_access_response, resolved_redirect_target};
use crate::commands::MutationResult;
use crate::definitions::{
    RESOURCE_AUX_ROUTE_SSE, Registry, ResourceAuxRoute, ResourceKind, ResourceRouteSet,
    find_resource_definition,
};
use crate::queries::{append_encoded_query, clone_query, request_query_map};

use super::managed_routes::{ManagedRoute, PermissionMode, RouteHandler};
use super::paths::RoutePathBuilder;

/// Builds a localized route target for a mutation result.
///
/// The response language, the trimmed redirect route and the lexically sorted redirect
/// parameter pairs are supplied; the builder owns URL encoding.
pub type MutationResultRoutePathBuilder =
    Arc<dyn Fn(&str, &str, &[(String, String)]) -> String + Send + Sync + 'static>;

/// One legacy redirect route description.
#[derive(Clone)]
pub struct LegacyRoute {
    /// The route name.
    pub name: String,
    /// The declared method.
    pub method: String,
    /// The declared pattern.
    pub pattern: String,
    /// The route the redirect targets.
    pub target_route: String,
    /// The path builder used to resolve the target.
    pub route_path: Option<RoutePathBuilder>,
    /// The declared permissions.
    pub permissions: Vec<String>,
    /// The declared category.
    pub category: String,
}

/// The not-found status for the response contract.
fn not_found_status() -> BodyStatus {
    BodyStatus::new(StatusCode::NOT_FOUND).unwrap_or(BodyStatus::Ok)
}

/// The see-other status for the response contract.
fn see_other_status() -> BodyStatus {
    BodyStatus::new(StatusCode::SEE_OTHER).unwrap_or(BodyStatus::Ok)
}

/// The not-found response.
fn not_found_reply<E, R>() -> Reply<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    reply! {
        status(not_found_status());
        header(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff")
        );
        text(String::from("404 page not found\n"));
    }
}

/// The bodyless redirect response for a body-forbidden redirection status.
///
/// The location value is resolved by the same request-aware rule a body-bearing
/// redirection uses; only the body differs.
fn bodyless_redirect_reply<E, R>(status: StatusCode, request: &Request, target: &str) -> Reply<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let location = resolved_redirect_target(request, target);
    let mut response = http::Response::new(Body::empty());
    *response.status_mut() = status;
    if let Ok(location) =
        HeaderValue::from_bytes(hex_escape_non_ascii(location.as_str()).as_bytes())
    {
        response
            .headers_mut()
            .insert(http::header::LOCATION, location);
    }
    reply! { native(response); }
}

/// Build a query-preserving redirect handler.
///
/// The declared route name is passed to the builder unchanged. See the module
/// documentation for the not-found, status, body and query rules.
#[must_use]
pub fn new_query_preserving_route_redirect_handler<E, R>(
    route_name: &str,
    route_path: Option<RoutePathBuilder>,
    status: StatusCode,
) -> RouteHandler<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let route_name = route_name.to_owned();
    let status = if (300..=399).contains(&status.as_u16()) {
        status
    } else {
        StatusCode::PERMANENT_REDIRECT
    };
    Arc::new(move |request: &Request| {
        let target_request = request.clone();
        let response_request = request.clone();
        let route_name = route_name.clone();
        let route_path = route_path.clone();
        effect! {
            let target_request = target_request.clone();
            let response_request = response_request.clone();
            let route_name = route_name.clone();
            let route_path = route_path.clone();
            let resolved = effect_sync! { once; body: move || {
                let route_path = route_path.as_ref()?;
                if route_name.trim().is_empty() {
                    return None;
                }
                let target = route_path(&target_request, route_name.as_str(), &[]);
                let target = target.trim();
                if target.is_empty() {
                    return None;
                }
                Some(append_encoded_query(target, &request_query_map(&target_request)))
            }, }?;
            let Some(target) = resolved else {
                return not_found_reply();
            };
            match BodyStatus::new(status) {
                Ok(body_status) => new_redirect_access_response(
                    Some(Arc::new(move |_: &Request| target.clone())),
                    body_status,
                )(&response_request)?,
                Err(_) => bodyless_redirect_reply(status, &response_request, target.as_str()),
            }
        }
    })
}

/// Build a legacy management redirect route.
///
/// The route reads its pattern and redirects permanently to the declared target while
/// preserving the request's query.
#[must_use]
pub fn new_legacy_management_redirect_route<E, R>(
    name: &str,
    pattern: &str,
    target_route: &str,
    route_path: Option<RoutePathBuilder>,
    permissions: &[String],
    category: &str,
) -> ManagedRoute<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    ManagedRoute {
        name: name.to_owned(),
        method: Method::GET.as_str().to_owned(),
        pattern: pattern.to_owned(),
        handler: Some(new_query_preserving_route_redirect_handler(
            target_route,
            route_path,
            StatusCode::PERMANENT_REDIRECT,
        )),
        permissions: permissions.to_vec(),
        permission_mode: PermissionMode::default(),
        display_name: String::new(),
        description: String::new(),
        category: category.to_owned(),
    }
}

/// Build the server-sent-events URL for a route set and page.
///
/// The request's query values are retained, its page value is replaced, and the supplied
/// builder resolves the declared auxiliary route even when that route is absent.
#[must_use]
pub fn build_resource_sse_url(
    request: &Request,
    route_set: &ResourceRouteSet,
    page: i64,
    route_path: Option<&RoutePathBuilder>,
) -> String {
    let mut query = clone_query(&request_query_map(request));
    query.insert("page".into(), vec![page.to_string().into()]);
    let declared = route_set
        .aux
        .get(&ResourceAuxRoute::new(RESOURCE_AUX_ROUTE_SSE))
        .unwrap_or_default();
    let base = match route_path {
        Some(route_path) => route_path(request, declared.as_str(), &[]),
        None => declared,
    };
    append_encoded_query(base.as_str(), &query)
}

/// Build the server-sent-events URL for a registered kind.
///
/// The management set is selected when management is requested and its auxiliary map
/// declares entries; the public set is selected when its auxiliary map declares entries;
/// the management set is the remaining case. An unregistered kind resolves the empty
/// route set.
#[must_use]
pub fn build_resource_sse_url_for_kind<R: Registry + ?Sized>(
    request: &Request,
    resources: Option<&R>,
    kind: &ResourceKind,
    manage: bool,
    page: i64,
    route_path: Option<&RoutePathBuilder>,
) -> String {
    let Some(definition) = find_resource_definition(resources, kind) else {
        return build_resource_sse_url(request, &ResourceRouteSet::default(), page, route_path);
    };
    let management_aux = !definition.management_routes.aux.with(|aux| aux.is_empty());
    let public_aux = !definition.public_routes.aux.with(|aux| aux.is_empty());
    let route_set = if manage && management_aux {
        definition.management_routes.clone()
    } else if public_aux {
        definition.public_routes.clone()
    } else {
        definition.management_routes.clone()
    };
    build_resource_sse_url(request, &route_set, page, route_path)
}

/// Flatten redirect parameters into lexically sorted key/value pairs.
///
/// The values are passed through unchanged; the supplied path builder owns encoding.
#[must_use]
pub fn mutation_result_param_pairs(params: &BTreeMap<String, String>) -> Vec<(String, String)> {
    params
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Resolve the redirect target for a mutation result.
///
/// See the module documentation for the absent-result, blank-route, language and
/// parameter rules.
#[must_use]
pub fn mutation_result_path(
    result: Option<&MutationResult>,
    fallback_language: &str,
    route_path: Option<&MutationResultRoutePathBuilder>,
) -> String {
    let Some(result) = result else {
        return String::new();
    };
    let route = result.redirect_route.trim();
    if route.is_empty() {
        return String::new();
    }
    let Some(route_path) = route_path else {
        return route.to_owned();
    };
    let language = if result.redirect_language.trim().is_empty() {
        fallback_language
    } else {
        result.redirect_language.as_str()
    };
    let params = result.redirect_params.with(mutation_result_param_pairs);
    route_path(language, route, params.as_slice())
}

/// Build the see-other redirect response for a mutation result.
///
/// The target is resolved when the response runs, so the result's shared parameter map is
/// read at run time. See the module documentation for the empty-target rule.
#[must_use]
pub fn redirect_mutation_result<E, R>(
    result: Option<&MutationResult>,
    fallback_language: &str,
    route_path: Option<MutationResultRoutePathBuilder>,
) -> RouteHandler<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let result = result.cloned();
    let fallback_language = fallback_language.to_owned();
    let response = new_redirect_access_response::<E, R>(
        Some(Arc::new(move |request: &Request| {
            let target = mutation_result_path(
                result.as_ref(),
                fallback_language.as_str(),
                route_path.as_ref(),
            );
            resolved_redirect_target(request, target.as_str())
        })),
        see_other_status(),
    );
    Arc::new(move |request: &Request| response(request))
}
