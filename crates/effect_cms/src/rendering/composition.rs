//! Rendering composition: the aggregate service and its callback binding.
//!
//! [`Service`] binds the W3 list and W4 detail services over one required definition
//! registry and one required SEO service. Its callbacks are the complete list/detail
//! rendering capability used by frame and page builders. Dashboard/resource-page
//! shells are separate owners; this composition intentionally binds only the two
//! rendering core services.

use std::sync::Arc;

use effect::Effect;
use effect_server::Request;

use crate::definitions::{Registry, ResourceKind};
use crate::queries::{ResourceDetailQuery, ResourceListQuery};
use crate::seo::Service as SEOService;

use super::contracts::{
    DetailPageOptions, ListPageOptions, RenderingCallbacks, RenderingService,
};
use super::detail_service::DetailService;
use super::detail_vms::PageVM;
use super::error::RenderingError;
use super::list_service::ListService;
use super::list_vms::ResourceListPageVM;

/// The source rendering service: list and detail page production.
pub struct Service<E>
where
    E: Send + 'static,
{
    list: ListService<E>,
    detail: DetailService<E>,
}

impl<E> Clone for Service<E>
where
    E: Send + 'static,
{
    fn clone(&self) -> Self {
        Self {
            list: self.list.clone(),
            detail: self.detail.clone(),
        }
    }
}

impl<E> Service<E>
where
    E: Send + 'static,
{
    /// Construct the aggregate from the two owner services.
    #[must_use]
    pub fn new(list: ListService<E>, detail: DetailService<E>) -> Self {
        Self { list, detail }
    }

    /// Produce the list page.
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
        self.list.list_page::<R>(request, kind, query)
    }

    /// Produce the detail page.
    #[must_use]
    pub fn detail_page<R>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        query: ResourceDetailQuery,
    ) -> Effect<PageVM, RenderingError<E>, R>
    where
        R: Send + Sync + 'static,
    {
        self.detail.detail_page::<R>(request, kind, query)
    }

    /// Bind the aggregate as the frame/page rendering callbacks.
    #[must_use]
    pub fn callbacks<R>(service: &Arc<Self>) -> RenderingCallbacks<RenderingError<E>, R>
    where
        R: Send + Sync + 'static,
    {
        let list = Arc::clone(service);
        let detail = Arc::clone(service);
        RenderingCallbacks {
            list_page: Some(Arc::new(move |request, kind, query| {
                list.list_page::<R>(request, kind, query)
            })),
            detail_page: Some(Arc::new(move |request, kind, query| {
                detail.detail_page::<R>(request, kind, query)
            })),
        }
    }
}

impl<E> RenderingService<E> for Service<E>
where
    E: Send + 'static,
{
    fn list_page<R: Send + Sync + 'static>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        query: ResourceListQuery,
    ) -> Effect<ResourceListPageVM, RenderingError<E>, R> {
        Service::list_page::<R>(self, request, kind, query)
    }

    fn detail_page<R: Send + Sync + 'static>(
        &self,
        request: &Request,
        kind: &ResourceKind,
        query: ResourceDetailQuery,
    ) -> Effect<PageVM, RenderingError<E>, R> {
        Service::detail_page::<R>(self, request, kind, query)
    }
}

/// Construct the rendering service from the required registry and SEO capability.
#[must_use]
pub fn new_service<E>(
    resources: Arc<dyn Registry + Send + Sync>,
    seo: SEOService<E>,
    list_options: ListPageOptions<E>,
    detail_options: DetailPageOptions<E>,
) -> Service<E>
where
    E: Send + 'static,
{
    Service::new(
        ListService::new(Arc::clone(&resources), seo.clone(), list_options),
        DetailService::new(resources, seo, detail_options),
    )
}
