//! Media presentation contracts shared by the media views and their renderers.
//!
//! The values keep the source's field meaning and defaults. Rendering callbacks are
//! lazy [`Markup`] factories because this workspace has no `templ.Component` type;
//! callers supply the actual components through the canonical component library.

use std::sync::Arc;

use maud::Markup;

use crate::attachments::MediaAttachmentFormItem;
use crate::media::assets::MediaAssetRecord;
use crate::media::contentmedia::{ContentMediaItem, ContentVideo};
use crate::media::surface::{MediaQuery, MediaSurfaceRequest};
use effect_server::Request;

/// One lazy markup component in the media presentation surface.
pub type MediaComponent = Arc<dyn Fn() -> Markup + Send + Sync + 'static>;
/// Builds one picker item card.
pub type MediaItemCardRenderer<'a> = Arc<dyn Fn(MediaPickerItemVM) -> Markup + Send + Sync + 'a>;
/// Builds one picker pagination component.
pub type MediaPaginationRenderer<'a> =
    Arc<dyn Fn(MediaPickerPaginationProps) -> Markup + Send + Sync + 'a>;
/// Builds the attachment payload input fields.
pub type MediaAttachmentPayloadInputs =
    Arc<dyn Fn(&[MediaAttachmentFormItem]) -> Markup + Send + Sync + 'static>;

/// One rendered picker item.
#[derive(Clone, Debug, Default)]
pub struct MediaPickerItemVM {
    /// Asset identity.
    pub id: String,
    /// Media kind.
    pub kind: String,
    /// Media status.
    pub status: String,
    /// Display title.
    pub title: String,
    /// Resolved preview image URL.
    pub resolved_preview: String,
    /// Meta text.
    pub meta: String,
    /// Card class.
    pub card_class: String,
    /// Selected attribute value.
    pub selected_attr: String,
    /// Selection input type.
    pub selection_type: String,
    /// Selection input name.
    pub selection_name: String,
    /// Whether the item is checked.
    pub checked: bool,
    /// Whether the item is disabled.
    pub disabled: bool,
}

/// One picker asset record.
#[derive(Clone, Debug, Default)]
pub struct MediaPickerAssetRecord {
    /// Asset identity.
    pub id: String,
    /// Media kind.
    pub kind: String,
    /// Media status.
    pub status: String,
    /// Display title.
    pub title: String,
    /// Resolved preview image URL.
    pub resolved_preview: String,
    /// Provider name.
    pub provider: String,
}

/// One picker filter pill.
#[derive(Clone, Debug, Default)]
pub struct MediaPickerFilterPillVM {
    /// Filter value.
    pub value: String,
    /// Localized label.
    pub label: String,
    /// CSS class.
    pub class: String,
}

/// The rendered picker page.
#[derive(Clone, Default)]
pub struct MediaPickerPageVM {
    /// Request owner.
    pub request: Option<Request>,
    /// Active picker root identity.
    pub root_id: String,
    /// Create root identity.
    pub create_root_id: String,
    /// Surface scope.
    pub scope: String,
    /// Default attachment role.
    pub role_default: String,
    /// Target section identity.
    pub target_id: String,
    /// Owner form identity.
    pub owner_form_id: String,
    /// Owner content identity.
    pub owner_content_id: String,
    /// Picker attach URL.
    pub attach_url: String,
    /// Error message.
    pub error_msg: String,
    /// Search value.
    pub search: String,
    /// Kind value.
    pub kind: String,
    /// Status value.
    pub status: String,
    /// Sort value.
    pub sort: String,
    /// Create URL.
    pub create_url: String,
    /// Context label.
    pub context_label: String,
    /// Datastar signals.
    pub signals: String,
    /// Filter action URL.
    pub filter_action: String,
    /// Filter form identity.
    pub filter_form_id: String,
    /// Attach form identity.
    pub attach_form_id: String,
    /// Filter submission action.
    pub filter_submit_action: String,
    /// Filter pills.
    pub filter_pills: Vec<MediaPickerFilterPillVM>,
    /// Picker items.
    pub items: Vec<MediaPickerItemVM>,
    /// Current attachments.
    pub current_attachments: Vec<MediaAttachmentFormItem>,
    /// Current page.
    pub page: i64,
    /// Effective page size.
    pub page_size: i64,
    /// Total rows.
    pub total: i64,
    /// Total pages.
    pub pages: i64,
}

impl MediaPickerPageVM {
    /// Whether the picker page has no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The picker render options.
#[derive(Clone, Default)]
pub struct MediaPickerRenderOptions<'a> {
    /// CSRF token.
    pub csrf_token: String,
    /// CSRF field name.
    pub csrf_field_name: String,
    /// Attachment payload input component.
    pub attachment_payload_inputs: Option<MediaAttachmentPayloadInputs>,
    /// Item card component.
    pub item_card: Option<MediaItemCardRenderer<'a>>,
    /// Pagination component.
    pub pagination: Option<MediaPaginationRenderer<'a>>,
}

/// The picker pagination props.
#[derive(Clone, Debug, Default)]
pub struct MediaPickerPaginationProps {
    /// Form identity.
    pub form_id: String,
    /// Current page.
    pub page: i64,
    /// Page size.
    pub page_size: i64,
    /// Total rows.
    pub total: i64,
    /// Total pages.
    pub pages: i64,
    /// Item count on the page.
    pub item_count: i64,
    /// Localized previous label.
    pub previous_label: String,
    /// Localized next label.
    pub next_label: String,
    /// CSS class.
    pub class: String,
}

/// The picker page options.
#[derive(Clone, Default)]
pub struct MediaPickerPageOptions {
    /// Request.
    pub request: Option<Request>,
    /// Decoded surface.
    pub surface: MediaSurfaceRequest,
    /// Admitted query.
    pub query: MediaQuery,
    /// Error message.
    pub error_msg: String,
    /// Total rows.
    pub total: i64,
    /// Default picker root.
    pub default_root_id: String,
    /// Default create root.
    pub default_create_root_id: String,
    /// Default role.
    pub default_role: String,
    /// Default page size.
    pub default_page_size: i64,
    /// Attach URL.
    pub attach_url: String,
    /// Create URL.
    pub create_url: String,
    /// Filter action.
    pub filter_action: String,
    /// Filter submit action.
    pub filter_submit_action: String,
    /// Context label.
    pub context_label: String,
    /// Datastar signals.
    pub signals: String,
    /// Current attachments.
    pub current_attachments: Vec<MediaAttachmentFormItem>,
    /// Items.
    pub items: Vec<MediaPickerItemVM>,
    /// Total-page resolver.
    pub total_pages: Option<Arc<dyn Fn(i64, i64) -> i64 + Send + Sync + 'static>>,
    /// Request-scoped translator for the page's filter labels.
    pub translate_key: Option<crate::localization::TranslateKeyFunc<Request>>,

}

/// The picker page-from-state options.
#[derive(Clone, Default)]
pub struct MediaPickerPageStateOptions {
    /// Request.
    pub request: Option<Request>,
    /// Decoded surface.
    pub surface: MediaSurfaceRequest,
    /// Admitted query.
    pub query: MediaQuery,
    /// Error message.
    pub error_msg: String,
    /// Total rows.
    pub total: i64,
    /// Default picker root.
    pub default_root_id: String,
    /// Default create root.
    pub default_create_root_id: String,
    /// Default role.
    pub default_role: String,
    /// Default page size.
    pub default_page_size: i64,
    /// Attach URL.
    pub attach_url: String,
    /// Create base path.
    pub create_base_path: String,
    /// Inline surface name.
    pub inline_surface: String,
    /// Context label.
    pub context_label: String,
    /// Filter action.
    pub filter_action: String,
    /// Items.
    pub items: Vec<MediaPickerItemVM>,
    /// Current attachments.
    pub current_attachments: Vec<MediaAttachmentFormItem>,
    /// Total-page resolver.
    pub total_pages: Option<Arc<dyn Fn(i64, i64) -> i64 + Send + Sync + 'static>>,
    /// Request-scoped translator for the page's filter labels.
    pub translate_key: Option<crate::localization::TranslateKeyFunc<Request>>,

}

/// The inline create options.
#[derive(Clone, Default)]
pub struct MediaInlineCreatePropsOptions {
    /// Request.
    pub request: Option<Request>,
    /// Initialized merged form used for request-local root overrides.
    pub form: Option<Arc<effect_server::ByteFormData>>,
    /// Submitted/default input.
    pub input: crate::media::assets::MediaAssetFormInput,
    /// Decoded surface.
    pub surface: MediaSurfaceRequest,
    /// Error message.
    pub error_msg: String,
    /// Attachments.
    pub attachments: Vec<MediaAttachmentFormItem>,
    /// Default root identity.
    pub default_root_id: String,
    /// Default role.
    pub default_role: String,
    /// Action URL.
    pub action_url: String,
    /// Preview URL.
    pub preview_url: String,
    /// CSRF token.
    pub csrf_token: String,
    /// CSRF field name.
    pub csrf_field_name: String,
    /// Asset URL resolver.
    pub resolve_url: Option<crate::media::assets::MediaAssetURLResolver>,
}

/// The inline create preview options.
#[derive(Clone, Default)]
pub struct MediaInlineCreatePreviewOptions {
    /// Request.
    pub request: Option<Request>,
    /// Initialized merged form used for request-local root overrides.
    pub form: Option<Arc<effect_server::ByteFormData>>,
    /// Submitted/default input.
    pub input: crate::media::assets::MediaAssetFormInput,
    /// Root identity.
    pub root_id: String,
    /// Asset URL resolver.
    pub resolve_url: Option<crate::media::assets::MediaAssetURLResolver>,
}

/// The inline create props.
#[derive(Clone, Debug, Default)]
pub struct MediaInlineCreateProps {
    /// Root identity.
    pub root_id: String,
    /// Dialog identity.
    pub dialog_id: String,
    /// Picker root identity.
    pub picker_root_id: String,
    /// Scope.
    pub scope: String,
    /// Default role.
    pub role_default: String,
    /// Owner form identity.
    pub owner_form_id: String,
    /// Owner content identity.
    pub owner_content_id: String,
    /// Attachment identity.
    pub attachment_id: String,
    /// Action URL.
    pub action_url: String,
    /// Preview URL.
    pub preview_url: String,
    /// Preview root identity.
    pub preview_root_id: String,
    /// Error message.
    pub error_msg: String,
    /// CSRF token.
    pub csrf_token: String,
    /// CSRF field name.
    pub csrf_field_name: String,
    /// Attachments.
    pub attachments: Vec<MediaAttachmentFormItem>,
    /// Preview props.
    pub preview: MediaInlineCreatePreviewProps,
    /// Title.
    pub title: String,
    /// Kind.
    pub kind: String,
    /// Source kind.
    pub source_kind: String,
    /// URL.
    pub url: String,
    /// Description.
    pub description: String,
    /// Thumbnail URL.
    pub thumbnail_url: String,
    /// Alt text.
    pub alt: String,
    /// Duration text.
    pub duration: String,
}

/// The inline create preview props.
#[derive(Clone, Debug, Default)]
pub struct MediaInlineCreatePreviewProps {
    /// Root identity.
    pub root_id: String,
    /// Provider name.
    pub provider: String,
    /// Resolved image URL.
    pub resolved_image_url: String,
    /// Whether a remote image is shown.
    pub show_remote_image: bool,
    /// Whether the video hint is shown.
    pub show_video_hint: bool,
    /// Whether the document hint is shown.
    pub show_document_hint: bool,
}

/// The shortcode tools props.
#[derive(Clone, Debug, Default)]
pub struct ShortcodeToolsProps {
    /// Root identity.
    pub root_id: String,
    /// Dialog identity.
    pub dialog_id: String,
    /// Scope.
    pub scope: String,
    /// Owner form identity.
    pub owner_form_id: String,
    /// Picker-open URL.
    pub picker_open_url: String,
    /// Library URL.
    pub library_url: String,
    /// Target field identity.
    pub target_field_id: String,
    /// Target label.
    pub target_label: String,
    /// Items.
    pub items: Vec<MediaAttachmentFormItem>,
    /// Search.
    pub search: String,
    /// Kind.
    pub kind: String,
    /// Sort.
    pub sort: String,
    /// Page.
    pub page: i64,
    /// Page size.
    pub page_size: i64,
    /// Total.
    pub total: i64,
    /// Pages.
    pub pages: i64,
}

/// The toolbar insert button props.
#[derive(Clone, Debug, Default)]
pub struct MediaInsertToolbarButtonProps {
    /// Root identity.
    pub root_id: String,
    /// Open URL.
    pub open_url: String,
    /// Owner form identity.
    pub owner_form_id: String,
    /// Attachment identity.
    pub attachment_id: String,
    /// Editor field key.
    pub editor_field_key: String,
    /// Editor field identity.
    pub editor_field_id: String,
    /// Editor label.
    pub editor_label: String,
    /// Button label.
    pub label: String,
}

/// The fragment button props.
#[derive(Clone, Debug, Default)]
pub struct MediaFragmentButtonProps {
    /// Button label.
    pub label: String,
    /// Fragment URL.
    pub url: String,
    /// Owner form identity.
    pub owner_form_id: String,
    /// Declared variant name.
    pub variant: String,
    /// Declared size name.
    pub size: String,
    /// Button class.
    pub button_class: String,
    /// Additional attributes.
    pub button_attributes: Vec<(String, String)>,
}

/// The library card props.
#[derive(Clone, Debug, Default)]
pub struct MediaLibraryCardProps {
    /// Title.
    pub title: String,
    /// Meta text.
    pub meta: String,
    /// Preview URL.
    pub preview_url: String,
    /// Alt text.
    pub alt: String,
    /// Media kind.
    pub kind: String,
    /// Select action.
    pub select_action: String,
    /// Whether selected.
    pub selected: bool,
    /// Selected expression.
    pub selected_expr: String,
}

/// The library pagination props.
#[derive(Clone, Debug, Default)]
pub struct MediaLibraryPaginationProps {
    /// Form identity.
    pub form_id: String,
    /// Page.
    pub page: i64,
    /// Page size.
    pub page_size: i64,
    /// Total.
    pub total: i64,
    /// Pages.
    pub pages: i64,
    /// Item count.
    pub item_count: i64,
    /// Previous label.
    pub previous_label: String,
    /// Next label.
    pub next_label: String,
    /// CSS class.
    pub class: String,
}

/// The slide-item options.
#[derive(Clone, Default)]
pub struct MediaSlideItemsOptions {
    /// Embedded content.
    pub content: String,
    /// Hero index.
    pub hero_index: i64,
    /// URL resolver.
    pub resolve_url: Option<crate::media::assets::MediaAssetURLResolver>,
}

/// The hero video view options.
#[derive(Clone, Default)]
pub struct ContentVideoMediaViewOptions {
    /// Title.
    pub title: String,
    /// Hero image.
    pub image: String,
    /// Stored video JSON.
    pub video_json: Option<String>,
    /// Asset URL resolver.
    pub resolve_url: Option<crate::media::assets::MediaAssetURLResolver>,
    /// Absolute asset URL resolver.
    pub resolve_absolute_url: Option<crate::media::assets::MediaAssetURLResolver>,
}

/// The hero video view.
#[derive(Clone, Debug, Default)]
pub struct ContentVideoMediaView {
    /// Parsed video model.
    pub video_model: Option<ContentVideo>,
    /// Player props.
    pub video_block: Option<VideoPlayerProps>,
    /// Hero image URL.
    pub hero_image_url: String,
    /// Share image URL.
    pub share_image_url: String,
}

/// The media collection view options.
#[derive(Clone, Default)]
pub struct ContentMediaCollectionViewOptions {
    /// Title.
    pub title: String,
    /// Description.
    pub description: String,
    /// Stored media JSON.
    pub media_json: String,
    /// Asset URL resolver.
    pub resolve_url: Option<crate::media::assets::MediaAssetURLResolver>,
    /// Absolute asset URL resolver.
    pub resolve_absolute_url: Option<crate::media::assets::MediaAssetURLResolver>,
}

/// The media collection view.
#[derive(Clone, Debug, Default)]
pub struct ContentMediaCollectionView {
    /// Parsed items.
    pub media_models: Vec<ContentMediaItem>,
    /// Hero item.
    pub hero_model: Option<ContentMediaItem>,
    /// Hero props.
    pub hero_block: Option<HeroMediaProps>,
    /// Slide items.
    pub slide_items: Vec<MediaSlideItem>,
    /// Share image URL.
    pub share_image_url: String,
}

/// The collection HTML rendering options.
#[derive(Clone, Debug, Default)]
pub struct ContentMediaCollectionHTMLRenderingOptions {
    /// State variable.
    pub state_var: String,
}

/// The detail-section options.
#[derive(Clone, Debug, Default)]
pub struct ContentMediaDetailSectionOptions {
    /// Slide title.
    pub slide_title: String,
    /// Slide eyebrow.
    pub slide_eyebrow: String,
    /// Slide subtitle.
    pub slide_subtitle: String,
    /// Slide state variable.
    pub slide_state_var: String,
}

/// The video player props.
#[derive(Clone, Debug, Default)]
pub struct VideoPlayerProps {
    /// Video URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// Provider.
    pub provider: String,
    /// Thumbnail URL.
    pub thumbnail: String,
    /// Description.
    pub description: String,
    /// CSS class.
    pub class: String,
}

/// The hero media props.
#[derive(Clone, Debug, Default)]
pub struct HeroMediaProps {
    /// Media kind.
    pub kind: String,
    /// Media URL.
    pub url: String,
    /// Thumbnail URL.
    pub thumbnail: String,
    /// Alt text.
    pub alt: String,
    /// Title.
    pub title: String,
    /// Description.
    pub description: String,
    /// Provider.
    pub provider: String,
}

/// The embedded media props.
#[derive(Clone, Debug, Default)]
pub struct EmbeddedMediaProps {
    /// Media kind.
    pub kind: String,
    /// Media URL.
    pub url: String,
    /// Thumbnail URL.
    pub thumbnail: String,
    /// Alt text.
    pub alt: String,
    /// Title.
    pub title: String,
    /// Description.
    pub description: String,
    /// Provider.
    pub provider: String,
    /// Caption.
    pub caption: String,
}

/// One slide item.
#[derive(Clone, Debug, Default)]
pub struct MediaSlideItem {
    /// Asset identity.
    pub id: String,
    /// Media kind.
    pub kind: String,
    /// Media URL.
    pub url: String,
    /// Thumbnail URL.
    pub thumbnail: String,
    /// Alt text.
    pub alt: String,
    /// Title.
    pub title: String,
    /// Description.
    pub description: String,
    /// Provider.
    pub provider: String,
    /// Rendered HTML.
    pub rendered_html: String,
}

/// Slide deck props.
#[derive(Clone, Debug, Default)]
pub struct MediaSlideDeckProps {
    /// State variable.
    pub state_var: String,
    /// Items.
    pub items: Vec<MediaSlideItem>,
}

/// Slide section props.
#[derive(Clone, Debug, Default)]
pub struct MediaSlideSectionProps {
    /// Title.
    pub title: String,
    /// Eyebrow.
    pub eyebrow: String,
    /// Subtitle.
    pub subtitle: String,
    /// State variable.
    pub state_var: String,
    /// Items.
    pub items: Vec<MediaSlideItem>,
}

/// Hero detail-section props.
#[derive(Clone, Debug, Default)]
pub struct ContentVideoDetailSectionProps {
    /// Video props.
    pub video: Option<VideoPlayerProps>,
}

/// Collection detail-section props.
#[derive(Clone, Debug, Default)]
pub struct ContentMediaDetailSectionProps {
    /// Hero props.
    pub hero: Option<HeroMediaProps>,
    /// Slide title.
    pub slide_title: String,
    /// Slide eyebrow.
    pub slide_eyebrow: String,
    /// Slide subtitle.
    pub slide_subtitle: String,
    /// Slide state variable.
    pub slide_state_var: String,
    /// Slides.
    pub slides: Vec<MediaSlideItem>,
}

/// The media list item projection options.
#[derive(Clone, Default)]
pub struct MediaAssetListItemOptions {
    /// Detail target resolver.
    pub detail_href: Option<Arc<dyn Fn(&MediaAssetRecord) -> String + Send + Sync + 'static>>,
    /// Delete target resolver.
    pub delete_href: Option<Arc<dyn Fn(&MediaAssetRecord) -> String + Send + Sync + 'static>>,
    /// Preview image kind.
    pub image_kind: String,
    /// Asset URL resolver.
    pub resolve_url: Option<crate::media::assets::MediaAssetURLResolver>,
}
