//! Source definition-runtime defaults over the canonical owner constructors.
use std::{error::Error,ops::{Deref,DerefMut},sync::Arc};
use effect_server::{ByteFormPolicy,Request};
use crate::{access,commands,definitions,localization::TranslateKeyFunc,queries,rendering,resourcepages,routing,seo,validation};
use super::*;
/// Shared service fields live once in `services`; dereferencing exposes those fields.
/// `principal_resolver` is the existing native identity port, not a CMS identity store.
pub struct DefinitionRuntimeFactoryOptions<E:Send+'static,R:Send+Sync+'static>{
    pub services:DefinitionServiceSetOptions<E,R>,pub translate:Option<routing::VariadicTranslateKeyFunc>,
    pub route_to_path:Option<routing::RouteToPathFunc>,pub prefix_path:Option<routing::LanguagePathPrefixer>,
    pub strip_language_prefix:Option<routing::LanguagePathStripper>,pub management_prefix:String,
    pub resource_page:DefinitionResourcePageOptions<rendering::RenderingError<E>,R>,
    pub resource_page_renderer:resourcepages::ResourcePageRendererOptions,
    pub access_policy:access::ManagementAccessPolicyOptions<E,R>,pub principal_resolver:access::PrincipalResolver<E,R>,
    pub seo_props:seo::SEOPropsOptions,
}
impl<E:Send+'static,R:Send+Sync+'static> Deref for DefinitionRuntimeFactoryOptions<E,R>{type Target=DefinitionServiceSetOptions<E,R>;fn deref(&self)->&Self::Target{&self.services}}
impl<E:Send+'static,R:Send+Sync+'static> DerefMut for DefinitionRuntimeFactoryOptions<E,R>{fn deref_mut(&mut self)->&mut Self::Target{&mut self.services}}
impl<E:Send+'static,R:Send+Sync+'static> DefinitionRuntimeFactoryOptions<E,R>{
    pub fn new(form_policy:ByteFormPolicy,principal_resolver:access::PrincipalResolver<E,R>)->Self{Self{services:DefinitionServiceSetOptions::new(form_policy),translate:None,route_to_path:None,prefix_path:None,strip_language_prefix:None,management_prefix:String::new(),resource_page:Default::default(),resource_page_renderer:Default::default(),access_policy:Default::default(),principal_resolver,seo_props:Default::default()}}
}
pub struct DefinitionRuntimeFactory<E:Send+'static,R:Send+Sync+'static>{
    pub resources:ResourceRegistry,pub translate_key:TranslateKeyFunc<Request>,pub action_label:commands::ActionLabelResolver,
    pub route_path:routing::RoutePathBuilder,pub is_management:routing::ManagementRequestDetector,
    pub page_renderer:resourcepages::ResourcePageRenderer,pub access_policy:Arc<access::ManagementAccessPolicy<E,R>>,
    options:DefinitionServiceSetOptions<E,R>,resource_page:DefinitionResourcePageOptions<rendering::RenderingError<E>,R>,seo_props:seo::SEOPropsOptions,
}
pub fn new_definition_runtime_factory<E:Error+Send+'static,R:Send+Sync+'static>(mut options:DefinitionRuntimeFactoryOptions<E,R>)->DefinitionRuntimeFactory<E,R>{
    let resources=options.services.resources.clone().unwrap_or_else(||Arc::new(definitions::new_definition_registry(options.services.definitions.clone())));
    let translate_key=options.services.translate_key.clone().unwrap_or_else(||routing::new_variadic_translate_key_resolver(options.translate));
    let action_label=options.services.action_label.clone().unwrap_or_else(||routing::new_default_action_label_resolver(Some(translate_key.clone()),crate::media::presentation::MEDIA_PICKER_ATTACH_SELECTED_LABEL_KEY));
    let language=options.services.request_language.clone().map(|language|Arc::new(move |request:&Request|crate::byte_text::encoding_text(language(request).as_bytes())) as routing::PathLanguageFunc);
    let route_path=options.services.route_path.clone().unwrap_or_else(||routing::new_localized_route_path_builder(options.route_to_path,language,options.prefix_path));
    let is_management=options.services.is_management.clone().unwrap_or_else(||routing::new_path_prefix_management_request_detector(options.strip_language_prefix,&options.management_prefix));
    options.services.resources=Some(resources.clone());options.services.translate_key=Some(translate_key.clone());options.services.action_label=Some(action_label.clone());options.services.route_path=Some(route_path.clone());options.services.is_management=Some(is_management.clone());
    if options.access_policy.translate_key.is_none(){options.access_policy.translate_key=Some(translate_key.clone());}
    let page_renderer=resourcepages::new_resource_page_renderer(options.resource_page_renderer);
    let access_policy=Arc::new(access::new_management_access_policy(options.access_policy,options.principal_resolver));
    DefinitionRuntimeFactory{resources,translate_key,action_label,route_path,is_management,page_renderer,access_policy,options:options.services,resource_page:options.resource_page,seo_props:options.seo_props}
}
pub fn defaulted_canonical_route_params(mut params:queries::CanonicalRouteParams)->queries::CanonicalRouteParams{
    let defaults=routing::default_canonical_route_params();
    if params.id_param.is_empty(){params.id_param=defaults.id_param;}if params.slug_param.is_empty(){params.slug_param=defaults.slug_param;}
    if params.parent_param.is_empty(){params.parent_param=defaults.parent_param;}if params.parent_name_param.is_empty(){params.parent_name_param=defaults.parent_name_param;}params
}
impl<E:Error+Send+'static,R:Send+Sync+'static> DefinitionRuntimeFactory<E,R>{
    pub fn service_set_options(&self,resources:Option<ResourceRegistry>)->DefinitionServiceSetOptions<E,R>{
        let mut options=self.options.clone();options.resources=Some(resources.unwrap_or_else(||self.resources.clone()));options.definitions.clear();
        // Source service-set options start with empty binder maps; callers supply their maps.
        options.validation_bindings=Default::default();options.command_bindings=Default::default();
        if options.route_set_for_request.is_none(){let management=self.is_management.clone();options.route_set_for_request=Some(Arc::new(move |request,definition|routing::route_set_for_request(request,definition,Some(&management))));}
        if options.route_set_for_mode.is_none(){options.route_set_for_mode=Some(Arc::new(routing::route_set_for_mode));}
        options.canonical_params=defaulted_canonical_route_params(options.canonical_params);options
    }
    pub fn new_service_set(&self,resources:Option<ResourceRegistry>,validation:validation::Bindings<commands::CommandInput,E,R>,commands:commands::ResourceCommandBindings<E,R>)->DefinitionServiceSet<E,R>{
        let mut options=self.service_set_options(resources);options.validation_bindings=validation;options.command_bindings=commands;new_definition_service_set(options)
    }
    pub fn new_runtime(&self,services:DefinitionServiceSet<E,R>)->DefinitionServiceRuntime<E,R>{new_definition_service_runtime(DefinitionServiceRuntimeOptions{services,default_page_size:self.options.default_page_size,route_path:Some(self.route_path.clone()),resource_page:self.resource_page.clone(),seo_props:self.seo_props.clone()})}
    pub fn resolve_validation_service(&self,current:Option<ValidationService<E,R>>,resources:Option<ResourceRegistry>,binders:&[&dyn validation::Binder<commands::CommandInput,E,R>])->ValidationService<E,R>{
        if let Some(current)=current{return current}resolve_definition_validation_service(None,self.service_set_options(resources),binders)
    }
    pub fn resolve_command_service(&self,current:Option<commands::CommandService<E,R>>,resources:Option<ResourceRegistry>,binders:&[&dyn commands::ResourceCommandBinder<E,R>])->commands::CommandService<E,R>{
        if let Some(current)=current{return current}resolve_definition_command_service(None,self.service_set_options(resources),binders)
    }
}
