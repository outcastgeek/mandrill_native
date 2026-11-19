//! Registered dashboard cards and pages, built lazily over consumer callbacks.
use std::sync::Arc;
use effect::{Effect,effect,effect_sync,effect_try_sync};
use effect_server::Request;
use crate::definitions::{Registry,ResourceKind,ResourceViewMode,SharedList,RESOURCE_VIEW_MODE_DASHBOARD};
use crate::localization::TranslateKeyFunc;
use crate::queries::{ResourceListQuery,first_non_blank};
use crate::routing::{RoutePathBuilder,RouteSetForModeResolver,ManagementBreadcrumbBuilder};
use crate::rendering::RenderingError;
use super::*;

pub type DashboardPageValidator<E> = Arc<dyn Fn(&mut ResourceDashboardPageVM)->Result<(),E>+Send+Sync+'static>;
pub struct DashboardServiceOptions<E> {
    pub route_path:Option<RoutePathBuilder>,pub route_set_for_mode:Option<RouteSetForModeResolver>,
    pub translate_key:Option<TranslateKeyFunc<Request>>,pub breadcrumb:Option<ManagementBreadcrumbBuilder>,
    pub validate_page:Option<DashboardPageValidator<E>>,
}
impl<E> Clone for DashboardServiceOptions<E>{fn clone(&self)->Self {Self{route_path:self.route_path.clone(),route_set_for_mode:self.route_set_for_mode.clone(),translate_key:self.translate_key.clone(),breadcrumb:self.breadcrumb.clone(),validate_page:self.validate_page.clone()}}}
impl<E> Default for DashboardServiceOptions<E>{fn default()->Self{Self{route_path:None,route_set_for_mode:None,translate_key:None,breadcrumb:None,validate_page:None}}}
pub struct DashboardService<E>{resources:Arc<dyn Registry+Send+Sync>,options:DashboardServiceOptions<E>}
impl<E> Clone for DashboardService<E>{fn clone(&self)->Self{Self{resources:self.resources.clone(),options:self.options.clone()}}}
impl<E:Send+'static> DashboardService<E>{
    pub fn new(resources:Arc<dyn Registry+Send+Sync>,options:DashboardServiceOptions<E>)->Self{Self{resources,options}}
    pub fn cards<R:Send+Sync+'static>(&self,request:&Request)->Effect<Vec<DashboardCardVM>,RenderingError<E>,R>{
        let service=self.clone();let request=request.clone();
        effect_sync! {body:move || {
            let text=|key:&str|service.options.translate_key.as_ref().map(|t|t(&request,key)).unwrap_or_else(||key.trim().to_owned());
            let mut cards=Vec::new();
            for definition in service.resources.definitions(){
                let route=first_non_blank(&[definition.management_routes.dashboard.clone(),definition.management_routes.list.clone(),definition.public_routes.list.clone()]);
                if route.is_empty(){continue;}
                let title=first_non_blank(&[text(&definition.display.dashboard_title_key),text(&definition.display.list_title_key)]);
                let description=first_non_blank(&[text(&definition.display.dashboard_subtitle_key),text(&definition.display.list_subtitle_key)]);
                let href=service.options.route_path.as_ref().map(|p|p(&request,&route,&[])).unwrap_or_else(||route.trim().to_owned());
                cards.push(DashboardCardVM{kind:definition.kind,title,description,href});
            }cards
        }}
    }
    pub fn page<R:Send+Sync+'static>(&self,request:&Request,kind:&ResourceKind)->Effect<ResourceDashboardPageVM,RenderingError<E>,R>{
        self.build_page(request,kind,None)
    }
    pub fn dashboard_page<R:Send+Sync+'static>(&self,request:&Request,kind:&ResourceKind,query:ResourceListQuery)->Effect<ResourceDashboardPageVM,RenderingError<E>,R>{
        self.build_page(request,kind,Some(query))
    }
    fn build_page<R:Send+Sync+'static>(&self,request:&Request,kind:&ResourceKind,query:Option<ResourceListQuery>)->Effect<ResourceDashboardPageVM,RenderingError<E>,R>{
        let service=self.clone();let request=request.clone();let kind=kind.clone();
        effect! {
            let service=service.clone();let request=request.clone();let kind=kind.clone();let query=query.clone();
            let registry=service.resources.clone();let lookup=kind.clone();
            let definition=effect_try_sync! {once;body:move ||registry.definition(&lookup),on_error:RenderingError::Definition} ?;
            let text=|key:&str|service.options.translate_key.as_ref().map(|t|t(&request,key)).unwrap_or_else(||key.trim().to_owned());
            let mode=ResourceViewMode::new(RESOURCE_VIEW_MODE_DASHBOARD);
            let title=first_non_blank(&[text(&definition.display.dashboard_title_key),text(&definition.display.list_title_key)]);
            let subtitle=first_non_blank(&[text(&definition.display.dashboard_subtitle_key),text(&definition.display.list_subtitle_key)]);
            let breadcrumb=service.options.breadcrumb.as_ref().map(|b|b(&request,&definition,&mode)).unwrap_or_default();
            let routes=service.options.route_set_for_mode.as_ref().map(|r|r(&definition,&mode)).unwrap_or_default();
            let path=service.options.route_path.clone();
            let path:Option<DashboardRoutePath>=path.map(|p|Arc::new(move |request:Option<&Request>,name:&str,params:&[(String,String)]| request.map(|r|p(r,name,params)).unwrap_or_else(||name.trim().to_owned())) as DashboardRoutePath);
            let actions=if query.is_none(){Some(SharedList::new(build_dashboard_actions(Arc::new(request.clone()),Some(request.clone()),definition.clone(),path.clone(),service.options.translate_key.clone())?))}else{Some(SharedList::new(Vec::new()))};
            let query=query.unwrap_or_else(||ResourceListQuery{search:dashboard_search_query_value(Some(&request)),..Default::default()});
            let search=resource_dashboard_search_vm(Arc::new(request.clone()),Some(request.clone()),definition,query,path,service.options.translate_key.clone())?;
            let mut page=ResourceDashboardPageVM{kind,mode,title,subtitle,breadcrumb,routes,actions,search,sections:Some(SharedList::new(Vec::new()))};
            let validate=service.options.validate_page.clone();
            effect_try_sync! {once;body:move ||{if let Some(validate)=validate{validate(&mut page).map_err(RenderingError::Consumer)?;}Ok::<_,RenderingError<E>>(page)}} ?
        }
    }
}

pub trait DashboardCapability<E:Send+'static>{
    fn cards<R:Send+Sync+'static>(&self,request:&Request)->Effect<Vec<DashboardCardVM>,RenderingError<E>,R>;
    fn page<R:Send+Sync+'static>(&self,request:&Request,kind:&ResourceKind)->Effect<ResourceDashboardPageVM,RenderingError<E>,R>;
}
impl<E:Send+'static> DashboardCapability<E> for DashboardService<E>{
    fn cards<R:Send+Sync+'static>(&self,r:&Request)->Effect<Vec<DashboardCardVM>,RenderingError<E>,R>{self.cards(r)}
    fn page<R:Send+Sync+'static>(&self,r:&Request,k:&ResourceKind)->Effect<ResourceDashboardPageVM,RenderingError<E>,R>{self.page(r,k)}
}
impl<E:Send+'static> DashboardService<E>{
    pub fn callbacks<R:Send+Sync+'static>(&self)->crate::rendering::FullDashboardCallbacks<RenderingError<E>,R>{
        let cards=self.clone();let page=self.clone();crate::rendering::FullDashboardCallbacks{cards:Arc::new(move |r|cards.cards(r)),page:Arc::new(move |r,k|page.page(r,k))}
    }
}
pub type Service<E> = DashboardService<E>;
pub type Options<E> = DashboardServiceOptions<E>;

pub fn new_service<E:Send+'static>(resources:Arc<dyn Registry+Send+Sync>,options:DashboardServiceOptions<E>)->DashboardService<E>{DashboardService::new(resources,options)}
pub fn new_rendering_dashboard<E:Send+'static>(resources:Arc<dyn Registry+Send+Sync>,options:DashboardServiceOptions<E>)->DashboardService<E>{DashboardService::new(resources,options)}
