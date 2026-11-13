//! Localized contract payload mapping (W17).
//!
//! The owner supplies the per-language contract body, the contract payload mapper
//! built from caller-supplied required language keys, the seeded language maps, the
//! draft maps, the serializable contract value and the completeness rules.
//!
//! Details that are easy to lose:
//!
//! - a seeded map is created from the **required language keys**: each key is
//!   trimmed, blank keys are skipped, and the remaining keys are present with the
//!   value type's zero value *before* any draft is applied;
//! - a draft is mapped only when its mapped language key is non-blank, so a draft
//!   whose language normalizes to blank never adds an entry;
//! - the caller's language-key normalizer trims its own result; an absent
//!   normalizer trims the draft language instead; a nil body mapper skips the
//!   drafts entirely and returns only the seeded map;
//! - the contract value trims every member and omits blank members, producing an
//!   empty object for a language whose body is entirely blank;
//! - completeness is "every *required* language — trimmed, blank skipped — has a
//!   non-blank value", so a map with extra languages still reports complete;
//! - `SplitCommaSeparatedUnique` returns no values for blank input and preserves
//!   the first occurrence order of the delimited values;
//! - `ContractContentBody`'s JSON members are `markdown`, `org` and `edn`, each
//!   omitted when empty.
//!
//! The maps have unspecified iteration order and their
//! JSON encoding sorts keys. [`std::collections::BTreeMap`] is used so the
//! observable JSON shape of a consumer that serializes these maps is the same
//! sorted-member document, and [`ContractContentValue`] returns the nested
//! `markdown`/`org`/`edn` members as strings rather than `any` values, because
//! every inserted member is a string.

use std::collections::BTreeMap;

use serde::ser::{SerializeStruct, Serializer};

use super::format::decide_content_format_bytes;
use crate::byte_text::encoding_text;
use crate::commands::InputBytes;
use crate::localization::{LocalizedContentDraft, LocalizedTextDraft, Normalizer, NormalizerRef};
use crate::validation::trim_unicode_whitespace;

/// The per-language contract content payload.
///
/// Exactly one member is set for a mapped draft; a caller-built body may carry
/// more. The serialized member names and their omission of empty values are the
/// serialization tags.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContractContentBody {
    /// The markdown body.
    pub markdown: InputBytes,
    /// The org body.
    pub org: InputBytes,
    /// The EDN body.
    pub edn: InputBytes,
}

impl serde::Serialize for ContractContentBody {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let members = usize::from(!self.markdown.is_empty())
            + usize::from(!self.org.is_empty())
            + usize::from(!self.edn.is_empty());
        let mut state = serializer.serialize_struct("ContractContentBody", members)?;
        if !self.markdown.is_empty() {
            state.serialize_field("markdown", &encoding_text(self.markdown.as_bytes()))?;
        }
        if !self.org.is_empty() {
            state.serialize_field("org", &encoding_text(self.org.as_bytes()))?;
        }
        if !self.edn.is_empty() {
            state.serialize_field("edn", &encoding_text(self.edn.as_bytes()))?;
        }
        state.end()
    }
}

/// The configuration of a contract payload mapper.
///
/// Both members are optional: a mapper with no keys and no normalizer is the zero
/// mapper.
#[derive(Clone, Default)]
pub struct ContractPayloadMapperOptions {
    /// The languages a contract payload must cover.
    pub required_language_keys: Vec<String>,
    /// The caller's language-key normalizer.
    pub language_key: Option<Normalizer>,
}

/// The contract payload mapper.
///
/// The mapper stores the trimmed required language keys and the caller's
/// language-key normalizer; every operation applies in the same order.
#[derive(Clone, Default)]
pub struct ContractPayloadMapper {
    required_language_keys: Vec<String>,
    language_key: Option<Normalizer>,
}

impl ContractPayloadMapper {
    /// The trimmed required language keys, copied for the caller.
    #[must_use]
    pub fn required_language_keys(&self) -> Vec<String> {
        self.required_language_keys.clone()
    }

    /// Map localized content drafts into contract bodies.
    #[must_use]
    pub fn content_map(
        &self,
        drafts: &[LocalizedContentDraft],
    ) -> BTreeMap<String, ContractContentBody> {
        let body: &dyn Fn(&LocalizedContentDraft) -> ContractContentBody =
            &contract_content_body_from_localized_draft;
        contract_content_map_from_localized_drafts(
            &self.required_language_keys,
            drafts,
            self.language_key.as_deref(),
            Some(body),
        )
    }

    /// Map localized content drafts into their descriptions.
    #[must_use]
    pub fn content_description_map(
        &self,
        drafts: &[LocalizedContentDraft],
    ) -> BTreeMap<String, InputBytes> {
        contract_string_map_from_localized_content_drafts(
            &self.required_language_keys,
            drafts,
            self.language_key.as_deref(),
        )
    }

    /// Map localized text drafts into their descriptions.
    #[must_use]
    pub fn text_description_map(&self, drafts: &[LocalizedTextDraft]) -> BTreeMap<String, InputBytes> {
        contract_string_map_from_localized_text_drafts(
            &self.required_language_keys,
            drafts,
            self.language_key.as_deref(),
        )
    }

    /// Map contract bodies into the serializable contract value.
    #[must_use]
    pub fn content_value(
        &self,
        content: &BTreeMap<String, ContractContentBody>,
    ) -> BTreeMap<String, BTreeMap<String, InputBytes>> {
        contract_content_value(&self.required_language_keys, content)
    }

    /// Whether every required language carries contract content.
    #[must_use]
    pub fn content_complete(&self, content: &BTreeMap<String, ContractContentBody>) -> bool {
        contract_content_complete(&self.required_language_keys, content)
    }

    /// Whether every required language carries a non-blank string value.
    #[must_use]
    pub fn strings_complete(&self, values: &BTreeMap<String, InputBytes>) -> bool {
        contract_strings_complete(&self.required_language_keys, values)
    }
}

/// Build a contract payload mapper.
///
/// The required language keys are trimmed and blank keys are removed before the
/// mapper holds them; the normalizer is stored as supplied.
#[must_use]
pub fn new_contract_payload_mapper(options: ContractPayloadMapperOptions) -> ContractPayloadMapper {
    ContractPayloadMapper {
        required_language_keys: clone_contract_language_keys(&options.required_language_keys),
        language_key: options.language_key,
    }
}

/// Copy the required language keys, trimming each and skipping blank keys.
#[must_use]
pub fn clone_contract_language_keys(keys: &[String]) -> Vec<String> {
    let mut cloned = Vec::with_capacity(keys.len());
    for key in keys {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            continue;
        }
        cloned.push(trimmed.to_owned());
    }
    cloned
}

/// Build a seeded string map for the required languages.
#[must_use]
pub fn contract_string_map(langs: &[String]) -> BTreeMap<String, InputBytes> {
    seeded_language_map::<InputBytes>(langs)
}

/// Build a seeded contract-body map for the required languages.
#[must_use]
pub fn contract_content_map(langs: &[String]) -> BTreeMap<String, ContractContentBody> {
    seeded_language_map::<ContractContentBody>(langs)
}

/// Map localized content drafts into contract bodies.
///
/// The body mapper is optional: an absent mapper skips the
/// drafts and returns only the seeded map.
#[must_use]
pub fn contract_content_map_from_localized_drafts(
    langs: &[String],
    drafts: &[LocalizedContentDraft],
    lang_key: Option<NormalizerRef<'_>>,
    body: Option<&dyn Fn(&LocalizedContentDraft) -> ContractContentBody>,
) -> BTreeMap<String, ContractContentBody> {
    let Some(body) = body else {
        return contract_content_map(langs);
    };
    draft_map(
        langs,
        drafts,
        &|draft: &LocalizedContentDraft| contract_payload_lang_key(&draft.lang, lang_key),
        &|draft: &LocalizedContentDraft| body(draft),
    )
}

/// Map localized content drafts into their descriptions.
#[must_use]
pub fn contract_string_map_from_localized_content_drafts(
    langs: &[String],
    drafts: &[LocalizedContentDraft],
    lang_key: Option<NormalizerRef<'_>>,
) -> BTreeMap<String, InputBytes> {
    draft_map(
        langs,
        drafts,
        &|draft: &LocalizedContentDraft| contract_payload_lang_key(&draft.lang, lang_key),
        &|draft: &LocalizedContentDraft| draft.description.clone(),
    )
}

/// Map localized text drafts into their descriptions.
#[must_use]
pub fn contract_string_map_from_localized_text_drafts(
    langs: &[String],
    drafts: &[LocalizedTextDraft],
    lang_key: Option<NormalizerRef<'_>>,
) -> BTreeMap<String, InputBytes> {
    draft_map(
        langs,
        drafts,
        &|draft: &LocalizedTextDraft| contract_payload_lang_key(&draft.lang, lang_key),
        &|draft: &LocalizedTextDraft| draft.description.clone(),
    )
}

/// Map contract bodies into the serializable contract value.
///
/// Every required language is present; each member is trimmed and blank members are
/// omitted, so a language with an entirely blank body maps to an empty object.
#[must_use]
pub fn contract_content_value(
    langs: &[String],
    content: &BTreeMap<String, ContractContentBody>,
) -> BTreeMap<String, BTreeMap<String, InputBytes>> {
    let mut value = BTreeMap::new();
    for lang in langs {
        let lang = lang.trim();
        if lang.is_empty() {
            continue;
        }
        let body = content.get(lang);
        let mut entry = BTreeMap::new();
        if let Some(body) = body {
            let markdown = trim_unicode_whitespace(body.markdown.as_bytes());
            if !markdown.is_empty() {
                entry.insert("markdown".to_owned(), InputBytes::from(markdown));
            }
            let org = trim_unicode_whitespace(body.org.as_bytes());
            if !org.is_empty() {
                entry.insert("org".to_owned(), InputBytes::from(org));
            }
            let edn = trim_unicode_whitespace(body.edn.as_bytes());
            if !edn.is_empty() {
                entry.insert("edn".to_owned(), InputBytes::from(edn));
            }
        }
        value.insert(lang.to_owned(), entry);
    }
    value
}

/// Whether every required language carries contract content.
#[must_use]
pub fn contract_content_complete(
    langs: &[String],
    content: &BTreeMap<String, ContractContentBody>,
) -> bool {
    for lang in langs {
        let lang = lang.trim();
        if lang.is_empty() {
            continue;
        }
        let body = content.get(lang);
        let blank = body.is_none_or(|body| {
            trim_unicode_whitespace(body.markdown.as_bytes()).is_empty()
                && trim_unicode_whitespace(body.org.as_bytes()).is_empty()
                && trim_unicode_whitespace(body.edn.as_bytes()).is_empty()
        });
        if blank {
            return false;
        }
    }
    true
}

/// Whether every required language carries a non-blank string value.
#[must_use]
pub fn contract_strings_complete(langs: &[String], values: &BTreeMap<String, InputBytes>) -> bool {
    for lang in langs {
        let lang = lang.trim();
        if lang.is_empty() {
            continue;
        }
        let blank = values.get(lang).is_none_or(|value| trim_unicode_whitespace(value.as_bytes()).is_empty());
        if blank {
            return false;
        }
    }
    true
}

/// Split a comma-separated list into unique trimmed values.
///
/// Blank input returns no values. The first occurrence of each value is kept and
/// the source order is preserved.
#[must_use]
pub fn split_comma_separated_unique(raw: &str) -> Vec<String> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let mut values: Vec<String> = Vec::new();
    for part in raw.split(',') {
        let value = part.trim();
        if value.is_empty() {
            continue;
        }
        if values.iter().any(|existing| existing == value) {
            continue;
        }
        values.push(value.to_owned());
    }
    values
}

/// Map one localized draft into its contract body.
///
/// The draft's content format decides the single member that carries its content.
#[must_use]
pub fn contract_content_body_from_localized_draft(
    draft: &LocalizedContentDraft,
) -> ContractContentBody {
    decide_content_format_bytes(draft.content_format.as_bytes()).contract_body(&draft.content)
}

/// Build a map seeded with the required languages and their zero values.
fn seeded_language_map<V>(langs: &[String]) -> BTreeMap<String, V>
where
    V: Default,
{
    let mut values = BTreeMap::new();
    for lang in langs {
        let lang = lang.trim();
        if lang.is_empty() {
            continue;
        }
        values.insert(lang.to_owned(), V::default());
    }
    values
}

/// Seed the map and apply one mapped value per draft with a non-blank key.
fn draft_map<D, V>(
    langs: &[String],
    drafts: &[D],
    key: &dyn Fn(&D) -> String,
    value: &dyn Fn(&D) -> V,
) -> BTreeMap<String, V>
where
    V: Default,
{
    let mut values = seeded_language_map::<V>(langs);
    for draft in drafts {
        let language = key(draft);
        if language.is_empty() {
            continue;
        }
        values.insert(language, value(draft));
    }
    values
}

/// The trimmed mapped language key of a draft.
fn contract_payload_lang_key(lang: &str, lang_key: Option<NormalizerRef<'_>>) -> String {
    match lang_key {
        Some(normalize) => normalize(lang).trim().to_owned(),
        None => lang.trim().to_owned(),
    }
}
