//! Localization: language normalization, localized drafts and the translation port.
//!
//! Localization owns:
//!
//! - the language normalizer built from caller-supplied options, and the single
//!   authoring-language normalization rule;
//! - localized field suffixes and form field names;
//! - the two localized draft records, the localized row-option accessors and the
//!   localized field keys;
//! - draft collection from a request, draft copy, draft/row selection, metadata-row
//!   precedence and localized drafts built from persisted rows;
//! - localized description JSON decoding/encoding and description completeness;
//! - the translation-key port type the consuming owners call.
//!
//! # API
//!
//! The public surface keeps the owner's names and responsibilities. Type aliases
//! exist only where Rust needs a name for a callback type:
//! [`Normalizer`]/[`NormalizerRef`] for `func(string) string`, [`RowAccessor`] for
//! the per-field row accessors and [`TranslateKeyFunc`]/
//! [`TranslateKeyFuncRef`] for the translation port, whose first argument stays the
//! caller's own context type.
//!
//! # Data and optionality
//!
//! Optional values here are genuinely optional in the source, not optional required
//! capabilities: every row accessor, the normalizer, the fallback key and the
//! translation port are optional, and callers supply only
//! the ones their data has. The owner holds no capability, opens no store and
//! creates no global or service locator.
//!
//! The translation port fixes no transport: the caller selects the context type, so
//! HTTP callers instantiate it with their request-scoped context and
//! request-independent callers (for example validation issue localization)
//! instantiate it with a plain context that carries no request. The native port keeps
//! that context as the caller-selected type parameter, so `effect_server::Request` is
//! one supported instantiation rather than a requirement, and this owner neither
//! fabricates a request nor reaches for ambient state.
//!
//! # Effects and transport
//!
//! The pure helpers are pure: they read their arguments and return data. The two
//! request-collection helpers perform I/O, so they are `Effect<A, E, R>` values
//! built with the workspace effect macros: the request comes from `R: HasRequest`
//! through `service!`, the body form is read through the native request owner
//! under explicit limits, and the effect stays lazy.
//!
//! # Dependencies
//!
//! `effect` (the `Effect` kernel and its macros), `effect_server` (`Request`,
//! `FormData`, `FormPolicy`, `RequestInput`, `HasRequest`, `ServerError`) and
//! `serde_json` (used only as the JSON syntax/type reader). No database, SQL, schema,
//! migration, pool, backend or
//! credential handling exists anywhere in this owner.
//!
//! # Private implementation
//!
//! `normalizer`, `drafts`, `json` and `request` are private implementation modules
//! of this owner: they hold the declarations and the mechanics, while this module
//! is the owner's deliberate public surface.

use std::sync::Arc;

use effect::Effect;
use effect_server::{Request, ServerError};

mod drafts;
mod json;
mod normalizer;
mod prepared_drafts;
mod request;

pub use self::drafts::{
    LOCALIZED_CONTENT_FIELD_KEY, LOCALIZED_CONTENT_FORMAT_FIELD_KEY,
    LOCALIZED_DESCRIPTION_FIELD_KEY, LocalizedContentDraft, LocalizedRowOptions,
    LocalizedTextDraft, RowAccessor, ValueRowAccessor,
};
pub use self::normalizer::{ContractLanguageNormalizerOptions, FormatNormalizer, FormatNormalizerRef, Normalizer, NormalizerRef};
pub use self::prepared_drafts::{
    LocalizedDraftCapacity, LocalizedDraftFields, LocalizedDraftSnapshot, LocalizedDraftView,
    PreparedLocalizedDrafts,
};
pub use self::request::{LocalizedFieldValues, LocalizedFormPolicy};

/// Compile-time witness that both request-collection helpers keep their required
/// request environment and their `Effect` result shape.
///
/// This is the platform-level signature
/// assertions (`var _ func(...) ...`): the helpers are usable for any caller error
/// family and environment that supplies a request, and their results stay lazy
/// `Effect` values rather than eagerly read drafts.
const _: fn() = || {
    fn witness<E, R>()
    where
        E: Send + 'static,
        R: effect_server::HasRequest + Send + Sync + 'static,
    {
        // The witness deliberately spells the complete required signature, so the
        // complexity lint is not applicable to these two assertions.
        #[allow(clippy::type_complexity)]
        let _: fn(
            Vec<String>,
            Option<Normalizer>,
            Option<FormatNormalizer>,
            Option<LocalizedFieldValues>,
            LocalizedFormPolicy,
            fn(ServerError) -> E,
        ) -> Effect<Vec<LocalizedContentDraft>, E, R> =
            collect_localized_content_drafts_from_request::<E, R>;
        #[allow(clippy::type_complexity)]
        let _: fn(
            Vec<String>,
            &str,
            Option<Normalizer>,
            Option<LocalizedFieldValues>,
            LocalizedFormPolicy,
            fn(ServerError) -> E,
        ) -> Effect<Vec<LocalizedTextDraft>, E, R> =
            collect_localized_text_drafts_from_request::<E, R>;
    }
    witness::<ServerError, effect_server::RequestEnv<()>>();
};

/// The key-to-message translation port.
///
/// The first argument is the caller's own context value; it is not an HTTP request
/// type. HTTP callers instantiate the
/// port with [`effect_server::Request`] (which carries the request's cancellation
/// source and native extensions); request-independent callers instantiate it with
/// their own context value, including `()` for a background context
/// paths such as validation issue localization. This owner therefore fixes no
/// transport in the port, creates no synthetic request and installs no ambient
/// locator.
///
/// The callback is stored and reused, so it keeps the workspace callback contract
/// (`Fn + Send + Sync + 'static`). A consumer that permits an absent port holds
/// `Option<TranslateKeyFunc<C>>` and applies its own fallback — that absent-function
/// branch belongs to those consumers' rules, not to this declaration.
pub type TranslateKeyFunc<C> = Arc<dyn Fn(&C, &str) -> String + Send + Sync + 'static>;

/// The borrowed spelling of [`TranslateKeyFunc`] for a caller that keeps the callback
/// in its own storage and for the absent-port `Option` case, over the same
/// caller-selected context type.
pub type TranslateKeyFuncRef<'a, C> = &'a (dyn Fn(&C, &str) -> String + Send + Sync);

/// Compile-time witness that the translation port stays context-independent.
///
/// The same port is type-checked for a request-free background context (`()`) and for
/// the HTTP request context, without constructing a request, a context value or a
/// locator: only the callback's argument type is constrained, so both calling styles
/// are expressible and neither is mandatory.
const _: fn() = || {
    fn invoke<C>(port: &TranslateKeyFunc<C>, context: &C, key: &str) -> String {
        port(context, key)
    }
    let _: for<'a, 'b, 'c> fn(&'a TranslateKeyFunc<()>, &'b (), &'c str) -> String = invoke::<()>;
    let _: for<'a, 'b, 'c> fn(&'a TranslateKeyFunc<Request>, &'b Request, &'c str) -> String =
        invoke::<Request>;
};

/// Normalize one authoring language value.
///
/// The caller's normalizer runs first and its trimmed result is returned; an
/// absent normalizer only trims.
#[must_use]
pub fn normalize_authoring_language(lang: &str, normalize: Option<NormalizerRef<'_>>) -> String {
    normalizer::normalized_language(lang, normalize)
}

/// Construct the owner's language normalizer.
///
/// The returned callback resolves an empty language to the trimmed configured
/// default (or `en` when that is blank), returns an explicitly preserved language
/// unchanged, and otherwise applies the caller's normalized value, the caller's
/// fallback key and finally that value.
#[must_use]
pub fn new_contract_language_normalizer(options: ContractLanguageNormalizerOptions) -> Normalizer {
    normalizer::new_contract_language_normalizer(options)
}

/// The normalized field suffix for a language.
///
/// The language is normalized first and each `-` in the result becomes `_`.
#[must_use]
pub fn localized_field_suffix(lang: &str, normalize: Option<NormalizerRef<'_>>) -> String {
    normalizer::localized_field_suffix(lang, normalize)
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
    normalizer::localized_field_name(prefix, lang, normalize)
}

/// Collect localized content drafts from decoded request values.
///
/// `None` is an absent request: no field name is built, so the language
/// normalizer is never invoked, and every draft is empty and required. `Some` of an
/// empty value set is a present request with no matching fields, where field names
/// are built and the normalizer runs. The optional format normalizer runs in both
/// cases.
#[must_use]
pub fn collect_localized_content_drafts_from_values(
    values: Option<&LocalizedFieldValues>,
    langs: &[String],
    normalize_language: Option<NormalizerRef<'_>>,
    normalize_format: Option<FormatNormalizerRef<'_>>,
) -> Vec<LocalizedContentDraft> {
    request::collect_localized_content_drafts_from_values(
        values,
        langs,
        normalize_language,
        normalize_format,
    )
}

/// Collect localized text drafts from decoded request values.
///
/// The absent/present distinction is the same as
/// [`collect_localized_content_drafts_from_values`].
#[must_use]
pub fn collect_localized_text_drafts_from_values(
    values: Option<&LocalizedFieldValues>,
    langs: &[String],
    field_prefix: &str,
    normalize_language: Option<NormalizerRef<'_>>,
) -> Vec<LocalizedTextDraft> {
    request::collect_localized_text_drafts_from_values(
        values,
        langs,
        field_prefix,
        normalize_language,
    )
}

/// Collect localized content drafts from the current request.
///
/// URL-encoded values precede query values; multipart fields follow query values.
/// The effect is lazy and uses the native request owner.
pub fn collect_localized_content_drafts_from_request<E, R>(
    langs: Vec<String>,
    normalize_language: Option<Normalizer>,
    normalize_format: Option<FormatNormalizer>,
    preinitialized: Option<LocalizedFieldValues>,
    policy: LocalizedFormPolicy,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<Vec<LocalizedContentDraft>, E, R>
where
    E: Send + 'static,
    R: effect_server::HasRequest + Send + Sync + 'static,
{
    request::collect_localized_content_drafts_from_request(
        langs,
        normalize_language,
        normalize_format,
        preinitialized,
        policy,
        on_error,
    )
}

/// Collect localized text drafts from the current request.
///
/// Field names come from `field_prefix` and the language, with the same request
/// initialization and ordering rules as content drafts.
pub fn collect_localized_text_drafts_from_request<E, R>(
    langs: Vec<String>,
    field_prefix: &str,
    normalize_language: Option<Normalizer>,
    preinitialized: Option<LocalizedFieldValues>,
    policy: LocalizedFormPolicy,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<Vec<LocalizedTextDraft>, E, R>
where
    E: Send + 'static,
    R: effect_server::HasRequest + Send + Sync + 'static,
{
    request::collect_localized_text_drafts_from_request(
        langs,
        field_prefix,
        normalize_language,
        preinitialized,
        policy,
        on_error,
    )
}

/// Copy the outer content-draft collection.
#[must_use]
pub fn clone_localized_content_drafts(
    drafts: &[LocalizedContentDraft],
) -> Vec<LocalizedContentDraft> {
    drafts::clone_localized_content_drafts(drafts)
}

/// Copy the outer text-draft collection.
#[must_use]
pub fn clone_localized_text_drafts(drafts: &[LocalizedTextDraft]) -> Vec<LocalizedTextDraft> {
    drafts::clone_localized_text_drafts(drafts)
}

/// Build localized content drafts from persisted rows.
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
    drafts::localized_content_drafts_from_rows(langs, rows, options)
}

/// Build localized text drafts from persisted rows.
#[must_use]
pub fn localized_text_drafts_from_rows<T>(
    langs: &[String],
    rows: &[T],
    options: &LocalizedRowOptions<T>,
) -> Vec<LocalizedTextDraft> {
    drafts::localized_text_drafts_from_rows(langs, rows, options)
}

/// Find the row whose language matches a route language.
///
/// Both sides are normalized; a row without a language accessor never matches. The
/// first matching row is borrowed, so the row type needs no `Clone`.
#[must_use]
pub fn find_localized_row_for_route_lang<'a, T>(
    rows: &'a [T],
    route_lang: &str,
    options: &LocalizedRowOptions<T>,
) -> Option<&'a T> {
    drafts::find_localized_row_for_route_lang(rows, route_lang, options)
}

/// Resolve the metadata row for one language.
///
/// Precedence: the default-language row, then the
/// caller's present fallback, then the first row, and otherwise absence. The
/// zero-`T` fallback and zero-`T` result are explicit optional data
/// here — `None` fallback means "no usable fallback" and a `None` result means
/// "absent" — so the row type needs no `Default`, `PartialEq` or `Clone` and no
/// reflection or type registry is involved. A caller maps both absences to its own
/// zero value where that value is expressible.
#[must_use]
pub fn metadata_localized_row<'a, T>(
    rows: &'a [T],
    fallback: Option<&'a T>,
    default_lang: &str,
    options: &LocalizedRowOptions<T>,
) -> Option<&'a T> {
    drafts::metadata_localized_row(rows, fallback, default_lang, options)
}

/// Find the content draft for a route language.
///
/// The first matching draft is returned by mutable reference, so a caller mutates
/// the original draft.
#[must_use]
pub fn find_localized_content_draft_for_route_lang<'a>(
    drafts: &'a mut [LocalizedContentDraft],
    route_lang: &str,
    normalize: Option<NormalizerRef<'_>>,
) -> Option<&'a mut LocalizedContentDraft> {
    drafts::find_localized_content_draft_for_route_lang(drafts, route_lang, normalize)
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
    drafts::find_localized_text_draft_for_route_lang(drafts, route_lang, normalize)
}

/// Decode localized descriptions into one text draft per requested language.
#[must_use]
pub fn localized_text_drafts_from_description_json(
    langs: &[String],
    raw: impl AsRef<[u8]>,
    normalize: Option<NormalizerRef<'_>>,
) -> Vec<LocalizedTextDraft> {
    json::localized_text_drafts_from_description_json(langs, raw, normalize)
}

/// Encode localized descriptions as JSON.
#[must_use]
pub fn serialize_localized_text_draft_descriptions(
    drafts: &[LocalizedTextDraft],
    normalize: Option<NormalizerRef<'_>>,
) -> String {
    json::serialize_localized_text_draft_descriptions(drafts, normalize)
}

/// Report whether every draft carries a description.
///
/// Only a trimmed-empty description is incomplete; an empty collection is
/// complete.
#[must_use]
pub fn localized_text_drafts_complete(drafts: &[LocalizedTextDraft]) -> bool {
    drafts::localized_text_drafts_complete(drafts)
}
