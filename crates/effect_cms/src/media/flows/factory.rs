//! Source media flow factories, sharing consumer capabilities and request-local policy.
use super::*;
use crate::media::runtime::{MediaRequestURLResolver,MediaRequestAssetURLResolver};

pub struct MediaFlowFactory<I,E:Send+'static,R:Send+Sync+'static> {
    pub io:MediaFlowIO<E,R>,
    pub library_page_size:i64,
    pub picker_page_size:i64,
    pub shortcode_page_size:i64,
    pub default_fragment_root_id:String,
    pub default_picker_root_id:String,
    pub default_create_root_id:String,
    pub default_shortcode_root_id:String,
    pub inline_surface:String,
    pub media_new_url:Option<MediaRequestURLResolver>,
    pub picker_url:Option<MediaRequestURLResolver>,
    pub attachment_mutation_url:Option<MediaRequestURLResolver>,
    pub library_url:Option<MediaRequestURLResolver>,
    pub inline_create_url:Option<MediaRequestURLResolver>,
    pub inline_preview_url:Option<MediaRequestURLResolver>,
    pub shortcode_url:Option<MediaRequestURLResolver>,
    pub csrf_token:Option<MediaRequestURLResolver>,
    pub csrf_field_name:Option<MediaRequestURLResolver>,
    pub resolve_url:Option<MediaRequestAssetURLResolver>,
    pub bad_request_message:Option<MediaRequestURLResolver>,
    pub internal_error_message:Option<MediaRequestURLResolver>,
    pub not_found_message:Option<MediaRequestURLResolver>,
    pub translate:Option<crate::localization::TranslateKeyFunc<Request>>,
    pub surface:Option<MediaSurfaceResolver>,
    pub default_role:Option<MediaDefaultRoleResolver>,
    pub normalize_role:Option<RoleNormalizer>,
    pub role_options:MediaAttachmentRoleOptions,
    pub build_picker_page:Option<MediaPickerPageBuilder<E,R>>,
    pub render_picker:Option<MediaPickerPageRenderer>,
    pub current_inputs:Option<MediaAttachmentInputsLoader<E,R>>,
    pub preview_items:Option<MediaAttachmentItemsPreviewer<E,R>>,
    pub build_manager:Option<MediaAttachmentManagerBuilder>,
    pub render_manager:Option<MediaAttachmentManagerRenderer>,
    pub load_shortcode_items:Option<MediaShortcodeItemsLoader<E,R>>,
    pub total_pages:Option<Arc<dyn Fn(i64,i64)->i64 + Send + Sync>>,
    pub render_modal:Option<MediaShortcodeModalRenderer>,
    pub mounts_for_scope:Option<MountsResolver>,
    pub parse_input:Option<MediaInputParser<I>>,
    pub validate_input:Option<MediaInputValidator<I>>,
    pub create_asset:Option<MediaAssetCreator<I,E,R>>,
    pub render_create:Option<MediaCreateRenderer<I>>,
    pub render_preview:Option<MediaPreviewRenderer<I>>,
}
impl<I,E:Send+'static,R:Send+Sync+'static> Clone for MediaFlowFactory<I,E,R> { fn clone(&self)->Self { Self {
io:self.io.clone(),
library_page_size:self.library_page_size.clone(),
picker_page_size:self.picker_page_size.clone(),
shortcode_page_size:self.shortcode_page_size.clone(),
default_fragment_root_id:self.default_fragment_root_id.clone(),
default_picker_root_id:self.default_picker_root_id.clone(),
default_create_root_id:self.default_create_root_id.clone(),
default_shortcode_root_id:self.default_shortcode_root_id.clone(),
inline_surface:self.inline_surface.clone(),
media_new_url:self.media_new_url.clone(),
picker_url:self.picker_url.clone(),
attachment_mutation_url:self.attachment_mutation_url.clone(),
library_url:self.library_url.clone(),
inline_create_url:self.inline_create_url.clone(),
inline_preview_url:self.inline_preview_url.clone(),
shortcode_url:self.shortcode_url.clone(),
csrf_token:self.csrf_token.clone(),
csrf_field_name:self.csrf_field_name.clone(),
resolve_url:self.resolve_url.clone(),
bad_request_message:self.bad_request_message.clone(),
internal_error_message:self.internal_error_message.clone(),
not_found_message:self.not_found_message.clone(),
translate:self.translate.clone(),
surface:self.surface.clone(),
default_role:self.default_role.clone(),
normalize_role:self.normalize_role.clone(),
role_options:self.role_options.clone(),
build_picker_page:self.build_picker_page.clone(),
render_picker:self.render_picker.clone(),
current_inputs:self.current_inputs.clone(),
preview_items:self.preview_items.clone(),
build_manager:self.build_manager.clone(),
render_manager:self.render_manager.clone(),
load_shortcode_items:self.load_shortcode_items.clone(),
total_pages:self.total_pages.clone(),
render_modal:self.render_modal.clone(),
mounts_for_scope:self.mounts_for_scope.clone(),
parse_input:self.parse_input.clone(),
validate_input:self.validate_input.clone(),
create_asset:self.create_asset.clone(),
render_create:self.render_create.clone(),
render_preview:self.render_preview.clone(),
} } }

pub fn static_media_request_value(value:&str)->MediaRequestURLResolver {
    let value=value.trim().to_owned();Arc::new(move |_|value.clone())
}
pub fn translated_media_message(key:&str,translate:Option<crate::localization::TranslateKeyFunc<Request>>)->MediaRequestURLResolver {
    let key=key.trim().to_owned();Arc::new(move |request|translate.as_ref().map(|translate|translate(request,&key)).unwrap_or_else(||key.clone()))
}
impl<I:Send+'static,E:Send+'static,R:Send+Sync+'static> MediaFlowFactory<I,E,R> {
    fn message(&self,request:&Request,resolve:Option<&MediaRequestURLResolver>)->String {
        resolve.map(|resolve|resolve(request).trim().to_owned()).unwrap_or_default()
    }
    pub fn patch_writer(&self)->MediaFlowPatchWriter<E,R> {
        let root=self.default_fragment_root_id.clone();
        Arc::new(move |_request,id,mode,component|{
            let root=root.clone();let mode=mode.clone();let component=component.clone();let id=id.clone();
            effect_sync! { body: move || crate::media::surface::write_media_patch(id.as_bytes(),&mode,component.clone(),&root) }
        })
    }
    pub fn picker_search(&self,request:&Request)->MediaPickerSearchFlowOptions<E,R> {
        MediaPickerSearchFlowOptions { io:self.io.clone(),page_size:self.picker_page_size,
            redirect_path:self.message(request,self.library_url.as_ref()),
            bad_request_message:self.message(request,self.bad_request_message.as_ref()),
            internal_error_message:self.message(request,self.internal_error_message.as_ref()),
            build_page:self.build_picker_page.clone(),render_picker:self.render_picker.clone() }
    }
    pub fn picker_attach(&self,request:&Request)->MediaPickerAttachFlowOptions<E,R> {
        MediaPickerAttachFlowOptions { io:self.io.clone(),page_size:self.picker_page_size,
            bad_request_message:self.message(request,self.bad_request_message.as_ref()),
            internal_error_message:self.message(request,self.internal_error_message.as_ref()),
            no_selection_message:self.translate.as_ref().map(|translate|translate(request,"media.picker.attach_existing_hint")).unwrap_or_else(||"media.picker.attach_existing_hint".into()),
            surface:self.surface.clone(),default_role:self.default_role.clone(),normalize_role:self.normalize_role.clone(),role_options:self.role_options.clone(),
            current_inputs:self.current_inputs.clone(),preview_items:self.preview_items.clone(),build_manager:self.build_manager.clone(),render_manager:Some(self.manager_renderer(request)),
            build_page:self.build_picker_page.clone(),render_picker:self.render_picker.clone() }
    }
    pub fn attachment_mutation(&self,request:&Request)->MediaAttachmentMutationFlowOptions<E,R> {
        MediaAttachmentMutationFlowOptions { io:self.io.clone(),
            bad_request_message:self.message(request,self.bad_request_message.as_ref()),
            internal_error_message:self.message(request,self.internal_error_message.as_ref()),
            surface:self.surface.clone(),normalize_role:self.normalize_role.clone(),role_options:self.role_options.clone(),
            current_inputs:self.current_inputs.clone(),preview_items:self.preview_items.clone(),build_manager:self.build_manager.clone(),render_manager:Some(self.manager_renderer(request)) }
    }
    pub fn shortcode_modal(&self,request:&Request)->MediaShortcodeModalFlowOptions<E,R> {
        MediaShortcodeModalFlowOptions { io:self.io.clone(),page_size:self.shortcode_page_size,default_page_size:self.library_page_size,
            default_root_id:self.default_shortcode_root_id.clone(),redirect_path:self.message(request,self.library_url.as_ref()),
            library_url:self.message(request,self.shortcode_url.as_ref()),
            bad_request_message:self.message(request,self.bad_request_message.as_ref()),internal_error_message:self.message(request,self.internal_error_message.as_ref()),
            surface:self.surface.clone(),mounts_for_scope:self.mounts_for_scope.clone(),load_items:self.load_shortcode_items.clone(),total_pages:self.total_pages.clone(),render_modal:Some(self.modal_renderer(request)) }
    }
    pub fn inline_create(&self,request:&Request)->MediaInlineCreateFlowOptions<I,E,R> {
        MediaInlineCreateFlowOptions { io:self.io.clone(),
            bad_request_message:self.message(request,self.bad_request_message.as_ref()),internal_error_message:self.message(request,self.internal_error_message.as_ref()),
            surface:self.surface.clone(),default_role:self.default_role.clone(),normalize_role:self.normalize_role.clone(),role_options:self.role_options.clone(),
            parse_input:self.parse_input.clone(),validate_input:self.validate_input.clone(),create_asset:self.create_asset.clone(),render_create:self.render_create.clone(),
            current_inputs:self.current_inputs.clone(),preview_items:self.preview_items.clone(),build_manager:self.build_manager.clone(),render_manager:Some(self.manager_renderer(request)) }
    }
    pub fn inline_preview(&self,request:&Request)->MediaInlinePreviewFlowOptions<I,E,R> {
        MediaInlinePreviewFlowOptions { io:self.io.clone(),bad_request_message:self.message(request,self.bad_request_message.as_ref()),
            internal_error_message:String::new(),parse_input:self.parse_input.clone(),render_preview:self.render_preview.clone() }
    }
}
impl<I:Send+'static,E:Send+'static,R:Send+Sync+'static> MediaFlowFactory<I,E,R> {
    fn manager_renderer(&self,request:&Request)->MediaAttachmentManagerRenderer {
        if let Some(render)=self.render_manager.clone(){return render;}
        let request=request.clone();let translate=self.translate.clone();
        Arc::new(move |props|crate::media::presentation::media_attachment_manager(
            crate::media::presentation::MediaRenderContext{request:Some(&request),translate:translate.as_ref(),translate_args:None},&props))
    }
    fn modal_renderer(&self,request:&Request)->MediaShortcodeModalRenderer {
        if let Some(render)=self.render_modal.clone(){return render;}
        let request=request.clone();let translate=self.translate.clone();
        Arc::new(move |props|crate::media::presentation::media_shortcode_modal(
            crate::media::presentation::MediaRenderContext{request:Some(&request),translate:translate.as_ref(),translate_args:None},&props))
    }
}
/// The concrete asset-form specialization of the generic media factory.
pub struct MediaAssetFlowFactory<E:Send+'static,R:Send+Sync+'static> { pub base:MediaFlowFactory<crate::media::assets::MediaAssetFormInput,E,R> }
impl<E:Send+'static,R:Send+Sync+'static> Clone for MediaAssetFlowFactory<E,R> {fn clone(&self)->Self{Self{base:self.base.clone()}}}
impl<E:Send+'static,R:Send+Sync+'static> MediaAssetFlowFactory<E,R> {
    pub fn inline_create(&self,request:&Request)->MediaInlineCreateFlowOptions<crate::media::assets::MediaAssetFormInput,E,R> {
        let mut base=self.base.clone();let factory=self.clone();
        base.parse_input=Some(Arc::new(|_request,form|crate::media::assets::media_asset_form_input_from_form(Some(form))));
        base.render_create=Some(Arc::new(move |request,form,input,message,items|factory.render_inline_create(request,form,input,message,items)));
        base.inline_create(request)
    }
    pub fn inline_preview(&self,request:&Request)->MediaInlinePreviewFlowOptions<crate::media::assets::MediaAssetFormInput,E,R> {
        let mut base=self.base.clone();let factory=self.clone();
        base.parse_input=Some(Arc::new(|_request,form|crate::media::assets::media_asset_form_input_from_form(Some(form))));
        base.render_preview=Some(Arc::new(move |request,form,input|factory.render_inline_preview(request,form,input)));
        base.inline_preview(request)
    }
    pub fn build_attachment_manager(&self,request:&Request,form:&ByteFormData,surface:MediaSurfaceRequest,items:Vec<MediaAttachmentFormItem>)->Option<MediaAttachmentManagerProps> {
        Some(crate::media::surface::build_media_attachment_manager_from_request(request,Some(form),&surface.scope,&surface.owner_content_id,items,self.base.mounts_for_scope.as_ref(),
            &self.base.message(request,self.base.picker_url.as_ref()),&self.base.message(request,self.base.media_new_url.as_ref()),
            &self.base.message(request,self.base.attachment_mutation_url.as_ref()),&self.base.inline_surface))
    }
    fn asset_resolver(&self,request:&Request)->crate::media::assets::MediaAssetURLResolver {
        let resolve=self.base.resolve_url.clone();let request=request.clone();
        Arc::new(move |raw|resolve.as_ref().map(|resolve|resolve(&request,raw).trim()).unwrap_or_else(||InputBytes::from(raw).trim()))
    }
    fn render_inline_create(&self,request:&Request,form:&ByteFormData,input:crate::media::assets::MediaAssetFormInput,message:String,items:Vec<MediaAttachmentFormItem>)->MediaInlineCreateRender {
        let surface=resolve_media_flow_surface(request,form,self.base.surface.as_ref());
        let default=self.base.default_role.as_ref().map(|resolve|resolve(request,&surface)).unwrap_or_default();
        let props=crate::media::presentation::build_media_inline_create_props(&crate::media::presentation::MediaInlineCreatePropsOptions {
            request:Some(request.clone()),form:Some(Arc::new(ByteFormData{values:form.values.clone()})),input,surface,error_msg:message,attachments:items,
            default_root_id:self.base.default_create_root_id.clone(),default_role:crate::byte_text::encoding_text(default.as_bytes()),
            action_url:self.base.message(request,self.base.inline_create_url.as_ref()),preview_url:self.base.message(request,self.base.inline_preview_url.as_ref()),
            csrf_token:self.base.message(request,self.base.csrf_token.as_ref()),csrf_field_name:self.base.message(request,self.base.csrf_field_name.as_ref()),resolve_url:Some(self.asset_resolver(request)),
        });
        MediaInlineCreateRender{root_id:props.root_id.clone().into(),component:crate::media::presentation::media_inline_create_drawer(crate::media::presentation::MediaRenderContext{request:Some(request),translate:self.base.translate.as_ref(),translate_args:None},&props)}
    }
    fn render_inline_preview(&self,request:&Request,form:&ByteFormData,input:crate::media::assets::MediaAssetFormInput)->MediaInlineCreateRender {
        let props=crate::media::presentation::build_media_inline_create_preview_props(&crate::media::presentation::MediaInlineCreatePreviewOptions {
            request:Some(request.clone()),form:Some(Arc::new(ByteFormData{values:form.values.clone()})),input,root_id:String::new(),resolve_url:Some(self.asset_resolver(request)),
        });
        MediaInlineCreateRender{root_id:props.root_id.clone().into(),component:crate::media::presentation::media_inline_create_preview(crate::media::presentation::MediaRenderContext{request:Some(request),translate:self.base.translate.as_ref(),translate_args:None},&props)}
    }
}
/// Native multi-patch writer; delivery errors remain with the reply transport.
pub fn write_media_datastar_patches<E:Send+'static,R:Send+Sync+'static>(patches:&[crate::media::surface::MediaPatch])->Reply<E,R> {crate::media::surface::write_media_patches(patches)}
