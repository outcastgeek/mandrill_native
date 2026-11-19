//! Concrete page publication over canonical page values and optional consumer shells.
use std::{collections::BTreeMap,sync::Arc};
use effect_server::Request;
use maud::Markup;
use crate::{forms::*,rendering::{ResourceListPageVM,ResourceDetailPageVM,SEOView},dashboards::ResourceDashboardPageVM,definitions::{ResourceKind,ResourceSectionKey},localization::TranslateKeyFunc};
use super::*;

#[derive(Clone,Default)]
pub struct ResourcePageLayoutProps{pub request:Option<Request>,pub title:String,pub seo:SEOView}
pub type ResourcePageLayoutRenderer=Arc<dyn Fn(&ResourcePageLayoutProps,Markup)->Option<Markup>+Send+Sync>;
pub type ResourcePageLayoutComponent=ResourcePageLayoutRenderer;
pub type ResourcePageOverride<P>=Arc<dyn for<'a> Fn(ResourceRenderContext<'a>,&P,&ResourcePageShellOptions)->Option<Markup>+Send+Sync>;
#[derive(Clone,Default)]
pub struct ResourcePageRendererOptions{
    pub csrf_token:Option<Arc<dyn Fn(&Request)->String+Send+Sync>>,pub csrf_field_name:String,
    pub form_page:Option<ResourcePageOverride<ResourceFormPageVM>>,pub list_page:Option<ResourcePageOverride<ResourceListPageVM>>,
    pub detail_page:Option<ResourcePageOverride<ResourceDetailPageVM>>,pub dashboard_page:Option<ResourcePageOverride<ResourceDashboardPageVM>>,
    pub breadcrumb:Option<ResourceBreadcrumbRenderer>,pub status_alert:Option<ResourceTextRenderer>,pub live_submit_alerts:Option<ResourceTextRenderer>,
    pub form_submit_actions:Option<ResourceFormActionsRenderer>,pub media_modal_roots:Option<ResourceMediaRootsRenderer>,
    pub form_attributes:Option<ResourceFormAttributesBuilder>,pub form_render_options:ResourceFormRenderOptions,
    pub section_container_id:Option<ResourceSectionContainerID>,pub layout:Option<ResourcePageLayoutRenderer>,
    pub translate_key:Option<TranslateKeyFunc<Request>>,pub translate_args:Option<Arc<dyn Fn(&str,&[String])->String+Send+Sync>>,
}
#[derive(Clone)]
pub struct ResourcePageRenderer{options:ResourcePageRendererOptions}
#[derive(Debug)]
pub enum ResourceRenderError{MissingTypedEditor(String)}
impl std::fmt::Display for ResourceRenderError{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{match self{Self::MissingTypedEditor(name)=>write!(f,"field {name}: typed editor component is required")}}}
impl std::error::Error for ResourceRenderError{}
pub fn prepared_field_editor(field:&ResourceFieldVM)->Result<Markup,ResourceRenderError>{field.prepared_editor.clone().ok_or_else(||ResourceRenderError::MissingTypedEditor(resource_field_name(field)))}
impl ResourcePageRenderer{
    pub fn new(options:ResourcePageRendererOptions)->Self{Self{options}}
    fn context<'a>(&'a self,request:&'a Request)->ResourceRenderContext<'a>{ResourceRenderContext{request:Some(request),translate:self.options.translate_key.as_ref(),translate_args:self.options.translate_args.as_deref()}}
    pub fn shell_options(&self,request:&Request)->ResourcePageShellOptions{
        let o=&self.options;let req=request.clone();let key=o.translate_key.clone();
        let status=o.status_alert.clone().or_else(||Some(Arc::new(move |message:&str|localized_form_error_alert(&req,key.as_ref(),message)) as ResourceTextRenderer));
        let req=request.clone();let key=o.translate_key.clone();
        let live=o.live_submit_alerts.clone().or_else(||Some(Arc::new(move |class:&str|localized_live_submit_alerts(&req,key.as_ref(),class)) as ResourceTextRenderer));
        ResourcePageShellOptions{inline_media_actions:false,csrf_token:o.csrf_token.as_ref().map(|f|f(request)).unwrap_or_default(),csrf_field_name:o.csrf_field_name.clone(),breadcrumb:o.breadcrumb.clone(),status_alert:status,live_submit_alerts:live,
            form_submit_actions:o.form_submit_actions.clone().or_else(||Some(Arc::new(management_form_submit_actions))),
            media_modal_roots:o.media_modal_roots.clone().or_else(||Some(Arc::new(crate::media::presentation::media_modal_roots))),
            form_attributes:o.form_attributes.clone().or_else(||Some(Arc::new(default_resource_form_page_attributes))),
            form_render_options:default_form_render_options(request,o.translate_key.clone(),o.form_render_options.clone()),section_container_id:o.section_container_id.clone()}
    }
    pub fn form_page(&self,request:&Request,page:&ResourceFormPageVM)->Result<Markup,ResourceRenderError>{
        let options=self.shell_options(request);let c=self.context(request);
        if let Some(render)=&self.options.form_page{if let Some(component)=render(c,page,&options){return Ok(component)}}
        managed_resource_form_page(c,page,&options,self.options.layout.as_ref())
    }
    pub fn list_page(&self,request:&Request,page:&ResourceListPageVM)->Markup{
        let o=self.shell_options(request);let c=self.context(request);if let Some(render)=&self.options.list_page{if let Some(component)=render(c,page,&o){return component}}
        managed_resource_list_page(c,page,&o,self.options.layout.as_ref())
    }
    pub fn detail_page(&self,request:&Request,page:&ResourceDetailPageVM)->Markup{
        let o=self.shell_options(request);let c=self.context(request);if let Some(render)=&self.options.detail_page{if let Some(component)=render(c,page,&o){return component}}
        managed_resource_detail_page(c,page,&o,self.options.layout.as_ref())
    }
    pub fn dashboard_page(&self,request:&Request,page:&ResourceDashboardPageVM)->Markup{
        let o=self.shell_options(request);let c=self.context(request);if let Some(render)=&self.options.dashboard_page{if let Some(component)=render(c,page,&o){return component}}
        managed_resource_dashboard_page(c,page,&o,self.options.layout.as_ref())
    }
}
fn default_form_render_options(request:&Request,key:Option<TranslateKeyFunc<Request>>,mut options:ResourceFormRenderOptions)->ResourceFormRenderOptions{
    if options.localized_tabs.is_none(){let req=request.clone();let key=key.clone();options.localized_tabs=Some(Arc::new(move |props|localized_tabs(&req,key.as_ref(),props)));}
    if options.media_attachment_manager.is_none(){let req=request.clone();options.media_attachment_manager=Some(Arc::new(move |props|crate::media::presentation::media_attachment_manager(ResourceRenderContext{request:Some(&req),translate:key.as_ref(),translate_args:None},props)));}
    options
}
pub fn default_resource_form_page_attributes(page:&ResourceFormPageVM)->BTreeMap<String,String>{
    let mut attrs=BTreeMap::new();if !page.form_id.trim().is_empty(){attrs.insert("id".into(),crate::byte_text::encoding_text(page.form_id.trim().as_bytes()));}
    localized_authoring_form_attributes_for_widgets(&resource_form_localized_tabs(page),attrs)
}
pub type ResourceSectionContainerIDMap=BTreeMap<ResourceKind,BTreeMap<ResourceSectionKey,String>>;
pub fn new_resource_section_container_id_resolver(mapping:ResourceSectionContainerIDMap)->ResourceSectionContainerID{Arc::new(move |kind,key|mapping.get(kind).and_then(|sections|sections.get(key)).map(|s|s.trim().to_owned()).unwrap_or_default())}
pub fn new_resource_page_layout_renderer(render:Option<ResourcePageLayoutComponent>)->ResourcePageLayoutRenderer{Arc::new(move |props,body|Some(render.as_ref().and_then(|r|r(props,body.clone())).unwrap_or(body)))}
fn layout(props:ResourcePageLayoutProps,body:Markup,render:Option<&ResourcePageLayoutRenderer>)->Markup{render.and_then(|r|r(&props,body.clone())).unwrap_or(body)}
pub fn managed_resource_form_page(c:ResourceRenderContext<'_>,p:&ResourceFormPageVM,o:&ResourcePageShellOptions,l:Option<&ResourcePageLayoutRenderer>)->Result<Markup,ResourceRenderError>{Ok(layout(ResourcePageLayoutProps{request:c.request.cloned(),title:resource_page_title(&p.title,&p.breadcrumb),seo:SEOView::none()},resource_form_page_content(c,p,o)?,l))}
pub fn managed_resource_list_page(c:ResourceRenderContext<'_>,p:&ResourceListPageVM,o:&ResourcePageShellOptions,l:Option<&ResourcePageLayoutRenderer>)->Markup{layout(ResourcePageLayoutProps{request:c.request.cloned(),title:resource_page_title(&p.title,&p.breadcrumb),seo:p.seo_view.clone()},resource_list_page_content(c,p,o),l)}
pub fn managed_resource_detail_page(c:ResourceRenderContext<'_>,p:&ResourceDetailPageVM,o:&ResourcePageShellOptions,l:Option<&ResourcePageLayoutRenderer>)->Markup{layout(ResourcePageLayoutProps{request:c.request.cloned(),title:resource_page_title(&p.title,&p.breadcrumb),seo:p.seo_view.clone()},resource_detail_page_content(c,p,o),l)}
pub fn managed_resource_dashboard_page(c:ResourceRenderContext<'_>,p:&ResourceDashboardPageVM,o:&ResourcePageShellOptions,l:Option<&ResourcePageLayoutRenderer>)->Markup{layout(ResourcePageLayoutProps{request:c.request.cloned(),title:resource_page_title(&p.title,&p.breadcrumb),seo:SEOView::none()},resource_dashboard_page_content(c,p,o),l)}

impl ResourcePageRenderer{
    /// Publish a form lazily, retaining missing-editor failures in the caller's error family.
    pub fn form_reply<E:Send+'static,R:Send+Sync+'static>(&self,request:&Request,page:ResourceFormPageVM,on_error:impl Fn(ResourceRenderError)->E+Send+Sync+'static)
        ->effect::Effect<effect_server::Reply<E,R>,E,R>{
        let renderer=self.clone();let request=request.clone();
        effect::effect_try_sync!{body:move ||renderer.form_page(&request,&page).map(resource_html_reply),on_error:on_error}
    }
    pub fn list_reply<E:Send+'static,R:Send+Sync+'static>(&self,request:&Request,page:ResourceListPageVM)->effect::Effect<effect_server::Reply<E,R>,E,R>{
        let renderer=self.clone();let request=request.clone();effect::effect_sync!{body:move ||resource_html_reply(renderer.list_page(&request,&page))}
    }
    pub fn detail_reply<E:Send+'static,R:Send+Sync+'static>(&self,request:&Request,page:ResourceDetailPageVM)->effect::Effect<effect_server::Reply<E,R>,E,R>{
        let renderer=self.clone();let request=request.clone();effect::effect_sync!{body:move ||resource_html_reply(renderer.detail_page(&request,&page))}
    }
    pub fn dashboard_reply<E:Send+'static,R:Send+Sync+'static>(&self,request:&Request,page:ResourceDashboardPageVM)->effect::Effect<effect_server::Reply<E,R>,E,R>{
        let renderer=self.clone();let request=request.clone();effect::effect_sync!{body:move ||resource_html_reply(renderer.dashboard_page(&request,&page))}
    }
}
fn resource_html_reply<E:Send+'static,R:Send+Sync+'static>(markup:Markup)->effect_server::Reply<E,R>{effect_server::reply!{html(effect_server::HtmlContent::trusted(markup.into_string()));}}

pub fn new_resource_page_renderer(options:ResourcePageRendererOptions)->ResourcePageRenderer{ResourcePageRenderer::new(options)}
