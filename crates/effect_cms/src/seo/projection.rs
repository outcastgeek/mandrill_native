//! Effectful query → rendering → SEO projection used by native page composition.
use std::sync::Arc;
use effect::{Effect,effect,effect_map};
use effect_server::Request;
use crate::definitions::ResourceKind;
use crate::rendering::{RenderingCallbacks,ResourceListPageVM,ResourceDetailPageVM};
use super::{SEOProps,SEOPropsOptions,SEOVM,ResourceSEOPropsOptions,seo_props_from_view_model};

pub type SEOQuerySource<Query,E,R>=Arc<dyn Fn(&Request,&ResourceKind)->Effect<Query,E,R>+Send+Sync+'static>;
pub type SEOPageSource<Query,Page,E,R>=Arc<dyn Fn(&Request,&ResourceKind,Query)->Effect<Option<Page>,E,R>+Send+Sync+'static>;
pub type SEOPageView<Page>=Arc<dyn Fn(&Page)->Option<SEOVM>+Send+Sync+'static>;

/// Expected render failure or no page gives empty props. All other native exits remain exits.
pub fn resource_seo_props_effect<Query,Page,E,R>(request:&Request,kind:&ResourceKind,options:SEOPropsOptions,
    query:Option<SEOQuerySource<Query,E,R>>,render:Option<SEOPageSource<Query,Page,E,R>>,view:SEOPageView<Page>)
    ->Effect<SEOProps,E,R>
where Query:Send+'static,Page:Send+'static,E:Send+'static,R:Send+Sync+'static {
    let request=request.clone();let kind=kind.clone();
    effect!{
        let request=request.clone();let kind=kind.clone();let options=options.clone();
        let query=query.clone();let render=render.clone();let view=view.clone();
        match (query,render){
            (Some(query),Some(render))=>{
                let admitted=query(&request,&kind)?;
                match effect! { either; program: render(&request,&kind,admitted), }?{
                    Ok(Some(page))=>{let vm=view(&page);seo_props_from_view_model(&request,vm.as_ref(),&options)},
                    Ok(None)|Err(_)=>SEOProps::default(),
                }
            },
            _=>SEOProps::default(),
        }
    }
}

/// The existing rendering callbacks keep the consumer's complete native error family.
pub type ResourceRenderingSEOPropsOptions<E,R>=ResourceSEOPropsOptions<Option<RenderingCallbacks<E,R>>>;
pub fn resource_list_seo_props<E:Send+'static,R:Send+Sync+'static>(request:&Request,kind:&ResourceKind,options:&ResourceRenderingSEOPropsOptions<E,R>)->Effect<SEOProps,E,R>{
    let query:Option<SEOQuerySource<crate::queries::ResourceListQuery,E,R>>=options.queries.clone().map(|queries|Arc::new(move |request:&Request,kind:&ResourceKind|queries.list_query(request,kind)) as SEOQuerySource<_,_,_>);
    let render:Option<SEOPageSource<crate::queries::ResourceListQuery,ResourceListPageVM,E,R>>=options.rendering.as_ref().and_then(|r|r.list_page.clone()).map(|render|Arc::new(move |request:&Request,kind:&ResourceKind,query|{
        effect_map!{program:render(request,kind,query),map:Some}
    }) as SEOPageSource<_,_,_,_>);
    resource_seo_props_effect(request,kind,options.props.clone(),query,render,Arc::new(|page:&ResourceListPageVM|Some(page.seo.clone())))
}
pub fn resource_detail_seo_props<E:Send+'static,R:Send+Sync+'static>(request:&Request,kind:&ResourceKind,options:&ResourceRenderingSEOPropsOptions<E,R>)->Effect<SEOProps,E,R>{
    let query:Option<SEOQuerySource<crate::queries::ResourceDetailQuery,E,R>>=options.queries.clone().map(|queries|Arc::new(move |request:&Request,kind:&ResourceKind|queries.detail_query(request,kind)) as SEOQuerySource<_,_,_>);
    let render:Option<SEOPageSource<crate::queries::ResourceDetailQuery,ResourceDetailPageVM,E,R>>=options.rendering.as_ref().and_then(|r|r.detail_page.clone()).map(|render|Arc::new(move |request:&Request,kind:&ResourceKind,query|{
        effect_map!{program:render(request,kind,query),map:Some}
    }) as SEOPageSource<_,_,_,_>);
    resource_seo_props_effect(request,kind,options.props.clone(),query,render,Arc::new(|page:&ResourceDetailPageVM|Some(page.seo.clone())))
}

/// Both page-resolver callbacks share the same query/rendering supplies.
pub struct ResourceSEOPropsResolvers<E:Send+'static,R:Send+Sync+'static>{
    pub list:crate::rendering::ResourceSEOPropsResolver<E,R>,
    pub detail:crate::rendering::ResourceSEOPropsResolver<E,R>,
}
pub fn new_resource_seo_props_resolvers<E:Send+'static,R:Send+Sync+'static>(options:ResourceRenderingSEOPropsOptions<E,R>)->ResourceSEOPropsResolvers<E,R>{
    let list=options.clone();let detail=options;
    ResourceSEOPropsResolvers{
        list:Arc::new(move |request,kind|{effect_map!{program:resource_list_seo_props(request,kind,&list),map:crate::rendering::SEOView::new}}),
        detail:Arc::new(move |request,kind|{effect_map!{program:resource_detail_seo_props(request,kind,&detail),map:crate::rendering::SEOView::new}}),
    }
}
/// Apply the source composition defaults while retaining caller-supplied resolvers.
pub fn bind_resource_seo_props_resolvers<E:Send+'static,R:Send+Sync+'static>(pages:&mut crate::rendering::DefinitionResourcePageBuilderOptions<E,R>,resolvers:ResourceSEOPropsResolvers<E,R>){
    if pages.list_seo.is_none(){pages.list_seo=Some(resolvers.list);}
    if pages.detail_seo.is_none(){pages.detail_seo=Some(resolvers.detail);}
}
