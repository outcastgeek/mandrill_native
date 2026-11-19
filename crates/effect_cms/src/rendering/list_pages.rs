//! List page state, frame resolution, page assembly and pagination.
//!
//! The frame resolves the caller's options in the source order: effective page size,
//! the admitted query, the rendering callback, the frame flags and finally
//! pagination. An expected rendering failure or an absent page keeps the empty page,
//! as the source does, while query admission stays lazy. Definition list assembly
//! resolves the list render props and alternate-language links before delegating to
//! the page builder; the items variant builds the supplied items/sections first.

use std::sync::Arc;

use effect::{Effect, effect, effect_ref, effect_sync};
use effect_server::Request;

use crate::definitions::{Registry, ResourceKind, ResourceRouteSet};
use crate::localization::TranslateKeyFunc;
use crate::queries::{
    PAGE_QUERY_PARAM, ResourceListQuery, append_encoded_query, first_non_blank,
    request_query_map,
};
use crate::routing::{RoutePathBuilder, optional_route_path};

use super::contracts::{
    DefinitionResourcePageBuilderOptions, FrameBuilderOptions, PageBuilderOptions, SEOView,
};
use super::list_items::{
    ResourceListItemState, ResourceListState, build_resource_list_items_for_kind,
    build_resource_list_sections_for_kind,
};
use super::list_vms::{
    ListFiltersVM, ListPageLinkVM, ListPaginationVM, ResourceListPageVM, ResourceListSectionVM,
};

/// One supplied list page state.
#[derive(Clone, Debug, Default)]
pub struct ResourceListPageState {
    /// Subtitle override.
    pub subtitle: String,
    /// Optional render props.
    pub seo_view: SEOView,
    /// Whether the caller is logged in.
    pub is_logged_in: bool,
    /// Whether the caller may create.
    pub can_create: bool,
    /// New-resource target.
    pub new_href: String,
    /// Section overrides.
    pub sections: Vec<ResourceListSectionVM>,
}

/// The resolved list frame.
#[derive(Clone, Debug, Default)]
pub struct ResourceListFrame {
    /// The page value.
    pub page_vm: ResourceListPageVM,
    /// Requested page.
    pub page: i64,
    /// Total page count.
    pub total_pages: i64,
    /// Total item count.
    pub total: i64,
    /// Whether the caller is logged in.
    pub is_logged_in: bool,
    /// Whether the caller may create.
    pub can_create: bool,
}

/// Build a list frame.
///
/// The effective page size, query admission, rendering callback, management filter
/// removal and pagination run in that order when the returned effect runs. An
/// expected rendering failure or absent page yields the empty page value.
pub fn build_resource_list_frame<E, R>(
    request: &Request,
    kind: &ResourceKind,
    manage: bool,
    page: i64,
    page_size: i64,
    total: i64,
    is_logged_in: bool,
    can_create: bool,
    options: FrameBuilderOptions<E, R>,
) -> Effect<ResourceListFrame, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let page_size = if page_size <= 0 {
        options.default_page_size
    } else {
        page_size
    };
    let queries = options.queries.clone();
    let rendering = options
        .rendering
        .as_ref()
        .and_then(|callbacks| callbacks.list_page.clone());
    let route_path = options.route_path.clone();
    let request = request.clone();
    let kind = kind.clone();
    effect! {
        let request = request.clone();
        let kind = kind.clone();
        let queries = queries.clone();
        let rendering = rendering.clone();
        let route_path = route_path.clone();
        let query = match queries.as_ref() {
            Some(source) => source.list_query::<E, R>(&request, &kind),
            None => effect_sync! {
                body { ResourceListQuery::default() }
            },
        }?;
        let mut page_vm = match rendering.as_ref() {
            Some(source) => {
                let selected = effect! { either; program: source(&request, &kind, query.clone()), }?;
                match selected {
                    Ok(rendered) => {
                        let mut rendered = rendered;
                        if !manage {
                            rendered.filters = Vec::new();
                            rendered.filter_state = ListFiltersVM::default();
                        }
                        rendered
                    }
                    Err(_expected) => ResourceListPageVM::default(),
                }
            }
            None => ResourceListPageVM::default(),
        };
        page_vm.pagination = build_resource_list_pagination(
            &request,
            &page_vm.routes,
            page,
            page_size,
            total,
            route_path.as_ref(),
        );
        ResourceListFrame {
            page_vm,
            page,
            total_pages: calc_total_pages(total, page_size),
            total,
            is_logged_in,
            can_create,
        }
    }
}

/// Build a list page from supplied state.
pub fn build_resource_list_page<E, R>(
    request: &Request,
    kind: &ResourceKind,
    manage: bool,
    page: i64,
    page_size: i64,
    total: i64,
    state: ResourceListPageState,
    options: PageBuilderOptions<E, R>,
) -> Effect<ResourceListPageVM, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let frames = options.frames;
    let request = request.clone();
    let kind = kind.clone();
    effect! {
        let request = request.clone();
        let kind = kind.clone();
        let frames = frames.clone();
        let state = state.clone();
        let route_path = frames.route_path.clone();
        let frame = build_resource_list_frame(
            &request,
            &kind,
            manage,
            page,
            page_size,
            total,
            state.is_logged_in,
            state.can_create,
            frames,
        )?;
        let mut page_vm = frame.page_vm;
        page_vm.manage_view = manage;
        page_vm.is_logged_in = frame.is_logged_in;
        page_vm.can_create = frame.can_create;
        page_vm.new_href = first_non_blank(&[
            state.new_href,
            optional_route_path(&request, &page_vm.routes.new, route_path.as_ref(), &[]),
        ]);
        page_vm.seo_view = state.seo_view;
        page_vm.subtitle = first_non_blank(&[state.subtitle, page_vm.subtitle]);
        page_vm.sections = state.sections;
        page_vm
    }
}

/// Build a definition-aware list page.
pub fn build_definition_list_page<E, R>(
    request: &Request,
    kind: &ResourceKind,
    manage: bool,
    page: i64,
    page_size: i64,
    total: i64,
    state: ResourceListPageState,
    options: DefinitionResourcePageBuilderOptions<E, R>,
) -> Effect<ResourceListPageVM, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = request.clone();
    let kind = kind.clone();
    effect! {
        let request = request.clone();
        let kind = kind.clone();
        let options = options.clone();
        let mut state = state.clone();
        if state.seo_view.is_none()
            && let Some(resolve) = options.list_seo.as_ref()
        {
            state.seo_view = resolve(&request, &kind)?;
        }
        apply_list_hreflangs(&state.seo_view, &request, &kind, manage, &options)?;
        build_resource_list_page(
            &request,
            &kind,
            manage,
            page,
            page_size,
            total,
            state,
            options.pages,
        )?
    }
}

/// Apply the alternate-language fallback to the shared render props.
///
/// The fallback resolver runs only when the shared props exist, their hreflang list
/// is empty and the route/hreflang/routing callbacks are all supplied, matching the
/// source callback order. The write mutates the shared value in place, so a caller
/// retaining the handle observes the alternates.
fn apply_list_hreflangs<E, R>(
    seo_view: &SEOView,
    request: &Request,
    kind: &ResourceKind,
    manage: bool,
    options: &DefinitionResourcePageBuilderOptions<E, R>,
) -> Effect<(), E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let Some(view) = seo_view.shared_handle() else {
        return effect_sync! { body { () } };
    };
    let request = request.clone();
    let kind = kind.clone();
    let route_set = options.route_set.clone();
    let hreflang_path = options.hreflang_path.clone();
    let hreflangs_for_path = options.hreflangs_for_path.clone();
    effect! {
        let view = view.clone();
        let request = request.clone();
        let kind = kind.clone();
        let route_set = route_set.clone();
        let hreflang_path = hreflang_path.clone();
        let hreflangs_for_path = hreflangs_for_path.clone();
        let empty = effect! {
            either;
            program: effect_ref! {
                inspect;
                reference: view,
                inspect: |props| props.hreflangs.is_empty(),
            },
        }?;
        if matches!(empty, Ok(true))
            && let (Some(route_set), Some(hreflang_path), Some(hreflangs_for_path)) = (
                route_set.as_ref(),
                hreflang_path.as_ref(),
                hreflangs_for_path.as_ref(),
            )
        {
            let route = route_set(&kind, manage).list;
            if !route.trim().is_empty() {
                let hreflangs = hreflangs_for_path(&request, &hreflang_path(&route));
                let _ = effect_ref! {
                    in_place;
                    reference: view.clone(),
                    modify: move |props| {
                        if props.hreflangs.is_empty() {
                            props.hreflangs = hreflangs.clone();
                        }
                    },
                }?;
            }
        }
    }
}

/// Build a definition-aware list page from supplied items.
pub fn build_definition_list_page_with_items<E, R>(
    request: &Request,
    resources: Arc<dyn Registry + Send + Sync>,
    kind: &ResourceKind,
    manage: bool,
    page: i64,
    page_size: i64,
    total: i64,
    items: Vec<ResourceListItemState>,
    highlight_leading: bool,
    state: ResourceListPageState,
    translate_key: Option<TranslateKeyFunc<Request>>,
    options: DefinitionResourcePageBuilderOptions<E, R>,
) -> Effect<ResourceListPageVM, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = request.clone();
    let kind = kind.clone();
    effect! {
        let request = request.clone();
        let kind = kind.clone();
        let resources = Arc::clone(&resources);
        let items = items.clone();
        let translate_key = translate_key.clone();
        let options = options.clone();
        let kind_for_sections = kind.clone();
        let request_for_sections = request.clone();
        let resources_for_sections = Arc::clone(&resources);
        let items_for_sections = items.clone();
        let translate_for_sections = translate_key.clone();
        let state = state.clone();
        let sections = effect_sync! {
            body {
                let list_state = ResourceListState {
                    items: build_resource_list_items_for_kind(
                        &*resources_for_sections,
                        &kind_for_sections,
                        &items_for_sections,
                        &request_for_sections,
                        translate_for_sections.as_ref(),
                    ),
                    highlight_leading,
                    sections: std::collections::BTreeMap::new(),
                };
                build_resource_list_sections_for_kind(
                    &*resources_for_sections,
                    &kind_for_sections,
                    &list_state,
                    &request_for_sections,
                    translate_for_sections.as_ref(),
                )
            }
        }?;
        let mut state = state;
        state.sections = sections;
        build_definition_list_page(
            &request,
            &kind,
            manage,
            page,
            page_size,
            total,
            state,
            options,
        )?
    }
}

/// Resolve the requested page from an offset and page size.
#[must_use]
pub fn resource_list_current_page(offset: i64, page_size: i64, fallback_page_size: i64) -> i64 {
    let page_size = if page_size <= 0 {
        fallback_page_size
    } else {
        page_size
    };
    if page_size <= 0 || offset <= 0 {
        return 1;
    }
    offset / page_size + 1
}

/// Build the pagination value with clamped page and query-preserving links.
#[must_use]
pub fn build_resource_list_pagination(
    request: &Request,
    routes: &ResourceRouteSet,
    page: i64,
    page_size: i64,
    total: i64,
    route_path: Option<&RoutePathBuilder>,
) -> ListPaginationVM {
    let pages = calc_total_pages(total, page_size).max(1);
    let page = page.clamp(1, pages);
    let mut links = Vec::new();
    for current in 1..=pages {
        links.push(ListPageLinkVM {
            page: current,
            href: resource_list_page_href(request, routes, current, route_path),
            current: current == page,
        });
    }
    let previous = if page > 1 {
        resource_list_page_href(request, routes, page - 1, route_path)
    } else {
        String::new()
    };
    let next = if page < pages {
        resource_list_page_href(request, routes, page + 1, route_path)
    } else {
        String::new()
    };
    ListPaginationVM {
        page,
        page_size,
        total_items: total,
        total_pages: pages,
        has_prev: page > 1,
        has_next: page < pages,
        prev_href: previous,
        next_href: next,
        links: Some(crate::definitions::SharedList::new(links)),
    }
}

/// Build one list page target.
#[must_use]
pub fn resource_list_page_href(
    request: &Request,
    routes: &ResourceRouteSet,
    page: i64,
    route_path: Option<&RoutePathBuilder>,
) -> String {
    let base = optional_route_path(request, &routes.list, route_path, &[]);
    if base.is_empty() {
        return base;
    }
    let mut query = request_query_map(request);
    query.insert(PAGE_QUERY_PARAM.into(), vec![page.to_string().into()]);
    append_encoded_query(&base, &query)
}

/// Count pages with the source's at-least-one rule.
fn calc_total_pages(total: i64, page_size: i64) -> i64 {
    if total <= 0 || page_size <= 0 {
        return 1;
    }
    let pages = total / page_size;
    if total % page_size != 0 {
        pages.saturating_add(1).max(1)
    } else {
        pages.max(1)
    }
}
