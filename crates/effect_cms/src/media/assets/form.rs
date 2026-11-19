//! Media asset form input parsing and defaults.
//!
//! These are the asset owner's form conversions: default new-asset input, record to
//! input, decoded merged form values to input, and the request form read under the
//! caller's explicit policy. No store,
//! database, SQL or backend exists here.

use crate::commands::InputBytes;
use std::sync::Arc;

use effect::{Effect, effect, effect_map, effect_sync};
use effect_server::{ByteFormData, Request, ServerError};

use crate::attachments::{InitializeForm, MediaFormPolicy, initialize_media_form};

use super::model::{
    MEDIA_KIND_IMAGE, MEDIA_SOURCE_KIND_REMOTE, MEDIA_STATUS_READY, MediaAssetFormInput,
    MediaAssetRecord,
};
use super::normalize::{
    normalize_media_kind, normalize_media_source_kind, normalize_media_status,
    parse_optional_positive_int,
};

/// The default new-asset form input.
#[must_use]
pub fn default_media_asset_form_input() -> MediaAssetFormInput {
    MediaAssetFormInput {
        kind: MEDIA_KIND_IMAGE.to_owned(),
        source_kind: MEDIA_SOURCE_KIND_REMOTE.to_owned(),
        status: MEDIA_STATUS_READY.to_owned(),
        ..MediaAssetFormInput::default()
    }
}

/// Parse a stored asset record into form input.
#[must_use]
pub fn media_asset_form_input_from_record(record: &MediaAssetRecord) -> MediaAssetFormInput {
    MediaAssetFormInput {
        kind: normalize_media_kind(&record.kind),
        source_kind: normalize_media_source_kind(&record.source_kind),
        url: record.url.trim().to_owned(),
        provider: record.provider.trim().to_owned(),
        title: record.title.trim().to_owned(),
        description: record.description.trim().to_owned(),
        thumbnail_url: record.thumbnail_url.trim().to_owned(),
        alt: record.alt.trim().to_owned(),
        duration: record.duration.clone(),
        status: normalize_media_status(&record.status),
    }
}

/// Parse media asset form input from decoded merged form values.
#[must_use]
pub fn media_asset_form_input_from_form(form: Option<&ByteFormData>) -> MediaAssetFormInput {
    from_values(|key| {
        form.and_then(|form| form.values.get(key.as_bytes()))
            .and_then(|values| values.first())
            .map(Vec::as_slice)
            .unwrap_or_default()
    })
}

fn from_values<'a>(value: impl Fn(&'static str) -> &'a [u8]) -> MediaAssetFormInput {
    let trimmed = |key| InputBytes::from(crate::validation::trim_unicode_whitespace(value(key)));
    MediaAssetFormInput {
        kind: normalize_media_kind(value("kind")),
        source_kind: normalize_media_source_kind(value("source_kind")),
        url: trimmed("url"),
        provider: trimmed("provider"),
        title: trimmed("title"),
        description: trimmed("description"),
        thumbnail_url: trimmed("thumbnail_url"),
        alt: trimmed("alt"),
        duration: parse_optional_positive_int(value("duration"))
            .map(|value| std::sync::Arc::new(std::sync::atomic::AtomicI64::new(value))),
        status: normalize_media_status(value("status")),
    }
}

/// Admit the shared merged form and copy only selected media metadata into the
/// existing consumer DTO. The field-selection and diagnostic policy is unchanged.
pub fn media_asset_form_input_from_prepared_request<E, R>(
    request: &Request,
    policy: MediaFormPolicy,
    storage: &Arc<crate::forms::PreparedMergedForms>,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<MediaAssetFormInput, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    effect_map! {
        program: storage.read_reusing_validation(request, policy, InitializeForm::FormValue, on_error),
        map: |form| media_asset_form_input_from_prepared_form(&form),
    }
}

/// Read an already admitted form with the existing media field precedence.
#[must_use]
pub fn media_asset_form_input_from_prepared_form(
    form: &crate::forms::PreparedMergedForm,
) -> MediaAssetFormInput {
    from_values(|key| form.values(key.as_bytes()).next().unwrap_or_default())
}

/// Decode one request's merged form values.
///
/// An initialized form bypasses parsing. Otherwise the native request owner supplies
/// body and query values under the selected policy; parse diagnostics are ignored.
/// This helper also supplies media surface and content identity reads.
pub(crate) fn media_request_form<E, R>(
    request: &Request,
    preinitialized: Option<Arc<ByteFormData>>,
    policy: MediaFormPolicy,
    on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
) -> Effect<ByteFormData, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    if let Some(form) = preinitialized {
        return effect_sync! { body: move || ByteFormData { values: form.values.clone() } };
    }
    let read =
        initialize_media_form::<E, R>(request, policy, InitializeForm::FormValue, move |error| {
            on_error(error)
        });
    effect! {
        let read = read.clone();
        let (form, _diagnostic) = read?;
        form
    }
}

/// Parse media asset form input from a request.
pub fn media_asset_form_input_from_request<E, R>(
    request: &Request,
    preinitialized: Option<Arc<ByteFormData>>,
    policy: MediaFormPolicy,
    on_error: impl Fn(ServerError) -> E + Send + Sync + 'static,
) -> Effect<MediaAssetFormInput, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync> = Arc::new(on_error);
    let form = media_request_form::<E, R>(request, preinitialized, policy, on_error);
    effect! {
        let form = form.clone();
        let form = form?;
        media_asset_form_input_from_form(Some(&form))
    }
}
