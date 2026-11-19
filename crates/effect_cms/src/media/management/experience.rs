//! Concrete media management composition over native forms, rendering and stores.
use std::sync::Arc;
use effect::Effect;
use effect_server::Request;
use crate::{definitions::{ResourceKind,Registry,find_resource_definition},localization::TranslateKeyFunc,routing::{RoutePathBuilder,MutationResultRoutePathBuilder}};
use crate::media::{assets::{MediaAssetFormInput,MediaAssetURLResolver},runtime::{MediaAssetStoreRuntime,MediaRequestURLResolver},flows::*,presentation::*};
use super::*;

pub const DEFAULT_MEDIA_PICKER_PAGE_SIZE:i64=8;
pub const DEFAULT_MEDIA_SHORTCODE_PAGE_SIZE:i64=4;
pub struct MediaAssetManagementOptions<E:Send+'static,R:Send+Sync+'static> {
    pub runtime:MediaAssetStoreRuntime<E,R>,
    pub resource_kind:ResourceKind,
    pub library_page_size:i64,
    pub picker_page_size:i64,
    pub shortcode_page_size:i64,
    pub io:MediaFlowIO<E,R>,
    pub require_access:Option<MediaAccessGuard<E,R>>,
    pub request_language:Option<MediaRequestURLResolver>,
    pub mutation_route_path:Option<MutationResultRoutePathBuilder>,
    pub csrf_token:Option<MediaRequestURLResolver>,
    pub csrf_field_name:Option<MediaRequestURLResolver>,
    pub render_html:Option<MediaHTMLWriter<E,R>>,
    pub render_picker:Option<MediaPickerPageRenderer>,
    pub list_page:Option<MediaAssetListPageBuilder<E,R>>,
    pub form_page:Option<MediaFormPageBuilder<E,R>>,
    pub validation:Option<Arc<dyn Fn()->Option<crate::validation::ActionValidationService<E,R>> + Send + Sync>>,
    pub commands:Option<Arc<dyn Fn()->Option<crate::commands::CommandService<E,R>> + Send + Sync>>,
    pub on_mutation_error:Arc<dyn Fn(crate::mutations::MutationFailure<E>)->E + Send + Sync>,
    pub validate_input:Option<MediaInputValidator<MediaAssetFormInput>>,
    pub translate_key:Option<TranslateKeyFunc<Request>>,
    pub translate_args:Option<Arc<dyn Fn(&Request,&str,&[String])->String + Send + Sync>>,
    pub url_required_key:String,
    pub bad_request_key:String,
    pub internal_error_key:String,
    pub not_found_key:String,
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for MediaAssetManagementOptions<E,R>{fn clone(&self)->Self{Self{
runtime:self.runtime.clone(),
resource_kind:self.resource_kind.clone(),
library_page_size:self.library_page_size.clone(),
picker_page_size:self.picker_page_size.clone(),
shortcode_page_size:self.shortcode_page_size.clone(),
io:self.io.clone(),
require_access:self.require_access.clone(),
request_language:self.request_language.clone(),
mutation_route_path:self.mutation_route_path.clone(),
csrf_token:self.csrf_token.clone(),
csrf_field_name:self.csrf_field_name.clone(),
render_html:self.render_html.clone(),
render_picker:self.render_picker.clone(),
list_page:self.list_page.clone(),
form_page:self.form_page.clone(),
validation:self.validation.clone(),
commands:self.commands.clone(),
on_mutation_error:self.on_mutation_error.clone(),
validate_input:self.validate_input.clone(),
translate_key:self.translate_key.clone(),
translate_args:self.translate_args.clone(),
url_required_key:self.url_required_key.clone(),
bad_request_key:self.bad_request_key.clone(),
internal_error_key:self.internal_error_key.clone(),
not_found_key:self.not_found_key.clone(),
}}}
fn page_size(value:i64,fallback:i64)->i64{if value>0{value}else{fallback}}
fn message<E:Send+'static,R:Send+Sync+'static>(options:&MediaAssetManagementOptions<E,R>,key:&str,fallback:&str)->MediaRequestURLResolver{
    translated_media_message(if key.is_empty(){fallback}else{key},options.translate_key.clone())
}
pub fn new_media_asset_flow_factory<E:Send+'static,R:Send+Sync+'static>(options:&MediaAssetManagementOptions<E,R>)->MediaAssetFlowFactory<E,R>{
    let runtime=options.runtime.clone();let rt=runtime.clone();
    let surface=Arc::new(move|request:&Request,form:&effect_server::ByteFormData|rt.surface_request(request,Some(form))) as MediaSurfaceResolver;
    let rt=runtime.clone();let default_role=Arc::new(move|_request:&Request,surface:&crate::media::surface::MediaSurfaceRequest|rt.default_role(surface).into()) as MediaDefaultRoleResolver;
    let rt=runtime.clone();let current=Arc::new(move|request:&Request,surface:crate::media::surface::MediaSurfaceRequest,form:Arc<effect_server::ByteFormData>|rt.current_attachment_inputs(request,&surface,Some(form))) as MediaAttachmentInputsLoader<E,R>;
    let rt=runtime.clone();let preview=Arc::new(move|request:&Request,inputs|rt.preview_attachment_items_from_inputs(request,inputs)) as MediaAttachmentItemsPreviewer<E,R>;
    let rt=runtime.clone();let shortcodes=Arc::new(move|request:&Request,query|rt.load_shortcode_items(request,query)) as MediaShortcodeItemsLoader<E,R>;
    let rt=runtime.clone();let create=Arc::new(move|request:&Request,input|{let created=rt.create_asset_from_form(request,input);effect::effect_map!{program:created,map: |id|crate::commands::InputBytes::from(id),}}) as MediaAssetCreator<MediaAssetFormInput,E,R>;
    let rt=runtime.clone();let io=options.io.clone();let translate=options.translate_key.clone();
    let build_picker=Arc::new(move|request:&Request,query,error,form|{
        let translate=translate.clone();let on_error=io.on_form_error.clone();
        rt.build_picker_page(request,query,error,Some(form),io.form_policy,on_error,Arc::new(move|state|{
            let items=media_picker_items_from_result(translate.as_ref(),&state.request,state.result.as_ref(),&state.current_attachments,
                &crate::byte_text::encoding_text(state.surface.role.as_bytes()),&state.image_kind,Some(&state.resolve_url),&state.hero_role);
            build_media_picker_page_from_state(&MediaPickerPageStateOptions{
                request:Some(state.request),surface:state.surface,query:state.query,error_msg:state.error_msg,total:state.total,
                default_root_id:state.default_picker_root_id,default_create_root_id:state.default_create_root_id,default_role:state.default_role,
                default_page_size:state.default_page_size,attach_url:state.attach_url,create_base_path:state.create_base_path,inline_surface:state.inline_surface,
                context_label:state.context_label,filter_action:state.filter_action,items,current_attachments:state.current_attachments,total_pages:state.total_pages,translate_key:translate.clone(),
            })
        }))
    }) as MediaPickerPageBuilder<E,R>;
    let render_picker=options.render_picker.clone().unwrap_or_else(||{
        let translate=options.translate_key.clone();let args=options.translate_args.clone();let token=options.csrf_token.clone();let field=options.csrf_field_name.clone();
        Arc::new(move|page:MediaPickerPageVM|{
            let request=page.request.as_ref();
            let args=|key:&str,values:&[String]|request.and_then(|request|args.as_ref().map(|args|args(request,key,values))).unwrap_or_else(||request.map(|request|media_translate(translate.as_ref(),request,key)).unwrap_or_else(||key.into()));
            let render=MediaPickerRenderOptions{
                csrf_token:request.and_then(|request|token.as_ref().map(|resolve|resolve(request))).unwrap_or_default(),
                csrf_field_name:request.and_then(|request|field.as_ref().map(|resolve|resolve(request))).unwrap_or_default(),
                attachment_payload_inputs:Some(Arc::new(media_attachment_payload_inputs)),
                item_card:None,pagination:None,
            };
            default_media_picker(MediaRenderContext{request,translate:translate.as_ref(),translate_args:Some(&args)},&page,&render.csrf_token,&render.csrf_field_name)
        })
    });
    let validate=options.validate_input.clone().unwrap_or_else(||{
        let translate=options.translate_key.clone();let key=if options.url_required_key.is_empty(){"media.form.validation.url_required".into()}else{options.url_required_key.clone()};
        Arc::new(move|request:&Request,input:&MediaAssetFormInput|crate::media::assets::validate_media_asset_url(input,&media_translate(translate.as_ref(),request,&key)))
    });
    let mut io=options.io.clone();
    io.write_html=options.render_html.clone().map(|render|Arc::new(move|request:&Request,markup|render(request,Some(markup))) as MediaFlowHTMLWriter<E,R>);
    io.write_patches=Some(Arc::new(|_request,patches|effect::effect_sync!{body:move||crate::media::surface::write_media_patches(&patches)}));
    let base=MediaFlowFactory{
        io,library_page_size:page_size(options.library_page_size,24),picker_page_size:page_size(options.picker_page_size,8),shortcode_page_size:page_size(options.shortcode_page_size,4),
        default_fragment_root_id:crate::media::surface::DEFAULT_MEDIA_FRAGMENT_ROOT_ID.into(),default_picker_root_id:crate::media::surface::DEFAULT_MEDIA_PICKER_ROOT_ID.into(),
        default_create_root_id:crate::media::surface::DEFAULT_MEDIA_CREATE_ROOT_ID.into(),default_shortcode_root_id:crate::media::surface::DEFAULT_MEDIA_SHORTCODE_ROOT_ID.into(),inline_surface:runtime.resolved_inline_surface(),
        media_new_url:runtime.media_new_url.clone(),picker_url:runtime.picker_url.clone(),attachment_mutation_url:runtime.attachment_mutation_url.clone(),library_url:runtime.library_url.clone(),inline_create_url:runtime.inline_create_url.clone(),inline_preview_url:runtime.inline_preview_url.clone(),shortcode_url:runtime.shortcode_url.clone(),
        csrf_token:options.csrf_token.clone(),csrf_field_name:options.csrf_field_name.clone(),resolve_url:runtime.resolve_url.clone(),
        bad_request_message:Some(message(options,&options.bad_request_key,"error.bad_request_message")),internal_error_message:Some(message(options,&options.internal_error_key,"error.internal_message")),not_found_message:Some(message(options,&options.not_found_key,"error.not_found_message")),translate:options.translate_key.clone(),
        surface:Some(surface),default_role:Some(default_role),normalize_role:Some(Arc::new(|role|crate::attachments::normalize_media_attachment_role(role).into())),role_options:runtime.role_options(),
        build_picker_page:Some(build_picker),render_picker:Some(render_picker),current_inputs:Some(current),preview_items:Some(preview),build_manager:None,render_manager:None,
        load_shortcode_items:Some(shortcodes),total_pages:runtime.total_pages.clone(),render_modal:None,mounts_for_scope:runtime.mounts_for_scope.clone(),
        parse_input:None,validate_input:Some(validate),create_asset:Some(create),render_create:None,render_preview:None,
    };
    let mut factory=MediaAssetFlowFactory{base};let builder=factory.clone();
    factory.base.build_manager=Some(Arc::new(move|request,form,surface,items|builder.build_attachment_manager(request,form,surface,items)));
    factory.base.io.write_patch=Some(factory.base.patch_writer());factory
}

pub fn new_media_asset_management_controller<E:std::error::Error+Send+'static,R:Send+Sync+'static>(options:MediaAssetManagementOptions<E,R>)->MediaManagementController<MediaAssetFormInput,E,R>{
    let factory=new_media_asset_flow_factory(&options);let rt=options.runtime.clone();
    let picker=factory.clone();let attach=factory.clone();let mutate=factory.clone();let shortcode=factory.clone();let create=factory.clone();let preview=factory.clone();
    new_media_management_controller(MediaManagementControllerOptions{
        resource_kind:options.resource_kind.clone(),inline_surface:rt.resolved_inline_surface(),list_page_size:page_size(options.library_page_size,24),
        io:options.io.clone(),require_access:options.require_access.clone(),library_path:rt.library_url.clone(),request_language:options.request_language.clone(),mutation_route_path:options.mutation_route_path.clone(),
        bad_request_message:Some(message(&options,&options.bad_request_key,"error.bad_request_message")),internal_error_message:Some(message(&options,&options.internal_error_key,"error.internal_message")),not_found_message:Some(message(&options,&options.not_found_key,"error.not_found_message")),
        render_html:options.render_html,list_page:options.list_page,form_page:options.form_page,item_exists:Some(Arc::new(move|id|rt.asset_exists(id))),validation:options.validation,commands:options.commands,on_mutation_error:options.on_mutation_error,
        picker_search:Some(Arc::new(move|request|picker.base.picker_search(request))),picker_attach:Some(Arc::new(move|request|attach.base.picker_attach(request))),attachment_mutation:Some(Arc::new(move|request|mutate.base.attachment_mutation(request))),
        shortcode_modal:Some(Arc::new(move|request|shortcode.base.shortcode_modal(request))),inline_create:Some(Arc::new(move|request|create.inline_create(request))),inline_preview:Some(Arc::new(move|request|preview.inline_preview(request))),
    })
}

/// Native form/rendering bindings for constructing the complete management controller.
pub struct MediaAssetManagementControllerBindingOptions<C,E:Send+'static,R:Send+Sync+'static> {
    pub management:MediaAssetManagementOptions<E,R>,
    pub resources:Arc<dyn Registry+Send+Sync>,
    pub route_path:Option<RoutePathBuilder>,
    pub forms:crate::forms::FormPageSource<C,E,R>,
    pub context:Arc<C>,
    pub render_page:Option<MediaAssetListRenderPage>,
    pub render_form_page:Option<MediaFormRenderPage>,
    pub resolve_url:Option<MediaAssetURLResolver>,
    pub state:Option<Arc<dyn Fn(&Request)->crate::rendering::ResourceListPageState+Send+Sync>>,
    pub page_options:crate::rendering::DefinitionResourcePageBuilderOptions<E,R>,
    pub invalid_request_error:MediaInvalidRequest<E>,
}

pub fn new_media_asset_management_controller_from_bindings<C:Send+Sync+'static,E:std::error::Error+Send+'static,R:Send+Sync+'static>(
    mut options:MediaAssetManagementControllerBindingOptions<C,E,R>,
)->MediaManagementController<MediaAssetFormInput,E,R>{
    let definition=find_resource_definition(Some(options.resources.as_ref()),&options.management.resource_kind);
    let target=|delete:bool|->Option<MediaAssetDetailHref>{
        let path=options.route_path.clone()?;let definition=definition.as_ref()?;
        let route=if delete{definition.management_routes.delete.clone()}else if !definition.management_routes.edit.is_empty(){definition.management_routes.edit.clone()}else{definition.management_routes.list.clone()};
        let route=route.trim().to_owned();if route.is_empty(){return None;}
        Some(Arc::new(move|request,asset|{let id=asset.id.trim();if id.is_empty(){String::new()}else{path(request,&route,&[("id".into(),id.into())])}}))
    };
    options.management.list_page=Some(new_media_asset_list_page_builder(MediaAssetListPageBuilderOptions{
        store:options.management.runtime.assets.clone(),resources:options.resources.clone(),kind:options.management.resource_kind.clone(),manage:true,
        default_page_size:24,detail_href:target(false),delete_href:target(true),image_kind:"image".into(),resolve_url:options.resolve_url,
        state:options.state,translate_key:options.management.translate_key.clone(),page_options:options.page_options,render_page:options.render_page,
    }));
    options.management.form_page=Some(new_media_asset_form_page_builder(MediaAssetFormPageBuilderOptions{
        forms:options.forms,context:options.context,resources:options.resources,kind:options.management.resource_kind.clone(),runtime:options.management.runtime.clone(),
        route_path:options.route_path,translate_key:options.management.translate_key.clone(),render_page:options.render_form_page,
        invalid_request_error:options.invalid_request_error,form_policy:options.management.io.form_policy,
    }));
    new_media_asset_management_controller(options.management)
}

/// The complete media service's caller-supplied capabilities and native adapters.
pub struct MediaAssetServiceOptions<C,E:Send+'static,R:Send+Sync+'static>{
    pub bindings:MediaAssetManagementControllerBindingOptions<C,E,R>,
    pub runtime_bindings:crate::media::runtime::MediaAssetStoreRuntimeBindingOptions<E,R>,
    pub route_names:MediaManagementRouteNames,
    pub route_permissions:MediaManagementRoutePermissions,
    pub access:crate::routing::AccessControl<E,R>,
    pub commands:MediaAssetCommandBindingOptions<E,R>,
    pub validation:MediaAssetValidationBindingOptions<E>,
}
/// Required native owners retained by the service after composition.
pub struct MediaAssetServiceBindings<E:Send+'static,R:Send+Sync+'static>{
    runtime:MediaAssetStoreRuntime<E,R>,controller:MediaManagementController<MediaAssetFormInput,E,R>,
    routes:crate::routing::Service<E,R>,commands:MediaAssetCommandBindingOptions<E,R>,validation:MediaAssetValidationBindingOptions<E>,
}
impl<E:std::error::Error+Send+'static,R:Send+Sync+'static> MediaAssetServiceBindings<E,R>{
    pub fn runtime(&self)->MediaAssetStoreRuntime<E,R>{self.runtime.clone()}
    pub fn controller(&self)->MediaManagementController<MediaAssetFormInput,E,R>{self.controller.clone()}
    pub fn route_service(&self)->&crate::routing::Service<E,R>{&self.routes}
    pub fn register_routes<X>(&self,owner:&effect::Scope<X>,router:Option<effect_server::Router<E,R>>,mounter:Option<&crate::routing::ManagedRouteMounter<E,R>>)
        ->Effect<(crate::routing::RegisteredRoutes<E,R>,Option<effect::Fiber<(),X>>),X,R>
    where X:From<crate::routing::ManagedStartError<E>>+Send+Sync+'static,R:effect::HasClock+effect::HasConsole+effect::HasTaskSpawner {
        crate::cms! { register_routes; service: self.routes, scope: owner, router: router, mounter: mounter, }
    }
    pub fn bind_resource_commands(&self,bindings:&crate::commands::ResourceCommandBindings<E,R>){bind_media_asset_resource_commands(bindings,self.commands.clone());}
    pub fn bind_resource_validation(&self,bindings:&crate::validation::Bindings<crate::commands::CommandInput,E,R>){bind_media_asset_resource_validation(bindings,self.validation.clone());}
}
impl<E:std::error::Error+Send+'static,R:Send+Sync+'static> crate::media::runtime::MediaAssetRuntimeProvider<E,R> for MediaAssetServiceBindings<E,R>{fn runtime(&self)->MediaAssetStoreRuntime<E,R>{self.runtime.clone()}}
impl<E:std::error::Error+Send+'static,R:Send+Sync+'static> crate::media::runtime::MediaAssetControllerProvider<MediaManagementController<MediaAssetFormInput,E,R>> for MediaAssetServiceBindings<E,R>{fn controller(&self)->MediaManagementController<MediaAssetFormInput,E,R>{self.controller.clone()}}
impl<E:std::error::Error+Send+'static,R:Send+Sync+'static> crate::media::runtime::MediaAssetService<E,R> for MediaAssetServiceBindings<E,R>{
    fn bind_resource_commands(&self,bindings:&crate::commands::ResourceCommandBindings<E,R>){self.bind_resource_commands(bindings)}
    fn bind_resource_validation(&self,bindings:&crate::validation::Bindings<crate::commands::CommandInput,E,R>){self.bind_resource_validation(bindings)}
}
pub fn new_media_asset_service_bindings<C:Send+Sync+'static,E:std::error::Error+Send+'static,R:Send+Sync+'static>(
    mut options:MediaAssetServiceOptions<C,E,R>,
)->Result<MediaAssetServiceBindings<E,R>,crate::routing::ManagedRouteError>{
    let runtime=crate::media::runtime::resolve_media_asset_service_runtime(options.bindings.management.runtime.clone(),options.runtime_bindings);
    let kind=crate::media::runtime::resolve_media_asset_service_kind(&options.bindings.management.resource_kind);
    let names=if options.route_names==MediaManagementRouteNames::default(){media_management_route_names_from_registry(Some(options.bindings.resources.as_ref()),&kind)}else{options.route_names};
    options.bindings.management.runtime=runtime.clone();options.bindings.management.resource_kind=kind.clone();
    options.commands.runtime=runtime.clone();options.commands.kind=kind.clone();options.validation.kind=kind;
    let controller=new_media_asset_management_controller_from_bindings(options.bindings);
    let routes=media_management_route_service(&names,&controller.handlers(),&options.access,&options.route_permissions)?;
    Ok(MediaAssetServiceBindings{runtime,controller,routes,commands:options.commands,validation:options.validation})
}
/// Construct the complete media experience without owning its application host.
pub fn new_media_asset_service<C:Send+Sync+'static,E:std::error::Error+Send+'static,R:Send+Sync+'static>(options:MediaAssetServiceOptions<C,E,R>)->Result<MediaAssetServiceBindings<E,R>,crate::routing::ManagedRouteError>{new_media_asset_service_bindings(options)}
