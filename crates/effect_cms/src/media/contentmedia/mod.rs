//! Stored content-media values and pure transformations.
//!
//! The owner declares the JSON shapes written into stored content fields and derives
//! the hero image/video and stored media-item list from presented attachment rows.
//! It performs no I/O, owns no store and produces no HTML.

use serde::{Deserialize, Serialize};

mod decode;
mod prepared;

pub use prepared::{
    MediaDecodeLimits, PreparedContentMedia, PreparedContentVideo, PreparedMediaDecoder, PreparedMediaError,
    write_content_media_json, write_content_video_json,
};

use crate::attachments::{MediaAttachmentFormItem, normalize_media_attachment_role};
use crate::commands::InputBytes;

const MEDIA_KIND_IMAGE: &str = "image";
const MEDIA_KIND_VIDEO: &str = "video";
const MEDIA_KIND_DOCUMENT: &str = "document";

/// The stored video provider.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ContentVideoProvider {
    /// Provider name.
    #[serde(default)]
    pub name: InputBytes,
    /// Provider base URL.
    #[serde(default, rename = "base-url")]
    pub base_url: String,
}

/// The stored video metadata.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ContentVideoMetadata {
    /// Description.
    #[serde(default, skip_serializing_if = "InputBytes::is_empty")]
    pub description: InputBytes,
    /// Thumbnail URL.
    #[serde(default, skip_serializing_if = "InputBytes::is_empty")]
    pub thumbnail: InputBytes,
    /// Duration.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub duration: i64,
    /// Width.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: i64,
    /// Height.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub height: i64,
    /// Provider identity.
    #[serde(default, rename = "provider-id", skip_serializing_if = "String::is_empty")]
    pub provider_id: String,
    /// Provider URL.
    #[serde(
        default,
        rename = "provider-url",
        skip_serializing_if = "String::is_empty"
    )]
    pub provider_url: String,
    /// Publication time.
    #[serde(default, rename = "published-at", skip_serializing_if = "String::is_empty")]
    pub published_at: String,
}

/// The stored hero video.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ContentVideo {
    /// Asset identity.
    #[serde(default)]
    pub id: InputBytes,
    /// Title.
    #[serde(default)]
    pub title: InputBytes,
    /// Source URL.
    #[serde(default)]
    pub url: InputBytes,
    /// Provider.
    #[serde(default)]
    pub provider: ContentVideoProvider,
    /// Metadata.
    #[serde(default)]
    pub metadata: ContentVideoMetadata,
    /// Series.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub series: String,
    /// Tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// One stored content-media item.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ContentMediaItem {
    /// Asset identity.
    #[serde(default, skip_serializing_if = "InputBytes::is_empty")]
    pub id: InputBytes,
    /// Attachment role.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// Media kind.
    #[serde(default, rename = "type")]
    pub kind: String,
    /// Source URL.
    #[serde(default)]
    pub url: InputBytes,
    /// Thumbnail URL.
    #[serde(default, skip_serializing_if = "InputBytes::is_empty")]
    pub thumbnail: InputBytes,
    /// Alternative text.
    #[serde(default, skip_serializing_if = "InputBytes::is_empty")]
    pub alt: InputBytes,
    /// Title.
    #[serde(default, skip_serializing_if = "InputBytes::is_empty")]
    pub title: InputBytes,
    /// Description.
    #[serde(default, skip_serializing_if = "InputBytes::is_empty")]
    pub description: InputBytes,
    /// Provider.
    #[serde(default, skip_serializing_if = "InputBytes::is_empty")]
    pub provider: InputBytes,
    /// Duration.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub duration: i64,
    /// Width.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: i64,
    /// Height.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub height: i64,
}

/// Marshal stored media items.
///
/// # Errors
///
/// Returns the JSON serialization failure.
pub fn marshal_content_media_json(
    items: &[ContentMediaItem],
) -> Result<String, serde_json::Error> {
    if items.is_empty() {
        return Ok("[]".to_owned());
    }
    encoded_json(items)
}

/// Parse a stored hero video.
///
/// # Errors
///
/// Returns the JSON parse failure.
pub fn parse_content_video_json(
    raw: Option<&str>,
) -> Result<Option<ContentVideo>, serde_json::Error> {
    parse_content_video_json_bytes(raw.map(str::as_bytes))
}

/// Parse a stored hero video from its original JSON bytes.
///
/// # Errors
///
/// Returns the JSON parse failure.
pub fn parse_content_video_json_bytes(
    raw: Option<&[u8]>,
) -> Result<Option<ContentVideo>, serde_json::Error> {
    let Some(raw) = raw else { return Ok(None); };
    if crate::validation::trim_unicode_whitespace(raw).is_empty() {
        return Ok(None);
    }
    decode::video(raw).map(Some)
}

/// Parse stored media items, retaining the original JSON bytes at the boundary.
///
/// # Errors
///
/// Returns the JSON parse failure.
pub fn parse_content_media_json(
    raw: impl AsRef<[u8]>,
) -> Result<Option<Vec<ContentMediaItem>>, serde_json::Error> {
    let raw = raw.as_ref();
    if crate::validation::trim_unicode_whitespace(raw).is_empty() {
        return Ok(None);
    }
    decode::items(raw)
}

/// Marshal a stored hero video.
///
/// # Errors
///
/// Returns the JSON serialization failure.
pub fn marshal_content_video_json(
    video: Option<&ContentVideo>,
) -> Result<Option<String>, serde_json::Error> {
    video.map(encoded_json).transpose()
}

fn encoded_json<T: Serialize + ?Sized>(value: &T) -> Result<String, serde_json::Error> {
    let mut bytes = Vec::new();
    prepared::encode_json(value, &mut bytes)?;
    String::from_utf8(bytes).map_err(|error| serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData, error,
    )))
}

/// Derive the hero image and video from presented attachment rows.
#[must_use]
pub fn content_video_from_attachment_items(
    items: &[MediaAttachmentFormItem],
) -> (InputBytes, Option<ContentVideo>) {
    let mut image = InputBytes::default();
    let mut video = None;
    for item in items {
        if normalize_media_kind(&item.kind) == MEDIA_KIND_IMAGE && image.is_empty() {
            image = first_non_blank(&[item.url.clone(), item.resolved_url.clone()]);
        }
        if normalize_media_kind(&item.kind) == MEDIA_KIND_VIDEO && video.is_none() {
            video = Some(ContentVideo {
                id: item.asset_id.clone(),
                title: first_non_blank(&[item.title_override.clone(), item.title.clone()]),
                url: first_non_blank(&[item.url.clone(), item.resolved_url.clone()]),
                provider: ContentVideoProvider {
                    name: normalize_media_provider_name(&item.provider),
                    base_url: String::new(),
                },
                metadata: ContentVideoMetadata {
                    description: item.description.clone(),
                    thumbnail: first_non_blank(&[
                        item.thumbnail_url.clone(),
                        item.resolved_thumbnail.clone(),
                    ]),
                    ..ContentVideoMetadata::default()
                },
                ..ContentVideo::default()
            });
        }
    }
    (image, video)
}

/// Derive stored media items from presented attachment rows.
#[must_use]
pub fn content_media_items_from_attachment_items(
    items: &[MediaAttachmentFormItem],
) -> Vec<ContentMediaItem> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(ContentMediaItem {
            id: item.asset_id.trim().to_owned(),
            role: normalize_media_attachment_role(&item.role),
            kind: item.kind.clone(),
            url: first_non_blank(&[item.url.clone(), item.resolved_url.clone()]),
            thumbnail: first_non_blank(&[
                item.thumbnail_url.clone(),
                item.resolved_thumbnail.clone(),
            ]),
            alt: first_non_blank(&[item.alt_override.clone(), item.alt.clone()]),
            title: first_non_blank(&[item.title_override.clone(), item.title.clone()]),
            description: item.description.clone(),
            provider: normalize_media_provider_name(&item.provider),
            ..ContentMediaItem::default()
        });
    }
    out
}

fn normalize_media_kind(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        MEDIA_KIND_VIDEO => MEDIA_KIND_VIDEO.to_owned(),
        MEDIA_KIND_DOCUMENT => MEDIA_KIND_DOCUMENT.to_owned(),
        _ => MEDIA_KIND_IMAGE.to_owned(),
    }
}

fn normalize_media_provider_name(provider: &InputBytes) -> InputBytes {
    crate::byte_text::lowercase(provider.trim().as_bytes()).into()
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}

fn first_non_blank(values: &[InputBytes]) -> InputBytes { values.iter().map(InputBytes::trim).find(|v| !v.is_empty()).unwrap_or_default() }
