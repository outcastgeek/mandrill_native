//! Media surface query and state values shared by media workflows.
//!
//! These are plain caller-supplied values. They carry no store, request adapter or
//! HTML. Media picker/panel rendering and request parsing belong to later media
//! owners and use these values without redefining them.

use std::sync::Arc;
use crate::commands::InputBytes;

mod helpers;
pub use self::helpers::{
    build_media_attachment_manager_from_request, media_create_fragment_path,
    media_fragment_path, media_picker_fragment_path, media_query_from_request,
    media_surface_is_inline, parse_media_surface_request, resolve_media_surface_mounts,
};

use crate::attachments::MediaSurfaceMounts;

/// The default media fragment root identity.
pub const DEFAULT_MEDIA_FRAGMENT_ROOT_ID: &str = "media-fragment-root";
/// The default media picker root identity.
pub const DEFAULT_MEDIA_PICKER_ROOT_ID: &str = "media-picker-root";
/// The default media create root identity.
pub const DEFAULT_MEDIA_CREATE_ROOT_ID: &str = "media-create-root";
/// The default media shortcode root identity.
pub const DEFAULT_MEDIA_SHORTCODE_ROOT_ID: &str = "media-shortcode-root";
/// The inline media surface name.
pub const MEDIA_SURFACE_INLINE: &str = "inline";
/// The image media kind name.
pub const MEDIA_KIND_IMAGE: &str = "image";

/// One media list/picker query.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaQuery {
    /// Free-text search value.
    pub search: InputBytes,
    /// Media kind filter.
    pub kind: InputBytes,
    /// Media status filter.
    pub status: InputBytes,
    /// Requested sort order.
    pub sort: InputBytes,
    /// Requested page size.
    pub limit: i64,
    /// Requested row offset.
    pub offset: i64,
}

/// One decoded media surface request state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaSurfaceRequest {
    /// The surface scope.
    pub scope: InputBytes,
    /// The target section identity.
    pub target_id: InputBytes,
    /// The owner form identity.
    pub owner_form_id: InputBytes,
    /// The owning content identity.
    pub owner_content_id: InputBytes,
    /// The attachment role.
    pub role: InputBytes,
    /// The active root identity.
    pub root_id: InputBytes,
    /// The create root identity.
    pub create_root_id: InputBytes,
    /// The picker root identity.
    pub picker_root_id: InputBytes,
}

/// Caller-supplied state normalizers for one media surface request.
#[derive(Clone, Default)]
pub struct MediaSurfaceOptions {
    /// Optional scope normalizer.
    pub normalize_scope: Option<Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync + 'static>>,
    /// Optional role normalizer.
    pub normalize_role: Option<Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync + 'static>>,
}

/// Resolves the surface mounts for one scope.
pub type MountsResolver =
    Arc<dyn Fn(&[u8]) -> MediaSurfaceMounts + Send + Sync + 'static>;

mod patch;
pub use patch::{MediaPatch,media_datastar_request,write_media_patch,write_media_patches};
