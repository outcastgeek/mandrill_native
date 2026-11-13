//! Detail page state, frame resolution and page assembly.
//!
//! The frame resolves the admitted detail query, invokes the rendering callback and
//! applies the current-title override when the callback returns a page. An expected
//! failure or absent page keeps the empty page. Page assembly then applies the
//! caller's extras, management flags, back/edit/delete targets, render props and
//! sections in the source order; definition assembly resolves detail render props,
//! alternate-language links and the management edit/delete targets before delegating.

use effect::{Effect, effect, effect_ref, effect_sync};
use effect_server::Request;

use crate::definitions::{ResourceKind, SharedMap};
use crate::queries::{ResourceDetailQuery, first_non_blank};
use crate::routing::optional_route_path;

use super::contracts::{
    DefinitionResourcePageBuilderOptions, FrameBuilderOptions, HreflangsForPathResolver,
    PageBuilderOptions, SEOView,
};
use super::detail_vms::{ExtrasVM, PageVM, SectionVM};

/// The resolved detail frame.
#[derive(Clone, Debug, Default)]
pub struct ResourceDetailFrame {
    /// The page value.
    pub page_vm: PageVM,
    /// Whether the caller is logged in.
    pub is_logged_in: bool,
}

/// One supplied detail page state.
#[derive(Clone, Debug, Default)]
pub struct ResourceDetailPageState {
    /// Optional render props.
    pub seo_view: SEOView,
    /// Whether the caller is logged in.
    pub is_logged_in: bool,
    /// Whether the caller may edit.
    pub can_edit: bool,
    /// List target override.
    pub back_href: String,
    /// Edit target override.
    pub edit_href: String,
    /// Delete target override.
    pub delete_action: String,
    /// Delete confirmation message.
    pub delete_confirm: String,
    /// Path whose alternate-language links are required.
    pub hreflang_path: String,
    /// Flat route parameter name/value pairs.
    pub route_params: Vec<String>,
    /// Section overrides.
    pub sections: Vec<SectionVM>,
    /// Auxiliary links.
    pub extras: ExtrasVM,
}

/// Build a detail frame.
pub fn build_resource_detail_frame<E, R>(
    request: &Request,
    kind: &ResourceKind,
    _manage: bool,
    current_title: &str,
    is_logged_in: bool,
    options: FrameBuilderOptions<E, R>,
) -> Effect<ResourceDetailFrame, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let queries = options.queries.clone();
    let rendering = options
        .rendering
        .as_ref()
        .and_then(|callbacks| callbacks.detail_page.clone());
    let request = request.clone();
    let kind = kind.clone();
    let current_title = current_title.to_owned();
    effect! {
        let request = request.clone();
        let kind = kind.clone();
        let current_title = current_title.clone();
        let queries = queries.clone();
        let rendering = rendering.clone();
        let query = match queries.as_ref() {
            Some(source) => source.detail_query::<E, R>(&request, &kind),
            None => effect_sync! {
                body { ResourceDetailQuery::default() }
            },
        }?;
        let page_vm = match rendering.as_ref() {
            Some(source) => {
                let selected = source(&request, &kind, query).either()?;
                match selected {
                    Ok(page) => {
                        let mut page = page;
                        if !current_title.is_empty() {
                            page.title = current_title;
                        }
                        page
                    }
                    Err(_expected) => PageVM::default(),
                }
            }
            None => PageVM::default(),
        };
        ResourceDetailFrame {
            page_vm,
            is_logged_in,
        }
    }
}

/// Build a detail page from supplied state.
pub fn build_resource_detail_page<E, R>(
    request: &Request,
    kind: &ResourceKind,
    manage: bool,
    current_title: &str,
    state: ResourceDetailPageState,
    options: PageBuilderOptions<E, R>,
) -> Effect<PageVM, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let frames = options.frames;
    let request = request.clone();
    let kind = kind.clone();
    let current_title = current_title.to_owned();
    effect! {
        let request = request.clone();
        let kind = kind.clone();
        let current_title = current_title.clone();
        let frames = frames.clone();
        let state = state.clone();
        let route_path = frames.route_path.clone();
        let frame = build_resource_detail_frame(
            &request,
            &kind,
            manage,
            &current_title,
            state.is_logged_in,
            frames,
        )?;
        let mut page = frame.page_vm;
        if state.extras.links.is_some() {
            page.extras = state.extras.clone();
        }
        if page.extras.links.is_none() {
            page.extras.links = Some(SharedMap::default());
        }
        page.manage_view = manage;
        page.is_logged_in = frame.is_logged_in;
        page.can_edit = state.can_edit;
        page.back_href = first_non_blank(&[
            state.back_href,
            optional_route_path(&request, &page.routes.list, route_path.as_ref(), &[]),
        ]);
        let params = detail_params(&state.route_params);
        page.edit_href = first_non_blank(&[
            state.edit_href,
            optional_route_path(&request, &page.routes.edit, route_path.as_ref(), &params),
        ]);
        page.delete_action = first_non_blank(&[
            state.delete_action,
            optional_route_path(&request, &page.routes.delete, route_path.as_ref(), &params),
        ]);
        page.delete_confirm = state.delete_confirm;
        page.seo_view = state.seo_view;
        page.sections = state.sections;
        page
    }
}

/// Build a definition-aware detail page.
pub fn build_definition_detail_page<E, R>(
    request: &Request,
    kind: &ResourceKind,
    manage: bool,
    current_title: &str,
    state: ResourceDetailPageState,
    options: DefinitionResourcePageBuilderOptions<E, R>,
) -> Effect<PageVM, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = request.clone();
    let kind = kind.clone();
    let current_title = current_title.to_owned();
    effect! {
        let request = request.clone();
        let kind = kind.clone();
        let current_title = current_title.clone();
        let options = options.clone();
        let mut state = state.clone();
        if state.seo_view.is_none()
            && let Some(resolve) = options.detail_seo.as_ref()
        {
            state.seo_view = resolve(&request, &kind)?;
        }
        apply_detail_hreflangs(
            &state.seo_view,
            &request,
            &state.hreflang_path,
            options.hreflangs_for_path.as_ref(),
        )?;
        if state.can_edit
            && let (Some(route_set), Some(route_path)) = (
                options.route_set.as_ref(),
                options.pages.frames.route_path.as_ref(),
            )
        {
            let routes = route_set(&kind, true);
            let params = detail_params(&state.route_params);
            if state.edit_href.is_empty() {
                state.edit_href = optional_route_path(
                    &request,
                    &routes.edit,
                    Some(route_path),
                    &params,
                );
            }
            if state.delete_action.is_empty() {
                state.delete_action = optional_route_path(
                    &request,
                    &routes.delete,
                    Some(route_path),
                    &params,
                );
            }
        }
        build_resource_detail_page(
            &request,
            &kind,
            manage,
            &current_title,
            state,
            options.pages,
        )?
    }
}

/// Apply the alternate-language fallback to the shared render props.
///
/// The fallback resolver runs only when the shared props exist, their hreflang list
/// is empty, the path is non-blank and the resolver is supplied, matching the source
/// callback order. The write mutates the shared value in place, so a caller retaining
/// the handle observes the alternates.
fn apply_detail_hreflangs<E, R>(
    seo_view: &SEOView,
    request: &Request,
    hreflang_path: &str,
    hreflangs_for_path: Option<&HreflangsForPathResolver>,
) -> Effect<(), E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let Some(view) = seo_view.shared_handle() else {
        return effect_sync! { body { () } };
    };
    let request = request.clone();
    let hreflang_path = hreflang_path.to_owned();
    let hreflangs_for_path = hreflangs_for_path.cloned();
    effect! {
        let view = view.clone();
        let request = request.clone();
        let hreflang_path = hreflang_path.clone();
        let hreflangs_for_path = hreflangs_for_path.clone();
        let empty = view
            .clone()
            .with::<bool, E, R>(|props| props.hreflangs.is_empty())
            .either()?;
        if matches!(empty, Ok(true))
            && !hreflang_path.trim().is_empty()
            && let Some(resolve) = hreflangs_for_path.as_ref()
        {
            let hreflangs = resolve(&request, &hreflang_path);
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

/// Pair the flat route parameter names and values for the path builder.
fn detail_params(values: &[String]) -> Vec<(String, String)> {
    let mut params = Vec::with_capacity(values.len() / 2);
    for pair in values.chunks_exact(2) {
        let Some(name) = pair.first() else {
            continue;
        };
        let Some(value) = pair.get(1) else {
            continue;
        };
        params.push((name.clone(), value.clone()));
    }
    params
}
