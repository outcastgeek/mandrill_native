//! The W4 detail page service and its prepared stages.
//!
//! One page call resolves the registered definition, decides the detail or
//! management-edit surface, resolves the SEO value through the SEO owner and
//! assembles the page before offering the exact value to the consumer's validator.
//! The stages are private prepared values consumed by the next stage, so the SEO
//! callback runs at most once per page call and no caller can forge or substitute a
//! stage. The detail query is part of the service contract and is not read, as in the
//! source workflow. Every step stays lazy inside the returned [`Effect`].

use std::sync::Arc;

use effect::{Effect, effect, effect_map_error, effect_try_sync};
use effect_server::Request;

use crate::definitions::{
    RESOURCE_VIEW_MODE_DETAIL, RESOURCE_VIEW_MODE_EDIT, Registry, ResourceDefinition,
    ResourceKind, ResourceRouteSet, ResourceViewMode, SharedMap,
};
use crate::queries::ResourceDetailQuery;
use crate::routing::has_route_set;
use crate::seo::{SEOVM, Service as SEOService};

use super::contracts::{DetailPageOptions, SEOView, resolve_breadcrumb};
use super::detail_vms::{ExtrasVM, PageVM};
use super::error::RenderingError;

/// The resolved definition stage.
#[derive(Clone)]
pub(super) struct DetailDefinitionResolved {
    /// Request context.
    request: Request,
    /// Resource kind.
    kind: ResourceKind,
    /// Registered definition.
    definition: ResourceDefinition,
}

/// The resolved mode stage.
#[derive(Clone)]
pub(super) struct DetailModeResolved {
    /// The definition stage.
    definition: DetailDefinitionResolved,
    /// Selected surface.
    mode: ResourceViewMode,
}

/// The resolved SEO stage.
#[derive(Clone)]
pub(super) struct DetailSeoResolved {
    /// The mode stage.
    mode: DetailModeResolved,
    /// Published SEO value.
    seo: SEOVM,
}

/// Produces detail page view models.
pub struct DetailService<E>
where
    E: Send + 'static,
{
    resources: Arc<dyn Registry + Send + Sync>,
    seo: SEOService<E>,
    options: DetailPageOptions<E>,
}

impl<E> Clone for DetailService<E>
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

impl<E> DetailService<E>
where
    E: Send + 'static,
{
    /// Construct the detail page service from the required registry and SEO capability.
    #[must_use]
    pub fn new(
        resources: Arc<dyn Registry + Send + Sync>,
        seo: SEOService<E>,
        options: DetailPageOptions<E>,
    ) -> Self {
        Self {
            resources,
            seo,
            options,
        }
    }

    /// Produce the detail page for a kind and admitted query.
    ///
    /// The query is part of the source service contract and is not read by this
    /// workflow. The returned effect is lazy and reusable.
    #[must_use]
    pub fn detail_page<R>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        _query: ResourceDetailQuery,
    ) -> Effect<PageVM, RenderingError<E>, R>
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
            let mode = decide_detail_mode(&definition, management);
            let mode_stage = DetailModeResolved {
                definition: DetailDefinitionResolved {
                    request: request.clone(),
                    kind: kind.clone(),
                    definition,
                },
                mode,
            };
            let seo_stage = resolve_detail_seo(mode_stage, &seo)?;
            build_detail_page(seo_stage, &options)?
        }
    }
}

/// Decide the detail surface.
pub(super) fn decide_detail_mode(
    definition: &ResourceDefinition,
    management: bool,
) -> ResourceViewMode {
    if management && has_route_set(&definition.management_routes) {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_EDIT)
    } else {
        ResourceViewMode::new(RESOURCE_VIEW_MODE_DETAIL)
    }
}

/// Resolve the SEO value for the mode stage.
pub(super) fn resolve_detail_seo<E, R>(
    mode: DetailModeResolved,
    seo: &SEOService<E>,
) -> Effect<DetailSeoResolved, RenderingError<E>, R>
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
        DetailSeoResolved {
            mode,
            seo: seo_vm,
        }
    }
}

/// Assemble the page from the prepared stages.
fn build_detail_page<E, R>(
    resolved: DetailSeoResolved,
    options: &DetailPageOptions<E>,
) -> Effect<PageVM, RenderingError<E>, R>
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
            let routes = match options.route_set_for_mode.as_ref() {
                Some(resolve) => resolve(definition, &mode),
                None => ResourceRouteSet::default(),
            };
            let title = match options.translate_key.as_ref() {
                Some(translate) => translate(&stage.request, &definition.display.detail_title_key),
                None => definition.display.detail_title_key.trim().to_owned(),
            };
            let mut page = PageVM {
                kind: stage.kind,
                mode: mode.clone(),
                title,
                breadcrumb: resolve_breadcrumb(
                    options.breadcrumb.as_ref(),
                    &stage.request,
                    definition,
                    &mode,
                ),
                routes,
                sections: Vec::new(),
                extras: ExtrasVM {
                    links: Some(SharedMap::default()),
                },
                seo: resolved.seo,
                seo_view: SEOView::none(),
                ..PageVM::default()
            };
            if let Some(validate) = options.validate_page.as_ref() {
                validate(&mut page).map_err(RenderingError::Consumer)?;
            }
            Ok::<PageVM, RenderingError<E>>(page)
        },
    }
}

/// Construct the detail page service.
#[must_use]
pub fn new_detail_service<E>(
    resources: Arc<dyn Registry + Send + Sync>,
    seo: SEOService<E>,
    options: DetailPageOptions<E>,
) -> DetailService<E>
where
    E: Send + 'static,
{
    DetailService::new(resources, seo, options)
}
