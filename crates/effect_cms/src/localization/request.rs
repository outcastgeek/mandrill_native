//! Request-backed localized draft collection.

use crate::form_media::recognizes_urlencoded;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use effect::{Effect, effect, effect_try_future, service};
use effect_server::{BodyLimit, ByteFieldBounds, ByteFormPolicy, BytePairs, HasRequest, MultipartPolicy, Request, ServerError, decode_url_encoded_pairs_partial};
use crate::commands::InputBytes;
use crate::validation::trim_unicode_whitespace;
use super::drafts::{LOCALIZED_CONTENT_FIELD_KEY, LOCALIZED_CONTENT_FORMAT_FIELD_KEY, LOCALIZED_DESCRIPTION_FIELD_KEY, LocalizedContentDraft, LocalizedTextDraft};
use super::normalizer::{FormatNormalizer, FormatNormalizerRef, Normalizer, NormalizerRef, localized_field_name};

type FieldValues = BTreeMap<InputBytes, Vec<InputBytes>>;

/// An initialized merged form, preserving repeated values and exact bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalizedFieldValues { values: FieldValues }
impl LocalizedFieldValues {
    /// Use an already initialized form, including an empty form.
    #[must_use]
    pub fn from_decoded(values: FieldValues) -> Self { Self { values } }
    /// Merge URL-encoded body values before query values.
    #[must_use]
    pub fn from_request_parts(body: Option<&BytePairs>, query: Option<&str>) -> Self {
        let mut values = body.map(copy_pairs).unwrap_or_default();
        append_values(&mut values, query_values(query));
        Self { values }
    }
    /// Merge query values before multipart fields.
    #[must_use]
    pub fn from_multipart_parts(body: Option<&BytePairs>, query: Option<&str>) -> Self {
        let mut values = query_values(query);
        if let Some(body) = body { append_values(&mut values, copy_pairs(body)); }
        Self { values }
    }
}
fn copy_pairs(pairs: &BytePairs) -> FieldValues {
    pairs.iter().map(|(key, values)| (InputBytes::from(key.as_slice()), values.iter().map(|v| InputBytes::from(v.as_slice())).collect())).collect()
}
fn append_values(target: &mut FieldValues, incoming: FieldValues) {
    for (key, values) in incoming { target.entry(key).or_default().extend(values); }
}
const MAX_QUERY_PARAMS: usize = 10_000;
fn query_values(query: Option<&str>) -> FieldValues {
    let bytes = query.unwrap_or_default().as_bytes();
    if bytes.split(|b| *b == b'&').count() > MAX_QUERY_PARAMS { return FieldValues::new(); }
    copy_pairs(&decode_url_encoded_pairs_partial(bytes).values)
}
fn first_value(values: &FieldValues, key: &str) -> InputBytes {
    values.get(key.as_bytes()).and_then(|items| items.first()).map(|value| InputBytes::from(trim_unicode_whitespace(value.as_bytes()))).unwrap_or_default()
}
fn reads_body_form(method: &str) -> bool { matches!(method, "POST" | "PUT" | "PATCH") }

/// Collect content drafts from an absent or initialized form.
#[must_use]
pub fn collect_localized_content_drafts_from_values(values: Option<&LocalizedFieldValues>, langs: &[String], normalize_language: Option<NormalizerRef<'_>>, normalize_format: Option<FormatNormalizerRef<'_>>) -> Vec<LocalizedContentDraft> {
    langs.iter().map(|lang| {
        let mut description = InputBytes::default();
        let mut content = InputBytes::default();
        let mut content_format = InputBytes::default();
        if let Some(values) = values {
            description = first_value(&values.values, &localized_field_name(LOCALIZED_DESCRIPTION_FIELD_KEY, lang, normalize_language));
            content = first_value(&values.values, &localized_field_name(LOCALIZED_CONTENT_FIELD_KEY, lang, normalize_language));
            content_format = first_value(&values.values, &localized_field_name(LOCALIZED_CONTENT_FORMAT_FIELD_KEY, lang, normalize_language));
        }
        if let Some(normalize) = normalize_format { content_format = normalize(content_format.as_bytes()); }
        LocalizedContentDraft { lang: lang.clone(), description, content, content_format, required: true }
    }).collect()
}
/// Collect text drafts from an absent or initialized form.
#[must_use]
pub fn collect_localized_text_drafts_from_values(values: Option<&LocalizedFieldValues>, langs: &[String], field_prefix: &str, normalize_language: Option<NormalizerRef<'_>>) -> Vec<LocalizedTextDraft> {
    langs.iter().map(|lang| LocalizedTextDraft { lang: lang.clone(), description: values.map(|values| first_value(&values.values, &localized_field_name(field_prefix, lang, normalize_language))).unwrap_or_default(), required: true }).collect()
}

/// Explicit native limits for each form representation.
#[derive(Clone, Copy, Debug)]
pub struct LocalizedFormPolicy {
    /// URL-encoded limits.
    pub urlencoded: ByteFormPolicy,
    /// Whether the caller already supplied a bounded request body.
    pub body_already_bounded: bool,
    /// Multipart streaming limits.
    pub multipart: MultipartPolicy,
    /// Multipart field count.
    pub multipart_fields: NonZeroUsize,
    /// Multipart field value limits.
    pub multipart_values: ByteFieldBounds,
}
fn initialize_values<E, R>(request: Request, policy: LocalizedFormPolicy, on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync>) -> Effect<LocalizedFieldValues, E, R>
where E: Send + 'static, R: Send + Sync + 'static {
    effect! {
        let request = request.clone();
        let on_error = on_error.clone();
        let media_type = request.headers().get("content-type")
            .map(|value| value.as_bytes().split(|byte| *byte == b';').next().unwrap_or_default())
            .and_then(|base| std::str::from_utf8(base).ok())
            .unwrap_or_default().trim().to_ascii_lowercase();
        let query = request.uri().query().map(str::to_owned);
        if reads_body_form(request.method().as_str()) && request.headers().get("content-type").is_some_and(|value| recognizes_urlencoded(value.as_bytes())) {
            let mut urlencoded = policy.urlencoded;
            if !policy.body_already_bounded && urlencoded.body.bytes() > 10 * 1024 * 1024 {
                urlencoded.body = BodyLimit::new(NonZeroUsize::new(10 * 1024 * 1024).expect("nonzero limit"));
            }
            let map = on_error.clone();
            let read = effect_try_future! {
                body: { let request = request.clone(); move |_| { let request = request.clone(); async move { request.read_partial_byte_form_with_segment_limit(urlencoded, NonZeroUsize::new(MAX_QUERY_PARAMS)).await } } },
                on_error: move |error| map(error),
            }.either()?;
            match read {
                Ok(body) => LocalizedFieldValues::from_request_parts(Some(&body.values), query.as_deref()),
                Err(_) => LocalizedFieldValues::from_request_parts(None, query.as_deref()),
            }
        } else if media_type == "multipart/form-data" {
            let map = on_error.clone();
            let read = effect_try_future! {
                body: { let request = request.clone(); move |_| { let request = request.clone(); async move { request.read_accounted_multipart_fields(policy.multipart, policy.multipart_fields, policy.multipart_values, 32 * 1024 * 1024).await } } },
                on_error: move |error| map(error),
            }.either()?;
            match read {
                Ok(body) => LocalizedFieldValues::from_multipart_parts(Some(&body.values), query.as_deref()),
                Err(_) => LocalizedFieldValues::from_multipart_parts(None, query.as_deref()),
            }
        } else { LocalizedFieldValues::from_request_parts(None, query.as_deref()) }
    }
}

/// Collect content drafts lazily from the current request or an initialized form.
pub fn collect_localized_content_drafts_from_request<E, R>(langs: Vec<String>, normalize_language: Option<Normalizer>, normalize_format: Option<FormatNormalizer>, preinitialized: Option<LocalizedFieldValues>, policy: LocalizedFormPolicy, on_error: impl Fn(ServerError) -> E + Send + Sync + 'static) -> Effect<Vec<LocalizedContentDraft>, E, R>
where E: Send + 'static, R: HasRequest + Send + Sync + 'static {
    let langs = Arc::new(langs);
    let on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync> = Arc::new(on_error);
    effect! {
        let langs = langs.clone();
        let normalize_language = normalize_language.clone();
        let normalize_format = normalize_format.clone();
        let on_error = on_error.clone();
        let mut values = preinitialized.clone();
        let mut drafts = Vec::with_capacity(langs.len());
        for lang in langs.iter() {
            let description_key = localized_field_name(LOCALIZED_DESCRIPTION_FIELD_KEY, lang, normalize_language.as_deref());
            if values.is_none() {
                let request = service! { env: |env: &R| env.request().clone(), }?;
                values = Some(initialize_values::<E, R>(request, policy, on_error.clone())?);
            }
            let fields = &values.as_ref().expect("initialized form").values;
            let description = first_value(fields, &description_key);
            let content_key = localized_field_name(LOCALIZED_CONTENT_FIELD_KEY, lang, normalize_language.as_deref());
            let content = first_value(fields, &content_key);
            let format_key = localized_field_name(LOCALIZED_CONTENT_FORMAT_FIELD_KEY, lang, normalize_language.as_deref());
            let mut content_format = first_value(fields, &format_key);
            if let Some(normalize) = normalize_format.as_deref() { content_format = normalize(content_format.as_bytes()); }
            drafts.push(LocalizedContentDraft { lang: lang.clone(), description, content, content_format, required: true });
        }
        drafts
    }
}
/// Collect text drafts lazily from the current request or an initialized form.
pub fn collect_localized_text_drafts_from_request<E, R>(langs: Vec<String>, field_prefix: &str, normalize_language: Option<Normalizer>, preinitialized: Option<LocalizedFieldValues>, policy: LocalizedFormPolicy, on_error: impl Fn(ServerError) -> E + Send + Sync + 'static) -> Effect<Vec<LocalizedTextDraft>, E, R>
where E: Send + 'static, R: HasRequest + Send + Sync + 'static {
    let langs = Arc::new(langs);
    let field_prefix = field_prefix.to_owned();
    let on_error: Arc<dyn Fn(ServerError) -> E + Send + Sync> = Arc::new(on_error);
    effect! {
        let langs = langs.clone();
        let field_prefix = field_prefix.clone();
        let normalize_language = normalize_language.clone();
        let on_error = on_error.clone();
        let mut values = preinitialized.clone();
        let mut drafts = Vec::with_capacity(langs.len());
        for lang in langs.iter() {
            let key = localized_field_name(&field_prefix, lang, normalize_language.as_deref());
            if values.is_none() {
                let request = service! { env: |env: &R| env.request().clone(), }?;
                values = Some(initialize_values::<E, R>(request, policy, on_error.clone())?);
            }
            let fields = &values.as_ref().expect("initialized form").values;
            drafts.push(LocalizedTextDraft { lang: lang.clone(), description: first_value(fields, &key), required: true });
        }
        drafts
    }
}
