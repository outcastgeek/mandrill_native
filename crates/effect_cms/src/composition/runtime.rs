//! Full page and SEO adapter wiring over the completed definition services.
use std::{error::Error,sync::Arc};
use effect::Effect;
use effect_server::Request;
use crate::{rendering,seo,routing,definitions};
use super::{DefinitionServiceSet,ResourceRegistry};
type PageError<E>=rendering::RenderingError<E>;
pub struct DefinitionResourcePageOptions<E:Send+'static,R:Send+Sync+'static>{
    pub default_page_size:i64,pub route_path:Option<routing::RoutePathBuilder>,
    pub list_seo:Option<rendering::ResourceSEOPropsResolver<E,R>>,pub detail_seo:Option<rendering::ResourceSEOPropsResolver<E,R>>,
    pub route_set:Option<rendering::ResourceRouteSetResolver>,pub hreflang_path:Option<rendering::HreflangPathResolver>,pub hreflangs_for_path:Option<rendering::HreflangsForPathResolver>,
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for DefinitionResourcePageOptions<E,R>{fn clone(&self)->Self{Self{default_page_size:self.default_page_size,route_path:self.route_path.clone(),list_seo:self.list_seo.clone(),detail_seo:self.detail_seo.clone(),route_set:self.route_set.clone(),hreflang_path:self.hreflang_path.clone(),hreflangs_for_path:self.hreflangs_for_path.clone()}}}
impl<E:Send+'static,R:Send+Sync+'static> Default for DefinitionResourcePageOptions<E,R>{fn default()->Self{Self{default_page_size:0,route_path:None,list_seo:None,detail_seo:None,route_set:None,hreflang_path:None,hreflangs_for_path:None}}}
impl<E:Error+Send+'static,R:Send+Sync+'static> DefinitionServiceSet<E,R>{
    pub fn frame_builder_options(&self,default_page_size:i64,route_path:Option<routing::RoutePathBuilder>)->rendering::FullFrameBuilderOptions<PageError<E>,R>{
        rendering::FullFrameBuilderOptions{queries:Some(self.queries.clone()),rendering:Some(self.rendering.callbacks()),default_page_size,route_path}
    }
    pub fn page_builder_options(&self,default_page_size:i64,route_path:Option<routing::RoutePathBuilder>)->rendering::FullPageBuilderOptions<PageError<E>,R>{
        rendering::FullPageBuilderOptions{frames:self.frame_builder_options(default_page_size,route_path),dashboards:Some(self.dashboards.callbacks())}
    }
    pub fn resource_page_builder_options(&self,options:DefinitionResourcePageOptions<PageError<E>,R>)->rendering::FullDefinitionResourcePageBuilderOptions<PageError<E>,R>{
        let resources=self.resources.clone();
        rendering::FullDefinitionResourcePageBuilderOptions{pages:self.page_builder_options(options.default_page_size,options.route_path),list_seo:options.list_seo,detail_seo:options.detail_seo,route_set:options.route_set.or_else(||Some(Arc::new(move |kind,manage|routing::resource_route_set_for_kind(Some(resources.as_ref()),kind,manage)))),hreflang_path:options.hreflang_path,hreflangs_for_path:options.hreflangs_for_path}
    }
    pub fn resource_seo_props_options(&self,props:seo::SEOPropsOptions)->seo::ResourceRenderingSEOPropsOptions<PageError<E>,R>{
        seo::ResourceSEOPropsOptions{queries:Some(self.queries.clone()),rendering:Some(self.rendering.callbacks().list_detail),props}
    }
    pub fn resource_list_seo_props(&self,request:&Request,kind:&definitions::ResourceKind,props:seo::SEOPropsOptions)->Effect<seo::SEOProps,PageError<E>,R>{seo::resource_list_seo_props(request,kind,&self.resource_seo_props_options(props))}
    pub fn resource_detail_seo_props(&self,request:&Request,kind:&definitions::ResourceKind,props:seo::SEOPropsOptions)->Effect<seo::SEOProps,PageError<E>,R>{seo::resource_detail_seo_props(request,kind,&self.resource_seo_props_options(props))}
}
pub struct DefinitionServiceRuntimeOptions<E:Send+'static,R:Send+Sync+'static>{
    pub services:DefinitionServiceSet<E,R>,pub default_page_size:i64,pub route_path:Option<routing::RoutePathBuilder>,
    pub resource_page:DefinitionResourcePageOptions<PageError<E>,R>,pub seo_props:seo::SEOPropsOptions,
}
pub type DefinitionServiceRuntime<E,R>=DefinitionServiceRuntimeOptions<E,R>;
impl<E:Send+'static,R:Send+Sync+'static> Clone for DefinitionServiceRuntimeOptions<E,R>{fn clone(&self)->Self{Self{services:self.services.clone(),default_page_size:self.default_page_size,route_path:self.route_path.clone(),resource_page:self.resource_page.clone(),seo_props:self.seo_props.clone()}}}
pub fn new_definition_service_runtime<E:Send+'static,R:Send+Sync+'static>(options:DefinitionServiceRuntimeOptions<E,R>)->DefinitionServiceRuntime<E,R>{options}
impl<E:Error+Send+'static,R:Send+Sync+'static> DefinitionServiceRuntimeOptions<E,R>{
    pub fn frame_builder_options(&self)->rendering::FullFrameBuilderOptions<PageError<E>,R>{self.services.frame_builder_options(self.default_page_size,self.route_path.clone())}
    pub fn page_builder_options(&self)->rendering::FullPageBuilderOptions<PageError<E>,R>{self.services.page_builder_options(self.default_page_size,self.route_path.clone())}
    pub fn resource_page_builder_options(&self)->rendering::FullDefinitionResourcePageBuilderOptions<PageError<E>,R>{
        let mut options=self.resource_page.clone();if options.default_page_size==0{options.default_page_size=self.default_page_size;}if options.route_path.is_none(){options.route_path=self.route_path.clone();}
        let resolvers=seo::new_resource_seo_props_resolvers(self.services.resource_seo_props_options(self.seo_props.clone()));
        if options.list_seo.is_none(){options.list_seo=Some(resolvers.list);}if options.detail_seo.is_none(){options.detail_seo=Some(resolvers.detail);}
        self.services.resource_page_builder_options(options)
    }
    pub fn resource_list_seo_props(&self,request:&Request,kind:&definitions::ResourceKind)->Effect<seo::SEOProps,PageError<E>,R>{self.services.resource_list_seo_props(request,kind,self.seo_props.clone())}
    pub fn resource_detail_seo_props(&self,request:&Request,kind:&definitions::ResourceKind)->Effect<seo::SEOProps,PageError<E>,R>{self.services.resource_detail_seo_props(request,kind,self.seo_props.clone())}
    pub fn resources(&self)->ResourceRegistry{self.services.resources.clone()}
}
