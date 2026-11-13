//! Media presentation: picker/dashboard/component contracts, labels and view props.
//!
//! This owner supplies the actual media presenter values over the corrected W14
//! runtime/management producers:
//!
//! - picker page/item/filter/pagination view models, sort/filter/selection policies,
//!   item card/meta/action state, list-item projection and Datastar action strings;
//! - stored content-media view derivation: hero video/player props, collection
//!   views, slide selection/filtering, structured video values and share images;
//! - inline-create and preview props, provider/role/stable-id helpers, labels and
//!   contract payload projections;
//! - shared presentation contracts for library cards, pagination, embedded media,
//!   toolbar keys, slide decks/sections and detail sections.
//!
//! # Capabilities and effects
//!
//! Asset URL resolvers, translators and total-page resolvers are caller-supplied
//! callbacks. The presentation helpers are pure over their inputs; asset/store
//! access remains with the runtime/management owners and the consumer stores.
//!
//! # Rendering
//!
//! The complete media component set is authored in Maud and shares the existing
//! component props and class builders. Native text projection happens only at
//! presentation/JSON boundaries; store records and callback arguments retain bytes.
//! The dashboard and shortcode producers invoke consumer stores through effects.
//!
//! # No store
//!
//! No database, SQL, schema, migration, pool, backend, host, scanner or runtime is
//! declared or constructed here.

mod actions;
mod components;
mod contract;
mod contracts;
mod inline;
mod labels;
mod media_views;
mod picker;
mod stable_id;

pub use self::actions::{
    js_string, media_dismiss_action, media_js_string, media_mount_clear_action,
    media_mount_clear_only_action, media_picker_dismiss_action,
    media_picker_mount_clear_action, media_scroll_lock_action,
    media_scroll_unlock_when_idle_action,
};
pub use self::components::{
    media_attachment_payload_inputs, media_mount_root, media_preview_frame, media_type_badge,
    normalized_kind_label, provider_chip,
};
pub use self::contract::{contract_media_items_value, contract_video_value};
pub use self::contracts::*;
pub use self::inline::{
    build_media_inline_create_preview_props, build_media_inline_create_props,
    media_inline_create_preview_root_id,
};
pub use crate::media::assets::resolve_media_asset_form_provider;
pub use self::labels::{
    MEDIA_ATTACHMENTS_SECTION_TITLE_KEY, MEDIA_DASHBOARD_RECENT_PAGE_PARAM,
    MEDIA_DASHBOARD_TOOLS_TITLE_KEY, MEDIA_PICKER_ATTACH_SELECTED_LABEL_KEY,
    MEDIA_PICKER_CONTEXT_HERO_KEY, media_kind_label, media_reference_count_label,
    media_status_label, media_translate,
};
pub use self::media_views::{
    DEFAULT_CONTENT_MEDIA_SLIDE_STATE_VAR, build_content_media_collection_view,
    build_content_media_detail_section_props, build_content_video_detail_section_props,
    build_content_video_media_view, build_media_slide_items, build_resource_preview_image_map,
    content_hero_media_props, content_media_share_image_url, content_media_slide_items,
    content_media_structured_video, content_video_player_props,
    content_video_structured_video, media_slide_deck_props, media_slide_section_props,
    parse_content_media, parse_content_video, select_content_media_hero_index,
};
pub use self::picker::{
    MEDIA_PICKER_SORT_ALPHA, MEDIA_PICKER_SORT_NEWEST, MEDIA_PICKER_SORT_OLDEST,
    default_media_picker, default_media_picker_render_options, build_media_picker_items, build_media_picker_page_from_state, build_media_picker_page_vm,
    media_asset_list_item_states, media_form_or_query_value, media_picker_asset_records,
    media_picker_attachment_state, media_picker_default_hero_role, media_picker_filter_pills,
    media_picker_filter_submit_action, media_picker_item_card_class, media_picker_item_meta,
    media_picker_items_from_result, media_picker_normalize_kind, media_picker_pagination_props,
    media_picker_selected_attr, media_picker_selection_config, media_picker_short_id,
    media_picker_sort_options, media_picker_sort_value,
};
pub use self::stable_id::stable_media_id;

/// The media player client asset name.
pub const MEDIA_PLAYER_CLIENT_ASSET: &str = "contentmgmt-media-player";

/// Project byte-preserving values at the native text presentation boundary.
pub(crate) fn view_text(value: impl AsRef<[u8]>) -> String {
    crate::byte_text::encoding_text(value.as_ref())
}

mod render;
pub use render::MediaRenderContext;

mod render_content;
pub use render_content::*;

mod render_library;
pub use render_library::*;

mod render_picker;
pub use render_picker::*;

mod render_actions;
pub use render_actions::*;

mod render_manager;
pub use render_manager::*;

mod render_inline;
pub use render_inline::*;

mod render_shortcode;
pub use render_shortcode::*;

mod shortcodes;
pub use shortcodes::*;

mod dashboard;
pub use dashboard::*;
