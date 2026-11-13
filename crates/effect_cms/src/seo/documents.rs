//! Sitemap documents, the sitemap index and sitemap date/time forms.
//!
//! The owner supplies the per-definition sitemap document, the index across the
//! registered definitions, and the lastmod value forms.
//!
//! # Details
//!
//! - A definition contributes an entry only when its sitemap is enabled and its
//!   declared route name is not blank. The document for one definition carries the
//!   requested kind, and contributes no entries when the definition does not qualify.
//! - The index keeps the declared definition order and skips every non-contributing
//!   definition. The entry carries the route name, change frequency and priority
//!   exactly as declared.
//! - A lastmod value is accepted as a calendar date in `YYYY-MM-DD` form, or as an
//!   RFC 3339 date-time with an explicit offset or `Z`; an accepted date-time is
//!   normalized to UTC with second precision. Anything else normalizes to the empty
//!   string.
//! - The formatting helpers return the empty string for an absent time and for the
//!   zero instant; otherwise the date is `YYYY-MM-DD` and the date-time is
//!   `YYYY-MM-DDTHH:MM:SSZ`, both in UTC.
//!
//! # Effects
//!
//! These are pure calculations over caller-supplied definitions and values.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::Serialize;

use crate::definitions::{ResourceDefinition, ResourceKind, SitemapChangeFrequency};

/// One sitemap entry.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SitemapEntryVM {
    /// The contributing resource kind.
    #[serde(rename = "Kind", serialize_with = "serialize_kind")]
    pub kind: ResourceKind,
    /// The declared sitemap route name.
    #[serde(rename = "RouteName")]
    pub route_name: String,
    /// The declared change frequency.
    #[serde(rename = "ChangeFrequency", serialize_with = "serialize_change_frequency")]
    pub change_frequency: SitemapChangeFrequency,
    /// The declared priority.
    #[serde(rename = "Priority")]
    pub priority: f64,
}

/// One sitemap document.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SitemapDocumentVM {
    /// The document's resource kind.
    #[serde(rename = "Kind", serialize_with = "serialize_kind")]
    pub kind: ResourceKind,
    /// The document entries.
    #[serde(rename = "Entries")]
    pub entries: Vec<SitemapEntryVM>,
}

/// The sitemap index.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SitemapIndexVM {
    /// The index entries.
    #[serde(rename = "Entries")]
    pub entries: Vec<SitemapEntryVM>,
}

/// Build the sitemap document for one definition's own kind.
#[must_use]
pub fn definition_sitemap_document(definition: &ResourceDefinition) -> SitemapDocumentVM {
    sitemap_document_for_kind(definition, &definition.kind)
}

/// Build the sitemap document for one definition under a requested kind.
pub(super) fn sitemap_document_for_kind(
    definition: &ResourceDefinition,
    kind: &ResourceKind,
) -> SitemapDocumentVM {
    if !definition.sitemap.enabled || definition.sitemap.route_name.trim().is_empty() {
        return SitemapDocumentVM {
            kind: kind.clone(),
            entries: Vec::new(),
        };
    }
    SitemapDocumentVM {
        kind: kind.clone(),
        entries: vec![sitemap_entry(definition, kind)],
    }
}

/// Build the sitemap index across the supplied definitions.
#[must_use]
pub fn definition_sitemap_index(definitions_list: &[ResourceDefinition]) -> SitemapIndexVM {
    let mut entries = Vec::with_capacity(definitions_list.len());
    for definition in definitions_list {
        if !definition.sitemap.enabled || definition.sitemap.route_name.trim().is_empty() {
            continue;
        }
        entries.push(sitemap_entry(definition, &definition.kind));
    }
    SitemapIndexVM { entries }
}

/// Build one sitemap entry.
fn sitemap_entry(definition: &ResourceDefinition, kind: &ResourceKind) -> SitemapEntryVM {
    SitemapEntryVM {
        kind: kind.clone(),
        route_name: definition.sitemap.route_name.clone(),
        change_frequency: definition.sitemap.change_frequency.clone(),
        priority: definition.sitemap.priority,
    }
}

/// Normalize a sitemap lastmod value.
///
/// A calendar date is returned exactly as supplied once it denotes a valid date in
/// the canonical `YYYY-MM-DD` form. An RFC 3339 date-time with an explicit offset is
/// converted to UTC and formatted with second precision. A blank value, an invalid
/// value or any other form returns the empty string.
#[must_use]
pub fn normalize_sitemap_lastmod(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        if date.format("%Y-%m-%d").to_string() == value {
            return value.to_owned();
        }
    }
    match parse_sitemap_date_time(value) {
        Some(parsed) => format_utc_date_time(parsed),
        None => String::new(),
    }
}

/// Format an instant as a sitemap calendar date.
///
/// An absent value and the zero instant return the empty string.
#[must_use]
pub fn format_sitemap_lastmod_date(value: Option<DateTime<Utc>>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if is_zero_instant(&value) {
        return String::new();
    }
    value.format("%Y-%m-%d").to_string()
}

/// Format an instant as a sitemap UTC date-time.
///
/// An absent value and the zero instant return the empty string.
#[must_use]
pub fn format_sitemap_lastmod_date_time(value: Option<DateTime<Utc>>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if is_zero_instant(&value) {
        return String::new();
    }
    format_utc_date_time(value)
}

/// Format an instant as `YYYY-MM-DDTHH:MM:SSZ`.
fn format_utc_date_time(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Whether the instant is the declared zero instant.
fn is_zero_instant(value: &DateTime<Utc>) -> bool {
    zero_instant().is_some_and(|zero| value.naive_utc() == zero)
}

/// The zero instant.
fn zero_instant() -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(1, 1, 1).and_then(|date| date.and_hms_opt(0, 0, 0))
}

/// Parse an RFC 3339 date-time with an explicit offset or `Z`.
fn parse_sitemap_date_time(value: &str) -> Option<DateTime<Utc>> {
    if !rfc3339_shape(value) {
        return None;
    }
    let normalized = value.replace(',', ".");
    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

/// Read two adjacent ASCII digits.
fn two_digits(bytes: &[u8], index: usize) -> Option<u32> {
    let high = *bytes.get(index)?;
    let low = *bytes.get(index.checked_add(1)?)?;
    if !high.is_ascii_digit() || !low.is_ascii_digit() {
        return None;
    }
    Some(u32::from(high - b'0') * 10 + u32::from(low - b'0'))
}

/// Whether the value has the RFC 3339 shape accepted by the lastmod normalizer.
///
/// The shape requires a four-digit year, `-` separators, an uppercase `T`, two-digit
/// hours/minutes and a two-digit second no greater than 59, an optional fractional
/// second with `.` or `,`, and either `Z` or a signed `HH:MM` offset, with no trailing
/// text.
fn rfc3339_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20 {
        return false;
    }
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return false;
    }
    if two_digits(bytes, 0).is_none()
        || two_digits(bytes, 2).is_none()
        || two_digits(bytes, 5).is_none()
        || two_digits(bytes, 8).is_none()
        || two_digits(bytes, 11).is_none()
        || two_digits(bytes, 14).is_none()
    {
        return false;
    }
    let Some(second) = two_digits(bytes, 17) else {
        return false;
    };
    if second > 59 {
        return false;
    }
    let mut index = 19;
    if matches!(bytes.get(index), Some(&b'.') | Some(&b',')) {
        index += 1;
        let digits_start = index;
        while matches!(bytes.get(index), Some(byte) if byte.is_ascii_digit()) {
            index += 1;
        }
        if index == digits_start {
            return false;
        }
    }
    match bytes.get(index) {
        Some(&b'Z') => index.checked_add(1) == Some(bytes.len()),
        Some(&b'+') | Some(&b'-') => {
            bytes.len() == index.saturating_add(6)
                && two_digits(bytes, index + 1).is_some()
                && bytes.get(index + 3) == Some(&b':')
                && two_digits(bytes, index + 4).is_some()
        }
        _ => false,
    }
}

/// Serialize a resource kind as its own text.
fn serialize_kind<S>(value: &ResourceKind, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}

/// Serialize a sitemap change frequency as its own text.
fn serialize_change_frequency<S>(
    value: &SitemapChangeFrequency,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}
