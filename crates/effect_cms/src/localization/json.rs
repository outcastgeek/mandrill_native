//! Localized description JSON: decoding and encoding.
//!
//! Localized description JSON has this decoding and encoding contract:
//!
//! - **Decoding.** Blank-or-whitespace input yields no values. Invalid JSON yields
//!   no values because the whole document is validated before decoding. A valid
//!   document whose top level is not an object yields no values. Inside an object,
//!   a member whose value is not a JSON string is stored as an empty string: the
//!   decoder records the type error, continues, and writes the zero value.
//!   Duplicate member names keep the last value, as successive map assignments do.
//! - **Encoding.** Keys are ordered in byte order, only trimmed-nonempty
//!   descriptions are written, and no usable data encodes as `{}`. Strings use
//!   JSON escaping with HTML-safe escapes for `<`, `>`, `&`, U+2028 and U+2029 and
//!   the `\u00XX` form for the remaining control characters. This is JSON encoding,
//!   not HTML.
//!
//! # Nesting depth and stack safety
//!
//! A document nesting deeper than `MAX_NESTING_DEPTH` (10,000) is rejected during
//! validation, so a deeper document yields no values. That contract is preserved
//! explicitly by the structural pre-check below.
//!
//! Decoding **never builds a recursive value tree**, so no deep recursive drop can
//! occur on any path:
//!
//! - the top-level object is streamed member by member by a `serde` map visitor;
//! - a member whose value is a JSON string is decoded into an owned `String`;
//! - every other member value is skipped with `serde::de::IgnoredAny`, which routes
//!   to `serde_json`'s `deserialize_ignored_any`/`ignore_value` scanner — an
//!   iterative loop over its own `Vec<u8>` closing-delimiter stack that neither
//!   recurses nor materializes the skipped data (nested objects and arrays at any
//!   accepted depth included);
//! - a non-object top level is a type error and yields no values, before any
//!   nested content is read.
//!
//! Consequently parsing, skipping, kept-string decoding and cleanup are all
//! independent of document depth: the only values dropped are flat
//! `BTreeMap<String, String>`/`String` data plus the deserializer's own scratch
//! buffer. Nothing in this module relies on a stack-growing deserializer, because
//! no recursive construction happens at all; `serde_json`'s own 128-level recursion
//! guard is never reached either, because nested data reaches the scanner only
//! through the iterative ignore path.
//!

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use serde::de::{Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::byte_text::encoding_text;
use crate::validation::trim_unicode_whitespace;

use super::drafts::LocalizedTextDraft;
use crate::json_text::normalize_json_string_tokens;
use super::normalizer::{NormalizerRef, normalized_language};

/// Decode localized descriptions into one text draft per requested language.
///
/// Each draft keeps the requested language, is required, and takes the stored
/// description for the *normalized* language, falling back to the stored
/// description for the trimmed language when that value is empty or absent.
/// Stored descriptions are not trimmed.
#[must_use]
pub fn localized_text_drafts_from_description_json(
    langs: &[String],
    raw: impl AsRef<[u8]>,
    normalize: Option<NormalizerRef<'_>>,
) -> Vec<LocalizedTextDraft> {
    let values = decode_description_values(raw.as_ref());
    langs
        .iter()
        .map(|lang| {
            let normalized = normalized_language(lang, normalize);
            let description = values
                .get(&normalized)
                .filter(|value| !value.is_empty())
                .or_else(|| values.get(lang.trim()))
                .cloned()
                .unwrap_or_default();
            LocalizedTextDraft {
                lang: lang.clone(),
                description: description.into(),
                required: true,
            }
        })
        .collect()
}

/// Encode localized descriptions as JSON.
///
/// Only trimmed-nonempty descriptions are written, keyed by the normalized
/// language; a later draft for the same normalized language replaces an earlier
/// one, and no usable data encodes as `{}`. A marshal-error branch cannot occur for
/// a string map and is therefore not modelled as an error path.
#[must_use]
pub fn serialize_localized_text_draft_descriptions(
    drafts: &[LocalizedTextDraft],
    normalize: Option<NormalizerRef<'_>>,
) -> String {
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    for draft in drafts {
        let description = trim_unicode_whitespace(draft.description.as_bytes());
        if !description.is_empty() {
            values.insert(
                normalized_language(&draft.lang, normalize),
                encoding_text(description),
            );
        }
    }
    if values.is_empty() {
        return "{}".to_owned();
    }
    encode_string_map(&values)
}

/// The nesting limit for one JSON document.
const MAX_NESTING_DEPTH: usize = 10_000;

/// Whether the document nests deeper than the decoder accepts.
///
/// A document whose structural nesting exceeds `MAX_NESTING_DEPTH` is rejected
/// while validating it, before any value is produced. The scan below mirrors that
/// structural measure: only `{`/`[` open a level, brackets inside strings are not
/// structural, and escaped quotes do not close a string. Malformed input is not
/// this check's subject — the decoder rejects it separately.
fn exceeds_nesting_limit(raw: &[u8]) -> bool {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in raw {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return true;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// One decoded description document.
struct DescriptionDocument {
    /// The decoded string members.
    values: BTreeMap<String, String>,
}

impl<'de> Deserialize<'de> for DescriptionDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(DescriptionVisitor)
    }
}

/// Reads the top-level object only.
///
/// Every other document shape is a type error and yields no values, so only
/// `visit_map` is provided; `deserialize_any` on a non-object reaches a default
/// visitor method, which returns the type error without reading nested content.
struct DescriptionVisitor;

impl<'de> Visitor<'de> for DescriptionVisitor {
    type Value = DescriptionDocument;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON description object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            let member = map.next_value::<DescriptionMember>()?;
            values.insert(key, member.value);
        }
        Ok(DescriptionDocument { values })
    }
}

/// One member value: a JSON string is kept, anything else is the zero value.
struct DescriptionMember {
    /// The kept string, or an empty string for a member that cannot type-check.
    value: String,
}

impl DescriptionMember {
    /// The zero value written for a member that cannot decode into a string.
    fn empty() -> Self {
        Self {
            value: String::new(),
        }
    }
}

impl<'de> Deserialize<'de> for DescriptionMember {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(MemberVisitor)
    }
}

/// Keeps JSON strings and skips every other value iteratively.
///
/// No branch descends recursively: compound values are drained through
/// `IgnoredAny`, whose deserialization is `serde_json`'s iterative
/// `deserialize_ignored_any` scanner, so an arbitrarily nested ignored member is
/// neither built as a tree nor dropped recursively.
struct MemberVisitor;

impl<'de> Visitor<'de> for MemberVisitor {
    type Value = DescriptionMember;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DescriptionMember {
            value: value.to_owned(),
        })
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DescriptionMember { value })
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DescriptionMember::empty())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DescriptionMember::empty())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DescriptionMember::empty())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DescriptionMember::empty())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DescriptionMember::empty())
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(DescriptionMember::empty())
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<String, IgnoredAny>()?.is_some() {}
        Ok(DescriptionMember::empty())
    }
}

/// Whether the document's first structural byte opens an object.
///
/// A string-map decoder accepts only an object (and treats `null` as no data), so
/// every other shape is an ignored type error. Checking the shape first also keeps
/// a non-object document from being parsed member by member under a number
/// representation that is not a JSON object.
fn starts_object_document(raw: &[u8]) -> bool {
    raw.iter().copied().find(|byte| !matches!(*byte, b' ' | b'\t' | b'\r' | b'\n')) == Some(b'{')
}

/// Decode one complete JSON document into its string members.
///
/// The trailing-content check rejects any data after the top-level value. A failed
/// decode yields no values, and only flat string data is dropped on that path.
fn decode_description_values(raw: &[u8]) -> BTreeMap<String, String> {
    if !starts_object_document(raw) || exceeds_nesting_limit(raw) {
        return BTreeMap::new();
    }
    let Some(compatible) = normalize_json_string_tokens(raw) else { return BTreeMap::new(); };
    let mut deserializer = serde_json::Deserializer::from_slice(&compatible);
    let decoded = DescriptionDocument::deserialize(&mut deserializer)
        .and_then(|document| deserializer.end().map(|()| document));
    match decoded {
        Ok(document) => document.values,
        Err(_error) => BTreeMap::new(),
    }
}

fn encode_string_map(values: &BTreeMap<String, String>) -> String {
    let mut encoded = String::from("{");
    for (position, (key, value)) in values.iter().enumerate() {
        if position > 0 {
            encoded.push(',');
        }
        push_json_string(&mut encoded, key);
        encoded.push(':');
        push_json_string(&mut encoded, value);
    }
    encoded.push('}');
    encoded
}

fn push_json_string(encoded: &mut String, value: &str) {
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{8}' => encoded.push_str("\\b"),
            '\u{c}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            '<' => encoded.push_str("\\u003c"),
            '>' => encoded.push_str("\\u003e"),
            '&' => encoded.push_str("\\u0026"),
            '\u{2028}' => encoded.push_str("\\u2028"),
            '\u{2029}' => encoded.push_str("\\u2029"),
            control if u32::from(control) < 0x20 => {
                push_unicode_escape(encoded, u32::from(control));
            }
            other => encoded.push(other),
        }
    }
    encoded.push('"');
}

fn push_unicode_escape(encoded: &mut String, code: u32) {
    // Writing into a `String` cannot fail; the result is discarded explicitly.
    let _ = write!(encoded, "\\u{code:04x}");
}
