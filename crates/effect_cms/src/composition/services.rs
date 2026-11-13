//! Required definition services built in source dependency order.
use std::{error::Error,sync::Arc};
use effect_server::{Request,ByteFormPolicy};
use crate::{commands,definitions,queries,validation,forms,rendering,dashboards,seo,routing,localization::TranslateKeyFunc};
use super::{CMSRequestContext,ValidationMessageTranslator};
pub type ResourceRegistry=Arc<dyn definitions::Registry+Send+Sync>;
pub type FormService<E,R>=forms::Service<CMSRequestContext,E,R>;
pub type ValidationService<E,R>=validation::ActionValidationService<E,R>;
pub type RenderingService<E>=rendering::FullService<E>;
pub struct DefinitionServiceValidators<E>{
    pub command_input:Option<commands::CommandInputValidator<E>>,pub validation:Option<validation::ValidationResultValidator<E>>,
    pub mutation:Option<commands::MutationResultValidator<E>>,pub form_page:Option<forms::FormPageValidator<E>>,
    pub list_page:Option<rendering::ListPageValidator<E>>,pub detail_page:Option<rendering::DetailPageValidator<E>>,
    pub dashboard_page:Option<dashboards::DashboardPageValidator<E>>,pub seo:Option<seo::SEOValidator<E>>,
    pub sitemap_document:Option<seo::SitemapDocumentValidator<E>>,pub sitemap_index:Option<seo::SitemapIndexValidator<E>>,
}
impl<E> Clone for DefinitionServiceValidators<E>{fn clone(&self)->Self{Self{command_input:self.command_input.clone(),validation:self.validation.clone(),mutation:self.mutation.clone(),form_page:self.form_page.clone(),list_page:self.list_page.clone(),detail_page:self.detail_page.clone(),dashboard_page:self.dashboard_page.clone(),seo:self.seo.clone(),sitemap_document:self.sitemap_document.clone(),sitemap_index:self.sitemap_index.clone()}}}
impl<E> Default for DefinitionServiceValidators<E>{fn default()->Self{Self{command_input:None,validation:None,mutation:None,form_page:None,list_page:None,detail_page:None,dashboard_page:None,seo:None,sitemap_document:None,sitemap_index:None}}}
pub struct DefinitionServiceSetOptions<E:Send+'static,R:Send+Sync+'static>{
    pub resources:Option<ResourceRegistry>,pub definitions:Vec<definitions::ResourceDefinition>,pub default_page_size:i64,
    pub parse_named_page:Option<queries::ParseNamedPageFunc>,pub request_language:Option<queries::RequestLanguageFunc>,
    pub validation_bindings:validation::Bindings<commands::CommandInput,E,R>,pub command_bindings:commands::ResourceCommandBindings<E,R>,
    pub action_label:Option<commands::ActionLabelResolver>,pub translate_key:Option<TranslateKeyFunc<Request>>,
    pub is_management:Option<routing::ManagementRequestDetector>,pub route_path:Option<routing::RoutePathBuilder>,
    pub route_set_for_request:Option<routing::RouteSetForRequestResolver>,pub route_set_for_mode:Option<routing::RouteSetForModeResolver>,
    pub canonical_route_path:Option<routing::CanonicalRoutePathBuilder>,pub canonical_params:queries::CanonicalRouteParams,
    pub validation_paths:validation::Catalog,pub validators:DefinitionServiceValidators<E>,pub form_policy:ByteFormPolicy,
    pub validation_translate:Option<ValidationMessageTranslator>,
}
impl<E:Send+'static,R:Send+Sync+'static> DefinitionServiceSetOptions<E,R>{
    pub fn new(form_policy:ByteFormPolicy)->Self{Self{resources:None,definitions:Vec::new(),default_page_size:0,parse_named_page:None,request_language:None,validation_bindings:Default::default(),command_bindings:Default::default(),action_label:None,translate_key:None,is_management:None,route_path:None,route_set_for_request:None,route_set_for_mode:None,canonical_route_path:None,canonical_params:Default::default(),validation_paths:Default::default(),validators:Default::default(),form_policy,validation_translate:None}}
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for DefinitionServiceSetOptions<E,R>{fn clone(&self)->Self{Self{resources:self.resources.clone(),definitions:self.definitions.clone(),default_page_size:self.default_page_size.clone(),parse_named_page:self.parse_named_page.clone(),request_language:self.request_language.clone(),validation_bindings:self.validation_bindings.clone(),command_bindings:self.command_bindings.clone(),action_label:self.action_label.clone(),translate_key:self.translate_key.clone(),is_management:self.is_management.clone(),route_path:self.route_path.clone(),route_set_for_request:self.route_set_for_request.clone(),route_set_for_mode:self.route_set_for_mode.clone(),canonical_route_path:self.canonical_route_path.clone(),canonical_params:self.canonical_params.clone(),validation_paths:self.validation_paths.clone(),validators:self.validators.clone(),form_policy:self.form_policy.clone(),validation_translate:self.validation_translate.clone()}}}
/// Every field is an actual constructed owner capability, including dashboard rendering.
pub struct DefinitionServiceSet<E:Send+'static,R:Send+Sync+'static>{
    pub resources:ResourceRegistry,pub queries:queries::QueryService,pub validation:ValidationService<E,R>,pub commands:commands::CommandService<E,R>,
    pub seo:seo::Service<E>,pub forms:FormService<E,R>,pub rendering:RenderingService<E>,pub dashboards:dashboards::DashboardService<E>,
    pub validation_translate:Option<ValidationMessageTranslator>,
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for DefinitionServiceSet<E,R>{fn clone(&self)->Self{Self{resources:self.resources.clone(),queries:self.queries.clone(),validation:self.validation.clone(),commands:self.commands.clone(),seo:self.seo.clone(),forms:self.forms.clone(),rendering:self.rendering.clone(),dashboards:self.dashboards.clone(),validation_translate:self.validation_translate.clone()}}}
pub fn new_definition_service_set<E:Error+Send+'static,R:Send+Sync+'static>(options:DefinitionServiceSetOptions<E,R>)->DefinitionServiceSet<E,R>{
    let resources=options.resources.unwrap_or_else(||Arc::new(definitions::new_definition_registry(options.definitions)));
    let queries=queries::new_query_service(resources.clone(),queries::Options{default_page_size:options.default_page_size,parse_named_page:options.parse_named_page,request_language:options.request_language,detail_params:options.canonical_params.clone()});
    let validation=ValidationService::new(resources.clone(),Some(queries.clone()),options.form_policy,validation::Options{bindings:options.validation_bindings,validators:validation::Validators{command_input:options.validators.command_input.clone(),validation_result:options.validators.validation},path_catalog:Default::default()});
    let commands=commands::CommandService::new(resources.clone(),commands::Options{resolve_action_label:options.action_label,validate_command_input:options.validators.command_input,validate_mutation_result:options.validators.mutation,executors:options.command_bindings});
    let canonical=options.canonical_route_path.or_else(||{let path=options.route_path.clone();let params=options.canonical_params.clone();Some(Arc::new(move |request:&Request,definition:&definitions::ResourceDefinition,mode:&definitions::ResourceViewMode|{
        let query:routing::RequestValueFunc=Arc::new(|request,key|crate::byte_text::encoding_text(queries::request_query_value(request,key).as_bytes()));
        let captured:routing::RequestValueFunc=Arc::new(queries::path_value);
        routing::canonical_route_path(request,definition,mode,path.as_ref(),Some(&query),Some(&captured),&params)
    }) as routing::CanonicalRoutePathBuilder)});
    let seo=seo::Service::new(resources.clone(),seo::Options{translate_key:options.translate_key.clone(),canonical_route_path:canonical,validate_seo:options.validators.seo,validate_sitemap_document:options.validators.sitemap_document,validate_sitemap_index:options.validators.sitemap_index});
    let title_key=options.translate_key.clone();let breadcrumb_key=options.translate_key.clone();
    let field_key=options.translate_key.clone();
    let breadcrumb:routing::ManagementBreadcrumbBuilder=Arc::new(move |request,definition,mode|routing::management_breadcrumb(request,definition,mode,breadcrumb_key.as_ref()));
    let forms=forms::Service::new(resources.clone(),commands.clone(),forms::Options{resource_mode_title:Some(Arc::new(move |request,definition,mode|routing::resource_mode_title(request,definition,mode,title_key.as_ref()))),breadcrumb:Some(breadcrumb.clone()),route_set_for_request:options.route_set_for_request.clone(),translate_key:field_key.map(|key|Arc::new(move |context:&CMSRequestContext,label:&str|key(&context.request,label)) as TranslateKeyFunc<CMSRequestContext>),validation_path_catalog:options.validation_paths,validate_page:options.validators.form_page});
    let rendering=rendering::new_definition_rendering_service(resources.clone(),seo.clone(),rendering::RenderingServiceOptions{is_management_request:options.is_management,route_set_for_request:options.route_set_for_request,route_set_for_mode:options.route_set_for_mode.clone(),route_path:options.route_path.clone(),breadcrumb:Some(breadcrumb.clone()),translate_key:options.translate_key.clone(),validate_list_page:options.validators.list_page,validate_detail_page:options.validators.detail_page,validate_dashboard_page:options.validators.dashboard_page.clone()});
    let dashboards=dashboards::DashboardService::new(resources.clone(),dashboards::DashboardServiceOptions{route_path:options.route_path,route_set_for_mode:options.route_set_for_mode,translate_key:options.translate_key,breadcrumb:Some(breadcrumb),validate_page:options.validators.dashboard_page});
    DefinitionServiceSet{resources,queries,validation,commands,seo,forms,rendering,dashboards,validation_translate:options.validation_translate}
}
impl<E:Error+Send+'static,R:Send+Sync+'static> DefinitionServiceSet<E,R>{
    pub fn context(&self,request:&Request)->CMSRequestContext{CMSRequestContext::new(request,self.validation_translate.clone())}
}
pub fn resolve_definition_validation_service<E:Error+Send+'static,R:Send+Sync+'static>(current:Option<ValidationService<E,R>>,mut options:DefinitionServiceSetOptions<E,R>,binders:&[&dyn validation::Binder<commands::CommandInput,E,R>])->ValidationService<E,R>{
    if let Some(current)=current{return current}options.validation_bindings=validation::new_bindings(binders);new_definition_service_set(options).validation
}
pub fn resolve_definition_command_service<E:Error+Send+'static,R:Send+Sync+'static>(current:Option<commands::CommandService<E,R>>,mut options:DefinitionServiceSetOptions<E,R>,binders:&[&dyn commands::ResourceCommandBinder<E,R>])->commands::CommandService<E,R>{
    if let Some(current)=current{return current}options.command_bindings=commands::new_resource_command_bindings(binders);new_definition_service_set(options).commands
}
