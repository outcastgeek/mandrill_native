//! Attachment item, surface and manager contracts.
//!
//! Every record here is plain declaration data with a zero value: no owner code
//! validates, normalizes or repairs a record, and no invariant is invented.
//!
//! None of these records authorizes storage. [`MediaAttachmentStoreRecord`] is the
//! persisted-row shape a consumer's own store implementation supplies and reads; it
//! is not a CMS table, schema, migration, query, pool or connection, and this owner
//! opens no store.
//!
//! No HTML is produced in this owner: a [`MediaAttachmentFormItem`] is data for the
//! rendering owners, and the media shortcode built in [`super::inputs`] is shortcode
//! text, not markup.

use crate::commands::InputBytes;

/// The hero attachment role.
pub const MEDIA_ATTACHMENT_ROLE_HERO: &str = "hero";

/// The gallery (additional) attachment role.
pub const MEDIA_ATTACHMENT_ROLE_GALLERY: &str = "gallery";

/// The inline attachment role, and the shortcode role used when a caller supplies
/// no role.
pub const MEDIA_ATTACHMENT_ROLE_INLINE: &str = "inline";

/// The download attachment role.
pub const MEDIA_ATTACHMENT_ROLE_DOWNLOAD: &str = "download";

/// One requested attachment before it is verified.
///
/// The consumer supplies these values from its own data; nothing here checks that
/// the asset exists, that the role is one of the four declared roles, or that the
/// override text is acceptable.
#[derive(Clone, Debug, Default)]
pub struct MediaAttachmentInput {
    /// The media asset identity; blank entries are removed by the input algebra.
    pub asset_id: InputBytes,
    /// The attachment role as supplied, possibly consumer-defined.
    pub role: InputBytes,
    /// The caller's title override.
    pub title_override: InputBytes,
    /// The caller's caption.
    pub caption: InputBytes,
    /// The caller's alternative-text override.
    pub alt_override: InputBytes,
}

/// One persisted attachment row.
///
/// This is the store *record* contract only. The consumer's store owns
/// the table, query and write mechanics; this owner neither implements nor calls
/// them.
#[derive(Clone, Debug, Default)]
pub struct MediaAttachmentStoreRecord {
    /// The attachment row identity.
    pub id: InputBytes,
    /// The attached media asset identity.
    pub media_asset_id: InputBytes,
    /// The stored attachment role.
    pub role: InputBytes,
    /// The stored title override.
    pub title_override: InputBytes,
    /// The stored caption.
    pub caption: InputBytes,
    /// The stored alternative-text override.
    pub alt_override: InputBytes,
}

/// One attachment row as presented in a form.
///
/// `position` is a signed 64-bit integer; the collection helpers order rows by it.
/// Every other field is text the rendering owners present.
#[derive(Clone, Debug, Default)]
pub struct MediaAttachmentFormItem {
    /// The row's form key.
    pub key: InputBytes,
    /// The row's ordering position.
    pub position: i64,
    /// The row's media asset identity.
    pub asset_id: InputBytes,
    /// The media kind.
    pub kind: String,
    /// The attachment role.
    pub role: InputBytes,
    /// The asset status.
    pub status: String,
    /// The asset title.
    pub title: InputBytes,
    /// The asset description.
    pub description: InputBytes,
    /// The asset provider.
    pub provider: InputBytes,
    /// The asset URL.
    pub url: InputBytes,
    /// The asset thumbnail URL.
    pub thumbnail_url: InputBytes,
    /// The resolved asset URL.
    pub resolved_url: InputBytes,
    /// The resolved thumbnail URL.
    pub resolved_thumbnail: InputBytes,
    /// The asset alternative text.
    pub alt: InputBytes,
    /// The attachment's title override.
    pub title_override: InputBytes,
    /// The attachment's caption.
    pub caption: InputBytes,
    /// The attachment's alternative-text override.
    pub alt_override: InputBytes,
    /// The row's media shortcode text.
    pub shortcode: InputBytes,
    /// The row's edit URL.
    pub edit_url: String,
}

/// The media surface root element identities.
#[derive(Clone, Debug, Default)]
pub struct MediaSurfaceMounts {
    /// The section element identity.
    pub section_id: InputBytes,
    /// The owner form identity.
    pub owner_form_id: InputBytes,
    /// The picker root identity.
    pub picker_root_id: InputBytes,
    /// The create root identity.
    pub create_root_id: InputBytes,
    /// The shortcode root identity.
    pub shortcode_root_id: InputBytes,
    /// The shortcode dialog identity.
    pub shortcode_dialog_id: InputBytes,
}

/// The configuration of one attachment manager surface.
#[derive(Clone, Debug, Default)]
pub struct MediaAttachmentManagerProps {
    /// The surface scope.
    pub scope: InputBytes,
    /// The owning content identity.
    pub owner_content_id: InputBytes,
    /// The surface root element identities.
    pub mounts: MediaSurfaceMounts,
    /// The picker URL.
    pub picker_url: String,
    /// The create URL.
    pub create_url: String,
    /// The mutation URL.
    pub mutation_url: String,
    /// The attachment rows to present, in presentation order.
    pub attachments: Vec<MediaAttachmentFormItem>,
}
