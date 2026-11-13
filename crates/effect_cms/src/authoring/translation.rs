//! The authoring translation workflow.
//!
//! This module owns field/target normalization, the request/provider JSON field
//! parser with its fence trimming and invalid-escape repair, the per-target
//! concurrency with per-target deadlines, and the ready/no-op/failure signal-patch
//! projection.
//!
//! The translator callback is supplied by the consumer. Every target starts its own
//! child effect; the first expected failure cancels the remaining children and is
//! returned unchanged in [`AuthoringTranslationError::Translator`]. A target's
//! deadline cancels and joins that child and reports
//! [`AuthoringTranslationError::Timeout`]. Request cancellation interrupts the
//! whole program through the effect's own channel; defects and finalization
//! failures keep their own channels.
//!
//! The source's runtime identity checks are replaced by ownership: the normalized
//! input and its execution owner are private values consumed by one execution, so a
//! forged or replayed stage cannot be constructed through the public contract.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::time::Duration;

use effect::{
    Effect, HasClock, HasTaskSpawner, effect_for_each_concurrent, effect_map, effect_map_error,
    effect_sync, effect_timeout,
};
use effect_server::Request;
use serde::Deserialize;

use super::contracts::{
    AuthoringTranslationError, AuthoringTranslationField, AuthoringTranslationInput,
    AuthoringTranslationPatchOptions, AuthoringTranslationRequest, AuthoringTranslationResult,
    AuthoringTranslator,
};
use super::patch::{
    AuthoringTranslationPatch, TranslationSignalValue, authoring_translation_status_patch,
};
use crate::commands::InputBytes;
use crate::definitions::SharedList;
use super::contracts::AuthoringLanguageNormalizerRef;

/// The one normalized translation input.
struct NormalizedTranslation<T, R>
where
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    scope: InputBytes,
    source: InputBytes,
    targets: Vec<InputBytes>,
    fields: SharedList<AuthoringTranslationField>,
    translator: Option<AuthoringTranslator<T, R>>,
}

/// Normalize one authoring language through the caller's normalizer.
#[must_use]
pub fn normalize_authoring_language(
    lang: &InputBytes,
    normalize: Option<AuthoringLanguageNormalizerRef<'_>>,
) -> InputBytes {
    normalize.map(|f| f(lang)).unwrap_or_else(|| lang.clone()).trim()
}

/// Normalize the target languages.
///
/// The source language and each candidate are normalized; empty candidates and
/// candidates equal to the source language are skipped, and the first occurrence of
/// a normalized target is kept.
#[must_use]
pub fn normalize_target_languages(
    source_language: &InputBytes,
    candidates: &[InputBytes],
    normalize: Option<AuthoringLanguageNormalizerRef<'_>>,
) -> Vec<InputBytes> {
    let source = normalize_authoring_language(source_language, normalize);
    let mut seen: Vec<InputBytes> = Vec::with_capacity(candidates.len());
    let mut targets = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let normalized = normalize_authoring_language(candidate, normalize);
        if normalized.is_empty() || normalized == source {
            continue;
        }
        if seen.contains(&normalized) {
            continue;
        }
        seen.push(normalized.clone());
        targets.push(normalized);
    }
    targets
}

/// Normalize the requested fields.
///
/// A blank name or a blank (after trimming) value removes the field; otherwise the
/// name and format are trimmed and the value is retained exactly as supplied.
#[must_use]
pub fn normalize_authoring_fields(
    fields: &[AuthoringTranslationField],
) -> Vec<AuthoringTranslationField> {
    let mut normalized = Vec::with_capacity(fields.len());
    for field in fields {
        let name = field.name.trim();
        if name.is_empty() || field.value.trim().is_empty() {
            continue;
        }
        normalized.push(AuthoringTranslationField {
            name: name.to_owned(),
            value: field.value.clone(),
            format: field.format.trim().to_owned(),
        });
    }
    normalized
}

/// The normalized field-format lookup.
#[must_use]
pub fn authoring_field_format_map(
    fields: &[AuthoringTranslationField],
) -> BTreeMap<InputBytes, InputBytes> {
    let mut formats = BTreeMap::new();
    for field in fields {
        let name = field.name.trim();
        if name.is_empty() {
            continue;
        }
        formats.insert(name.to_owned(), field.format.trim().to_owned());
    }
    formats
}

/// The provider fields response.
#[derive(Default, Deserialize)]
struct AuthoringTranslationFieldsResponse {
    #[serde(default)]
    fields: Option<BTreeMap<String, String>>,
}

/// Parse the provider's field JSON after fence trimming and escape repair.
///
/// # Errors
///
/// Returns the JSON reader's syntax/type error unchanged.
pub fn parse_authoring_translation_fields_json(
    raw: &str,
) -> Result<BTreeMap<InputBytes, InputBytes>, serde_json::Error> {
    let trimmed = trim_authoring_json_fence(raw);
    let repaired = fix_invalid_json_escapes(trimmed.as_bytes());
    let response: AuthoringTranslationFieldsResponse = serde_json::from_slice(&repaired)?;
    Ok(response.fields.unwrap_or_default().into_iter().map(|(k,v)| (k.into(),v.into())).collect())
}

/// Trim one Markdown JSON fence, matching the source's prefix/suffix order.
fn trim_authoring_json_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_prefix("```json").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix("```").unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix("```").unwrap_or(trimmed);
    trimmed.trim().to_owned()
}

/// Repair backslash escapes that JSON does not recognize inside strings.
///
/// Recognized escapes are retained; a backslash followed by any other byte is
/// doubled so the JSON reader sees a literal backslash followed by that byte. A
/// trailing backslash is left for the JSON reader to reject.
fn fix_invalid_json_escapes(data: &[u8]) -> Vec<u8> {
    if !data.contains(&b'\\') {
        return data.to_vec();
    }
    let mut result = Vec::with_capacity(data.len().saturating_add(64));
    let mut in_string = false;
    let mut index = 0_usize;
    while let Some(byte) = data.get(index) {
        if !in_string {
            result.push(*byte);
            if *byte == b'"' {
                in_string = true;
            }
            index += 1;
            continue;
        }
        if *byte == b'\\' {
            let Some(next) = data.get(index + 1) else {
                result.push(*byte);
                index += 1;
                continue;
            };
            match *next {
                b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                    result.push(*byte);
                    result.push(*next);
                    index += 2;
                }
                b'u' => {
                    result.push(*byte);
                    result.push(*next);
                    index += 2;
                    for _ in 0..4 {
                        let Some(digit) = data.get(index) else {
                            break;
                        };
                        result.push(*digit);
                        index += 1;
                    }
                }
                _ => {
                    result.push(b'\\');
                    result.push(b'\\');
                    result.push(*next);
                    index += 2;
                }
            }
            continue;
        }
        if *byte == b'"' {
            in_string = false;
        }
        result.push(*byte);
        index += 1;
    }
    result
}

/// Translate one field set for one target under the supplied deadline.
fn translate_one<T, R>(
    translator: Option<AuthoringTranslator<T, R>>,
    request: Request,
    input: AuthoringTranslationInput,
    timeout: Duration,
) -> Effect<Option<AuthoringTranslationResult>, AuthoringTranslationError<T>, R>
where
    T: Send + 'static,
    R: HasClock + HasTaskSpawner + Send + Sync + 'static,
{
    let Some(translator) = translator else {
        return effect_sync! { once; body: move || None };
    };
    let program = effect_map_error! {
        program: effect_map! {
            program: translator(&request, timeout, input),
            map: Some,
        },
        map: AuthoringTranslationError::<T>::Translator,
    };
    if timeout.is_zero() {
        return program;
    }
    effect_timeout! {
        program: program,
        duration: timeout,
        on_timeout: || AuthoringTranslationError::<T>::Timeout,
    }
}

/// Translate one field set with the source's optional per-call deadline.
///
/// An absent translator keeps the source's `(nil, nil)` result: the effect succeeds
/// with no result. A zero duration performs the call without a deadline.
#[must_use]
pub fn translate_authoring_fields_with_timeout<T, R>(
    request: &Request,
    translator: Option<AuthoringTranslator<T, R>>,
    timeout: Duration,
    input: AuthoringTranslationInput,
) -> Effect<Option<AuthoringTranslationResult>, AuthoringTranslationError<T>, R>
where
    T: Send + 'static,
    R: HasClock + HasTaskSpawner + Send + Sync + 'static,
{
    translate_one(translator, request.clone(), input, timeout)
}

/// Build one translation signal patch.
///
/// A request with no source language, no target languages or no fields produces the
/// no-op status patch. Otherwise each target is translated concurrently; the ready
/// patch carries the done status, the cleared submit-error signals and every
/// returned field's value/format/has-content signals.
#[must_use]
pub fn build_authoring_translation_patch<T, R>(
    request: &Request,
    translator: Option<AuthoringTranslator<T, R>>,
    timeout: Duration,
    translation_request: AuthoringTranslationRequest,
    options: AuthoringTranslationPatchOptions,
) -> Effect<AuthoringTranslationPatch, AuthoringTranslationError<T>, R>
where
    T: Send + 'static,
    R: HasClock + HasTaskSpawner + Send + Sync + 'static,
{
    let input = normalize_translation(
        &translation_request,
        options.normalize_language.as_deref(),
        translator,
    );
    if input.source.is_empty() || input.targets.is_empty() || input.fields.is_empty() {
        let patch =
            authoring_translation_status_patch(&options.signals, &options.ready_status_text);
        return effect_sync! { once; body: move || patch };
    }

    let ready_options = options.clone();
    let request = request.clone();
    let field_formats = authoring_field_format_map(&input.fields.to_vec());
    let targets = input.targets.clone();
    let fields = input.fields.clone();
    let scope = input.scope.clone();
    let source = input.source.clone();
    let translator = input.translator.clone();
    let concurrency = NonZeroUsize::new(targets.len()).unwrap_or(NonZeroUsize::MIN);
    let program = effect_for_each_concurrent! {
        concurrency: concurrency,
        values: targets,
        body: move |target: &InputBytes| {
            let input = AuthoringTranslationInput {
                scope: scope.clone(),
                source_language: source.clone(),
                target_language: target.clone(),
                fields: fields.clone(),
            };
            translate_one(translator.clone(), request.clone(), input, timeout)
        },
    };
    effect_map! {
        program: program,
        map: move |results| ready_patch(&ready_options, &field_formats, &results),
    }
}

/// Normalize one translation request.
fn normalize_translation<T, R>(
    request: &AuthoringTranslationRequest,
    normalize: Option<AuthoringLanguageNormalizerRef<'_>>,
    translator: Option<AuthoringTranslator<T, R>>,
) -> NormalizedTranslation<T, R>
where
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    let source = normalize_authoring_language(&request.source_language, normalize);
    NormalizedTranslation {
        scope: request.scope.trim().to_owned(),
        targets: normalize_target_languages(
            &source,
            &request.target_languages,
            normalize,
        ),
        source,
        fields: SharedList::new(normalize_authoring_fields(&request.fields)),
        translator,
    }
}

/// Project the ready patch from the per-target results.
fn ready_patch(
    options: &AuthoringTranslationPatchOptions,
    field_formats: &BTreeMap<InputBytes, InputBytes>,
    results: &[Option<AuthoringTranslationResult>],
) -> AuthoringTranslationPatch {
    let signals = &options.signals;
    let mut patch = AuthoringTranslationPatch::new();
    patch.insert(
        signals.translation_available.clone(),
        TranslationSignalValue::Bool(true),
    );
    patch.insert(
        signals.translating.clone(),
        TranslationSignalValue::Bool(false),
    );
    patch.insert(
        signals.status_text.clone(),
        TranslationSignalValue::Text(options.done_status_text.clone()),
    );
    if !options.submit_error_open_signal.trim().is_empty() {
        patch.insert(
            options.submit_error_open_signal.clone(),
            TranslationSignalValue::Bool(false),
        );
    }
    if !options.submit_error_text_signal.trim().is_empty() {
        patch.insert(
            options.submit_error_text_signal.clone(),
            TranslationSignalValue::Text(String::new()),
        );
    }

    for result in results.iter().flatten() {
        let target = normalize_authoring_language(
            &result.target_language,
            options.normalize_language.as_deref(),
        );
        for (field_name, value) in result.fields.with(Clone::clone) {
            let normalized_field_name = field_name.trim();
            if normalized_field_name.is_empty() {
                continue;
            }
            if let Some(field_signal_name) = options.field_signal_name.as_ref() {
                patch.insert(
                    field_signal_name(&target, &normalized_field_name),
                    TranslationSignalValue::Text(crate::byte_text::encoding_text(value.as_bytes())),
                );
            }
            if let Some(field_format_signal_name) = options.field_format_signal_name.as_ref() {
                patch.insert(
                    field_format_signal_name(&target, &normalized_field_name),
                    TranslationSignalValue::Text(
                        crate::byte_text::encoding_text(field_formats
                            .get(&normalized_field_name)
                            .cloned()
                            .unwrap_or_default().as_bytes()),
                    ),
                );
            }
            if let Some(has_content_signal_name) = options.has_content_signal_name.as_ref() {
                patch.insert(
                    has_content_signal_name(
                        &InputBytes::from(signals.widget_key.as_str()),
                        &target,
                    ),
                    TranslationSignalValue::Bool(!value.trim().is_empty()),
                );
            }
        }
    }
    patch
}

