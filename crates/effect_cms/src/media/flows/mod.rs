//! Media fragment flows over supplied consumer capabilities and native replies.

use std::sync::Arc;
use effect::{Effect, effect, effect_sync};
use effect_server::{ByteFormData, Request, Reply, ServerError};
use maud::Markup;
use crate::{commands::InputBytes, attachments::{MediaAttachmentInput,MediaAttachmentFormItem,
    MediaAttachmentManagerProps,MediaAttachmentRoleOptions,MediaFormPolicy,RoleNormalizer}};
use crate::media::{surface::{MediaQuery,MediaSurfaceRequest,MountsResolver},
    presentation::{MediaPickerPageVM,ShortcodeToolsProps}};

mod handlers;
mod factory;
pub use handlers::*;
pub use factory::*;

pub type MediaFlowErrorRenderer<E,R> = Arc<dyn Fn(&Request,u16,&str)->Effect<Reply<E,R>,E,R> + Send + Sync>;
pub type MediaFlowPatchWriter<E,R> = Arc<dyn Fn(&Request,InputBytes,String,Markup)->Effect<Reply<E,R>,E,R> + Send + Sync>;
pub type MediaFlowPatchesWriter<E,R> = Arc<dyn Fn(&Request,Vec<crate::media::surface::MediaPatch>)->Effect<Reply<E,R>,E,R> + Send + Sync>;
pub type MediaFlowHTMLWriter<E,R> = Arc<dyn Fn(&Request,Markup)->Effect<Reply<E,R>,E,R> + Send + Sync>;
pub type MediaFlowHTMLStatusWriter<E,R> = Arc<dyn Fn(&Request,u16,Markup)->Effect<Reply<E,R>,E,R> + Send + Sync>;
pub type MediaSurfaceResolver = Arc<dyn Fn(&Request,&ByteFormData)->MediaSurfaceRequest + Send + Sync>;
pub type MediaDefaultRoleResolver = Arc<dyn Fn(&Request,&MediaSurfaceRequest)->InputBytes + Send + Sync>;
pub type MediaPickerPageBuilder<E,R> = Arc<dyn Fn(&Request,MediaQuery,String,Arc<ByteFormData>)->Effect<MediaPickerPageVM,E,R> + Send + Sync>;
pub type MediaPickerPageRenderer = Arc<dyn Fn(MediaPickerPageVM)->Markup + Send + Sync>;
pub type MediaAttachmentInputsLoader<E,R> = Arc<dyn Fn(&Request,MediaSurfaceRequest,Arc<ByteFormData>)->Effect<Vec<MediaAttachmentInput>,E,R> + Send + Sync>;
pub type MediaAttachmentItemsPreviewer<E,R> = Arc<dyn Fn(&Request,Vec<MediaAttachmentInput>)->Effect<Vec<MediaAttachmentFormItem>,E,R> + Send + Sync>;
pub type MediaAttachmentManagerBuilder = Arc<dyn Fn(&Request,&ByteFormData,MediaSurfaceRequest,Vec<MediaAttachmentFormItem>)->Option<MediaAttachmentManagerProps> + Send + Sync>;
pub type MediaAttachmentManagerRenderer = Arc<dyn Fn(MediaAttachmentManagerProps)->Markup + Send + Sync>;
pub type MediaShortcodeItemsLoader<E,R> = Arc<dyn Fn(&Request,MediaQuery)->Effect<(Vec<MediaAttachmentFormItem>,i64),E,R> + Send + Sync>;
pub type MediaShortcodeModalRenderer = Arc<dyn Fn(ShortcodeToolsProps)->Markup + Send + Sync>;
pub type MediaInputParser<I> = Arc<dyn Fn(&Request,&ByteFormData)->I + Send + Sync>;
pub type MediaInputValidator<I> = Arc<dyn Fn(&Request,&I)->String + Send + Sync>;
pub type MediaAssetCreator<I,E,R> = Arc<dyn Fn(&Request,I)->Effect<InputBytes,E,R> + Send + Sync>;
pub type MediaCreateRenderer<I> = Arc<dyn Fn(&Request,&ByteFormData,I,String,Vec<MediaAttachmentFormItem>)->MediaInlineCreateRender + Send + Sync>;
pub type MediaPreviewRenderer<I> = Arc<dyn Fn(&Request,&ByteFormData,I)->MediaInlineCreateRender + Send + Sync>;

#[derive(Clone)]
pub struct MediaInlineCreateRender { pub root_id: InputBytes, pub component: Markup }
/// Shared transport capabilities; parsing retains the caller's explicit native bounds.
pub struct MediaFlowIO<E:Send+'static,R:Send+Sync+'static> {
    pub form: Option<Arc<ByteFormData>>,
    pub form_policy: MediaFormPolicy,
    pub on_form_error: Arc<dyn Fn(ServerError)->E + Send + Sync>,
    pub write_error: Option<MediaFlowErrorRenderer<E,R>>,
    pub write_patch: Option<MediaFlowPatchWriter<E,R>>,
    pub write_patches: Option<MediaFlowPatchesWriter<E,R>>,
    pub write_html_status: Option<MediaFlowHTMLStatusWriter<E,R>>,
    pub write_html: Option<MediaFlowHTMLWriter<E,R>>,
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for MediaFlowIO<E,R> {
    fn clone(&self)->Self { Self { form:self.form.clone(),form_policy:self.form_policy,
        on_form_error:self.on_form_error.clone(),write_error:self.write_error.clone(),
        write_patch:self.write_patch.clone(),write_patches:self.write_patches.clone(),
        write_html_status:self.write_html_status.clone(),write_html:self.write_html.clone() } }
}

pub struct MediaPickerSearchFlowOptions<E:Send+'static,R:Send+Sync+'static> {
    pub io: MediaFlowIO<E,R>,
    pub bad_request_message: String,
    pub internal_error_message: String,
    pub page_size: i64,
    pub redirect_path: String,
    pub build_page: Option<MediaPickerPageBuilder<E,R>>,
    pub render_picker: Option<MediaPickerPageRenderer>,
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for MediaPickerSearchFlowOptions<E,R> {
    fn clone(&self)->Self { Self {
        io:self.io.clone(),
        bad_request_message:self.bad_request_message.clone(),
        internal_error_message:self.internal_error_message.clone(),
        page_size:self.page_size.clone(),
        redirect_path:self.redirect_path.clone(),
        build_page:self.build_page.clone(),
        render_picker:self.render_picker.clone(),
    } }
}

pub struct MediaPickerAttachFlowOptions<E:Send+'static,R:Send+Sync+'static> {
    pub io: MediaFlowIO<E,R>,
    pub bad_request_message: String,
    pub internal_error_message: String,
    pub surface: Option<MediaSurfaceResolver>,
    pub normalize_role: Option<RoleNormalizer>,
    pub role_options: MediaAttachmentRoleOptions,
    pub current_inputs: Option<MediaAttachmentInputsLoader<E,R>>,
    pub preview_items: Option<MediaAttachmentItemsPreviewer<E,R>>,
    pub build_manager: Option<MediaAttachmentManagerBuilder>,
    pub render_manager: Option<MediaAttachmentManagerRenderer>,
    pub page_size: i64,
    pub no_selection_message: String,
    pub default_role: Option<MediaDefaultRoleResolver>,
    pub build_page: Option<MediaPickerPageBuilder<E,R>>,
    pub render_picker: Option<MediaPickerPageRenderer>,
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for MediaPickerAttachFlowOptions<E,R> {
    fn clone(&self)->Self { Self {
        io:self.io.clone(),
        bad_request_message:self.bad_request_message.clone(),
        internal_error_message:self.internal_error_message.clone(),
        surface:self.surface.clone(),
        normalize_role:self.normalize_role.clone(),
        role_options:self.role_options.clone(),
        current_inputs:self.current_inputs.clone(),
        preview_items:self.preview_items.clone(),
        build_manager:self.build_manager.clone(),
        render_manager:self.render_manager.clone(),
        page_size:self.page_size.clone(),
        no_selection_message:self.no_selection_message.clone(),
        default_role:self.default_role.clone(),
        build_page:self.build_page.clone(),
        render_picker:self.render_picker.clone(),
    } }
}

pub struct MediaAttachmentMutationFlowOptions<E:Send+'static,R:Send+Sync+'static> {
    pub io: MediaFlowIO<E,R>,
    pub bad_request_message: String,
    pub internal_error_message: String,
    pub surface: Option<MediaSurfaceResolver>,
    pub normalize_role: Option<RoleNormalizer>,
    pub role_options: MediaAttachmentRoleOptions,
    pub current_inputs: Option<MediaAttachmentInputsLoader<E,R>>,
    pub preview_items: Option<MediaAttachmentItemsPreviewer<E,R>>,
    pub build_manager: Option<MediaAttachmentManagerBuilder>,
    pub render_manager: Option<MediaAttachmentManagerRenderer>,
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for MediaAttachmentMutationFlowOptions<E,R> {
    fn clone(&self)->Self { Self {
        io:self.io.clone(),
        bad_request_message:self.bad_request_message.clone(),
        internal_error_message:self.internal_error_message.clone(),
        surface:self.surface.clone(),
        normalize_role:self.normalize_role.clone(),
        role_options:self.role_options.clone(),
        current_inputs:self.current_inputs.clone(),
        preview_items:self.preview_items.clone(),
        build_manager:self.build_manager.clone(),
        render_manager:self.render_manager.clone(),
    } }
}

pub struct MediaInlineCreateFlowOptions<I,E:Send+'static,R:Send+Sync+'static> {
    pub io: MediaFlowIO<E,R>,
    pub bad_request_message: String,
    pub internal_error_message: String,
    pub surface: Option<MediaSurfaceResolver>,
    pub normalize_role: Option<RoleNormalizer>,
    pub role_options: MediaAttachmentRoleOptions,
    pub current_inputs: Option<MediaAttachmentInputsLoader<E,R>>,
    pub preview_items: Option<MediaAttachmentItemsPreviewer<E,R>>,
    pub build_manager: Option<MediaAttachmentManagerBuilder>,
    pub render_manager: Option<MediaAttachmentManagerRenderer>,
    pub default_role: Option<MediaDefaultRoleResolver>,
    pub parse_input: Option<MediaInputParser<I>>,
    pub validate_input: Option<MediaInputValidator<I>>,
    pub create_asset: Option<MediaAssetCreator<I,E,R>>,
    pub render_create: Option<MediaCreateRenderer<I>>,
}
impl<I,E:Send+'static,R:Send+Sync+'static> Clone for MediaInlineCreateFlowOptions<I,E,R> {
    fn clone(&self)->Self { Self {
        io:self.io.clone(),
        bad_request_message:self.bad_request_message.clone(),
        internal_error_message:self.internal_error_message.clone(),
        surface:self.surface.clone(),
        normalize_role:self.normalize_role.clone(),
        role_options:self.role_options.clone(),
        current_inputs:self.current_inputs.clone(),
        preview_items:self.preview_items.clone(),
        build_manager:self.build_manager.clone(),
        render_manager:self.render_manager.clone(),
        default_role:self.default_role.clone(),
        parse_input:self.parse_input.clone(),
        validate_input:self.validate_input.clone(),
        create_asset:self.create_asset.clone(),
        render_create:self.render_create.clone(),
    } }
}

pub struct MediaInlinePreviewFlowOptions<I,E:Send+'static,R:Send+Sync+'static> {
    pub io: MediaFlowIO<E,R>,
    pub bad_request_message: String,
    pub internal_error_message: String,
    pub parse_input: Option<MediaInputParser<I>>,
    pub render_preview: Option<MediaPreviewRenderer<I>>,
}
impl<I,E:Send+'static,R:Send+Sync+'static> Clone for MediaInlinePreviewFlowOptions<I,E,R> {
    fn clone(&self)->Self { Self {
        io:self.io.clone(),
        bad_request_message:self.bad_request_message.clone(),
        internal_error_message:self.internal_error_message.clone(),
        parse_input:self.parse_input.clone(),
        render_preview:self.render_preview.clone(),
    } }
}

pub struct MediaShortcodeModalFlowOptions<E:Send+'static,R:Send+Sync+'static> {
    pub io: MediaFlowIO<E,R>,
    pub bad_request_message: String,
    pub internal_error_message: String,
    pub page_size: i64,
    pub default_page_size: i64,
    pub default_root_id: String,
    pub redirect_path: String,
    pub library_url: String,
    pub surface: Option<MediaSurfaceResolver>,
    pub mounts_for_scope: Option<MountsResolver>,
    pub load_items: Option<MediaShortcodeItemsLoader<E,R>>,
    pub total_pages: Option<Arc<dyn Fn(i64,i64)->i64 + Send + Sync>>,
    pub render_modal: Option<MediaShortcodeModalRenderer>,
}
impl<E:Send+'static,R:Send+Sync+'static> Clone for MediaShortcodeModalFlowOptions<E,R> {
    fn clone(&self)->Self { Self {
        io:self.io.clone(),
        bad_request_message:self.bad_request_message.clone(),
        internal_error_message:self.internal_error_message.clone(),
        page_size:self.page_size.clone(),
        default_page_size:self.default_page_size.clone(),
        default_root_id:self.default_root_id.clone(),
        redirect_path:self.redirect_path.clone(),
        library_url:self.library_url.clone(),
        surface:self.surface.clone(),
        mounts_for_scope:self.mounts_for_scope.clone(),
        load_items:self.load_items.clone(),
        total_pages:self.total_pages.clone(),
        render_modal:self.render_modal.clone(),
    } }
}
