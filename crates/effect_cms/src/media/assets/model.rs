//! Media asset records, preparation options and consumer store contracts.
//!
//! Every record is plain data with a zero value. The stores are consumer-owned
//! capabilities: their methods return lazy effects and perform the actual reads and
//! writes outside the CMS. No table, query, schema, migration, pool or backend is
//! declared or constructed here.

use crate::commands::InputBytes;
use std::collections::BTreeMap;
use std::sync::{Arc,atomic::AtomicI64};

use effect::{AtomicRef, Effect};
use effect_server::Request;

use crate::attachments::{MediaAttachmentStoreRecord, RoleNormalizer};

use super::super::surface::MediaQuery;

/// The image media kind.
pub const MEDIA_KIND_IMAGE: &str = "image";
/// The video media kind.
pub const MEDIA_KIND_VIDEO: &str = "video";
/// The document media kind.
pub const MEDIA_KIND_DOCUMENT: &str = "document";

/// A remotely hosted media source.
pub const MEDIA_SOURCE_KIND_REMOTE: &str = "remote";
/// An uploaded media source.
pub const MEDIA_SOURCE_KIND_UPLOAD: &str = "upload";
/// An embedded media source.
pub const MEDIA_SOURCE_KIND_EMBED: &str = "embed";

/// A draft media status.
pub const MEDIA_STATUS_DRAFT: &str = "draft";
/// A ready media status.
pub const MEDIA_STATUS_READY: &str = "ready";
/// An archived media status.
pub const MEDIA_STATUS_ARCHIVED: &str = "archived";

/// The default media library page size.
pub const DEFAULT_MEDIA_LIBRARY_PAGE_SIZE: i64 = 24;

/// The newest-first media sort.
pub const MEDIA_SORT_NEWEST: &str = "newest";
/// The oldest-first media sort.
pub const MEDIA_SORT_OLDEST: &str = "oldest";
/// The alphabetical media sort.
pub const MEDIA_SORT_ALPHA: &str = "alpha";

/// Resolves a stored media URL.
pub type MediaAssetURLResolver =
    Arc<dyn Fn(&[u8]) -> InputBytes + Send + Sync + 'static>;

/// Resolves an asset record's edit URL.
pub type MediaAssetEditUrl =
    Arc<dyn Fn(Option<&Request>, &MediaAssetRecord) -> String + Send + Sync + 'static>;

/// Generates a media asset or attachment identity.
pub type MediaAssetIdGenerator = Arc<dyn Fn() -> String + Send + Sync + 'static>;

/// Reads the current Unix time in seconds.
pub type MediaAssetNowUnix = Arc<dyn Fn() -> i64 + Send + Sync + 'static>;

/// One stored media asset.
#[derive(Clone, Debug, Default)]
pub struct MediaAssetRecord {
    /// Asset identity.
    pub id: String,
    /// Media kind.
    pub kind: String,
    /// Media source kind.
    pub source_kind: String,
    /// Media status.
    pub status: String,
    /// Source URL.
    pub url: InputBytes,
    /// CDN URL.
    pub cdn_url: InputBytes,
    /// Provider name.
    pub provider: InputBytes,
    /// Title.
    pub title: InputBytes,
    /// Description.
    pub description: InputBytes,
    /// Thumbnail URL.
    pub thumbnail_url: InputBytes,
    /// Alternative text.
    pub alt: InputBytes,
    /// Optional duration.
    pub duration: Option<Arc<AtomicI64>>,
    /// Creator identity.
    pub created_by: String,
    /// Creation time in Unix seconds.
    pub created_at: i64,
    /// Last update time in Unix seconds.
    pub updated_at: i64,
}

/// One media asset form input.
#[derive(Clone, Debug, Default)]
pub struct MediaAssetFormInput {
    /// Media kind.
    pub kind: String,
    /// Media source kind.
    pub source_kind: String,
    /// Source URL.
    pub url: InputBytes,
    /// Provider name.
    pub provider: InputBytes,
    /// Title.
    pub title: InputBytes,
    /// Description.
    pub description: InputBytes,
    /// Thumbnail URL.
    pub thumbnail_url: InputBytes,
    /// Alternative text.
    pub alt: InputBytes,
    /// Optional duration.
    pub duration: Option<Arc<AtomicI64>>,
    /// Media status.
    pub status: String,
}

/// One media asset list result.
#[derive(Clone, Debug, Default)]
pub struct MediaAssetListResult {
    /// Result rows in store order.
    pub items: Vec<MediaAssetRecord>,
    /// Total matching rows.
    pub total: i64,
}

/// A consumer's media asset store contract.
pub trait MediaAssetStore<E, R>: Send + Sync + 'static
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Resolve one asset, or absence.
    fn get_by_id(&self, id: InputBytes) -> Effect<Option<MediaAssetRecord>, E, R>;

    /// Resolve assets by identity; absent identities are omitted.
    fn get_by_ids(&self, ids: Vec<InputBytes>) -> Effect<BTreeMap<InputBytes, MediaAssetRecord>, E, R>;

    /// List assets for one query, or no result.
    fn list(&self, query: MediaQuery) -> Effect<Option<MediaAssetListResult>, E, R>;

    /// Create one mutable asset; the store may assign identity and fields.
    fn create(&self, record: AtomicRef<MediaAssetRecord>) -> Effect<(), E, R>;

    /// Update one mutable asset; the store may assign fields.
    fn update(&self, record: AtomicRef<MediaAssetRecord>) -> Effect<(), E, R>;

    /// Delete one asset by identity.
    fn delete(&self, id: InputBytes) -> Effect<(), E, R>;
}

/// A consumer's content-media attachment store contract.
pub trait ContentMediaAttachmentStore<E, R>: Send + Sync + 'static
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// List attachment rows for one owner.
    fn list_by_owner(
        &self,
        owner_type: InputBytes,
        owner_content_id: InputBytes,
    ) -> Effect<Vec<MediaAttachmentStoreRecord>, E, R>;

    /// Replace the attachment rows for one owner.
    fn replace_by_owner(
        &self,
        owner_type: InputBytes,
        owner_content_id: InputBytes,
        rows: Vec<MediaAttachmentStoreRecord>,
    ) -> Effect<(), E, R>;

    /// Delete the attachment rows for one owner.
    fn delete_by_owner(
        &self,
        owner_type: InputBytes,
        owner_content_id: InputBytes,
    ) -> Effect<(), E, R>;
}

/// Options for media attachment store workflows.
pub struct MediaStoreAttachmentOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The current request, when one exists.
    pub request: Option<Request>,
    /// The initialized merged form, when available.
    pub form: Option<Arc<effect_server::ByteFormData>>,
    /// Optional asset store capability.
    pub assets: Option<Arc<dyn MediaAssetStore<E, R>>>,
    /// Optional attachment store capability.
    pub attachments: Option<Arc<dyn ContentMediaAttachmentStore<E, R>>>,
    /// The owner type.
    pub owner_type: InputBytes,
    /// The owning content identity.
    pub owner_content_id: InputBytes,
    /// Optional attachment-role normalizer.
    pub normalize_role: Option<RoleNormalizer>,
    /// Optional asset edit-URL resolver.
    pub edit_url: Option<MediaAssetEditUrl>,
    /// The image media kind used for previews.
    pub image_kind: String,
    /// Optional asset URL resolver.
    pub resolve_url: Option<MediaAssetURLResolver>,
}

impl<E, R> Default for MediaStoreAttachmentOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            request: None,
            form: None,
            assets: None,
            attachments: None,
            owner_type: InputBytes::default(),
            owner_content_id: InputBytes::default(),
            normalize_role: None,
            edit_url: None,
            image_kind: String::new(),
            resolve_url: None,
        }
    }
}

impl<E, R> Clone for MediaStoreAttachmentOptions<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            form: self.form.clone(),
            assets: self.assets.clone(),
            attachments: self.attachments.clone(),
            owner_type: self.owner_type.clone(),
            owner_content_id: self.owner_content_id.clone(),
            normalize_role: self.normalize_role.clone(),
            edit_url: self.edit_url.clone(),
            image_kind: self.image_kind.clone(),
            resolve_url: self.resolve_url.clone(),
        }
    }
}

/// One prepared attachment row for replacement.
///
/// The attachment store record is reused directly; this value adds the row position
/// and assignment timestamps without shadowing the stored row.
#[derive(Clone, Debug, Default)]
pub struct PreparedMediaAttachmentStoreRecord {
    /// The stored attachment row.
    pub record: MediaAttachmentStoreRecord,
    /// The row position.
    pub position: i64,
    /// Creation time in Unix seconds.
    pub created_at: i64,
    /// Last update time in Unix seconds.
    pub updated_at: i64,
}
