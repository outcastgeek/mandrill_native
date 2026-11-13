//! Resource-specific renderer callbacks; all page and field DTOs remain canonical.
use std::{collections::BTreeMap,sync::Arc};
use effect_server::Request;
use maud::Markup;
use crate::{attachments::{MediaAttachmentManagerProps,MediaSurfaceMounts},forms::{LocalizedTabsProps,ResourceFormPageVM},definitions::{ResourceKind,ResourceSectionKey}};
pub type ResourceRenderContext<'a> = crate::media::presentation::MediaRenderContext<'a>;
pub type LocalizedTabsRenderer=Arc<dyn Fn(&LocalizedTabsProps)->Markup+Send+Sync>;
pub type MediaAttachmentManagerRenderer=Arc<dyn Fn(&MediaAttachmentManagerProps)->Markup+Send+Sync>;
#[derive(Clone,Default)]
pub struct ResourceFormRenderOptions {pub localized_tabs:Option<LocalizedTabsRenderer>,pub media_attachment_manager:Option<MediaAttachmentManagerRenderer>}
#[derive(Clone,Default)]
pub struct ResourcePageBreadcrumbProps {pub request:Option<Request>,pub dashboard_route:String,pub dashboard_label:String,pub current:String,pub class:String}
pub type ResourceBreadcrumbRenderer=Arc<dyn Fn(&ResourcePageBreadcrumbProps)->Markup+Send+Sync>;
pub type ResourceTextRenderer=Arc<dyn Fn(&str)->Markup+Send+Sync>;
pub type ResourceFormActionsRenderer=Arc<dyn Fn(&[ManagementFormActionProps],&str)->Markup+Send+Sync>;
pub type ResourceMediaRootsRenderer=Arc<dyn Fn(&MediaSurfaceMounts)->Markup+Send+Sync>;
pub type ResourceFormAttributesBuilder=Arc<dyn Fn(&ResourceFormPageVM)->BTreeMap<String,String>+Send+Sync>;
pub type ResourceSectionContainerID=Arc<dyn Fn(&ResourceKind,&ResourceSectionKey)->String+Send+Sync>;
#[derive(Clone,Default)]
pub struct ResourcePageShellOptions {
    pub inline_media_actions:bool,pub csrf_token:String,pub csrf_field_name:String,
    pub breadcrumb:Option<ResourceBreadcrumbRenderer>,pub status_alert:Option<ResourceTextRenderer>,pub live_submit_alerts:Option<ResourceTextRenderer>,
    pub form_submit_actions:Option<ResourceFormActionsRenderer>,pub media_modal_roots:Option<ResourceMediaRootsRenderer>,
    pub form_attributes:Option<ResourceFormAttributesBuilder>,pub form_render_options:ResourceFormRenderOptions,pub section_container_id:Option<ResourceSectionContainerID>,
}
#[derive(Clone,Debug)]
pub struct ManagementFormActionProps {pub name:String,pub value:String,pub label:String,pub icon:String,pub variant:&'static str}
impl Default for ManagementFormActionProps {fn default()->Self {Self{name:String::new(),value:String::new(),label:String::new(),icon:String::new(),variant:web_components::components::shared::VARIANT_DEFAULT}}}
#[derive(Clone,Debug,Default)]
pub struct SelectOptionDataSyncField {pub field_id:String,pub data_key:String,pub use_selected_value:bool}
#[derive(Clone,Debug,Default)]
pub struct ResourceFieldOptionRecord {pub value:String,pub label:String,pub attrs:BTreeMap<String,String>}
