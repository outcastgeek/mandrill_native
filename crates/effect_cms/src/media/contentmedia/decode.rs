//! Ordered updates for stored media JSON objects.

use super::{ContentMediaItem, ContentVideo, ContentVideoMetadata, ContentVideoProvider};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::fmt;

fn field_key<'a>(key: &str, storage: &'a mut [u8; 16]) -> &'a str {
    let mut length = 0;
    for character in key.chars() {
        let byte = match character {
            '\u{212a}' => b'k',
            '\u{017f}' => b's',
            ascii if ascii.is_ascii() => (ascii as u8).to_ascii_lowercase(),
            _ => return "",
        };
        let Some(destination) = storage.get_mut(length) else {
            return "";
        };
        *destination = byte;
        length += 1;
    }
    storage
        .get(..length)
        .and_then(|key| std::str::from_utf8(key).ok())
        .unwrap_or("")
}

trait Object {
    fn read_map<'de, M: MapAccess<'de>>(&mut self, map: M) -> Result<(), M::Error>;
}

struct ObjectSeed<'a, T>(&'a mut T);
impl<'de, T: Object> DeserializeSeed<'de> for ObjectSeed<'_, T> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_option(self)
    }
}
impl<'de, T: Object> Visitor<'de> for ObjectSeed<'_, T> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a stored media object or null")
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_map(self)
    }
    fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<(), M::Error> {
        self.0.read_map(map)
    }
}

macro_rules! object_fields {
    ($object:ty, {$($key:literal => $field:ident : $value:ty),* $(,)?}) => {
        impl Object for $object {
            fn read_map<'de, M: MapAccess<'de>>(&mut self, mut map: M) -> Result<(), M::Error> {
                while let Some(key) = map.next_key::<String>()? {
                    match field_key(&key, &mut [0; 16]) {
                        $($key => if let Some(value) = map.next_value::<Option<$value>>()? {
                            self.$field = value;
                        },)*
                        _ => { map.next_value::<IgnoredAny>()?; }
                    }
                }
                Ok(())
            }
        }
    };
}

object_fields!(ContentVideoProvider, {
    "name" => name: crate::commands::InputBytes,
    "base-url" => base_url: String,
});
object_fields!(ContentVideoMetadata, {
    "description" => description: crate::commands::InputBytes,
    "thumbnail" => thumbnail: crate::commands::InputBytes,
    "duration" => duration: i64,
    "width" => width: i64,
    "height" => height: i64,
    "provider-id" => provider_id: String,
    "provider-url" => provider_url: String,
    "published-at" => published_at: String,
});
object_fields!(ContentMediaItem, {
    "id" => id: crate::commands::InputBytes,
    "role" => role: String,
    "type" => kind: String,
    "url" => url: crate::commands::InputBytes,
    "thumbnail" => thumbnail: crate::commands::InputBytes,
    "alt" => alt: crate::commands::InputBytes,
    "title" => title: crate::commands::InputBytes,
    "description" => description: crate::commands::InputBytes,
    "provider" => provider: crate::commands::InputBytes,
    "duration" => duration: i64,
    "width" => width: i64,
    "height" => height: i64,
});

struct VideoReader<'a> {
    video: &'a mut ContentVideo,
    tags: Option<&'a mut TagBudget>,
}

struct TagBudget {
    limit: usize,
    full: bool,
}

impl Object for VideoReader<'_> {
    fn read_map<'de, M: MapAccess<'de>>(&mut self, mut map: M) -> Result<(), M::Error> {
        let mut tags_length = self.video.tags.len();
        while let Some(key) = map.next_key::<String>()? {
            match field_key(&key, &mut [0; 16]) {
                "id" => {
                    if let Some(value) = map.next_value()? {
                        self.video.id = value;
                    }
                }
                "title" => {
                    if let Some(value) = map.next_value()? {
                        self.video.title = value;
                    }
                }
                "url" => {
                    if let Some(value) = map.next_value()? {
                        self.video.url = value;
                    }
                }
                "series" => {
                    if let Some(value) = map.next_value()? {
                        self.video.series = value;
                    }
                }
                "provider" => map.next_value_seed(ObjectSeed(&mut self.video.provider))?,
                "metadata" => map.next_value_seed(ObjectSeed(&mut self.video.metadata))?,
                "tags" => map.next_value_seed(TagsSeed {
                    values: &mut self.video.tags,
                    length: &mut tags_length,
                    budget: self.tags.as_deref_mut(),
                })?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        self.video.tags.truncate(tags_length);
        Ok(())
    }
}

struct TagsSeed<'a> {
    values: &'a mut Vec<String>,
    length: &'a mut usize,
    budget: Option<&'a mut TagBudget>,
}
impl<'de> DeserializeSeed<'de> for TagsSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_option(self)
    }
}
impl<'de> Visitor<'de> for TagsSeed<'_> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string array or null")
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        self.values.clear();
        *self.length = 0;
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_seq(self)
    }
    fn visit_seq<S: SeqAccess<'de>>(mut self, mut sequence: S) -> Result<(), S::Error> {
        let mut length = 0;
        loop {
            if let Some(budget) = self.budget.as_deref_mut()
                && length == budget.limit
            {
                if sequence.next_element::<IgnoredAny>()?.is_some() {
                    budget.full = true;
                    return Err(serde::de::Error::custom("prepared media tag capacity"));
                }
                break;
            }
            let Some(value) = sequence.next_element::<Option<String>>()? else {
                break;
            };
            if length == self.values.len() {
                self.values.push(String::new());
            }
            if let Some(value) = value {
                let destination = self
                    .values
                    .get_mut(length)
                    .ok_or_else(|| serde::de::Error::custom("media tag position"))?;
                *destination = value;
            }
            length += 1;
        }
        if length == 0 {
            self.values.clear();
        }
        *self.length = length;
        Ok(())
    }
}

struct ItemsSeed<'a>(&'a mut Option<Vec<ContentMediaItem>>);
impl<'de> DeserializeSeed<'de> for ItemsSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_option(self)
    }
}
impl<'de> Visitor<'de> for ItemsSeed<'_> {
    type Value = ();
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a stored media array or null")
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_seq(self)
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut sequence: S) -> Result<(), S::Error> {
        let mut items = Vec::new();
        loop {
            let mut item = ContentMediaItem::default();
            if sequence.next_element_seed(ObjectSeed(&mut item))?.is_none() {
                break;
            }
            items.push(item);
        }
        *self.0 = Some(items);
        Ok(())
    }
}

fn document(raw: &[u8]) -> Vec<u8> {
    let repaired =
        crate::json_text::normalize_json_string_tokens(raw).unwrap_or_else(|| raw.to_vec());
    crate::json_text::normalize_integer_negative_zero(&repaired)
}

pub(super) fn video(raw: &[u8]) -> Result<ContentVideo, serde_json::Error> {
    let document = document(raw);
    video_document(&document)
}

pub(super) fn video_document(document: &[u8]) -> Result<ContentVideo, serde_json::Error> {
    let mut decoder = serde_json::Deserializer::from_slice(document);
    let mut video = ContentVideo::default();
    ObjectSeed(&mut VideoReader {
        video: &mut video,
        tags: None,
    })
    .deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(video)
}

pub(super) fn bounded_video_document(
    document: &[u8],
    maximum_tags: usize,
) -> Result<ContentVideo, super::PreparedMediaError> {
    let mut decoder = serde_json::Deserializer::from_slice(document);
    let mut video = ContentVideo::default();
    let mut tags = TagBudget {
        limit: maximum_tags,
        full: false,
    };
    let result = ObjectSeed(&mut VideoReader {
        video: &mut video,
        tags: Some(&mut tags),
    })
    .deserialize(&mut decoder);
    if tags.full {
        return Err(super::PreparedMediaError::Memory(
            effect::memory::MemoryError::NoSlot,
        ));
    }
    result.map_err(super::PreparedMediaError::Json)?;
    decoder.end().map_err(super::PreparedMediaError::Json)?;
    Ok(video)
}

pub(super) fn items(raw: &[u8]) -> Result<Option<Vec<ContentMediaItem>>, serde_json::Error> {
    let document = document(raw);
    let mut decoder = serde_json::Deserializer::from_slice(&document);
    let mut items = None;
    ItemsSeed(&mut items).deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(items)
}

struct FixedItems<'a, 'memory> {
    output: &'a mut effect::memory::FixedVector<'memory, ContentMediaItem>,
    full: &'a mut bool,
}
impl<'de> DeserializeSeed<'de> for FixedItems<'_, '_> {
    type Value = bool;

    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<bool, D::Error> {
        decoder.deserialize_option(self)
    }
}
impl<'de> Visitor<'de> for FixedItems<'_, '_> {
    type Value = bool;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded stored media array or null")
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<bool, E> {
        Ok(false)
    }

    fn visit_some<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<bool, D::Error> {
        decoder.deserialize_seq(self)
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut sequence: S) -> Result<bool, S::Error> {
        while self.output.len() < self.output.capacity() {
            let mut item = ContentMediaItem::default();
            if sequence.next_element_seed(ObjectSeed(&mut item))?.is_none() {
                return Ok(true);
            }
            self.output.push(item).map_err(|_| {
                *self.full = true;
                serde::de::Error::custom("prepared media item capacity")
            })?;
        }
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            *self.full = true;
            return Err(serde::de::Error::custom("prepared media item capacity"));
        }
        Ok(true)
    }
}

pub(super) fn fixed_items(
    document: &[u8],
    output: &mut effect::memory::FixedVector<'_, ContentMediaItem>,
) -> Result<bool, super::PreparedMediaError> {
    let mut decoder = serde_json::Deserializer::from_slice(document);
    let mut full = false;
    let result = FixedItems {
        output,
        full: &mut full,
    }
    .deserialize(&mut decoder);
    if full {
        return Err(super::PreparedMediaError::Memory(
            effect::memory::MemoryError::NoSlot,
        ));
    }
    let present = result.map_err(super::PreparedMediaError::Json)?;
    decoder.end().map_err(super::PreparedMediaError::Json)?;
    Ok(present)
}
