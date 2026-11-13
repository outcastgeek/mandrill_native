//! Media contract payload projections.

use super::view_text;

use std::collections::BTreeMap;

use crate::attachments::normalize_media_attachment_role;
use crate::media::contentmedia::{ContentMediaItem, ContentVideo};

/// Project one stored hero video into the content contract map.
#[must_use]
pub fn contract_video_value(video: Option<&ContentVideo>) -> Option<BTreeMap<String, serde_json::Value>> {
    let video = video?;
    let mut value = BTreeMap::new();
    value.insert("id".to_owned(), serde_json::Value::String(view_text(video.id.trim().to_owned())));
    value.insert(
        "title".to_owned(),
        serde_json::Value::String(view_text(video.title.trim().to_owned())),
    );
    value.insert("url".to_owned(), serde_json::Value::String(view_text(video.url.trim().to_owned())));
    let mut provider = BTreeMap::new();
    provider.insert(
        "name".to_owned(),
        serde_json::Value::String(view_text(video.provider.name.trim().to_owned())),
    );
    provider.insert(
        "base-url".to_owned(),
        serde_json::Value::String(video.provider.base_url.trim().to_owned()),
    );
    value.insert(
        "provider".to_owned(),
        serde_json::Value::Object(provider.into_iter().collect()),
    );
    let mut metadata = BTreeMap::new();
    if !video.metadata.description.trim().is_empty() {
        metadata.insert(
            "description".to_owned(),
            serde_json::Value::String(view_text(video.metadata.description.trim().to_owned())),
        );
    }
    if !video.metadata.thumbnail.trim().is_empty() {
        metadata.insert(
            "thumbnail".to_owned(),
            serde_json::Value::String(view_text(video.metadata.thumbnail.trim().to_owned())),
        );
    }
    if video.metadata.duration > 0 {
        metadata.insert(
            "duration".to_owned(),
            serde_json::Value::Number(video.metadata.duration.into()),
        );
    }
    if !video.metadata.provider_id.trim().is_empty() {
        metadata.insert(
            "provider-id".to_owned(),
            serde_json::Value::String(video.metadata.provider_id.trim().to_owned()),
        );
    }
    if !video.metadata.provider_url.trim().is_empty() {
        metadata.insert(
            "provider-url".to_owned(),
            serde_json::Value::String(video.metadata.provider_url.trim().to_owned()),
        );
    }
    if !video.metadata.published_at.trim().is_empty() {
        metadata.insert(
            "published-at".to_owned(),
            serde_json::Value::String(video.metadata.published_at.trim().to_owned()),
        );
    }
    if video.metadata.width > 0 {
        metadata.insert(
            "width".to_owned(),
            serde_json::Value::Number(video.metadata.width.into()),
        );
    }
    if video.metadata.height > 0 {
        metadata.insert(
            "height".to_owned(),
            serde_json::Value::Number(video.metadata.height.into()),
        );
    }
    value.insert(
        "metadata".to_owned(),
        serde_json::Value::Object(metadata.into_iter().collect()),
    );
    Some(value)
}

/// Project stored media items into the content contract list.
#[must_use]
pub fn contract_media_items_value(
    items: &[ContentMediaItem],
) -> Option<Vec<BTreeMap<String, serde_json::Value>>> {
    if items.is_empty() {
        return None;
    }
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let mut entry = BTreeMap::new();
        entry.insert(
            "type".to_owned(),
            serde_json::Value::String(item.kind.trim().to_owned()),
        );
        entry.insert(
            "url".to_owned(),
            serde_json::Value::String(view_text(item.url.trim())),
        );
        insert_string(&mut entry, "id", &item.id);
        if !item.role.trim().is_empty() {
            entry.insert(
                "role".to_owned(),
                serde_json::Value::String(normalize_media_attachment_role(&item.role)),
            );
        }
        insert_string(&mut entry, "thumbnail", &item.thumbnail);
        insert_string(&mut entry, "alt", &item.alt);
        insert_string(&mut entry, "title", &item.title);
        insert_string(&mut entry, "description", &item.description);
        let provider = crate::byte_text::lowercase(item.provider.trim().as_bytes());
        if !provider.is_empty() {
            entry.insert(
                "provider".to_owned(),
                serde_json::Value::String(provider),
            );
        }
        insert_number(&mut entry, "duration", item.duration);
        insert_number(&mut entry, "width", item.width);
        insert_number(&mut entry, "height", item.height);
        values.push(entry);
    }
    Some(values)
}

fn insert_string(entry: &mut BTreeMap<String, serde_json::Value>, key: &str, value: impl AsRef<[u8]>) {
    let value = crate::validation::trim_unicode_whitespace(value.as_ref());
    if !value.is_empty() {
        entry.insert(
            key.to_owned(),
            serde_json::Value::String(view_text(value)),
        );
    }
}

fn insert_number(entry: &mut BTreeMap<String, serde_json::Value>, key: &str, value: i64) {
    if value > 0 {
        entry.insert(
            key.to_owned(),
            serde_json::Value::Number(value.into()),
        );
    }
}
