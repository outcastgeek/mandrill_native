//! Content formats, persisted storage columns and contract bodies (W17).
//!
//! The owner supplies the three storage formats, the EDN storage prefix, the format
//! decision for a requested format name and for already-stored columns, the persisted
//! column selection and the per-language contract body.
//!
//! Details that are easy to lose:
//!
//! - the format name is an **open string**: an unknown, blank or differently-cased
//!   name is not an error, it decides `markdown`. Format detection lowercases and
//!   trims; nothing else is accepted.
//! - stored format precedence is org first (a *trimmed, non-blank* org column),
//!   then EDN (a *trimmed* markdown column starting with `edn:`), then markdown.
//!   A blank org column is absent, not an org document.
//! - stored source text is **not** trimmed before the `edn:` prefix is removed, so
//!   a leading space keeps the prefix and yields the column text unchanged.
//! - a blank content body selects no column at all (every column `None`, rendering
//!   suppression false); the content is trimmed before it is stored.
//! - EDN content is stored in the *markdown* column with the `edn:` prefix and
//!   suppresses rendered HTML; org content is stored only in the org column.
//! - a contract body selects exactly one member, so the other two stay empty.
//!
//! Owner-private format dispatch uses generic
//! `FormatFactories` closures so that the concrete format values stay unexported.
//! Rust privacy already gives the owner that boundary, so the dispatch is a private
//! [`StorageFormat`] value with the same three rules and the same public string
//! results. No public trait, type bound or factory aggregate is invented here.

use super::payload::ContractContentBody;
use crate::commands::InputBytes;

/// The markdown storage format.
pub const CONTENT_FORMAT_MARKDOWN: &str = "markdown";

/// The org storage format.
pub const CONTENT_FORMAT_ORG: &str = "org";

/// The EDN storage format.
pub const CONTENT_FORMAT_EDN: &str = "edn";

/// The prefix that marks EDN content stored in the markdown column.
pub const CONTENT_EDN_STORAGE_PREFIX: &str = "edn:";

/// The resolved persisted column selection.
///
/// A member is `None` when that column is absent, and
/// [`Self::suppress_rendered_html`] is the rendering-suppression flag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentStorageColumns {
    /// The value to persist in the markdown column.
    pub markdown: Option<String>,
    /// The value to persist in the org column.
    pub org: Option<String>,
    /// Whether rendered HTML is suppressed for this content.
    pub suppress_rendered_html: bool,
}

/// The owner-private format decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StorageFormat {
    /// The markdown format.
    Markdown,
    /// The org format.
    Org,
    /// The EDN format.
    Edn,
}

impl StorageFormat {
    /// The public format name of this decision.
    pub(crate) fn format_value(self) -> &'static str {
        match self {
            Self::Markdown => CONTENT_FORMAT_MARKDOWN,
            Self::Org => CONTENT_FORMAT_ORG,
            Self::Edn => CONTENT_FORMAT_EDN,
        }
    }

    /// The persisted column selection this format stores `content` in.
    pub(crate) fn storage_columns(self, content: &str) -> ContentStorageColumns {
        match self {
            Self::Markdown => ContentStorageColumns {
                markdown: Some(content.to_owned()),
                ..ContentStorageColumns::default()
            },
            Self::Org => ContentStorageColumns {
                org: Some(content.to_owned()),
                ..ContentStorageColumns::default()
            },
            Self::Edn => {
                let mut stored = String::from(CONTENT_EDN_STORAGE_PREFIX);
                stored.push_str(content);
                ContentStorageColumns {
                    markdown: Some(stored),
                    org: None,
                    suppress_rendered_html: true,
                }
            }
        }
    }

    /// The per-language contract body this format produces.
    pub(crate) fn contract_body(self, content: &InputBytes) -> ContractContentBody {
        match self {
            Self::Markdown => ContractContentBody {
                markdown: content.clone(),
                ..ContractContentBody::default()
            },
            Self::Org => ContractContentBody {
                org: content.clone(),
                ..ContractContentBody::default()
            },
            Self::Edn => ContractContentBody {
                edn: content.clone(),
                ..ContractContentBody::default()
            },
        }
    }
}

/// Decide the format of a requested format name.
///
/// The name is trimmed and lowercased; `org` and `edn` are recognized and every
/// other value, including a blank one, is markdown.
pub(crate) fn decide_content_format(raw: &str) -> StorageFormat {
    let normalized = raw.trim().to_lowercase();
    if normalized == CONTENT_FORMAT_ORG {
        StorageFormat::Org
    } else if normalized == CONTENT_FORMAT_EDN {
        StorageFormat::Edn
    } else {
        StorageFormat::Markdown
    }
}

/// Decide a format without altering malformed input bytes.
pub(crate) fn decide_content_format_bytes(raw: &[u8]) -> StorageFormat {
    match std::str::from_utf8(raw) {
        Ok(text) => decide_content_format(text),
        Err(_) => StorageFormat::Markdown,
    }
}

/// Decide the format of already-stored columns.
///
/// Org wins over EDN and EDN wins over markdown.
pub(crate) fn decide_stored_content_format(
    markdown: Option<&str>,
    org: Option<&str>,
) -> StorageFormat {
    if let Some(org) = org
        && !org.trim().is_empty()
    {
        return StorageFormat::Org;
    }
    if let Some(markdown) = markdown
        && markdown.trim().starts_with(CONTENT_EDN_STORAGE_PREFIX)
    {
        return StorageFormat::Edn;
    }
    StorageFormat::Markdown
}

/// Normalize a requested content format name.
///
/// Normalize the content format: the recognized name of the
/// format decision, so an unknown name normalizes to `markdown`.
#[must_use]
pub fn normalize_content_format(raw: &str) -> String {
    decide_content_format(raw).format_value().to_owned()
}

/// Decide the stored content format from the persisted columns.
#[must_use]
pub fn content_format_from_stored_fields(markdown: Option<&str>, org: Option<&str>) -> String {
    decide_stored_content_format(markdown, org)
        .format_value()
        .to_owned()
}

/// Return the source content of the stored columns.
///
/// A non-blank org column is the source, otherwise the markdown column with the
/// EDN prefix removed when present; otherwise there is no source content. The
/// markdown column is not trimmed, so its prefix removal is exact.
#[must_use]
pub fn source_content_from_stored_fields(markdown: Option<&str>, org: Option<&str>) -> String {
    if let Some(org) = org
        && !org.trim().is_empty()
    {
        return org.to_owned();
    }
    if let Some(markdown) = markdown {
        return markdown
            .strip_prefix(CONTENT_EDN_STORAGE_PREFIX)
            .unwrap_or(markdown)
            .to_owned();
    }
    String::new()
}

/// Select the persisted column for a content body and its format.
///
/// Blank content selects no column; otherwise the content is trimmed and stored in
/// the column its format owns.
#[must_use]
pub fn content_storage_columns_for_format(
    content: &str,
    content_format: &str,
) -> ContentStorageColumns {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return ContentStorageColumns::default();
    }
    decide_content_format(content_format).storage_columns(trimmed)
}
