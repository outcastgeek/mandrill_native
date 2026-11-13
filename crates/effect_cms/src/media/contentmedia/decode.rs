//! Ordered updates for stored media JSON objects.

use std::fmt;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use super::{ContentMediaItem, ContentVideo, ContentVideoMetadata, ContentVideoProvider};

fn field_key(key: &str) -> String {
    key.chars().map(|character| match character {
        '\u{212a}' => 'k',
        '\u{017f}' => 's',
        _ => character.to_ascii_lowercase(),
    }).collect()
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
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> { Ok(()) }
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
                    match field_key(&key).as_str() {
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

impl Object for ContentVideo {
    fn read_map<'de, M: MapAccess<'de>>(&mut self, mut map: M) -> Result<(), M::Error> {
        let mut tags_length = self.tags.len();
        while let Some(key) = map.next_key::<String>()? {
            match field_key(&key).as_str() {
                "id" => if let Some(value) = map.next_value()? { self.id = value; },
                "title" => if let Some(value) = map.next_value()? { self.title = value; },
                "url" => if let Some(value) = map.next_value()? { self.url = value; },
                "series" => if let Some(value) = map.next_value()? { self.series = value; },
                "provider" => map.next_value_seed(ObjectSeed(&mut self.provider))?,
                "metadata" => map.next_value_seed(ObjectSeed(&mut self.metadata))?,
                "tags" => map.next_value_seed(TagsSeed(&mut self.tags, &mut tags_length))?,
                _ => { map.next_value::<IgnoredAny>()?; }
            }
        }
        self.tags.truncate(tags_length);
        Ok(())
    }
}

struct TagsSeed<'a>(&'a mut Vec<String>, &'a mut usize);
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
        self.0.clear();
        *self.1 = 0;
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_seq(self)
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut sequence: S) -> Result<(), S::Error> {
        let mut length = 0;
        while let Some(value) = sequence.next_element::<Option<String>>()? {
            if length == self.0.len() { self.0.push(String::new()); }
            if let Some(value) = value { self.0[length] = value; }
            length += 1;
        }
        if length == 0 { self.0.clear(); }
        *self.1 = length;
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
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> { Ok(()) }
    fn visit_some<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_seq(self)
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut sequence: S) -> Result<(), S::Error> {
        let mut items = Vec::new();
        loop {
            let mut item = ContentMediaItem::default();
            if sequence.next_element_seed(ObjectSeed(&mut item))?.is_none() { break; }
            items.push(item);
        }
        *self.0 = Some(items);
        Ok(())
    }
}

fn document(raw: &[u8]) -> Vec<u8> {
    let repaired = crate::json_text::normalize_json_string_tokens(raw)
        .unwrap_or_else(|| raw.to_vec());
    crate::json_text::normalize_integer_negative_zero(&repaired)
}

pub(super) fn video(raw: &[u8]) -> Result<ContentVideo, serde_json::Error> {
    let document = document(raw);
    let mut decoder = serde_json::Deserializer::from_slice(&document);
    let mut video = ContentVideo::default();
    ObjectSeed(&mut video).deserialize(&mut decoder)?;
    decoder.end()?;
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
