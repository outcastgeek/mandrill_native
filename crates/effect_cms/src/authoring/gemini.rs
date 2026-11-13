//! The Gemini authoring translator.
//!
//! This is the reference provider call expressed through the workspace HTTP
//! capability: a `POST` to `v1beta/models/{model}:generateContent` with the concrete
//! Gemini request body (`contents` with one user text part and the
//! `generationConfig` temperature/response media type), the `x-goog-api-key`
//! credential header, and the context-derived `x-server-timeout` header when a
//! deadline is supplied. The response body is read as strict UTF-8, non-2xx
//! statuses are reported as the provider's error message, and the first
//! candidate's text parts are concatenated before the source field parser applies
//! its fence trimming and invalid-escape repair.
//!
//! Credentials, the provider connection and the HTTP runtime are consumer supplies:
//! the source read `GEMINI_API_KEY` and constructed the vendor client in its
//! constructor; the native contract takes the credential as a constructor value and
//! requires `R: HasHttpTransport` for the actual call. Construction performs no
//! I/O. No retry policy, model, prompt or response-repair rule is added.
//!
//! Provider failures keep the transport/status cause in their own variants; the
//! expected error type is never required to be `Clone` or `Sync`.

use std::collections::BTreeMap;
use crate::commands::InputBytes;
use std::sync::Arc;
use std::time::Duration;

use effect::http::{FetchFailure, HttpHeaders, HttpMethod, HttpRequest, RequestOptions};
use effect::{
    Effect, HasHttpTransport, effect, effect_die, effect_map, effect_scope, effect_sync,
    effect_try_sync,
};
use effect_server::Request;
use serde_json::{Value, json};

use super::contracts::{
    AuthoringTranslationField, AuthoringTranslationInput, AuthoringTranslationResult,
};
use super::translation::{
    normalize_authoring_fields, parse_authoring_translation_fields_json,
};

/// The source's default Gemini authoring translation model.
pub const DEFAULT_GEMINI_AUTHORING_TRANSLATION_MODEL: &str = "gemini-3.1-flash-lite";

/// The Gemini Developer API base URL used by the reference client.
pub const GEMINI_API_BASE_URL: &str = "https://generativelanguage.googleapis.com/";

/// The Gemini authoring translator.
#[derive(Clone)]
pub struct GeminiTranslator {
    api_key: String,
    model: String,
}

impl std::fmt::Debug for GeminiTranslator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeminiTranslator")
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

/// A Gemini translator construction or call failure.
#[derive(Debug)]
pub enum GeminiTranslatorError {
    /// The consumer-supplied credential was blank.
    MissingApiKey,
    /// The source and target languages are required.
    SourceAndTargetRequired,
    /// The normalized fields could not be encoded as JSON.
    FieldsEncode(String),
    /// The generation request body could not be encoded.
    RequestEncode(String),
    /// The HTTP request or its response body failed.
    Fetch(FetchFailure),
    /// The provider returned a non-success status.
    Provider(GeminiProviderError),
    /// The provider response was not the expected JSON shape.
    ResponseDecode(String),
    /// The provider response carried no candidate content.
    NoContent,
    /// The provider's field JSON could not be parsed.
    ParseResponse(String),
}

impl std::fmt::Display for GeminiTranslatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingApiKey => {
                formatter.write_str("GEMINI_API_KEY environment variable not set")
            }
            Self::SourceAndTargetRequired => {
                formatter.write_str("source and target languages are required")
            }
            Self::FieldsEncode(error) => {
                write!(formatter, "failed to marshal translation fields: {error}")
            }
            Self::RequestEncode(error) => {
                write!(formatter, "failed to encode generation request: {error}")
            }
            Self::Fetch(failure) => write!(formatter, "failed to translate fields: {}", fetch_text(failure)),
            Self::Provider(error) => write!(formatter, "failed to translate fields: {error}"),
            Self::ResponseDecode(error) => write!(
                formatter,
                "failed to translate fields: deserializeUnaryResponse: error unmarshalling response: {error}"
            ),
            Self::NoContent => formatter.write_str("no translation content generated"),
            Self::ParseResponse(error) => {
                write!(formatter, "failed to parse translation response: {error}")
            }
        }
    }
}

impl std::error::Error for GeminiTranslatorError {}

/// The provider's structured non-success response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeminiProviderError {
    /// The provider status code.
    pub code: u16,
    /// The provider status text.
    pub status: String,
    /// The provider message.
    pub message: String,
}

impl std::fmt::Display for GeminiProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Error {}, Message: {}, Status: {}",
            self.code, self.message, self.status
        )
    }
}

impl std::error::Error for GeminiProviderError {}

/// Construct the translator with the source's default model.
///
/// # Errors
///
/// Returns [`GeminiTranslatorError::MissingApiKey`] when the supplied credential is
/// blank.
pub fn new_gemini_translator(
    api_key: impl Into<String>,
) -> Result<GeminiTranslator, GeminiTranslatorError> {
    new_gemini_translator_with_model(api_key, "")
}

/// Construct the translator with an explicit model.
///
/// A blank model selects [`DEFAULT_GEMINI_AUTHORING_TRANSLATION_MODEL`].
///
/// # Errors
///
/// Returns [`GeminiTranslatorError::MissingApiKey`] when the supplied credential is
/// blank.
pub fn new_gemini_translator_with_model(
    api_key: impl Into<String>,
    model: impl Into<String>,
) -> Result<GeminiTranslator, GeminiTranslatorError> {
    let api_key = api_key.into();
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(GeminiTranslatorError::MissingApiKey);
    }
    let model = model.into();
    let model = model.trim();
    let model = if model.is_empty() {
        DEFAULT_GEMINI_AUTHORING_TRANSLATION_MODEL.to_owned()
    } else {
        model.to_owned()
    };
    Ok(GeminiTranslator {
        api_key: api_key.to_owned(),
        model,
    })
}

impl GeminiTranslator {
    /// Translate one field set.
    ///
    /// `timeout` is the per-call deadline the workflow applied; a zero duration
    /// performs the provider call without the deadline header. The request is the
    /// caller's request-scoped context and is not read by this provider.
    #[must_use]
    pub fn translate_fields<R>(
        &self,
        _request: &Request,
        timeout: Duration,
        input: AuthoringTranslationInput,
    ) -> Effect<AuthoringTranslationResult, GeminiTranslatorError, R>
    where
        R: HasHttpTransport + Send + Sync + 'static,
    {
        let fields = normalize_authoring_fields(&input.fields.to_vec());
        if fields.is_empty() {
            let target_language = input.target_language.trim().to_owned();
            return effect_sync! {
                once;
                body: move || AuthoringTranslationResult {
                    target_language,
                    fields: Default::default(),
                },
            };
        }

        let source_language = input.source_language.trim().to_owned();
        let target_language = input.target_language.trim().to_owned();
        if source_language.is_empty() || target_language.is_empty() {
            return translator_failure(GeminiTranslatorError::SourceAndTargetRequired);
        }

        let fields_json = match serde_json::to_string(&fields) {
            Ok(fields_json) => fields_json,
            Err(error) => {
                return translator_failure(GeminiTranslatorError::FieldsEncode(error.to_string()));
            }
        };
        let scope = {
            let scope = input.scope.trim();
            if scope.is_empty() { InputBytes::from("content") } else { scope }
        };
        let prompt = translation_prompt(
            &crate::byte_text::encoding_text(source_language.as_bytes()),
            &crate::byte_text::encoding_text(target_language.as_bytes()),
            &crate::byte_text::encoding_text(scope.as_bytes()),
            fields_json.as_str(),
        );
        let body = json!({
            "contents": [{
                "parts": [{ "text": prompt }],
                "role": "user",
            }],
            "generationConfig": {
                "temperature": 0.2,
                "responseMimeType": "application/json",
            },
        });
        let body = match serde_json::to_vec(&body) {
            Ok(body) => body,
            Err(error) => {
                return translator_failure(GeminiTranslatorError::RequestEncode(error.to_string()));
            }
        };
        let mut headers = HttpHeaders::new();
        headers.insert(Arc::from("content-type"), Arc::from("application/json"));
        headers.insert(
            Arc::from("x-goog-api-key"),
            Arc::from(self.api_key.as_str()),
        );
        if !timeout.is_zero() {
            let seconds = timeout.as_secs_f64().ceil();
            let seconds = if seconds.is_finite() && seconds >= 0.0 {
                format!("{seconds:.0}")
            } else {
                String::new()
            };
            if !seconds.is_empty() {
                headers.insert(Arc::from("x-server-timeout"), Arc::from(seconds));
            }
        }
        let url = format!(
            "{GEMINI_API_BASE_URL}v1beta/models/{}:generateContent",
            self.model
        );
        let http_request = HttpRequest {
            url: Arc::from(url),
            options: RequestOptions {
                method: HttpMethod::Post,
                headers: Arc::new(headers),
                body: Some(Arc::from(body)),
            },
        };
        let fetch = fetch_generation::<R>(http_request);
        let fields = fields.clone();
        effect! {
            bind;
            program: fetch,
            body: move |(status, text)| {
            let fields = fields.clone();
            let target_language = target_language.clone();
            effect_try_sync! {
                once;
                body: {
                    let __dsl_result =
                        response_result(status, text.as_str(), &fields, &target_language);
                    move || __dsl_result
                },
                on_error: ::core::convert::identity,
            }
            },
        }
    }
}

// Lift one provider failure without requiring `Clone` from the error type.
fn translator_failure<R>(
    error: GeminiTranslatorError,
) -> Effect<AuthoringTranslationResult, GeminiTranslatorError, R>
where
    R: Send + Sync + 'static,
{
    effect_try_sync! {
        once;
        body: {
            let __dsl_result =
                Err::<AuthoringTranslationResult, GeminiTranslatorError>(error);
            move || __dsl_result
        },
        on_error: ::core::convert::identity,
    }
}

/// The exact source translation prompt.
fn translation_prompt(
    source_language: &str,
    target_language: &str,
    scope: &str,
    fields_json: &str,
) -> String {
    format!(
        "Translate the provided authoring fields from {source_language} to {target_language}.

Scope: {scope}

Return JSON with this exact shape:
{{
  \"fields\": {{
    \"field_name\": \"translated value\"
  }}
}}

Rules:
1. Preserve the field keys exactly as provided.
2. Preserve Markdown, Org markup, headings, bullet lists, tables, links, image URLs, code fences, inline code, and YAML/front matter structure.
3. Preserve shortcode tokens exactly, including anything inside square brackets such as [content-link slug=\"...\"].
4. Do not translate URLs, slugs, role names, identifiers, email addresses, or code.
5. If a field is effectively plain text, translate naturally for the target locale.
6. If a field contains formatted content, translate only the human-readable prose while keeping the syntax intact.
7. Do not add commentary, explanations, or extra fields.
8. Keep whitespace reasonable and preserve paragraph boundaries.

Fields JSON:
{fields_json}"
    )
}

/// Fetch the provider response under the caller's HTTP capability.
fn fetch_generation<R>(
    http_request: HttpRequest,
) -> Effect<(u16, String), GeminiTranslatorError, R>
where
    R: HasHttpTransport + Send + Sync + 'static,
{
    effect_scope! {
        body: move |scope| {
            let request_map: Arc<
                dyn Fn(FetchFailure) -> GeminiTranslatorError + Send + Sync,
            > = Arc::new(GeminiTranslatorError::Fetch);
            let response_map = Arc::clone(&request_map);
            let acquire = effect::http::request(
                scope,
                http_request.clone(),
                |_| true,
                move |failure| response_map(failure),
            );
            effect! {
                bind;
                program: acquire,
                body: move |response| {
                    let status = match response.with(|response| response.status().0) {
                        Ok(status) => status,
                        Err(defect) => return effect_die! { defect: defect },
                    };
                    let text_map = Arc::clone(&request_map);
                    effect_map! {
                        program: effect::http::text(response, move |failure| text_map(failure)),
                        map: move |text| (status, text),
                    }
                },
            }
        },
    }
}

/// Project one fetched response into the translated fields.
fn response_result(
    status: u16,
    text: &str,
    fields: &[AuthoringTranslationField],
    target_language: &InputBytes,
) -> Result<AuthoringTranslationResult, GeminiTranslatorError> {
    if !(200..300).contains(&status) {
        return Err(GeminiTranslatorError::Provider(provider_error(status, text)));
    }
    decode_response(text, fields, target_language)
}

/// Decode the provider response and apply the source's field parser.
fn decode_response(
    text: &str,
    fields: &[AuthoringTranslationField],
    target_language: &InputBytes,
) -> Result<AuthoringTranslationResult, GeminiTranslatorError> {
    if text.is_empty() {
        return Err(GeminiTranslatorError::NoContent);
    }
    let value: Value = serde_json::from_str(text).map_err(|error| {
        GeminiTranslatorError::ResponseDecode(format!("{error}\n{text}"))
    })?;
    let candidates = match value.get("candidates") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(candidates)) => candidates.as_slice(),
        Some(_) => {
            return Err(GeminiTranslatorError::ResponseDecode(
                "candidates is not an array".to_owned(),
            ));
        }
    };
    let Some(candidate) = candidates.first() else {
        return Err(GeminiTranslatorError::NoContent);
    };
    let Some(candidate) = candidate.as_object() else {
        return Err(GeminiTranslatorError::ResponseDecode(
            "candidate is not an object".to_owned(),
        ));
    };
    let content = match candidate.get("content") {
        None | Some(Value::Null) => return Err(GeminiTranslatorError::NoContent),
        Some(Value::Object(content)) => content,
        Some(_) => {
            return Err(GeminiTranslatorError::ResponseDecode(
                "candidate content is not an object".to_owned(),
            ));
        }
    };
    let parts = match content.get("parts") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(parts)) => parts.as_slice(),
        Some(_) => {
            return Err(GeminiTranslatorError::ResponseDecode(
                "content parts is not an array".to_owned(),
            ));
        }
    };
    let mut response_text = String::new();
    for part in parts {
        let Some(part) = part.as_object() else {
            return Err(GeminiTranslatorError::ResponseDecode(
                "part is not an object".to_owned(),
            ));
        };
        match part.get("text") {
            None | Some(Value::Null) => {}
            Some(Value::String(text)) => response_text.push_str(text),
            Some(_) => {
                return Err(GeminiTranslatorError::ResponseDecode(
                    "part text is not a string".to_owned(),
                ));
            }
        }
    }
    let parsed = parse_authoring_translation_fields_json(response_text.as_str())
        .map_err(|error| GeminiTranslatorError::ParseResponse(error.to_string()))?;
    let mut translated = BTreeMap::new();
    for field in fields {
        let value = parsed
            .get(&field.name)
            .cloned()
            .unwrap_or_else(|| field.value.clone());
        translated.insert(field.name.clone(), value);
    }
    Ok(AuthoringTranslationResult {
        target_language: target_language.to_owned(),
        fields: crate::definitions::SharedMap::new(translated),
    })
}

/// Build the provider error from a non-success response.
fn provider_error(status: u16, body: &str) -> GeminiProviderError {
    let status_text = http::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .map(|reason| format!("{status} {reason}"))
        .unwrap_or_else(|| status.to_string());
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return GeminiProviderError {
            code: status,
            status: status_text,
            message: body.to_owned(),
        };
    };
    let Some(error) = value.get("error").and_then(Value::as_object) else {
        return GeminiProviderError {
            code: status,
            status: status_text,
            message: body.to_owned(),
        };
    };
    let code = error
        .get("code")
        .and_then(Value::as_u64)
        .and_then(|code| u16::try_from(code).ok())
        .unwrap_or_default();
    let provider_status = error
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    GeminiProviderError {
        code,
        status: provider_status,
        message,
    }
}

/// Render one fetch failure without requiring a display implementation from it.
fn fetch_text(failure: &FetchFailure) -> String {
    match failure {
        FetchFailure::Transport(defect) => format!("native transport: {}", defect.message()),
        FetchFailure::Http(status) => format!("HTTP status {}", status.0),
        FetchFailure::Decode(defect) => format!("response decode: {}", defect.message()),
    }
}
