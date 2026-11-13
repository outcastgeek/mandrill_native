//! The rendering owner's page-builder and service option contracts.
//!
//! The option values preserve the source's optional consumer callbacks. A missing
//! query source, rendering source, translation port, route builder, breadcrumb,
//! SEO resolver or page validator is genuine optional behavior with the source's
//! documented empty result; none supplies a required dependency.

use std::fmt;
use std::sync::Arc;

use effect::{AtomicRef, Effect};
use effect_server::Request;

use crate::definitions::{
    ResourceDefinition, ResourceKind, ResourceRouteSet, ResourceViewMode,
};
use crate::localization::TranslateKeyFunc;
use crate::queries::{QueryService, ResourceDetailQuery, ResourceListQuery};
use crate::routing::{
    ManagementBreadcrumbBuilder, ManagementBreadcrumbVM, RoutePathBuilder, RouteSetForModeResolver,
    RouteSetForRequestResolver,
};
use crate::seo::{Hreflang, SEOProps};

use super::detail_vms::PageVM;
use super::error::RenderingError;
use super::list_vms::ResourceListPageVM;

/// Resolves the optional render props for a list/detail surface.
///
/// The effectful resolver returns a shared [`SEOView`] handle, so a caller that retains the
/// same handle observes the render-props updates this owner applies (canonical URLs,
/// alternate-language links) instead of receiving a detached copy.
pub type ResourceSEOPropsResolver<E,R> =
    Arc<dyn Fn(&Request, &ResourceKind) -> Effect<SEOView,E,R> + Send + Sync + 'static>;

/// A shared handle to caller-retained render props.
///
/// The handle preserves the render-props pointer identity: an absent value is a
/// missing render-props value, and a present value shares one [`SEOProps`] allocation
/// with the caller. Reading and mutating it use the workspace's canonical
/// [`AtomicRef`] operations, so every consumer holding the handle observes the same
/// updates.
#[derive(Clone, Default)]
pub struct SEOView {
    handle: Option<AtomicRef<SEOProps>>,
}

impl SEOView {
    /// No render props were supplied.
    #[must_use]
    pub fn none() -> Self {
        Self { handle: None }
    }

    /// Wrap a freshly assembled render-props value.
    #[must_use]
    pub fn new(props: SEOProps) -> Self {
        Self {
            handle: Some(AtomicRef::new(props)),
        }
    }

    /// Share one caller-retained render-props handle.
    #[must_use]
    pub fn shared(handle: AtomicRef<SEOProps>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    /// Clone the shared handle, when render props are present.
    ///
    /// Cloning the handle shares the same allocation; it is not a props copy.
    #[must_use]
    pub fn shared_handle(&self) -> Option<AtomicRef<SEOProps>> {
        self.handle.clone()
    }

    /// Whether render props are present.
    #[must_use]
    pub fn is_some(&self) -> bool {
        self.handle.is_some()
    }

    /// Whether render props are absent.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.handle.is_none()
    }
}

impl fmt::Debug for SEOView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SEOView")
            .field("present", &self.handle.is_some())
            .finish()
    }
}

/// Resolves the route names for a surface.
pub type ResourceRouteSetResolver =
    Arc<dyn Fn(&ResourceKind, bool) -> ResourceRouteSet + Send + Sync + 'static>;

/// Resolves the path whose alternate-language links are required.
pub type HreflangPathResolver = Arc<dyn Fn(&str) -> String + Send + Sync + 'static>;

/// Resolves the alternate-language links for one path.
pub type HreflangsForPathResolver =
    Arc<dyn Fn(&Request, &str) -> Vec<Hreflang> + Send + Sync + 'static>;

/// Produces one list page for the frame.
pub type ListPageSource<E, R> = Arc<
    dyn Fn(&Request, &ResourceKind, ResourceListQuery) -> Effect<ResourceListPageVM, E, R>
        + Send
        + Sync
        + 'static,
>;

/// Produces one detail page for the frame.
pub type DetailPageSource<E, R> = Arc<
    dyn Fn(&Request, &ResourceKind, ResourceDetailQuery) -> Effect<PageVM, E, R>
        + Send
        + Sync
        + 'static,
>;

/// Validates and may mutate one completed list page.
pub type ListPageValidator<E> =
    Arc<dyn Fn(&mut ResourceListPageVM) -> Result<(), E> + Send + Sync + 'static>;

/// Validates and may mutate one completed detail page.
pub type DetailPageValidator<E> =
    Arc<dyn Fn(&mut PageVM) -> Result<(), E> + Send + Sync + 'static>;

/// The consumer's rendering callbacks for frame/page assembly.
pub struct RenderingCallbacks<E: Send + 'static, R: Send + Sync + 'static> {
    /// The list-page source, when supplied.
    pub list_page: Option<ListPageSource<E, R>>,
    /// The detail-page source, when supplied.
    pub detail_page: Option<DetailPageSource<E, R>>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for RenderingCallbacks<E, R> {
    fn clone(&self) -> Self {
        Self {
            list_page: self.list_page.clone(),
            detail_page: self.detail_page.clone(),
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default for RenderingCallbacks<E, R> {
    fn default() -> Self {
        Self {
            list_page: None,
            detail_page: None,
        }
    }
}

/// The complete rendering capability a frame or page consumer supplies.
pub trait RenderingService<E: Send + 'static> {
    /// Produce the list page for a kind and admitted query.
    fn list_page<R: Send + Sync + 'static>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        query: ResourceListQuery,
    ) -> Effect<ResourceListPageVM, RenderingError<E>, R>;

    /// Produce the detail page for a kind and admitted query.
    fn detail_page<R: Send + Sync + 'static>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        query: ResourceDetailQuery,
    ) -> Effect<PageVM, RenderingError<E>, R>;
}

/// Configures the W3 list page service.
pub struct ListPageOptions<E: Send + 'static> {
    /// Detects a management request.
    pub is_management_request: Option<Arc<dyn Fn(&Request) -> bool + Send + Sync + 'static>>,
    /// Resolves routes for the request.
    pub route_set_for_request: Option<RouteSetForRequestResolver>,
    /// Resolves routes for the selected surface.
    pub route_set_for_mode: Option<RouteSetForModeResolver>,
    /// Builds route targets.
    pub route_path: Option<RoutePathBuilder>,
    /// Resolves the management breadcrumb.
    pub breadcrumb: Option<ManagementBreadcrumbBuilder>,
    /// The consumer's translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// Validates the completed page.
    pub validate_page: Option<ListPageValidator<E>>,
}

impl<E: Send + 'static> Clone for ListPageOptions<E> {
    fn clone(&self) -> Self {
        Self {
            is_management_request: self.is_management_request.clone(),
            route_set_for_request: self.route_set_for_request.clone(),
            route_set_for_mode: self.route_set_for_mode.clone(),
            route_path: self.route_path.clone(),
            breadcrumb: self.breadcrumb.clone(),
            translate_key: self.translate_key.clone(),
            validate_page: self.validate_page.clone(),
        }
    }
}

impl<E: Send + 'static> Default for ListPageOptions<E> {
    fn default() -> Self {
        Self {
            is_management_request: None,
            route_set_for_request: None,
            route_set_for_mode: None,
            route_path: None,
            breadcrumb: None,
            translate_key: None,
            validate_page: None,
        }
    }
}

/// Configures the W4 detail page service.
pub struct DetailPageOptions<E: Send + 'static> {
    /// Detects a management request.
    pub is_management_request: Option<Arc<dyn Fn(&Request) -> bool + Send + Sync + 'static>>,
    /// Resolves routes for the selected surface.
    pub route_set_for_mode: Option<RouteSetForModeResolver>,
    /// Resolves the management breadcrumb.
    pub breadcrumb: Option<ManagementBreadcrumbBuilder>,
    /// The consumer's translation port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// Validates the completed page.
    pub validate_page: Option<DetailPageValidator<E>>,
}

impl<E: Send + 'static> Clone for DetailPageOptions<E> {
    fn clone(&self) -> Self {
        Self {
            is_management_request: self.is_management_request.clone(),
            route_set_for_mode: self.route_set_for_mode.clone(),
            breadcrumb: self.breadcrumb.clone(),
            translate_key: self.translate_key.clone(),
            validate_page: self.validate_page.clone(),
        }
    }
}

impl<E: Send + 'static> Default for DetailPageOptions<E> {
    fn default() -> Self {
        Self {
            is_management_request: None,
            route_set_for_mode: None,
            breadcrumb: None,
            translate_key: None,
            validate_page: None,
        }
    }
}

/// Configures frame construction.
pub struct FrameBuilderOptions<E: Send + 'static, R: Send + Sync + 'static> {
    /// The admitted-query source.
    pub queries: Option<QueryService>,
    /// The page-rendering callbacks.
    pub rendering: Option<RenderingCallbacks<E, R>>,
    /// Fallback page size.
    pub default_page_size: i64,
    /// Builds route targets.
    pub route_path: Option<RoutePathBuilder>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for FrameBuilderOptions<E, R> {
    fn clone(&self) -> Self {
        Self {
            queries: self.queries.clone(),
            rendering: self.rendering.clone(),
            default_page_size: self.default_page_size,
            route_path: self.route_path.clone(),
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default for FrameBuilderOptions<E, R> {
    fn default() -> Self {
        Self {
            queries: None,
            rendering: None,
            default_page_size: 0,
            route_path: None,
        }
    }
}

/// Configures list page assembly.
pub struct PageBuilderOptions<E: Send + 'static, R: Send + Sync + 'static> {
    /// The frame options.
    pub frames: FrameBuilderOptions<E, R>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for PageBuilderOptions<E, R> {
    fn clone(&self) -> Self {
        Self {
            frames: self.frames.clone(),
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default for PageBuilderOptions<E, R> {
    fn default() -> Self {
        Self {
            frames: FrameBuilderOptions::default(),
        }
    }
}

/// Configures definition-aware list/detail page assembly.
pub struct DefinitionResourcePageBuilderOptions<E: Send + 'static, R: Send + Sync + 'static> {
    /// The page/frame options.
    pub pages: PageBuilderOptions<E, R>,
    /// Resolves list render props.
    pub list_seo: Option<ResourceSEOPropsResolver<E,R>>,
    /// Resolves detail render props.
    pub detail_seo: Option<ResourceSEOPropsResolver<E,R>>,
    /// Resolves the route names.
    pub route_set: Option<ResourceRouteSetResolver>,
    /// Resolves the alternate-language path.
    pub hreflang_path: Option<HreflangPathResolver>,
    /// Resolves alternate-language links.
    pub hreflangs_for_path: Option<HreflangsForPathResolver>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone
    for DefinitionResourcePageBuilderOptions<E, R>
{
    fn clone(&self) -> Self {
        Self {
            pages: self.pages.clone(),
            list_seo: self.list_seo.clone(),
            detail_seo: self.detail_seo.clone(),
            route_set: self.route_set.clone(),
            hreflang_path: self.hreflang_path.clone(),
            hreflangs_for_path: self.hreflangs_for_path.clone(),
        }
    }
}

impl<E: Send + 'static, R: Send + Sync + 'static> Default
    for DefinitionResourcePageBuilderOptions<E, R>
{
    fn default() -> Self {
        Self {
            pages: PageBuilderOptions::default(),
            list_seo: None,
            detail_seo: None,
            route_set: None,
            hreflang_path: None,
            hreflangs_for_path: None,
        }
    }
}

/// Resolve the breadcrumb for a definition and surface.
pub(super) fn resolve_breadcrumb(
    builder: Option<&ManagementBreadcrumbBuilder>,
    request: &Request,
    definition: &ResourceDefinition,
    mode: &ResourceViewMode,
) -> ManagementBreadcrumbVM {
    match builder {
        Some(builder) => builder(request, definition, mode),
        None => ManagementBreadcrumbVM::default(),
    }
}
