//! Media asset normalization and form/record preparation.

use std::collections::HashSet;
use crate::commands::InputBytes;
use crate::validation::trim_unicode_whitespace;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;
use effect::{Effect, effect_sync};


use crate::attachments::{MediaAttachmentStoreRecord, normalize_media_attachment_role};

use super::super::surface::MediaQuery;
use super::model::{
    DEFAULT_MEDIA_LIBRARY_PAGE_SIZE, MEDIA_KIND_DOCUMENT, MEDIA_KIND_IMAGE, MEDIA_KIND_VIDEO,
    MEDIA_SORT_ALPHA, MEDIA_SORT_NEWEST, MEDIA_SORT_OLDEST, MEDIA_SOURCE_KIND_EMBED,
    MEDIA_SOURCE_KIND_REMOTE, MEDIA_SOURCE_KIND_UPLOAD, MEDIA_STATUS_ARCHIVED, MEDIA_STATUS_DRAFT,
    MEDIA_STATUS_READY, MediaAssetFormInput, MediaAssetIdGenerator, MediaAssetNowUnix,
    MediaAssetRecord, PreparedMediaAttachmentStoreRecord,
};

const NON_IMAGE_EXTENSIONS: &[&str] = &[
    ".pdf", ".doc", ".docx", ".ppt", ".pptx", ".xls", ".xlsx", ".csv", ".txt", ".mp4",
    ".webm", ".mov", ".m4v", ".avi", ".mkv", ".m3u8",
];

/// Normalize a provider name.
#[must_use]
pub fn normalize_media_provider_name(provider: &str) -> String {
    provider.trim().to_lowercase()
}

/// Normalize a media kind, defaulting to image.
#[must_use]
pub fn normalize_media_kind(raw: impl AsRef<[u8]>) -> String {
    match crate::byte_text::lowercase(trim_unicode_whitespace(raw.as_ref())).as_str() {
        MEDIA_KIND_VIDEO => MEDIA_KIND_VIDEO.to_owned(),
        MEDIA_KIND_DOCUMENT => MEDIA_KIND_DOCUMENT.to_owned(),
        _ => MEDIA_KIND_IMAGE.to_owned(),
    }
}

/// Normalize a media source kind, defaulting to remote.
#[must_use]
pub fn normalize_media_source_kind(raw: impl AsRef<[u8]>) -> String {
    match crate::byte_text::lowercase(trim_unicode_whitespace(raw.as_ref())).as_str() {
        MEDIA_SOURCE_KIND_UPLOAD => MEDIA_SOURCE_KIND_UPLOAD.to_owned(),
        MEDIA_SOURCE_KIND_EMBED => MEDIA_SOURCE_KIND_EMBED.to_owned(),
        _ => MEDIA_SOURCE_KIND_REMOTE.to_owned(),
    }
}

/// Normalize a media status, defaulting to draft.
#[must_use]
pub fn normalize_media_status(raw: impl AsRef<[u8]>) -> String {
    match crate::byte_text::lowercase(trim_unicode_whitespace(raw.as_ref())).as_str() {
        MEDIA_STATUS_READY => MEDIA_STATUS_READY.to_owned(),
        MEDIA_STATUS_ARCHIVED => MEDIA_STATUS_ARCHIVED.to_owned(),
        _ => MEDIA_STATUS_DRAFT.to_owned(),
    }
}

/// Parse an optional positive integer.
#[must_use]
pub fn parse_optional_positive_int(raw: impl AsRef<[u8]>) -> Option<i64> {
    std::str::from_utf8(trim_unicode_whitespace(raw.as_ref())).ok()?
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
}

/// Parse a positive integer, or zero.
#[must_use]
pub fn parse_positive_int(raw: &str) -> i64 {
    parse_optional_positive_int(raw).unwrap_or_default()
}

/// Normalize and deduplicate asset identities, preserving first occurrence order.
#[must_use]
pub fn normalize_media_asset_ids(ids: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id.trim();
        if id.is_empty() || !seen.insert(id.to_owned()) {
            continue;
        }
        normalized.push(id.to_owned());
    }
    normalized
}

/// Normalize a media query.
#[must_use]
pub fn normalize_media_query(mut query: MediaQuery) -> MediaQuery {
    query.kind = query.kind.trim().to_owned();
    query.status = query.status.trim().to_owned();
    query.search = query.search.trim().to_owned();
    query.sort = crate::byte_text::lowercase(query.sort.trim().as_bytes()).into();
    if query.sort != MEDIA_SORT_OLDEST && query.sort != MEDIA_SORT_ALPHA {
        query.sort = MEDIA_SORT_NEWEST.into();
    }
    if query.limit <= 0 {
        query.limit = DEFAULT_MEDIA_LIBRARY_PAGE_SIZE;
    }
    if query.offset < 0 {
        query.offset = 0;
    }
    query
}

/// Resolve the provider for one form input.
#[must_use]
pub fn resolve_media_asset_form_provider(input: &MediaAssetFormInput) -> InputBytes {
    resolve_media_provider(
        &input.provider,
        &input.url,
        &input.source_kind,
        &input.kind,
    )
}

/// Resolve a provider from an explicit provider value and source URL.
#[must_use]
pub fn resolve_media_provider(
    provider: &InputBytes,
    raw_url: &InputBytes,
    source_kind: &str,
    kind: &str,
) -> InputBytes {
    let provider = provider.trim();
    if !provider.is_empty() {
        return provider.to_owned().into();
    }
    if raw_url.trim().is_empty() {
        return InputBytes::default();
    }
    let parsed = match super::url::parse(raw_url.trim().as_bytes()) {
        Some(parsed) => parsed,
        None => return InputBytes::default(),
    };
    let host = crate::byte_text::lowercase(trim_unicode_whitespace(&parsed.host));
    let host = host.trim_prefix("www.");
    if host == "youtu.be" || host.contains("youtube.com") {
        return "YouTube".to_owned().into();
    }
    if host.contains("vimeo.com") {
        return "Vimeo".to_owned().into();
    }
    if host.contains("cloudflarestream.com") || host.contains("videodelivery.net") {
        return "Cloudflare Stream".to_owned().into();
    }
    if host.is_empty() { return InputBytes::default(); }
    if source_kind.trim().eq_ignore_ascii_case(MEDIA_SOURCE_KIND_EMBED)
        || kind.trim().eq_ignore_ascii_case(MEDIA_KIND_VIDEO)
    {
        return capitalize_first_host_label(host);
    }
    InputBytes::default()
}

fn capitalize_first_host_label(host: &str) -> InputBytes {
    let label = host.split('.').next().unwrap_or("").as_bytes();
    if label.is_empty() { return InputBytes::default(); }
    let mut capitalized = crate::byte_text::encoding_text(&label[..1])
        .chars().map(|character| character.to_uppercase().next().unwrap_or(character))
        .collect::<String>().into_bytes();
    capitalized.extend_from_slice(&label[1..]);
    capitalized.into()
}

/// Whether a media URL is renderable as an image.
///
/// A blank value is false. A case-insensitive `data:image/` prefix is true. A URL
/// that cannot be parsed falls back to true. Path and fragment escapes are validated;
/// query escape text is left untouched. Otherwise the percent-decoded path extension decides: known document and
/// video extensions are false; known image extensions and unknown or empty
/// extensions are true. Query and fragment text is excluded, a literal `+` stays a
/// plus in the path, and relative paths are accepted.
#[must_use]
pub fn media_looks_renderable_as_image(raw: impl AsRef<[u8]>) -> bool {
    let raw = trim_unicode_whitespace(raw.as_ref());
    if raw.is_empty() { return false; }
    if raw.get(..11).is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"data:image/")) {
        return true;
    }
    let Some(parsed) = super::url::parse(raw) else { return true; };
    let extension = path_extension(&parsed.path);
    !has_extension(&extension, NON_IMAGE_EXTENSIONS)
}

fn path_extension(path: &[u8]) -> Vec<u8> {
    let name = path
        .iter()
        .rposition(|byte| *byte == b'/')
        .map_or(path, |index| &path[index + 1..]);
    let Some(index) = name.iter().rposition(|byte| *byte == b'.') else {
        return Vec::new();
    };
    name[index..].to_vec()
}

fn has_extension(extension: &[u8], candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate.as_bytes()))
}

/// Validate a media asset URL, returning the supplied message for a blank URL.
#[must_use]
pub fn validate_media_asset_url(input: &MediaAssetFormInput, message: &str) -> String {
    if input.url.trim().is_empty() {
        return message.trim().to_owned();
    }
    String::new()
}

/// Apply one form input to a record created by the supplied creator identity.
#[must_use]
pub fn record_from_form(input: &MediaAssetFormInput, created_by: &str) -> MediaAssetRecord {
    let record = MediaAssetRecord {
        created_by: created_by.trim().to_owned(),
        ..MediaAssetRecord::default()
    };
    apply_media_asset_form_record(record, input)
}

/// Apply one form input to an existing record.
#[must_use]
pub fn apply_media_asset_form_record(
    mut record: MediaAssetRecord,
    input: &MediaAssetFormInput,
) -> MediaAssetRecord {
    record.kind = normalize_media_kind(&input.kind);
    record.source_kind = normalize_media_source_kind(&input.source_kind);
    record.status = normalize_media_status(&input.status);
    record.url = input.url.trim().to_owned();
    record.provider = resolve_media_asset_form_provider(input);
    record.title = input.title.trim().to_owned();
    record.description = input.description.trim().to_owned();
    record.thumbnail_url = input.thumbnail_url.trim().to_owned();
    record.alt = input.alt.trim().to_owned();
    record.duration = input.duration.clone();
    record
}

/// Prepare a record for creation.
///
/// The identity generator runs only when the normalized identity is blank, and the
/// Unix-time source runs once after normalization.
#[must_use]
pub fn prepare_record_for_create<E: Send + 'static, R: Send + Sync + 'static>(
    record: MediaAssetRecord,
    new_id: Option<&MediaAssetIdGenerator>,
    now_unix: Option<&MediaAssetNowUnix>,
) -> Effect<MediaAssetRecord, E, R> {
    let new_id = new_id.cloned();
    let now_unix = now_unix.cloned();
    effect_sync! { body: move || prepare_record_for_create_inner(record.clone(), new_id.as_ref(), now_unix.as_ref()) }
}

fn prepare_record_for_create_inner(
    record: MediaAssetRecord,
    new_id: Option<&MediaAssetIdGenerator>,
    now_unix: Option<&MediaAssetNowUnix>,
) -> MediaAssetRecord {
    let mut record = normalize_media_asset_store_record(record);
    if record.id.is_empty() {
        record.id = media_store_new_id(new_id).into();
    }
    let now = media_store_now(now_unix);
    if record.created_at <= 0 {
        record.created_at = now;
    }
    if record.updated_at <= 0 {
        record.updated_at = now;
    }
    record
}

/// Prepare a record for update.
#[must_use]
pub fn prepare_record_for_update<E: Send + 'static, R: Send + Sync + 'static>(
    record: MediaAssetRecord,
    now_unix: Option<&MediaAssetNowUnix>,
) -> Effect<MediaAssetRecord, E, R> {
    let now_unix = now_unix.cloned();
    effect_sync! { body: move || prepare_record_for_update_inner(record.clone(), now_unix.as_ref()) }
}

fn prepare_record_for_update_inner(
    record: MediaAssetRecord,
    now_unix: Option<&MediaAssetNowUnix>,
) -> MediaAssetRecord {
    let mut record = normalize_media_asset_store_record(record);
    record.updated_at = media_store_now(now_unix);
    record
}

/// Normalize one stored asset record.
#[must_use]
pub fn normalize_media_asset_store_record(mut record: MediaAssetRecord) -> MediaAssetRecord {
    record.id = record.id.trim().to_owned();
    record.kind = normalize_media_kind(&record.kind);
    record.source_kind = normalize_media_source_kind(&record.source_kind);
    record.status = normalize_media_status(&record.status);
    record.url = record.url.trim().to_owned();
    record.cdn_url = record.cdn_url.trim().to_owned();
    record.provider = record.provider.trim().to_owned();
    record.title = record.title.trim().to_owned();
    record.description = record.description.trim().to_owned();
    record.thumbnail_url = record.thumbnail_url.trim().to_owned();
    record.alt = record.alt.trim().to_owned();
    record.created_by = record.created_by.trim().to_owned();
    record
}

/// Prepare attachment rows for replacement.
///
/// Rows without an asset identity are skipped. Each kept row gets a trimmed
/// identity, a normalized role, trimmed overrides, its original index as position,
/// and the same creation/update time.
#[must_use]
pub fn prepare_media_attachment_store_records_for_replace<E: Send + 'static, R: Send + Sync + 'static>(
    items: &[MediaAttachmentStoreRecord],
    new_id: Option<&MediaAssetIdGenerator>,
    now_unix: Option<&MediaAssetNowUnix>,
) -> Effect<Vec<PreparedMediaAttachmentStoreRecord>, E, R> {
    let items = items.to_vec();
    let new_id = new_id.cloned();
    let now_unix = now_unix.cloned();
    effect_sync! { body: move || prepare_attachment_records_inner(&items, new_id.as_ref(), now_unix.as_ref()) }
}

fn prepare_attachment_records_inner(
    items: &[MediaAttachmentStoreRecord],
    new_id: Option<&MediaAssetIdGenerator>,
    now_unix: Option<&MediaAssetNowUnix>,
) -> Vec<PreparedMediaAttachmentStoreRecord> {
    if items.is_empty() {
        return Vec::new();
    }
    let now = media_store_now(now_unix);
    let mut prepared = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let mut item = item.clone();
        item.media_asset_id = item.media_asset_id.trim().to_owned();
        if item.media_asset_id.is_empty() {
            continue;
        }
        item.id = item.id.trim().to_owned();
        if item.id.is_empty() {
            item.id = media_store_new_id(new_id).into();
        }
        item.role = normalize_media_attachment_role(&item.role).into();
        item.title_override = item.title_override.trim().to_owned();
        item.caption = item.caption.trim().to_owned();
        item.alt_override = item.alt_override.trim().to_owned();
        prepared.push(PreparedMediaAttachmentStoreRecord {
            record: item,
            position: i64::try_from(index).unwrap_or(i64::MAX),
            created_at: now,
            updated_at: now,
        });
    }
    prepared
}

fn media_store_new_id(new_id: Option<&MediaAssetIdGenerator>) -> String {
    let id = new_id.map(|generate| generate().trim().to_owned()).unwrap_or_default();
    if id.is_empty() { Uuid::new_v4().to_string() } else { id }
}

fn media_store_now(now_unix: Option<&MediaAssetNowUnix>) -> i64 {
    let now = now_unix.map(|clock| clock()).unwrap_or_default();
    if now > 0 { now } else {
        SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_else(|error| -(error.duration().as_secs() as i64))
    }
}
