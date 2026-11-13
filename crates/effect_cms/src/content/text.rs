//! Content text, tag, slug and JSON helpers.
//!
//! The owner supplies pointer/optional strings, tag list decoding and encoding, slug
//! derivation, delimited value splitting, the keyed text-object helpers, the paired
//! delimited JSON helpers and the HTML sanitizer entry point (whose policy and engine
//! are the private `sanitize` module).
//!
//! Details that are easy to lose:
//!
//! - `TagsFromJSON` accepts only a JSON array of strings (or `null`); an empty
//!   array, a malformed document, a non-array document and an array holding a
//!   non-string, non-`null` element all yield no tags. A `null` element is the zero
//!   string, so `[null,"x"]` decodes to
//!   `["","x"]`.
//! - `TagsToJSON` returns `[]` for blank input and encodes the delimited values in
//!   their source order.
//! - JSON output follows the JSON encoder's sorted-key, compact form:
//!   strings escape `"`, `\`, the control characters, `<`, `>`, `&` (as
//!   `\u003c`, `\u003e`, `\u0026`) and `U+2028`/`U+2029`, and object members are
//!   written in sorted key order. `TagsToJSON`, `JSONTextObjectPointer` and
//!   `DelimitedJSONTextPairs` all produce the same bytes.
//! - `Slugify` lowercases, collapses every run of characters outside `[a-z0-9]`
//!   into one `-`, trims leading and trailing separators and keeps at most 120
//!   characters.
//! - `SplitDelimitedValues` splits on newline, carriage return, comma and semicolon
//!   and drops blank values.
//! - `JSONTextObjectValue` and `JSONTextPairValues` return blank values (never an
//!   error) for a malformed document. A `null` member or record slot decodes to the
//!   zero string/empty record instead of rejecting the
//!   document, and a paired record whose two keys are equal keeps the second value,
//!   because a map assignment overwrites the first.
//! - `DelimitedJSONTextPairs` pairs values by index, keeps the later values of the
//!   longer list, skips an index where both values are blank and returns `[]` when
//!   no record survives.
//!
//! Every function here is pure. The HTML sanitizer's policy, URL component adapter
//! and native tokenizer/serializer boundaries are documented in `sanitize`.

use std::collections::BTreeMap;

use super::sanitize::sanitize_html as sanitize_with_content_policy;

/// The longest slug [`slugify`] returns.
const SLUG_MAX_CHARS: usize = 120;

/// Dereference an optional string, or the empty string.
#[must_use]
pub fn ptr_or_empty(value: Option<&str>) -> String {
    value.unwrap_or_default().to_owned()
}

/// Dereference a trimmed optional string, or the empty string.
#[must_use]
pub fn string_value(value: Option<&str>) -> String {
    ptr_or_empty(value).trim().to_owned()
}

/// Wrap a non-blank trimmed string, or no value.
#[must_use]
pub fn optional_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Decode a stored tag list.
///
/// `null`, an empty array, a malformed document, a non-array document and an array
/// holding an incompatible element all yield no tags. A `null` element is the zero
/// string, so it is kept as the empty string rather than rejecting the document.
#[must_use]
pub fn tags_from_json(tags_json: &str) -> Vec<String> {
    if tags_json.is_empty() || tags_json == "[]" {
        return Vec::new();
    }
    let Some(document) = crate::json_text::normalize_json_string_tokens(tags_json.as_bytes())
    else {
        return Vec::new();
    };
    match serde_json::from_slice::<Option<Vec<Option<String>>>>(&document) {
        Ok(Some(tags)) => tags
            .into_iter()
            .map(std::option::Option::unwrap_or_default)
            .collect(),
        Ok(None) | Err(_) => Vec::new(),
    }
}

/// Render a stored tag list for display, comma-and-space separated.
#[must_use]
pub fn tags_to_string(tags_json: &str) -> String {
    tags_from_json(tags_json).join(", ")
}

/// Encode delimited tags as a stored JSON tag list.
///
/// Blank input stores `[]`; otherwise the delimited values are encoded in source
/// order.
#[must_use]
pub fn tags_to_json(raw: &str) -> String {
    if raw.trim().is_empty() {
        return "[]".to_owned();
    }
    let tags = split_delimited_values(raw);
    let mut encoded = String::from("[");
    for (index, tag) in tags.iter().enumerate() {
        if index > 0 {
            encoded.push(',');
        }
        push_json_string(&mut encoded, tag);
    }
    encoded.push(']');
    encoded
}

/// Sanitize author-supplied HTML with the content policy.
#[must_use]
pub fn sanitize_html(html: &str) -> String {
    sanitize_with_content_policy(html)
}

/// Derive a stable slug from a title.
#[must_use]
pub fn slugify(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut separator_pending = false;
    for character in title
        .chars()
        .map(|ch| ch.to_lowercase().next().unwrap_or(ch))
    {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            if separator_pending && !slug.is_empty() {
                slug.push('-');
            }
            separator_pending = false;
            slug.push(character);
        } else {
            separator_pending = true;
        }
    }
    slug.chars().take(SLUG_MAX_CHARS).collect()
}

/// Build a filename from a slug and an extension.
///
/// A blank slug falls back to the trimmed fallback; an extension gains a leading
/// `.` when it has none.
#[must_use]
pub fn slug_filename(slug: &str, extension: &str, fallback: &str) -> String {
    let slug = slug.trim();
    if slug.is_empty() {
        return fallback.trim().to_owned();
    }
    let extension = extension.trim();
    let mut filename = String::with_capacity(slug.len() + extension.len() + 1);
    filename.push_str(slug);
    if extension.is_empty() {
        return filename;
    }
    if !extension.starts_with('.') {
        filename.push('.');
    }
    filename.push_str(extension);
    filename
}

/// Split newline-, comma- and semicolon-separated values.
#[must_use]
pub fn split_delimited_values(raw: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    for character in raw.chars() {
        if matches!(character, '\n' | '\r' | ',' | ';') {
            push_trimmed(&mut values, &current);
            current.clear();
            continue;
        }
        current.push(character);
    }
    push_trimmed(&mut values, &current);
    values
}

/// Encode one keyed text object.
///
/// A blank value or key yields no document.
#[must_use]
pub fn json_text_object_pointer(raw: &str, key: &str) -> Option<String> {
    let raw = raw.trim();
    let key = key.trim();
    if raw.is_empty() || key.is_empty() {
        return None;
    }
    let mut encoded = String::from("{");
    push_json_string(&mut encoded, key);
    encoded.push(':');
    push_json_string(&mut encoded, raw);
    encoded.push('}');
    Some(encoded)
}

/// Decode one keyed text object.
///
/// A `null` member is the zero string, so it decodes to the empty value instead of
/// rejecting the document; a genuinely incompatible member type still rejects it.
#[must_use]
pub fn json_text_object_value(raw: &str, key: &str) -> String {
    let raw = raw.trim();
    let key = key.trim();
    if raw.is_empty() || key.is_empty() {
        return String::new();
    }
    let Some(document) = crate::json_text::normalize_json_string_tokens(raw.as_bytes()) else {
        return String::new();
    };
    match serde_json::from_slice::<BTreeMap<String, Option<String>>>(&document) {
        Ok(payload) => payload
            .get(key)
            .and_then(std::option::Option::as_deref)
            .map_or_else(String::new, |value| value.trim().to_owned()),
        Err(_) => String::new(),
    }
}

/// Encode paired delimited values as a JSON record list.
///
/// Values are paired by index; an index whose two values are both blank is skipped
/// and the result is `[]` when no record survives or both keys are blank.
#[must_use]
pub fn delimited_json_text_pairs(
    first_raw: &str,
    second_raw: &str,
    first_key: &str,
    second_key: &str,
) -> String {
    let first_key = first_key.trim();
    let second_key = second_key.trim();
    if first_key.is_empty() && second_key.is_empty() {
        return "[]".to_owned();
    }
    let first_values = split_delimited_values(first_raw);
    let second_values = split_delimited_values(second_raw);
    let max_len = first_values.len().max(second_values.len());
    if max_len == 0 {
        return "[]".to_owned();
    }
    let mut records: Vec<BTreeMap<&str, &str>> = Vec::new();
    for index in 0..max_len {
        let first = string_at(&first_values, index);
        let second = string_at(&second_values, index);
        if first.is_empty() && second.is_empty() {
            continue;
        }
        // One string map is built per record, so two equal keys
        // keep the second value instead of emitting a duplicate JSON member.
        let mut record: BTreeMap<&str, &str> = BTreeMap::new();
        if !first_key.is_empty() && !first.is_empty() {
            record.insert(first_key, first);
        }
        if !second_key.is_empty() && !second.is_empty() {
            record.insert(second_key, second);
        }
        records.push(record);
    }
    if records.is_empty() {
        return "[]".to_owned();
    }
    let mut encoded = String::from("[");
    for (index, record) in records.iter().enumerate() {
        if index > 0 {
            encoded.push(',');
        }
        encoded.push_str(&json_object(record));
    }
    encoded.push(']');
    encoded
}

/// Decode paired JSON text values.
///
/// The first and second values of every record are trimmed, blanks are dropped and
/// each side is returned newline-separated. A malformed document yields two blank
/// values.
#[must_use]
pub fn json_text_pair_values(raw: &str, first_key: &str, second_key: &str) -> (String, String) {
    let raw = raw.trim();
    let first_key = first_key.trim();
    let second_key = second_key.trim();
    if raw.is_empty() || (first_key.is_empty() && second_key.is_empty()) {
        return (String::new(), String::new());
    }
    let Some(document) = crate::json_text::normalize_json_string_tokens(raw.as_bytes()) else {
        return (String::new(), String::new());
    };
    let payload: Vec<BTreeMap<String, String>> =
        match serde_json::from_slice::<Vec<Option<BTreeMap<String, Option<String>>>>>(&document) {
            // A string-map collection accepts a `null` record and a `null` member;
            // both are the zero value there, so they are dropped here before the
            // shared blank-dropping loop.
            Ok(payload) => payload
                .into_iter()
                .flatten()
                .map(|record| {
                    record
                        .into_iter()
                        .filter_map(|(key, value)| value.map(|value| (key, value)))
                        .collect()
                })
                .collect(),
            Err(_) => return (String::new(), String::new()),
        };
    let mut first_values: Vec<String> = Vec::new();
    let mut second_values: Vec<String> = Vec::new();
    for item in &payload {
        if !first_key.is_empty()
            && let Some(value) = item.get(first_key)
        {
            let value = value.trim();
            if !value.is_empty() {
                first_values.push(value.to_owned());
            }
        }
        if !second_key.is_empty()
            && let Some(value) = item.get(second_key)
        {
            let value = value.trim();
            if !value.is_empty() {
                second_values.push(value.to_owned());
            }
        }
    }
    (first_values.join("\n"), second_values.join("\n"))
}

/// Append one trimmed value when it is not blank.
fn push_trimmed(values: &mut Vec<String>, value: &str) {
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        values.push(trimmed.to_owned());
    }
}

/// The trimmed value at an index, or the empty string.
fn string_at(values: &[String], index: usize) -> &str {
    values
        .get(index)
        .map(|value| value.trim())
        .unwrap_or_default()
}

/// Encode one JSON object with its members in sorted key order.
fn json_object(pairs: &BTreeMap<&str, &str>) -> String {
    let mut encoded = String::from("{");
    for (index, (key, value)) in pairs.iter().enumerate() {
        if index > 0 {
            encoded.push(',');
        }
        push_json_string(&mut encoded, key);
        encoded.push(':');
        push_json_string(&mut encoded, value);
    }
    encoded.push('}');
    encoded
}

/// Append one JSON-encoded string.
///
/// The encoder escapes the quote and the reverse solidus, uses the short forms for
/// newline, carriage return and tab, `\u00XX` for the remaining control characters,
/// and escapes `<`, `>`, `&` and the two JavaScript line separators so the output is
/// safe to embed in HTML.
fn push_json_string(encoded: &mut String, value: &str) {
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            '\u{0008}' => encoded.push_str("\\b"),
            '\u{000c}' => encoded.push_str("\\f"),
            '<' => encoded.push_str("\\u003c"),
            '>' => encoded.push_str("\\u003e"),
            '&' => encoded.push_str("\\u0026"),
            '\u{2028}' => encoded.push_str("\\u2028"),
            '\u{2029}' => encoded.push_str("\\u2029"),
            other if (other as u32) < 0x20 => {
                let code = other as u32;
                encoded.push_str("\\u00");
                push_lower_hex_byte(encoded, u8::try_from(code).unwrap_or_default());
            }
            other => encoded.push(other),
        }
    }
    encoded.push('"');
}

/// Append one byte as two lowercase hexadecimal digits.
fn push_lower_hex_byte(encoded: &mut String, byte: u8) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let high = usize::from(byte >> 4);
    let low = usize::from(byte & 0x0f);
    if let Some(digit) = DIGITS.get(high) {
        encoded.push(char::from(*digit));
    }
    if let Some(digit) = DIGITS.get(low) {
        encoded.push(char::from(*digit));
    }
}
