//! The W3 list page service and its prepared stages.
//!
//! One page call resolves the registered definition, decides the list or management
//! surface, resolves the SEO value through the SEO owner, applies the filtered-list
//! indexability rule, projects the filter options/toolbar/pagination skeleton and
//! finally offers the exact page value to the consumer's validator. The stages are
//! private prepared values consumed by the next stage, so the SEO callback runs at
//! most once per page call and no caller can forge or substitute a stage. The
//! definition lookup, SEO effect and validator all stay lazy inside the returned
//! [`Effect`]; defects, interruption and cleanup failures retain their own channels.

use std::sync::Arc;

use effect::{Effect, effect, effect_map_error, effect_try_sync};
use effect_server::Request;

use crate::definitions::{
    RESOURCE_FIELD_KEY_SEARCH, RESOURCE_FIELD_KEY_SORT, RESOURCE_FILTER_MODE_MULTI_ALL,
    RESOURCE_FILTER_SEARCH, RESOURCE_FILTER_SORT, RESOURCE_VIEW_MODE_LIST,
    RESOURCE_VIEW_MODE_MANAGE, Registry, ResourceDefinition, ResourceFieldKey,
    ResourceFilterDefinition, ResourceKind, ResourceRouteSet, ResourceViewMode,
};
use crate::queries::{
    DEFAULT_MAX_FILTER_VALUES, FilterValueOptions, ResourceListQuery, append_encoded_query,
    filter_query_param, first_non_blank, normalize_filter_values, request_query_map,
    resource_filter_mode,
};
use crate::routing::has_route_set;
use crate::seo::{SEOVM, Service as SEOService};

use super::contracts::{ListPageOptions, SEOView, resolve_breadcrumb};
use super::error::RenderingError;
use super::list_vms::{
    ListFiltersVM, ListPaginationVM, ListToolbarVM, ResourceFilterOptionVM, ResourceListPageVM,
};

/// The resolved definition stage.
#[derive(Clone)]
pub(super) struct ListDefinitionResolved {
    /// Request context.
    request: Request,
    /// Resource kind.
    kind: ResourceKind,
    /// Admitted query.
    query: ResourceListQuery,
    /// Registered definition.
    definition: ResourceDefinition,
}

/// The resolved mode stage.
#[derive(Clone)]
pub(super) struct ListModeResolved {
    /// The definition stage.
    definition: ListDefinitionResolved,
    /// Selected surface.
    mode: ResourceViewMode,
}

/// The resolved SEO stage.
#[derive(Clone)]
pub(super) struct ListSeoResolved {
    /// The mode stage.
    mode: ListModeResolved,
    /// Published SEO value.
    seo: SEOVM,
}

/// Produces list page view models.
pub struct ListService<E>
where
    E: Send + 'static,
{
    resources: Arc<dyn Registry + Send + Sync>,
    seo: SEOService<E>,
    options: ListPageOptions<E>,
}

impl<E> Clone for ListService<E>
where
    E: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            resources: Arc::clone(&self.resources),
            seo: self.seo.clone(),
            options: self.options.clone(),
        }
    }
}

impl<E> ListService<E>
where
    E: Send + 'static,
{
    /// Construct the list page service from the required registry and SEO capability.
    #[must_use]
    pub fn new(
        resources: Arc<dyn Registry + Send + Sync>,
        seo: SEOService<E>,
        options: ListPageOptions<E>,
    ) -> Self {
        Self {
            resources,
            seo,
            options,
        }
    }

    /// Produce the list page for a kind and admitted query.
    ///
    /// The returned effect is lazy and reusable. It resolves the definition, decides
    /// the surface, resolves SEO, assembles the page and invokes the validator in
    /// that order when it runs.
    #[must_use]
    pub fn list_page<R>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        query: ResourceListQuery,
    ) -> Effect<ResourceListPageVM, RenderingError<E>, R>
    where
        R: Send + Sync + 'static,
    {
        let resources = Arc::clone(&self.resources);
        let seo = self.seo.clone();
        let options = self.options.clone();
        let request = request.clone();
        let kind = kind.clone();
        effect! {
            let request = request.clone();
            let kind = kind.clone();
            let query = query.clone();
            let resources = Arc::clone(&resources);
            let seo = seo.clone();
            let options = options.clone();
            let kind_for_definition = kind.clone();
            let definition = effect_try_sync! {
                once;
                body: move || {
                    resources
                        .definition(&kind_for_definition)
                        .map_err(RenderingError::Definition)
                },
            }?;
            let management = options
                .is_management_request
                .as_ref()
                .is_some_and(|detect| detect(&request));
            let mode = decide_list_mode(&definition, management);
            let mode_stage = ListModeResolved {
                definition: ListDefinitionResolved {
                    request: request.clone(),
                    kind: kind.clone(),
                    query: query.clone(),
                    definition,
                },
                mode,
            };
            let seo_stage = resolve_list_seo(mode_stage, &seo)?;
            build_list_page(seo_stage, &options)?
        }
    }
}

/// Decide the list surface.
pub(super) fn decide_list_mode(
    definition: &ResourceDefinition,
    management: bool,
) -> ResourceViewMode {
    if management && has_route_set(&definition.management_routes) {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_MANAGE)
    } else {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_LIST)
    }
}

/// Resolve the SEO value for the mode stage.
pub(super) fn resolve_list_seo<E, R>(
    mode: ListModeResolved,
    seo: &SEOService<E>,
) -> Effect<ListSeoResolved, RenderingError<E>, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = mode.definition.request.clone();
    let kind = mode.definition.kind.clone();
    let view_mode = mode.mode.clone();
    let seo = seo.clone();
    effect! {
        let mode = mode.clone();
        let request = request.clone();
        let kind = kind.clone();
        let view_mode = view_mode.clone();
        let seo = seo.clone();
        let seo_vm = effect_map_error! {
            program: seo.page::<R>(&request, &kind, &view_mode),
            map |error| { RenderingError::Seo(error) },
        }?;
        ListSeoResolved {
            mode,
            seo: seo_vm,
        }
    }
}

/// Assemble the page from the prepared stages.
fn build_list_page<E, R>(
    resolved: ListSeoResolved,
    options: &ListPageOptions<E>,
) -> Effect<ResourceListPageVM, RenderingError<E>, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let options = options.clone();
    effect_try_sync! {
        once;
        body: move || {
            let mode = resolved.mode.mode;
            let stage = resolved.mode.definition;
            let definition = &stage.definition;
            let mut seo = resolved.seo;
            if stage.query.filtered() && !definition.seo.indexable_filtered_list {
                seo.indexable = false;
                seo.included_in_sitemap = false;
            }
            let base_list_path = stage_list_path(&stage.request, definition, &options);
            let filter_options = build_filter_options(
                &stage.request,
                definition,
                &stage.query,
                &base_list_path,
                &options,
            );
            let mut page = ResourceListPageVM {
                kind: stage.kind,
                mode: mode.clone(),
                title: translate_key(&stage.request, options.translate_key.as_ref(), &definition.display.list_title_key),
                subtitle: translate_key(&stage.request, options.translate_key.as_ref(), &definition.display.list_subtitle_key),
                breadcrumb: resolve_breadcrumb(
                    options.breadcrumb.as_ref(),
                    &stage.request,
                    definition,
                    &mode,
                ),
                routes: route_set_for_mode(
                    options.route_set_for_mode.as_ref(),
                    definition,
                    &mode,
                ),
                query: stage.query,
                filters: filter_options.clone(),
                toolbar: ListToolbarVM {
                    action: base_list_path,
                    search_key: ResourceFieldKey::new(RESOURCE_FIELD_KEY_SEARCH),
                    search_value: crate::commands::InputBytes::default(),
                    sort_key: ResourceFieldKey::new(RESOURCE_FIELD_KEY_SORT),
                    sort_value: crate::commands::InputBytes::default(),
                },
                filter_state: ListFiltersVM {
                    options: filter_options,
                },
                pagination: ListPaginationVM {
                    page: 0,
                    page_size: 0,
                    total_items: 0,
                    total_pages: 1,
                    has_prev: false,
                    has_next: false,
                    prev_href: String::new(),
                    next_href: String::new(),
                    links: Some(crate::definitions::SharedList::new(Vec::new())),
                },
                sections: Vec::new(),
                seo,
                seo_view: SEOView::none(),
                ..ResourceListPageVM::default()
            };
            page.toolbar.search_value = page.query.search.clone();
            page.toolbar.sort_value = page.query.sort.clone();
            page.pagination.page = page.query.page;
            page.pagination.page_size = page.query.page_size;
            page.pagination.has_prev = page.query.page > 1;
            if let Some(validate) = options.validate_page.as_ref() {
                validate(&mut page).map_err(RenderingError::Consumer)?;
            }
            Ok::<ResourceListPageVM, RenderingError<E>>(page)
        },
    }
}

/// Resolve the base list target.
fn stage_list_path<E: Send + 'static>(
    request: &Request,
    definition: &ResourceDefinition,
    options: &ListPageOptions<E>,
) -> String {
    let routes = match options.route_set_for_request.as_ref() {
        Some(resolve) => resolve(request, definition),
        None => ResourceRouteSet::default(),
    };
    let route = first_non_blank(&[routes.list, definition.public_routes.list.clone()]);
    match options.route_path.as_ref() {
        Some(build) => build(request, &route, &[]),
        None => route.trim().to_owned(),
    }
}

/// Resolve the routes for the selected surface.
fn route_set_for_mode(
    resolve: Option<&crate::routing::RouteSetForModeResolver>,
    definition: &ResourceDefinition,
    mode: &ResourceViewMode,
) -> ResourceRouteSet {
    match resolve {
        Some(resolve) => resolve(definition, mode),
        None => ResourceRouteSet::default(),
    }
}

/// Build the declared filter options in declaration/option order.
fn build_filter_options<E: Send + 'static>(
    request: &Request,
    definition: &ResourceDefinition,
    query: &ResourceListQuery,
    base_list_path: &str,
    options: &ListPageOptions<E>,
) -> Vec<ResourceFilterOptionVM> {
    let mut values = Vec::new();
    for filter in definition.filters.to_vec() {
        let declared_options = filter.options.to_vec();
        if declared_options.is_empty() {
            continue;
        }
        let active_value = filter_active_value(&filter, query);
        for option in declared_options {
            let option_value = option.value.as_str().to_owned();
            let href = filter_option_href(request, &filter, &option_value, base_list_path);
            values.push(ResourceFilterOptionVM {
                key: filter.key.clone(),
                active: filter_option_active(query, &filter, &option_value, &active_value),
                value: option_value,
                href,
                label: translate_key(
                    request,
                    options.translate_key.as_ref(),
                    &option.label_key,
                ),
            });
        }
    }
    values
}

/// Resolve the active filter value used by single-value options.
fn filter_active_value(filter: &ResourceFilterDefinition, query: &ResourceListQuery) -> crate::commands::InputBytes {
    let active = query
        .filters
        .get(&filter.key)
        .map(|value| value.trim().to_owned())
        .unwrap_or_default();
    match filter.key.as_str() {
        RESOURCE_FILTER_SEARCH => query.search.clone(),
        RESOURCE_FILTER_SORT => query.sort.clone(),
        _ => {
            if active.is_empty() {
                filter.default_value.as_str().into()
            } else {
                active
            }
        }
    }
}

/// Build one option target, preserving the caller's query.
fn filter_option_href(
    request: &Request,
    filter: &ResourceFilterDefinition,
    option_value: &str,
    base_list_path: &str,
) -> String {
    let mut query = request_query_map(request);
    let param = filter_query_param(filter);
    if resource_filter_mode(filter).as_str() == RESOURCE_FILTER_MODE_MULTI_ALL {
        if option_value.is_empty() {
            query.remove(param.as_bytes());
        } else {
            let mut current = query.get(param.as_bytes()).cloned().unwrap_or_default();
            current.push(option_value.into());
            let normalized = normalize_filter_values(
                &current,
                &FilterValueOptions {
                    sort: true,
                    lowercase: false,
                    max_values: DEFAULT_MAX_FILTER_VALUES,
                },
            );
            query.insert(param.into(), normalized);
        }
    } else if option_value.is_empty() {
        query.remove(param.as_bytes());
    } else {
        query.insert(param.into(), vec![option_value.into()]);
    }
    query.insert(
        crate::queries::PAGE_QUERY_PARAM.into(),
        vec!["1".into()],
    );
    if query.is_empty() {
        base_list_path.to_owned()
    } else {
        append_encoded_query(base_list_path, &query)
    }
}

/// Whether one option is active.
fn filter_option_active(
    query: &ResourceListQuery,
    filter: &ResourceFilterDefinition,
    option_value: &str,
    active_value: &crate::commands::InputBytes,
) -> bool {
    if resource_filter_mode(filter).as_str() == RESOURCE_FILTER_MODE_MULTI_ALL {
        return query
            .values_for_filter(&filter.key)
            .iter()
            .any(|value| value == option_value);
    }
    !active_value.is_empty() && active_value == option_value
}

/// Resolve one translated key.
fn translate_key(
    request: &Request,
    translate: Option<&crate::localization::TranslateKeyFunc<Request>>,
    key: &str,
) -> String {
    match translate {
        Some(translate) => translate(request, key),
        None => key.trim().to_owned(),
    }
}

/// Construct the list page service.
#[must_use]
pub fn new_list_service<E>(
    resources: Arc<dyn Registry + Send + Sync>,
    seo: SEOService<E>,
    options: ListPageOptions<E>,
) -> ListService<E>
where
    E: Send + 'static,
{
    ListService::new(resources, seo, options)
}
