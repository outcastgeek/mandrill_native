//! Localized authoring values, row options and localized selection.
//!
//! The two draft records, the three localized field keys, the generic
//! [`LocalizedRowOptions`] accessors, draft collection/copy from rows,
//! route-language row and draft selection, and the metadata-row precedence.
//!
//! Details that are easy to lose:
//!
//! - a form draft keeps the *requested* language string (it is never normalized)
//!   and is always `required`;
//! - row mapping trims the description, the default content format and the row
//!   content format, but **does not** trim the row content;
//! - a present row content format overrides the default content format even when
//!   the row's value is empty — the default is not a fallback;
//! - row selection normalizes both the requested language and each row language
//!   with the caller's normalizer, and a row without a language accessor can never
//!   match;
//! - draft selection returns the first matching *original* draft, so a caller that
//!   mutates the result mutates the collection it passed in;
//! - metadata selection uses this precedence: default-language row, then the
//!   caller's present fallback, then the first row, then absence;
//! - no row type is required to implement `Clone`, `Default` or `PartialEq`. The
//!   contract borrows the caller's rows and represents the "is the fallback the zero
//!   value" test and its zero return as explicit optional data supplied and consumed
//!   by the caller, instead of manufacturing unrelated trait bounds.

use std::sync::Arc;

use crate::commands::InputBytes;
use crate::validation::trim_unicode_whitespace;

use super::normalizer::{Normalizer, NormalizerRef, normalized_language};

/// Field key of a localized description.
pub const LOCALIZED_DESCRIPTION_FIELD_KEY: &str = "description";

/// Field key of a localized content body.
pub const LOCALIZED_CONTENT_FIELD_KEY: &str = "content";

/// Field key of a localized content format.
pub const LOCALIZED_CONTENT_FORMAT_FIELD_KEY: &str = "content_format";

/// One localized content draft: the owner-controlled multilingual content value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalizedContentDraft {
    /// The requested language, exactly as supplied.
    pub lang: String,
    /// The trimmed description, or the stored description when read back.
    pub description: InputBytes,
    /// The content body, which row mapping does not trim.
    pub content: InputBytes,
    /// The content format after the owner's optional format normalization.
    pub content_format: InputBytes,
    /// Whether authoring requires this language; the collection helpers set it.
    pub required: bool,
}

/// One localized text draft: the owner-controlled multilingual text value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalizedTextDraft {
    /// The requested language, exactly as supplied.
    pub lang: String,
    /// The trimmed description.
    pub description: InputBytes,
    /// Whether authoring requires this language; the collection helpers set it.
    pub required: bool,
}

/// A caller-supplied row accessor.
///
/// Every accessor in [`LocalizedRowOptions`] is optional: a caller supplies only
/// the accessors its row type actually has.
pub type RowAccessor<T> = Arc<dyn Fn(&T) -> String + Send + Sync + 'static>;

/// A caller-supplied byte-valued row accessor.
pub type ValueRowAccessor<T> = Arc<dyn Fn(&T) -> InputBytes + Send + Sync + 'static>;

/// How a persisted row exposes its localized values.
///
/// The default value has no accessors and no default content format.
pub struct LocalizedRowOptions<T> {
    /// The caller's language normalizer, applied to both sides of a comparison.
    pub normalize_language: Option<Normalizer>,
    /// The row's language; without it no row can match.
    pub row_language: Option<RowAccessor<T>>,
    /// The row's description; trimmed when a row is mapped.
    pub description: Option<ValueRowAccessor<T>>,
    /// The row's content; not trimmed when a row is mapped.
    pub content: Option<ValueRowAccessor<T>>,
    /// The row's content format; trimmed when a row is mapped.
    pub content_format: Option<ValueRowAccessor<T>>,
    /// The content format a draft starts from before any row is applied.
    pub default_content_format: InputBytes,
}

impl<T> Clone for LocalizedRowOptions<T> {
    fn clone(&self) -> Self {
        Self {
            normalize_language: self.normalize_language.clone(),
            row_language: self.row_language.clone(),
            description: self.description.clone(),
            content: self.content.clone(),
            content_format: self.content_format.clone(),
            default_content_format: self.default_content_format.clone(),
        }
    }
}

impl<T> Default for LocalizedRowOptions<T> {
    fn default() -> Self {
        Self {
            normalize_language: None,
            row_language: None,
            description: None,
            content: None,
            content_format: None,
            default_content_format: InputBytes::default(),
        }
    }
}

/// Copy the outer draft collection.
///
/// The outer collection is copied; the draft values themselves hold no shared
/// state, so a copy is independent.
#[must_use]
pub fn clone_localized_content_drafts(
    drafts: &[LocalizedContentDraft],
) -> Vec<LocalizedContentDraft> {
    drafts.to_vec()
}

/// Copy the outer text-draft collection.
#[must_use]
pub fn clone_localized_text_drafts(drafts: &[LocalizedTextDraft]) -> Vec<LocalizedTextDraft> {
    drafts.to_vec()
}

/// Find the row whose language matches a route language.
///
/// Both sides are normalized; a row without a language accessor never matches. The
/// first matching row is returned by reference, so no row type must implement
/// `Clone`; a caller that needs an owned row clones the borrow itself.
#[must_use]
pub fn find_localized_row_for_route_lang<'a, T>(
    rows: &'a [T],
    route_lang: &str,
    options: &LocalizedRowOptions<T>,
) -> Option<&'a T> {
    let target = normalized_language(route_lang, options.normalize_language.as_deref());
    let row_language = options.row_language.as_deref()?;
    rows.iter().find(|row| {
        normalized_language(&row_language(row), options.normalize_language.as_deref()) == target
    })
}

/// Resolve the metadata row for one language.
///
/// Precedence: the default-language row, then the caller's present fallback, then
/// the first row, and otherwise absence.
///
/// A fallback that is not a usable row is genuinely optional *data*, so the caller
/// passes `None` for "no usable fallback" and `Some(&row)` for a present one, and
/// the absence result is `None` instead of a fabricated zero row. The caller maps
/// those two absences to its own zero/default value at its boundary, which is the
/// only place the row's zero value is expressible. No `Default`, `PartialEq`,
/// `Clone` or reflection machinery is required of the row type, and no runtime
/// type registry is involved.
#[must_use]
pub fn metadata_localized_row<'a, T>(
    rows: &'a [T],
    fallback: Option<&'a T>,
    default_lang: &str,
    options: &LocalizedRowOptions<T>,
) -> Option<&'a T> {
    if let Some(row) = find_localized_row_for_route_lang(rows, default_lang, options) {
        return Some(row);
    }
    fallback.or_else(|| rows.first())
}

/// Build localized content drafts from rows.
///
/// Each draft keeps the requested language, starts from the trimmed default
/// content format and is required; a matching row then supplies the trimmed
/// description, the untrimmed content and the trimmed content format when the
/// caller supplied those accessors.
#[must_use]
pub fn localized_content_drafts_from_rows<T>(
    langs: &[String],
    rows: &[T],
    options: &LocalizedRowOptions<T>,
) -> Vec<LocalizedContentDraft> {
    langs
        .iter()
        .map(|lang| {
            let mut draft = LocalizedContentDraft {
                lang: lang.clone(),
                description: InputBytes::default(),
                content: InputBytes::default(),
                content_format: InputBytes::from(trim_unicode_whitespace(options.default_content_format.as_bytes())),
                required: true,
            };
            if let Some(row) = find_localized_row_for_route_lang(rows, lang, options) {
                if let Some(description) = options.description.as_deref() {
                    draft.description = InputBytes::from(trim_unicode_whitespace(description(row).as_bytes()));
                }
                if let Some(content) = options.content.as_deref() {
                    draft.content = content(row);
                }
                if let Some(content_format) = options.content_format.as_deref() {
                    draft.content_format = InputBytes::from(trim_unicode_whitespace(content_format(row).as_bytes()));
                }
            }
            draft
        })
        .collect()
}

/// Build localized text drafts from rows.
///
/// Each draft keeps the requested language and is required; a matching row with a
/// description accessor supplies the trimmed description.
#[must_use]
pub fn localized_text_drafts_from_rows<T>(
    langs: &[String],
    rows: &[T],
    options: &LocalizedRowOptions<T>,
) -> Vec<LocalizedTextDraft> {
    langs
        .iter()
        .map(|lang| {
            let mut draft = LocalizedTextDraft {
                lang: lang.clone(),
                description: InputBytes::default(),
                required: true,
            };
            if let Some(row) = find_localized_row_for_route_lang(rows, lang, options)
                && let Some(description) = options.description.as_deref()
            {
                draft.description = InputBytes::from(trim_unicode_whitespace(description(row).as_bytes()));
            }
            draft
        })
        .collect()
}

/// Find the content draft for a route language.
///
/// The first matching draft is returned by mutable reference, so the caller sees
/// and mutates the original draft, not an independent copy.
#[must_use]
pub fn find_localized_content_draft_for_route_lang<'a>(
    drafts: &'a mut [LocalizedContentDraft],
    route_lang: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> Option<&'a mut LocalizedContentDraft> {
    find_localized_draft(drafts, route_lang, normalize, |draft| draft.lang.as_str())
}

/// Find the text draft for a route language.
///
/// The first matching draft is returned by mutable reference.
#[must_use]
pub fn find_localized_text_draft_for_route_lang<'a>(
    drafts: &'a mut [LocalizedTextDraft],
    route_lang: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> Option<&'a mut LocalizedTextDraft> {
    find_localized_draft(drafts, route_lang, normalize, |draft| draft.lang.as_str())
}

/// Report whether every draft carries a description.
///
/// Only a trimmed-empty description is incomplete; an empty collection is
/// complete.
#[must_use]
pub fn localized_text_drafts_complete(drafts: &[LocalizedTextDraft]) -> bool {
    drafts
        .iter()
        .all(|draft| !trim_unicode_whitespace(draft.description.as_bytes()).is_empty())
}

fn find_localized_draft<'a, T>(
    drafts: &'a mut [T],
    route_lang: &str,
    normalize: Option<NormalizerRef<'_>>,
    language: impl Fn(&T) -> &str,
) -> Option<&'a mut T> {
    let target = normalized_language(route_lang, normalize);
    drafts
        .iter_mut()
        .find(|draft| normalized_language(language(draft), normalize) == target)
}
