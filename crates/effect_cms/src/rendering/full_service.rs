//! List, detail and dashboard capabilities retained through full composition.
use std::sync::Arc;
use effect::Effect;
use effect_server::Request;
use crate::definitions::{Registry,ResourceKind};
use crate::queries::{ResourceListQuery,ResourceDetailQuery};
use crate::dashboards::{DashboardService,DashboardServiceOptions,DashboardPageValidator,ResourceDashboardPageVM};
use crate::localization::TranslateKeyFunc;
use crate::routing::{RoutePathBuilder,RouteSetForModeResolver,RouteSetForRequestResolver,ManagementBreadcrumbBuilder};
use crate::seo::Service as SEOService;
use super::*;

pub trait FullRenderingService<E:Send+'static>:RenderingService<E>{
    fn dashboard_page<R:Send+Sync+'static>(&self,request:&Request,kind:&ResourceKind,query:ResourceListQuery)
        ->Effect<ResourceDashboardPageVM,RenderingError<E>,R>;
}
/// All three capabilities are required; none is synthesized from an empty page.
pub struct FullService<E:Send+'static>{pub rendering:Service<E>,pub dashboard:DashboardService<E>}
impl<E:Send+'static> Clone for FullService<E>{fn clone(&self)->Self{Self{rendering:self.rendering.clone(),dashboard:self.dashboard.clone()}}}
impl<E:Send+'static> FullService<E>{
    pub fn new(rendering:Service<E>,dashboard:DashboardService<E>)->Self{Self{rendering,dashboard}}
    pub fn list_page<R:Send+Sync+'static>(&self,request:&Request,kind:&ResourceKind,query:ResourceListQuery)->Effect<ResourceListPageVM,RenderingError<E>,R>{self.rendering.list_page(request,kind,query)}
    pub fn detail_page<R:Send+Sync+'static>(&self,request:&Request,kind:&ResourceKind,query:ResourceDetailQuery)->Effect<PageVM,RenderingError<E>,R>{self.rendering.detail_page(request,kind,query)}
    pub fn dashboard_page<R:Send+Sync+'static>(&self,request:&Request,kind:&ResourceKind,query:ResourceListQuery)->Effect<ResourceDashboardPageVM,RenderingError<E>,R>{self.dashboard.dashboard_page(request,kind,query)}
    pub fn callbacks<R:Send+Sync+'static>(&self)->FullRenderingCallbacks<RenderingError<E>,R>{
        let list=self.clone();let detail=self.clone();let dashboard=self.clone();
        FullRenderingCallbacks{
            list_detail:RenderingCallbacks{list_page:Some(Arc::new(move |r,k,q|list.list_page(r,k,q))),detail_page:Some(Arc::new(move |r,k,q|detail.detail_page(r,k,q)))},
            dashboard_page:Arc::new(move |r,k,q|dashboard.dashboard_page(r,k,q)),
        }
    }
}
impl<E:Send+'static> RenderingService<E> for FullService<E>{
    fn list_page<R:Send+Sync+'static>(&self,r:&Request,k:&ResourceKind,q:ResourceListQuery)->Effect<ResourceListPageVM,RenderingError<E>,R>{self.list_page(r,k,q)}
    fn detail_page<R:Send+Sync+'static>(&self,r:&Request,k:&ResourceKind,q:ResourceDetailQuery)->Effect<PageVM,RenderingError<E>,R>{self.detail_page(r,k,q)}
}
impl<E:Send+'static> FullRenderingService<E> for FullService<E>{
    fn dashboard_page<R:Send+Sync+'static>(&self,r:&Request,k:&ResourceKind,q:ResourceListQuery)->Effect<ResourceDashboardPageVM,RenderingError<E>,R>{self.dashboard_page(r,k,q)}
}
pub type RenderingDashboardPageSource<E,R>=Arc<dyn Fn(&Request,&ResourceKind,ResourceListQuery)->Effect<ResourceDashboardPageVM,E,R>+Send+Sync+'static>;
pub struct FullRenderingCallbacks<E:Send+'static,R:Send+Sync+'static>{pub list_detail:RenderingCallbacks<E,R>,pub dashboard_page:RenderingDashboardPageSource<E,R>}
impl<E:Send+'static,R:Send+Sync+'static> Clone for FullRenderingCallbacks<E,R>{fn clone(&self)->Self{Self{list_detail:self.list_detail.clone(),dashboard_page:self.dashboard_page.clone()}}}
pub struct FullFrameBuilderOptions<E:Send+'static,R:Send+Sync+'static>{
    pub queries:Option<crate::queries::QueryService>,pub rendering:Option<FullRenderingCallbacks<E,R>>,
    pub default_page_size:i64,pub route_path:Option<RoutePathBuilder>,
}
impl<E:Send+'static,R:Send+Sync+'static> FullFrameBuilderOptions<E,R>{
    pub fn core(&self)->FrameBuilderOptions<E,R>{FrameBuilderOptions{queries:self.queries.clone(),rendering:self.rendering.as_ref().map(|r|r.list_detail.clone()),default_page_size:self.default_page_size,route_path:self.route_path.clone()}}
}
pub struct FullPageBuilderOptions<E:Send+'static,R:Send+Sync+'static>{pub frames:FullFrameBuilderOptions<E,R>,pub dashboards:Option<FullDashboardCallbacks<E,R>>}
pub type DashboardCardsSource<E,R>=Arc<dyn Fn(&Request)->Effect<Vec<crate::dashboards::DashboardCardVM>,E,R>+Send+Sync+'static>;
pub type DashboardPageSource<E,R>=Arc<dyn Fn(&Request,&ResourceKind)->Effect<ResourceDashboardPageVM,E,R>+Send+Sync+'static>;
pub struct FullDashboardCallbacks<E:Send+'static,R:Send+Sync+'static>{pub cards:DashboardCardsSource<E,R>,pub page:DashboardPageSource<E,R>}
impl<E:Send+'static,R:Send+Sync+'static> FullPageBuilderOptions<E,R>{pub fn core(&self)->PageBuilderOptions<E,R>{PageBuilderOptions{frames:self.frames.core()}}}
pub struct FullDefinitionResourcePageBuilderOptions<E:Send+'static,R:Send+Sync+'static>{
    pub pages:FullPageBuilderOptions<E,R>,pub list_seo:Option<ResourceSEOPropsResolver<E,R>>,pub detail_seo:Option<ResourceSEOPropsResolver<E,R>>,
    pub route_set:Option<ResourceRouteSetResolver>,pub hreflang_path:Option<HreflangPathResolver>,pub hreflangs_for_path:Option<HreflangsForPathResolver>,
}
impl<E:Send+'static,R:Send+Sync+'static> FullDefinitionResourcePageBuilderOptions<E,R>{
    pub fn core(&self)->DefinitionResourcePageBuilderOptions<E,R>{DefinitionResourcePageBuilderOptions{pages:self.pages.core(),list_seo:self.list_seo.clone(),detail_seo:self.detail_seo.clone(),route_set:self.route_set.clone(),hreflang_path:self.hreflang_path.clone(),hreflangs_for_path:self.hreflangs_for_path.clone()}}
}
pub struct RenderingServiceOptions<E:Send+'static>{
    pub is_management_request:Option<Arc<dyn Fn(&Request)->bool+Send+Sync+'static>>,
    pub route_set_for_request:Option<RouteSetForRequestResolver>,pub route_set_for_mode:Option<RouteSetForModeResolver>,
    pub route_path:Option<RoutePathBuilder>,pub breadcrumb:Option<ManagementBreadcrumbBuilder>,pub translate_key:Option<TranslateKeyFunc<Request>>,
    pub validate_list_page:Option<ListPageValidator<E>>,pub validate_detail_page:Option<DetailPageValidator<E>>,pub validate_dashboard_page:Option<DashboardPageValidator<E>>,
}
pub fn new_definition_rendering_service<E:Send+'static>(resources:Arc<dyn Registry+Send+Sync>,seo:SEOService<E>,options:RenderingServiceOptions<E>)->FullService<E>{
    let list=ListPageOptions{is_management_request:options.is_management_request.clone(),route_set_for_request:options.route_set_for_request,route_set_for_mode:options.route_set_for_mode.clone(),route_path:options.route_path.clone(),breadcrumb:options.breadcrumb.clone(),translate_key:options.translate_key.clone(),validate_page:options.validate_list_page};
    let detail=DetailPageOptions{is_management_request:options.is_management_request,route_set_for_mode:options.route_set_for_mode.clone(),breadcrumb:options.breadcrumb.clone(),translate_key:options.translate_key.clone(),validate_page:options.validate_detail_page};
    let dashboard=DashboardService::new(resources.clone(),DashboardServiceOptions{route_path:options.route_path,route_set_for_mode:options.route_set_for_mode,translate_key:options.translate_key,breadcrumb:options.breadcrumb,validate_page:options.validate_dashboard_page});
    FullService::new(Service::new(ListService::new(resources.clone(),seo.clone(),list),DetailService::new(resources,seo,detail)),dashboard)
}

impl<E:Send+'static,R:Send+Sync+'static> Clone for FullFrameBuilderOptions<E,R>{fn clone(&self)->Self{Self{queries:self.queries.clone(),rendering:self.rendering.clone(),default_page_size:self.default_page_size,route_path:self.route_path.clone()}}}
impl<E:Send+'static,R:Send+Sync+'static> Default for FullFrameBuilderOptions<E,R>{fn default()->Self{Self{queries:None,rendering:None,default_page_size:0,route_path:None}}}
impl<E:Send+'static,R:Send+Sync+'static> Clone for FullDashboardCallbacks<E,R>{fn clone(&self)->Self{Self{cards:self.cards.clone(),page:self.page.clone()}}}
impl<E:Send+'static,R:Send+Sync+'static> Clone for FullPageBuilderOptions<E,R>{fn clone(&self)->Self{Self{frames:self.frames.clone(),dashboards:self.dashboards.clone()}}}
impl<E:Send+'static,R:Send+Sync+'static> Default for FullPageBuilderOptions<E,R>{fn default()->Self{Self{frames:Default::default(),dashboards:None}}}
impl<E:Send+'static,R:Send+Sync+'static> Clone for FullDefinitionResourcePageBuilderOptions<E,R>{fn clone(&self)->Self{Self{pages:self.pages.clone(),list_seo:self.list_seo.clone(),detail_seo:self.detail_seo.clone(),route_set:self.route_set.clone(),hreflang_path:self.hreflang_path.clone(),hreflangs_for_path:self.hreflangs_for_path.clone()}}}
impl<E:Send+'static,R:Send+Sync+'static> Default for FullDefinitionResourcePageBuilderOptions<E,R>{fn default()->Self{Self{pages:Default::default(),list_seo:None,detail_seo:None,route_set:None,hreflang_path:None,hreflangs_for_path:None}}}
impl<E:Send+'static> Default for RenderingServiceOptions<E>{fn default()->Self{Self{is_management_request:None,route_set_for_request:None,route_set_for_mode:None,route_path:None,breadcrumb:None,translate_key:None,validate_list_page:None,validate_detail_page:None,validate_dashboard_page:None}}}
impl<E:Send+'static> Clone for RenderingServiceOptions<E>{fn clone(&self)->Self{Self{is_management_request:self.is_management_request.clone(),route_set_for_request:self.route_set_for_request.clone(),route_set_for_mode:self.route_set_for_mode.clone(),route_path:self.route_path.clone(),breadcrumb:self.breadcrumb.clone(),translate_key:self.translate_key.clone(),validate_list_page:self.validate_list_page.clone(),validate_detail_page:self.validate_detail_page.clone(),validate_dashboard_page:self.validate_dashboard_page.clone()}}}
