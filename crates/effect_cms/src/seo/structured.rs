//! Structured data: schema.org nodes, item lists, video objects and JSON text.
//!
//! The owner supplies the generic collection schema document, the item-list document
//! with its item positions, the caller-provided schema type and property rules, the
//! video object with its optional members, and the Open Graph/Twitter metadata that
//! accompanies a structured video.
//!
//! # Details
//!
//! - A schema object always carries `@context` and `@type`. Property keys are
//!   trimmed; blank keys and absent values are skipped. When string trimming is
//!   requested, string values are trimmed and blank values are skipped.
//! - [`schema_object_json_ld_from_map`] keeps the caller's map as the published map:
//!   it adds the schema context when absent and adds the video object when one is
//!   supplied, and the caller observes those members afterwards.
//! - [`item_list_json_ld`] skips an entry with both a blank URL and a blank name. A
//!   non-positive position becomes the item's current one-based position among the
//!   emitted items.
//! - A video object exists only when a video with a non-blank URL is supplied. Its
//!   thumbnail, upload date and duration members are added only when present or
//!   positive.
//! - The video Twitter card is `player` for a video with a non-blank URL and
//!   `summary_large_image` otherwise.
//!
//! # JSON text
//!
//! Documents are serialized by the workspace JSON facility with the object keys in
//! sorted order and with HTML-sensitive characters in string values escaped as
//! `\u003c`, `\u003e`, `\u0026` and the two JavaScript line separators. The result is
//! JSON text bound for a script element; it is data, not authored markup.

use serde::Serialize;
use serde_json::{Map, Value, ser::Formatter};

/// One rendered meta tag.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MetaTag {
    /// The `name` attribute value.
    #[serde(rename = "Name")]
    pub name: String,
    /// The `property` attribute value.
    #[serde(rename = "Property")]
    pub property: String,
    /// The `content` attribute value.
    #[serde(rename = "Content")]
    pub content: String,
}

/// The structured facts of one video.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StructuredVideo {
    /// The video title.
    pub title: String,
    /// The video description.
    pub description: String,
    /// The video URL used for content and embedding.
    pub url: String,
    /// The thumbnail URL.
    pub thumbnail_url: String,
    /// The publication value.
    pub published_at: String,
    /// The duration in seconds.
    pub duration_seconds: i64,
    /// The player width.
    pub width: i64,
    /// The player height.
    pub height: i64,
}

/// One item-list entry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ItemListEntry {
    /// The declared one-based position; a non-positive value derives one.
    pub position: i64,
    /// The entry URL.
    pub url: String,
    /// The entry name.
    pub name: String,
}

/// Describes one schema object document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SchemaObjectJSONLDInput {
    /// The schema.org type.
    pub schema_type: String,
    /// The caller-provided properties.
    pub props: Map<String, Value>,
    /// The structured video, when the object has one.
    pub video: Option<StructuredVideo>,
    /// Whether string property values are trimmed and blank values skipped.
    pub trim_string: bool,
}

/// Build item-list entries, preserving each row's original one-based position.
///
/// Absent rows or an absent build callback produce no entries. A refused row is
/// skipped, and an entry's non-positive position becomes the row's one-based index.
#[must_use]
pub fn build_item_list_entries<Row>(
    rows: &[Row],
    build: Option<&dyn Fn(usize, &Row) -> (ItemListEntry, bool)>,
) -> Vec<ItemListEntry> {
    let Some(build) = build else {
        return Vec::new();
    };
    if rows.is_empty() {
        return Vec::new();
    }
    let mut entries = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let (mut entry, ok) = build(index, row);
        if !ok {
            continue;
        }
        if entry.position <= 0 {
            entry.position = i64::try_from(index).unwrap_or(i64::MAX).saturating_add(1);
        }
        entries.push(entry);
    }
    entries
}

/// Build one schema object document.
///
/// The context and the caller's trimmed schema type are always present. Every
/// property is read without changing the caller's map.
#[must_use]
pub fn schema_object_json_ld(input: SchemaObjectJSONLDInput) -> String {
    let mut data = Map::new();
    data.insert(
        "@context".to_owned(),
        Value::String("https://schema.org".to_owned()),
    );
    data.insert(
        "@type".to_owned(),
        Value::String(input.schema_type.trim().to_owned()),
    );
    for (key, value) in &input.props {
        let key = key.trim();
        if key.is_empty() || value.is_null() {
            continue;
        }
        if input.trim_string {
            if let Value::String(raw) = value {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    continue;
                }
                data.insert(key.to_owned(), Value::String(trimmed.to_owned()));
                continue;
            }
        }
        data.insert(key.to_owned(), value.clone());
    }
    schema_object_json_ld_from_map(&mut data, input.video.as_ref())
}

/// Build one collection page document.
#[must_use]
pub fn collection_page_json_ld(name: &str, description: &str, url: &str, count: i64) -> String {
    let mut data = Map::new();
    data.insert(
        "@context".to_owned(),
        Value::String("https://schema.org".to_owned()),
    );
    data.insert(
        "@type".to_owned(),
        Value::String("CollectionPage".to_owned()),
    );
    data.insert("name".to_owned(), Value::String(name.trim().to_owned()));
    data.insert(
        "description".to_owned(),
        Value::String(description.trim().to_owned()),
    );
    data.insert("url".to_owned(), Value::String(url.trim().to_owned()));
    data.insert("numberOfItems".to_owned(), Value::from(count));
    json_ld(&data)
}

/// Build one item-list document.
#[must_use]
pub fn item_list_json_ld(name: &str, url: &str, entries: &[ItemListEntry]) -> String {
    let mut items: Vec<Value> = Vec::with_capacity(entries.len());
    for entry in entries {
        let url = entry.url.trim();
        let entry_name = entry.name.trim();
        if url.is_empty() && entry_name.is_empty() {
            continue;
        }
        let position = if entry.position <= 0 {
            i64::try_from(items.len())
                .unwrap_or(i64::MAX)
                .saturating_add(1)
        } else {
            entry.position
        };
        let mut item = Map::new();
        item.insert("@type".to_owned(), Value::String("ListItem".to_owned()));
        item.insert("position".to_owned(), Value::from(position));
        item.insert("url".to_owned(), Value::String(url.to_owned()));
        item.insert("name".to_owned(), Value::String(entry_name.to_owned()));
        items.push(Value::Object(item));
    }
    let mut data = Map::new();
    data.insert(
        "@context".to_owned(),
        Value::String("https://schema.org".to_owned()),
    );
    data.insert("@type".to_owned(), Value::String("ItemList".to_owned()));
    data.insert("name".to_owned(), Value::String(name.trim().to_owned()));
    data.insert("url".to_owned(), Value::String(url.trim().to_owned()));
    data.insert("itemListElement".to_owned(), Value::Array(items));
    json_ld(&data)
}

/// Build the collection-page and item-list documents in that order.
#[must_use]
pub fn collection_list_json_ld_blocks(
    name: &str,
    description: &str,
    url: &str,
    count: i64,
    entries: &[ItemListEntry],
) -> Vec<String> {
    vec![
        collection_page_json_ld(name, description, url, count),
        item_list_json_ld(name, url, entries),
    ]
}

/// Add the schema context and video object to the caller's map and serialize it.
///
/// The caller's map keeps the added members. The video member is added only when a
/// video with a non-blank URL is supplied.
#[must_use]
pub fn schema_object_json_ld_from_map(
    data: &mut Map<String, Value>,
    video: Option<&StructuredVideo>,
) -> String {
    if !data.contains_key("@context") {
        data.insert(
            "@context".to_owned(),
            Value::String("https://schema.org".to_owned()),
        );
    }
    if let Some(video_data) = video_object_json_ld(video) {
        data.insert("video".to_owned(), Value::Object(video_data));
    }
    json_ld(data)
}

/// Build one typed schema node.
///
/// The node always carries the trimmed type. Property keys are trimmed; blank keys,
/// absent values and blank string values are skipped.
#[must_use]
pub fn schema_typed_node(schema_type: &str, props: &Map<String, Value>) -> Map<String, Value> {
    let mut node = Map::new();
    node.insert(
        "@type".to_owned(),
        Value::String(schema_type.trim().to_owned()),
    );
    for (key, value) in props {
        let key = key.trim();
        if key.is_empty() || value.is_null() {
            continue;
        }
        if let Value::String(raw) = value {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            node.insert(key.to_owned(), Value::String(trimmed.to_owned()));
            continue;
        }
        node.insert(key.to_owned(), value.clone());
    }
    node
}

/// Build one video object node.
///
/// An absent video or a video with a blank URL yields no node. The thumbnail, upload
/// date and duration members are added only when present or positive.
#[must_use]
pub fn video_object_json_ld(video: Option<&StructuredVideo>) -> Option<Map<String, Value>> {
    let video = video?;
    let url = video.url.trim();
    if url.is_empty() {
        return None;
    }
    let mut data = Map::new();
    data.insert("@type".to_owned(), Value::String("VideoObject".to_owned()));
    data.insert(
        "name".to_owned(),
        Value::String(video.title.trim().to_owned()),
    );
    data.insert(
        "description".to_owned(),
        Value::String(video.description.trim().to_owned()),
    );
    data.insert("contentUrl".to_owned(), Value::String(url.to_owned()));
    data.insert("embedUrl".to_owned(), Value::String(url.to_owned()));
    let thumbnail = video.thumbnail_url.trim();
    if !thumbnail.is_empty() {
        data.insert(
            "thumbnailUrl".to_owned(),
            Value::String(thumbnail.to_owned()),
        );
    }
    let published = video.published_at.trim();
    if !published.is_empty() {
        data.insert("uploadDate".to_owned(), Value::String(published.to_owned()));
    }
    if video.duration_seconds > 0 {
        data.insert(
            "duration".to_owned(),
            Value::String(format!("PT{}S", video.duration_seconds)),
        );
    }
    Some(data)
}

/// Build the Open Graph video metadata.
///
/// An absent video or a video with a blank URL yields no tags. A positive duration
/// adds its duration tag after the video tag.
#[must_use]
pub fn video_open_graph_meta(video: Option<&StructuredVideo>) -> Vec<MetaTag> {
    let Some(video) = video else {
        return Vec::new();
    };
    let url = video.url.trim();
    if url.is_empty() {
        return Vec::new();
    }
    let mut tags = vec![MetaTag {
        property: "og:video".to_owned(),
        content: url.to_owned(),
        ..MetaTag::default()
    }];
    if video.duration_seconds > 0 {
        tags.push(MetaTag {
            property: "og:video:duration".to_owned(),
            content: video.duration_seconds.to_string(),
            ..MetaTag::default()
        });
    }
    tags
}

/// Resolve the video Twitter card type.
#[must_use]
pub fn video_twitter_card(video: Option<&StructuredVideo>) -> String {
    let Some(video) = video else {
        return "summary_large_image".to_owned();
    };
    if video.url.trim().is_empty() {
        return "summary_large_image".to_owned();
    }
    "player".to_owned()
}

/// Build the Twitter player metadata.
///
/// An absent video or a video with a blank URL yields no tags. Positive width and
/// height add their tags after the player tag.
#[must_use]
pub fn video_twitter_meta(video: Option<&StructuredVideo>) -> Vec<MetaTag> {
    let Some(video) = video else {
        return Vec::new();
    };
    let url = video.url.trim();
    if url.is_empty() {
        return Vec::new();
    }
    let mut tags = vec![MetaTag {
        name: "twitter:player".to_owned(),
        content: url.to_owned(),
        ..MetaTag::default()
    }];
    if video.width > 0 {
        tags.push(MetaTag {
            name: "twitter:player:width".to_owned(),
            content: video.width.to_string(),
            ..MetaTag::default()
        });
    }
    if video.height > 0 {
        tags.push(MetaTag {
            name: "twitter:player:height".to_owned(),
            content: video.height.to_string(),
            ..MetaTag::default()
        });
    }
    tags
}

/// Serialize one JSON object as compact JSON text.
#[must_use]
pub fn json_ld(data: &Map<String, Value>) -> String {
    let mut buffer = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, HtmlSafeFormatter);
    match data.serialize(&mut serializer) {
        Ok(()) => String::from_utf8(buffer).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// A JSON formatter that escapes HTML-sensitive string characters.
///
/// The escaped forms are the `\u003c`, `\u003e`, `\u0026`, `\u2028` and `\u2029`
/// sequences, which keep the JSON text safe to embed in an HTML script element
/// without changing its decoded value.
struct HtmlSafeFormatter;

impl Formatter for HtmlSafeFormatter {
    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        let mut start = 0_usize;
        for (index, character) in fragment.char_indices() {
            let escaped = match character {
                '<' => Some("\\u003c"),
                '>' => Some("\\u003e"),
                '&' => Some("\\u0026"),
                '\u{2028}' => Some("\\u2028"),
                '\u{2029}' => Some("\\u2029"),
                _ => None,
            };
            let Some(escaped) = escaped else {
                continue;
            };
            writer.write_all(fragment.get(start..index).unwrap_or_default().as_bytes())?;
            writer.write_all(escaped.as_bytes())?;
            start = index + character.len_utf8();
        }
        writer.write_all(fragment.get(start..).unwrap_or_default().as_bytes())
    }
}
