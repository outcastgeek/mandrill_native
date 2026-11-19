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

use crate::commands::InputBytes;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crate::byte_text::BufferWriter;
use effect::http::{FetchFailure, HttpHeaders, HttpMethod, HttpRequest, RequestOptions};
use effect::memory::{
    Buffer, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use effect::{
    Effect, Exit, HasHttpTransport, effect, effect_die, effect_from_result, effect_future,
    effect_map, effect_scope, effect_sync,
};
use effect_server::Request;
use serde_json::{Value, json};
use std::num::NonZeroUsize;

use super::contracts::{
    AuthoringTranslationField, AuthoringTranslationInput, AuthoringTranslationResult,
    AuthoringTranslator,
};
use super::translation::{
    normalize_authoring_fields, parse_authoring_translation_fields_json, write_normalized_fields,
};

/// The source's default Gemini authoring translation model.
pub const DEFAULT_GEMINI_AUTHORING_TRANSLATION_MODEL: &str = "gemini-3.1-flash-lite";

/// The Gemini Developer API base URL used by the reference client.
pub const GEMINI_API_BASE_URL: &str = "https://generativelanguage.googleapis.com/";

/// The Gemini authoring translator.
#[derive(Clone)]
pub struct GeminiTranslator {
    api_key: Arc<str>,
    model: Arc<str>,
    workspace: Option<Arc<PreparedGeminiWorkspace>>,
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
    /// Prepared working-set admission or native response envelope was exceeded.
    Memory(MemoryError),
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
            Self::Fetch(failure) => write!(
                formatter,
                "failed to translate fields: {}",
                fetch_text(failure)
            ),
            Self::Provider(error) => write!(formatter, "failed to translate fields: {error}"),
            Self::ResponseDecode(error) => write!(
                formatter,
                "failed to translate fields: deserializeUnaryResponse: error unmarshalling response: {error}"
            ),
            Self::NoContent => formatter.write_str("no translation content generated"),
            Self::ParseResponse(error) => {
                write!(formatter, "failed to parse translation response: {error}")
            }
            Self::Memory(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for GeminiTranslatorError {}

/// The selected authoring input, request construction and decode envelope.
#[derive(Clone, Copy)]
#[must_use]
pub struct GeminiWorkingLimits {
    /// Maximum supplied field entries before normalization.
    pub fields: NonZeroUsize,
    /// Aggregate scope, language and field bytes before copies or normalization.
    pub input_bytes: NonZeroUsize,
    /// Maximum encoded JSON for the normalized field list.
    pub field_json_bytes: NonZeroUsize,
    /// Maximum UTF-8 bytes in the provider prompt.
    pub prompt_bytes: NonZeroUsize,
    /// Maximum encoded provider request body bytes.
    pub request_bytes: NonZeroUsize,
    /// Maximum provider-owned response bytes accepted for Effectful decode.
    pub response_bytes: NonZeroUsize,
}

/// Region-backed request construction and concurrent provider call slots.
pub struct PreparedGeminiWorkspace {
    field_json: SlotPool<'static, Buffer<'static>>,
    prompts: SlotPool<'static, Buffer<'static>>,
    requests: SlotPool<'static, Buffer<'static>>,
    fields: SlotPool<'static, FixedVector<'static, AuthoringTranslationField>>,
    limits: GeminiWorkingLimits,
}

impl PreparedGeminiWorkspace {
    /// Size three physical buffers for each admitted call.
    ///
    /// The native HTTP boundary owns its single bounded request copy and its response
    /// allocation. The existing transport API returns that response as a complete body.
    pub fn storage_bytes(
        calls: NonZeroUsize,
        limits: GeminiWorkingLimits,
    ) -> Result<Bytes, MemoryError> {
        let fields = std::alloc::Layout::array::<AuthoringTranslationField>(limits.fields.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Buffer<'static>>::storage_bytes(calls.get())?
            .checked_mul(3)?
            .checked_add(
                SlotPool::<FixedVector<'static, AuthoringTranslationField>>::storage_bytes(
                    calls.get(),
                )?,
            )?
            .checked_add(
                Bytes::new(limits.field_json_bytes.get())
                    .checked_add(Bytes::new(limits.prompt_bytes.get()))?
                    .checked_add(Bytes::new(limits.request_bytes.get()))?
                    .checked_add(Bytes::new(fields.size()))?
                    .checked_mul(calls.get())?,
            )
    }

    /// Prepare the input and request workspaces in the selected region.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        calls: NonZeroUsize,
        limits: GeminiWorkingLimits,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(calls, limits)?;
        Ok(Self {
            field_json: memory.buffers(
                region,
                calls.get(),
                Bytes::new(limits.field_json_bytes.get()),
            )?,
            prompts: memory.buffers(region, calls.get(), Bytes::new(limits.prompt_bytes.get()))?,
            requests: memory.buffers(
                region,
                calls.get(),
                Bytes::new(limits.request_bytes.get()),
            )?,
            fields: memory.slots(region, calls.get(), |_| {
                FixedVector::prepare(memory.allocator(region)?, limits.fields.get())
            })?,
            limits,
        })
    }
}

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
        api_key: Arc::from(api_key),
        model: Arc::from(model),
        workspace: None,
    })
}

impl GeminiTranslator {
    /// Install this translator as the existing CMS authoring provider callback.
    #[must_use]
    pub fn authoring_port<R: HasHttpTransport + Send + Sync + 'static>(
        self,
    ) -> AuthoringTranslator<GeminiTranslatorError, R> {
        Arc::new(move |request, timeout, input| self.translate_fields(request, timeout, input))
    }

    /// Use the same provider and transport with prepared request workspaces.
    #[must_use]
    pub fn with_prepared_workspace(mut self, workspace: Arc<PreparedGeminiWorkspace>) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Translate one field set.
    ///
    /// `timeout` is the per-call deadline the workflow applied; a zero duration
    /// performs the provider call without the deadline header. The request is the
    /// caller's request-scoped context and is not read by this provider.
    pub fn translate_fields<R>(
        &self,
        _request: &Request,
        timeout: Duration,
        input: AuthoringTranslationInput,
    ) -> Effect<AuthoringTranslationResult, GeminiTranslatorError, R>
    where
        R: HasHttpTransport + Send + Sync + 'static,
    {
        if let Some(workspace) = &self.workspace {
            return self.translate_prepared(timeout, input, workspace.clone());
        }
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
            if scope.is_empty() {
                InputBytes::from("content")
            } else {
                scope
            }
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
        let fetch = fetch_generation::<R>(self.http_request(timeout, Arc::from(body)));
        let fields = fields.clone();
        effect! {
            bind;
            program: fetch,
            body: move |(status, text)| {
            let fields = fields.clone();
            let target_language = target_language.clone();
            effect_from_result! {
                once;
                result: response_result(status, text.as_str(), &fields, &target_language),
            }
            },
        }
    }

    fn http_request(&self, timeout: Duration, body: Arc<[u8]>) -> HttpRequest {
        let mut headers = HttpHeaders::new();
        headers.insert(Arc::from("content-type"), Arc::from("application/json"));
        headers.insert(Arc::from("x-goog-api-key"), self.api_key.clone());
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
        HttpRequest {
            url: Arc::from(url),
            options: RequestOptions {
                method: HttpMethod::Post,
                headers: Arc::new(headers),
                body: Some(body),
            },
        }
    }

    fn translate_prepared<R: HasHttpTransport + Send + Sync + 'static>(
        &self,
        timeout: Duration,
        input: AuthoringTranslationInput,
        workspace: Arc<PreparedGeminiWorkspace>,
    ) -> Effect<AuthoringTranslationResult, GeminiTranslatorError, R> {
        let translator = self.clone();
        let input = effect_sync! { once; body: move || input, };
        effect_future! {
            exit_with_context;
            body: move |env: Arc<R>, context| {
                let input = input.clone();
                let workspace = workspace.clone();
                let translator = translator.clone();
                        async move {
                            let input = match effect::effect! {
                                run_in_context;
                                context: context,
                                program: input,
                                environment: env.clone(),
                            }.await {
                                Exit::Success(input) => input,
                                Exit::Fail(error) => return Exit::Fail(error),
                                Exit::Die(defect) => return Exit::Die(defect),
                                Exit::Interrupted => return Exit::Interrupted,
                                Exit::FinalizationFailed(failure) => return Exit::FinalizationFailed(failure),
                            };
                            if context.cancellation().is_requested() {
                                return Exit::Interrupted;
                            }
                            let result = prepare_generation(&workspace, input);
                            let (body, fields, target) = match result {
                                Ok(PreparedGeneration::Empty(target_language)) => return Exit::Success(AuthoringTranslationResult {
                                    target_language, fields: Default::default(),
                                }),
                                Ok(PreparedGeneration::Ready { body, fields, target }) => (body, fields, target),
                                Err(error) => return Exit::Fail(error),
                            };
                            let request = translator.http_request(timeout, Arc::from(body.as_slice()));
                            let response = effect::effect! {
                                run_in_context;
                                context: context,
                                program: fetch_generation(request),
                                environment: env,
                            }.await;
                            let outcome = match response {
                                Exit::Success((status, text)) => {
                                    if let Err(error) = check_bytes(text.len(), workspace.limits.response_bytes) {
                                        return Exit::Fail(error);
                                    }
                                    match response_result(status, &text, fields.0.as_slice(), &target) {
                                        Ok(result) => Exit::Success(result),
                                        Err(error) => Exit::Fail(error),
                                    }
                                }
                                Exit::Fail(error) => Exit::Fail(error),
                                Exit::Die(defect) => Exit::Die(defect),
                                Exit::Interrupted => Exit::Interrupted,
                                Exit::FinalizationFailed(failure) => Exit::FinalizationFailed(failure),
                            };
                            // Retain admission through native cleanup and the bounded decoder.
                            drop(body);
                            outcome
                        }
            },
        }
    }
}

enum PreparedGeneration {
    Empty(InputBytes),
    Ready {
        body: effect::memory::BufferLease<'static>,
        fields: NormalizedFields,
        target: InputBytes,
    },
}

struct NormalizedFields(SlotLease<'static, FixedVector<'static, AuthoringTranslationField>>);
impl Drop for NormalizedFields {
    fn drop(&mut self) {
        self.0.clear();
    }
}

fn check_bytes(length: usize, limit: NonZeroUsize) -> Result<(), GeminiTranslatorError> {
    if length <= limit.get() {
        Ok(())
    } else {
        Err(GeminiTranslatorError::Memory(MemoryError::ValueTooLarge {
            requested: Bytes::new(length),
            capacity: Bytes::new(limit.get()),
        }))
    }
}

fn prepare_generation(
    workspace: &PreparedGeminiWorkspace,
    input: AuthoringTranslationInput,
) -> Result<PreparedGeneration, GeminiTranslatorError> {
    let mut body = workspace
        .requests
        .try_checkout()
        .map_err(GeminiTranslatorError::Memory)?;
    let mut field_json = workspace
        .field_json
        .try_checkout()
        .map_err(GeminiTranslatorError::Memory)?;
    let mut prompt = workspace
        .prompts
        .try_checkout()
        .map_err(GeminiTranslatorError::Memory)?;
    let mut fields = NormalizedFields(
        workspace
            .fields
            .try_checkout()
            .map_err(GeminiTranslatorError::Memory)?,
    );

    let base_bytes = input
        .scope
        .len()
        .checked_add(input.source_language.len())
        .and_then(|total| total.checked_add(input.target_language.len()))
        .ok_or(GeminiTranslatorError::Memory(
            MemoryError::ArithmeticOverflow,
        ))?;
    input.fields.with(|input_fields| {
        if input_fields.len() > workspace.limits.fields.get() {
            return Err(GeminiTranslatorError::Memory(MemoryError::NoSlot));
        }
        let total = input_fields
            .iter()
            .try_fold(base_bytes, |total, field| {
                total
                    .checked_add(field.name.len())?
                    .checked_add(field.value.len())?
                    .checked_add(field.format.len())
            })
            .ok_or(GeminiTranslatorError::Memory(
                MemoryError::ArithmeticOverflow,
            ))?;
        check_bytes(total, workspace.limits.input_bytes)?;
        write_normalized_fields(input_fields, |field| {
            fields
                .0
                .push(field)
                .map_err(|_| GeminiTranslatorError::Memory(MemoryError::NoSlot))
        })
    })?;
    let target = input.target_language.trim();
    if fields.0.is_empty() {
        return Ok(PreparedGeneration::Empty(target));
    }
    let source = input.source_language.trim();
    if source.is_empty() || target.is_empty() {
        return Err(GeminiTranslatorError::SourceAndTargetRequired);
    }

    field_json.clear();
    let mut field_writer = BufferWriter::new(&mut field_json);
    if let Err(error) = serde_json::to_writer(&mut field_writer, fields.0.as_slice()) {
        return Err(field_writer.memory_error().map_or_else(
            || GeminiTranslatorError::FieldsEncode(error.to_string()),
            GeminiTranslatorError::Memory,
        ));
    }
    let field_text = std::str::from_utf8(field_json.as_slice())
        .map_err(|error| GeminiTranslatorError::FieldsEncode(error.to_string()))?;
    let scope = input.scope.trim();
    let scope = if scope.is_empty() {
        b"content"
    } else {
        scope.as_bytes()
    };
    prompt.clear();
    let mut prompt_writer = BufferWriter::new(&mut prompt);
    if write_translation_prompt(
        &crate::byte_text::encoding_text(source.as_bytes()),
        &crate::byte_text::encoding_text(target.as_bytes()),
        &crate::byte_text::encoding_text(scope),
        field_text,
        &mut prompt_writer,
    )
    .is_err()
    {
        return Err(prompt_writer.memory_error().map_or_else(
            || GeminiTranslatorError::RequestEncode("translation prompt formatting".into()),
            GeminiTranslatorError::Memory,
        ));
    }

    let prompt = std::str::from_utf8(prompt.as_slice())
        .map_err(|error| GeminiTranslatorError::RequestEncode(error.to_string()))?;
    body.clear();
    encode_generation_body(prompt, &mut body)?;
    Ok(PreparedGeneration::Ready {
        body,
        fields,
        target,
    })
}

fn encode_generation_body(
    prompt: &str,
    buffer: &mut Buffer<'_>,
) -> Result<(), GeminiTranslatorError> {
    #[derive(serde::Serialize)]
    struct Part<'a> {
        text: &'a str,
    }
    #[derive(serde::Serialize)]
    struct Content<'a> {
        parts: [Part<'a>; 1],
        role: &'static str,
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Configuration {
        response_mime_type: &'static str,
        temperature: f32,
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Generation<'a> {
        contents: [Content<'a>; 1],
        generation_config: Configuration,
    }

    let request = Generation {
        contents: [Content {
            parts: [Part { text: prompt }],
            role: "user",
        }],
        generation_config: Configuration {
            response_mime_type: "application/json",
            temperature: 0.2,
        },
    };
    let mut writer = BufferWriter::new(buffer);
    serde_json::to_writer(&mut writer, &request).map_err(|error| {
        writer.memory_error().map_or_else(
            || GeminiTranslatorError::RequestEncode(error.to_string()),
            GeminiTranslatorError::Memory,
        )
    })
}

// Lift one provider failure without requiring `Clone` from the error type.
fn translator_failure<R>(
    error: GeminiTranslatorError,
) -> Effect<AuthoringTranslationResult, GeminiTranslatorError, R>
where
    R: Send + Sync + 'static,
{
    effect_from_result! { once; result: Err(error), }
}

/// The exact source translation prompt.
fn translation_prompt(
    source_language: &str,
    target_language: &str,
    scope: &str,
    fields_json: &str,
) -> String {
    let mut prompt = String::new();
    let _ = write_translation_prompt(
        source_language,
        target_language,
        scope,
        fields_json,
        &mut prompt,
    );
    prompt
}

fn write_translation_prompt(
    source_language: &str,
    target_language: &str,
    scope: &str,
    fields_json: &str,
    output: &mut impl std::fmt::Write,
) -> std::fmt::Result {
    write!(
        output,
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
fn fetch_generation<R>(http_request: HttpRequest) -> Effect<(u16, String), GeminiTranslatorError, R>
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
        return Err(GeminiTranslatorError::Provider(provider_error(
            status, text,
        )));
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
    let value: Value = serde_json::from_str(text)
        .map_err(|error| GeminiTranslatorError::ResponseDecode(format!("{error}\n{text}")))?;
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
