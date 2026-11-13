//! Composition helpers for route path builders, request detection, translated
//! action labels and canonical detail paths.
//!
//! These helpers bind the caller's narrow capabilities into the contracts the rest of
//! the routing owner and the presentation owners consume. They contain no persistence
//! and no hidden lookup: every capability is a constructor argument and stays inside
//! the returned closure.
//!
//! # Builder fallbacks
//!
//! A localized path builder trims the route name and yields the empty target for a
//! blank name. A route resolver that reports no path, or that is absent, keeps the
//! trimmed route name as the target; a successful resolution is kept even when it
//! resolves to the empty string. Language prefixing runs only when both the request
//! language and the prefix capabilities are supplied.
//!
//! # Canonical detail parameters
//!
//! `id` and `slug` prefer the captured path value; the parent value prefers the
//! captured parent identifier, then the captured parent name, then the same two
//! request query values, and one resolved value is published under each configured
//! parent key. Parameters are appended in identifier, slug, parent, parent-name order;
//! forms without a captured value are omitted. A missing path builder returns the
//! trimmed route name.

use std::sync::Arc;

use effect_server::Request;

use crate::commands::{ActionLabelKeys, ActionLabelResolver, new_action_label_resolver};
use crate::definitions::{
    RESOURCE_ID_PATH_PARAM, RESOURCE_PARENT_NAME_PATH_PARAM, RESOURCE_PARENT_PATH_PARAM,
    RESOURCE_SLUG_PATH_PARAM, RESOURCE_VIEW_MODE_DETAIL, RESOURCE_VIEW_MODE_LIST,
    RESOURCE_VIEW_MODE_PUBLIC, ResourceDefinition, ResourceViewMode,
};
use crate::localization::TranslateKeyFunc;
use crate::queries::{CanonicalRouteParams, first_non_blank};

use super::paths;
use super::paths::RoutePathBuilder;

/// The default save-action label key.
pub const DEFAULT_SAVE_ACTION_LABEL_KEY: &str = "common.save";
/// The default delete-action label key.
pub const DEFAULT_DELETE_ACTION_LABEL_KEY: &str = "common.delete";
/// The default publish-action label key.
pub const DEFAULT_PUBLISH_ACTION_LABEL_KEY: &str = "content.form.publish";
/// The default draft-action label key.
pub const DEFAULT_DRAFT_ACTION_LABEL_KEY: &str = "content.form.save_draft";
/// The default translate-action label key.
pub const DEFAULT_TRANSLATE_ACTION_LABEL_KEY: &str = "localized_tabs.translate_now";

/// Resolves a route name and ordered parameters to a path.
///
/// No path means the name could not be resolved; the caller then keeps the route name.
pub type RouteToPathFunc =
    Arc<dyn Fn(&str, &[(String, String)]) -> Option<String> + Send + Sync + 'static>;

/// Resolves the request's language.
pub type PathLanguageFunc = Arc<dyn Fn(&Request) -> String + Send + Sync + 'static>;

/// Prefixes a path with a language segment.
pub type LanguagePathPrefixer = Arc<dyn Fn(&str, &str) -> String + Send + Sync + 'static>;

/// Removes a language segment from a path.
pub type LanguagePathStripper = Arc<dyn Fn(&[u8]) -> crate::commands::InputBytes + Send + Sync + 'static>;

/// Resolves a translation key over the request.
pub type VariadicTranslateKeyFunc = Arc<dyn Fn(&Request, &str) -> String + Send + Sync + 'static>;

/// Detects whether a request addresses a management surface.
pub type ManagementRequestDetector = Arc<dyn Fn(&Request) -> bool + Send + Sync + 'static>;

/// Reads one request value for a key.
pub type RequestValueFunc = Arc<dyn Fn(&Request, &str) -> String + Send + Sync + 'static>;

/// The action-label keys the CMS supplies for its own actions.
#[must_use]
pub fn default_action_label_keys(attach: &str) -> ActionLabelKeys {
    ActionLabelKeys {
        save: DEFAULT_SAVE_ACTION_LABEL_KEY.to_owned(),
        delete: DEFAULT_DELETE_ACTION_LABEL_KEY.to_owned(),
        publish: DEFAULT_PUBLISH_ACTION_LABEL_KEY.to_owned(),
        draft: DEFAULT_DRAFT_ACTION_LABEL_KEY.to_owned(),
        attach: attach.to_owned(),
        translate: DEFAULT_TRANSLATE_ACTION_LABEL_KEY.to_owned(),
    }
}

/// Build the default action-label resolver.
///
/// The supplied port is always wrapped: a blank key resolves to the empty string and an
/// absent port resolves the key itself, so the resolver receives one translation
/// contract regardless of the caller's port.
#[must_use]
pub fn new_default_action_label_resolver(
    translate_key: Option<TranslateKeyFunc<Request>>,
    attach: &str,
) -> ActionLabelResolver {
    let resolver: TranslateKeyFunc<Request> =
        Arc::new(move |request, key| paths::translate(translate_key.as_ref(), request, key));
    new_action_label_resolver(default_action_label_keys(attach), Some(resolver))
}

/// Bind a translation port to the owner's key fallback.
#[must_use]
pub fn new_translate_key_resolver(
    translate_key: Option<TranslateKeyFunc<Request>>,
) -> TranslateKeyFunc<Request> {
    Arc::new(move |request, key| paths::translate(translate_key.as_ref(), request, key))
}

/// Bind a translation port to the owner's key fallback, dropping the unused
/// parameter list.
#[must_use]
pub fn new_variadic_translate_key_resolver(
    translate_key: Option<VariadicTranslateKeyFunc>,
) -> TranslateKeyFunc<Request> {
    Arc::new(move |request, key| {
        let key = key.trim();
        if key.is_empty() {
            return String::new();
        }
        match translate_key.as_ref() {
            Some(translate_key) => translate_key(request, key),
            None => key.to_owned(),
        }
    })
}

/// Build a localized route path builder.
#[must_use]
pub fn new_localized_route_path_builder(
    route_to_path: Option<RouteToPathFunc>,
    request_language: Option<PathLanguageFunc>,
    prefix_path: Option<LanguagePathPrefixer>,
) -> RoutePathBuilder {
    Arc::new(move |request, route_name, params| {
        let route_name = route_name.trim();
        if route_name.is_empty() {
            return String::new();
        }
        let mut path = route_name.to_owned();
        if let Some(route_to_path) = route_to_path.as_ref()
            && let Some(resolved) = route_to_path(route_name, params)
        {
            path = resolved;
        }
        match (request_language.as_ref(), prefix_path.as_ref()) {
            (Some(request_language), Some(prefix_path)) => {
                prefix_path(request_language(request).as_str(), path.as_str())
            }
            _ => path,
        }
    })
}

/// Build a management-request detector for a path prefix.
///
/// The prefix is trimmed; an empty prefix never matches. The request path is
/// percent-decoded before it is optionally stripped of a language segment and compared
/// with the prefix, so an escaped spelling of the prefix matches and a literal `+` stays
/// a plus. Decoded bytes remain intact through the optional stripper and prefix
/// comparison, including non-UTF8 path segments.
#[must_use]
pub fn new_path_prefix_management_request_detector(
    strip_language_prefix: Option<LanguagePathStripper>,
    prefix: &str,
) -> ManagementRequestDetector {
    let prefix = prefix.trim().to_owned();
    Arc::new(move |request| {
        if prefix.is_empty() {
            return false;
        }
        let escaped = request.uri().path();
        let decoded = percent_encoding::percent_decode_str(escaped).collect::<Vec<u8>>();
        let path = match strip_language_prefix.as_ref() {
            Some(strip) => strip(&decoded),
            None => crate::commands::InputBytes::from(decoded),
        };
        path.as_bytes().starts_with(prefix.as_bytes())
    })
}

/// The canonical detail route parameters.
#[must_use]
pub fn default_canonical_route_params() -> CanonicalRouteParams {
    CanonicalRouteParams {
        id_param: RESOURCE_ID_PATH_PARAM.to_owned(),
        slug_param: RESOURCE_SLUG_PATH_PARAM.to_owned(),
        parent_param: RESOURCE_PARENT_PATH_PARAM.to_owned(),
        parent_name_param: RESOURCE_PARENT_NAME_PATH_PARAM.to_owned(),
    }
}

/// Build the canonical detail path for a definition and surface.
///
/// See the module documentation for the parameter order and fallbacks. An absent
/// builder returns the resolved route name; an unresolved route name returns the empty
/// target.
#[must_use]
pub fn canonical_route_path(
    request: &Request,
    definition: &ResourceDefinition,
    mode: &ResourceViewMode,
    route_path: Option<&RoutePathBuilder>,
    request_query_value: Option<&RequestValueFunc>,
    path_value: Option<&RequestValueFunc>,
    params: &CanonicalRouteParams,
) -> String {
    let management_list = definition.management_routes.list.as_str();
    let public_list = definition.public_routes.list.as_str();
    let is_detail = mode.as_str() == RESOURCE_VIEW_MODE_DETAIL;
    let route_name = if is_detail {
        first_non_blank(&[
            definition.public_routes.detail.clone(),
            definition.management_routes.detail.clone(),
            public_list.to_owned(),
            management_list.to_owned(),
        ])
    } else if matches!(
        mode.as_str(),
        RESOURCE_VIEW_MODE_LIST | RESOURCE_VIEW_MODE_PUBLIC
    ) {
        first_non_blank(&[public_list.to_owned(), management_list.to_owned()])
    } else {
        first_non_blank(&[management_list.to_owned(), public_list.to_owned()])
    };
    if route_name.is_empty() {
        return String::new();
    }
    if !is_detail {
        return match route_path {
            Some(route_path) => route_path(request, route_name.as_str(), &[]),
            None => route_name,
        };
    }

    let id_param = first_non_blank(&[params.id_param.clone(), RESOURCE_ID_PATH_PARAM.to_owned()]);
    let slug_param = first_non_blank(&[
        params.slug_param.clone(),
        RESOURCE_SLUG_PATH_PARAM.to_owned(),
    ]);
    let parent_param = params.parent_param.trim().to_owned();
    let parent_name_param = params.parent_name_param.trim().to_owned();

    let request_value = |key: &str| -> String {
        if key.trim().is_empty() {
            return String::new();
        }
        match request_query_value {
            Some(read) => read(request, key),
            None => String::new(),
        }
    };
    let captured_value = |key: &str| -> String {
        if key.trim().is_empty() {
            return String::new();
        }
        match path_value {
            Some(read) => read(request, key),
            None => String::new(),
        }
    };

    let mut values: Vec<(String, String)> = Vec::with_capacity(4);
    let id = first_non_blank(&[
        captured_value(id_param.as_str()),
        request_value(id_param.as_str()),
    ]);
    if !id.is_empty() {
        values.push((id_param, id));
    }
    let slug = first_non_blank(&[
        captured_value(slug_param.as_str()),
        request_value(slug_param.as_str()),
    ]);
    if !slug.is_empty() {
        values.push((slug_param, slug));
    }
    let parent = first_non_blank(&[
        captured_value(parent_param.as_str()),
        captured_value(parent_name_param.as_str()),
        request_value(parent_param.as_str()),
        request_value(parent_name_param.as_str()),
    ]);
    if !parent.is_empty() {
        if !parent_param.is_empty() {
            values.push((parent_param, parent.clone()));
        }
        if !parent_name_param.is_empty() {
            values.push((parent_name_param, parent));
        }
    }
    match route_path {
        Some(route_path) => route_path(request, route_name.as_str(), values.as_slice()),
        None => route_name,
    }
}
