//! Language normalization and localized field naming.
//!
//! Language normalization and localized field naming apply these rules in order:
//!
//! - the configured default language is trimmed, and a blank result becomes `en`;
//! - preserved entries are trimmed and blank entries are dropped, and the set is
//!   matched exactly against the *normalized* value;
//! - the caller's normalizer runs first and its result is trimmed afterwards, so
//!   an absent normalizer only trims;
//! - the fallback-key function is consulted only for a nonempty normalized value
//!   that is not preserved, and only a nonblank fallback replaces it.
//!
//! No lowercasing, locale-fallback chain, separator rewriting or case folding is
//! invented here.

use std::collections::BTreeSet;
use std::sync::Arc;

/// A caller-supplied language or content-format normalizer.
///
/// This is a stored, shareable string-to-string callback (`Arc`). The callback
/// receives the value exactly as supplied — it is not pre-trimmed — and its result
/// is trimmed by this owner afterwards.
pub type Normalizer = Arc<dyn Fn(&str) -> String + Send + Sync + 'static>;

/// A content-format callback over exact input bytes.
pub type FormatNormalizer = Arc<dyn Fn(&[u8]) -> crate::commands::InputBytes + Send + Sync + 'static>;
/// A borrowed content-format callback.
pub type FormatNormalizerRef<'a> = &'a (dyn Fn(&[u8]) -> crate::commands::InputBytes + Send + Sync);

/// The borrowed form of a [`Normalizer`], and how an absent normalizer is spelled.
///
/// `None` means the value is trimmed and otherwise left unchanged.
pub type NormalizerRef<'a> = &'a (dyn Fn(&str) -> String + Send + Sync);

/// Options for [`new_contract_language_normalizer`].
///
/// Every field is caller-supplied; nothing is derived from configuration that the
/// caller did not supply.
#[derive(Clone, Default)]
pub struct ContractLanguageNormalizerOptions {
    /// The language used for an empty value; trimmed, and `en` when blank.
    pub default_language: String,
    /// Languages returned unchanged even when a fallback key could apply.
    pub preserve: Vec<String>,
    /// The caller's normalizer, run before trimming; absent means trim only.
    pub normalize: Option<Normalizer>,
    /// The caller's fallback-key function, run after a nonempty normalized value.
    pub fallback_key: Option<Normalizer>,
}

/// Normalize one language value.
///
/// The caller's normalizer runs first, the result is trimmed, and an absent
/// normalizer means trim only.
#[must_use]
pub fn normalized_language(lang: &str, normalize: Option<NormalizerRef<'_>>) -> String {
    match normalize {
        Some(normalize) => normalize(lang).trim().to_owned(),
        None => lang.trim().to_owned(),
    }
}

/// Construct the owner's language normalizer.
///
/// The returned callback resolves an empty language to the configured default,
/// returns an explicitly preserved language unchanged, and otherwise applies the
/// caller's normalized value, the caller's fallback key and finally that value.
#[must_use]
pub fn new_contract_language_normalizer(options: ContractLanguageNormalizerOptions) -> Normalizer {
    let default_language = {
        let trimmed = options.default_language.trim();
        if trimmed.is_empty() {
            "en".to_owned()
        } else {
            trimmed.to_owned()
        }
    };
    let preserved: BTreeSet<String> = options
        .preserve
        .iter()
        .map(|lang| lang.trim())
        .filter(|lang| !lang.is_empty())
        .map(str::to_owned)
        .collect();
    let normalize = options.normalize;
    let fallback_key = options.fallback_key;
    Arc::new(move |lang: &str| {
        let normalized = normalized_language(lang, normalize.as_deref());
        if normalized.is_empty() {
            return default_language.clone();
        }
        if preserved.contains(&normalized) {
            return normalized;
        }
        if let Some(fallback_key) = fallback_key.as_deref() {
            let fallback = fallback_key(&normalized).trim().to_owned();
            if !fallback.is_empty() {
                return fallback;
            }
        }
        normalized
    })
}

/// The normalized field suffix for a language.
///
/// The language is normalized first and each `-` in the result becomes `_`.
#[must_use]
pub fn localized_field_suffix(lang: &str, normalize: Option<NormalizerRef<'_>>) -> String {
    normalized_language(lang, normalize).replace('-', "_")
}

/// The localized form field name for a prefix and language.
///
/// The prefix is trimmed and joined with `_` and the field suffix, including when
/// the suffix is empty.
#[must_use]
pub fn localized_field_name(
    prefix: &str,
    lang: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> String {
    format!(
        "{}_{}",
        prefix.trim(),
        localized_field_suffix(lang, normalize)
    )
}
