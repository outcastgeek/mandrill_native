//! The authoring translation handler and the localized authoring service.
//!
//! The handler follows the source's branches in order: the consumer's access
//! decision, the translator-availability check, the form parse, then the
//! translation. An absent translator renders the unavailable patch; a failed form
//! read renders the failed-status patch; a translation failure renders the failure
//! patch. Every patch is returned as a completed Datastar SSE reply, exactly as the
//! source wrote one signal event.
//!
//! The source's `RequireAccess` callback wrote its own response and returned false.
//! Here the consumer supplies an effectful decision: `None` continues and
//! `Some(reply)` is returned unchanged. The access decision keeps its own error
//! family inside [`AuthoringHandlerError`].
//!
//! Form initialization uses the canonical byte-preserving bounded input owner.
//! The source's fixed `net/http` form cap is therefore replaced by the caller's
//! declared bounds, matching the localization owner's native form boundary.
//!
//! An optional consumer write adapter selects the configured internal-error reply
//! on an expected precommit failure. Native postcommit transport failures remain
//! owned by the reply/request lifecycle and cannot rewrite an already sent status.
//! Translation diagnostics use named native log annotations and the injected clock.

use std::sync::Arc;
use std::time::Duration;

use effect::{
    Clock, Effect, HasClock, HasLogger, HasTaskSpawner, LogLevel, effect,
    effect_map_error, effect_sync, effect_recover, service, span,
};
use effect_server::{BodyStatus, ByteFormData, Reply, Request, ServerError, reply};
use crate::commands::InputBytes;
use crate::form_media::{MergedFormPolicy, InitializeForm};

use super::contracts::{
    AuthoringAccessCheck, AuthoringHandlerError, AuthoringStatusTextFunc,
    AuthoringTranslationError, AuthoringTranslationField, AuthoringTranslationPatchOptions,
    AuthoringTranslationRequest, AuthoringTranslationSignals, AuthoringTranslator,
    LocalizedAuthoringTranslationPatchDefaults, PatchOptionsFunc, SignalsForWidgetFunc,
};
use super::patch::{
    AuthoringTranslationPatch, authoring_translation_failure_patch,
    authoring_translation_signals_from_localized, authoring_translation_status_patch,
    authoring_translation_unavailable_patch, write_authoring_translation_signal_patch,
};
use super::request::{AuthoringForm, parse_authoring_translation_request, read_authoring_form};
use super::translation::{
    build_authoring_translation_patch, normalize_authoring_fields, normalize_authoring_language,
    normalize_target_languages,
};
use crate::forms::{
    LOCALIZED_TRANSLATION_WIDGET_KEY_FIELD_NAME, LocalizedAuthoringSignals,
    localized_authoring_signals_for_widget, localized_field_signal_name,
    localized_format_signal_name, localized_has_content_signal_name,
};
use crate::localization::TranslateKeyFunc;
use super::contracts::AuthoringLanguageNormalizer;
use crate::routing::{RouteHandler, translate as translate_key};

/// The default localized authoring translation deadline.
pub const DEFAULT_AUTHORING_TRANSLATION_TIMEOUT: Duration = Duration::from_secs(60);

/// The ready-label key.
pub const AUTHORING_TRANSLATION_READY_LABEL_KEY: &str = "localized_tabs.auto_translate_ready";
/// The done-label key.
pub const AUTHORING_TRANSLATION_DONE_LABEL_KEY: &str = "localized_tabs.auto_translate_done";
/// The failed-label key.
pub const AUTHORING_TRANSLATION_FAILED_LABEL_KEY: &str = "localized_tabs.auto_translate_failed";
/// The unavailable-label key.
pub const AUTHORING_TRANSLATION_UNAVAILABLE_LABEL_KEY: &str =
    "localized_tabs.auto_translate_unavailable";

/// Optional consumer response-write boundary. Native completed replies need no adapter.
/// An expected adapter failure selects the configured internal-error response;
/// native committed-body delivery failures remain owned by the request host.
pub type AuthoringPatchDelivery<A,T,R> = Arc<dyn Fn(&Request,Reply<AuthoringHandlerError<A,T>,R>)
    ->Effect<Reply<AuthoringHandlerError<A,T>,R>,ServerError,R>+Send+Sync+'static>;

/// Configures the authoring translation handler.
pub struct AuthoringTranslationHandlerOptions<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The consumer's translation callback; absent keeps the unavailable branch.
    pub translator: Option<AuthoringTranslator<T, R>>,
    /// The per-target deadline; zero performs the call without a deadline.
    pub timeout: Duration,
    /// The consumer's access decision.
    pub require_access: Option<AuthoringAccessCheck<A, T, R>>,
    /// The submitted widget-key field name.
    pub widget_key_field_name: String,
    /// Resolves the authoring signals for a widget key.
    pub signals_for_widget: Option<SignalsForWidgetFunc>,
    /// Resolves the patch configuration for a request and its signals.
    pub patch_options: Option<PatchOptionsFunc>,
    /// Resolves the unavailable status text.
    pub unavailable_status_text: Option<AuthoringStatusTextFunc>,
    /// Resolves the failed status text.
    pub failed_status_text: Option<AuthoringStatusTextFunc>,
    /// The source's status code for a failed response write.
    pub internal_error_status_code: u16,
    /// Optional native consumer write boundary.
    pub deliver_patch: Option<AuthoringPatchDelivery<A,T,R>>,
    /// The native form-read policy.
    pub form_policy: MergedFormPolicy,
    /// Already initialized form values; bypasses body reads.
    pub form: Option<Arc<ByteFormData>>,
}

/// Configures the localized authoring translation handler.
pub struct LocalizedAuthoringTranslationHandlerOptions<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    /// The consumer's translation callback; absent keeps the unavailable branch.
    pub translator: Option<AuthoringTranslator<T, R>>,
    /// The per-target deadline; zero selects the local default.
    pub timeout: Duration,
    /// The consumer's access decision.
    pub require_access: Option<AuthoringAccessCheck<A, T, R>>,
    /// The translation-key port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// The caller's language normalizer.
    pub normalize_language: Option<AuthoringLanguageNormalizer>,
    /// The source's status code for a failed response write.
    pub internal_error_status_code: u16,
    /// Optional native consumer write boundary.
    pub deliver_patch: Option<AuthoringPatchDelivery<A,T,R>>,
    /// The native form-read policy.
    pub form_policy: MergedFormPolicy,
    /// Already initialized form values; bypasses body reads.
    pub form: Option<Arc<ByteFormData>>,
}

/// A consumer service that produces the localized authoring handler.
pub trait AuthoringService<A, T, R>: Send + Sync
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Produce the localized authoring translation handler.
    fn translate_localized_content(&self) -> RouteHandler<AuthoringHandlerError<A, T>, R>;
}

/// The consumer-facing localized authoring service.
pub struct LocalizedAuthoringService<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    options: LocalizedAuthoringTranslationHandlerOptions<A, T, R>,
}

impl<A, T, R> Clone for LocalizedAuthoringService<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            options: clone_localized_options(&self.options),
        }
    }
}

impl<A, T, R> LocalizedAuthoringService<A, T, R>
where
    A: Send + 'static,
    T: std::fmt::Display + Send + 'static,
    R: HasClock + HasLogger + HasTaskSpawner + Send + Sync + 'static,
{
    /// Construct the service from its complete configuration.
    #[must_use]
    pub fn new(options: LocalizedAuthoringTranslationHandlerOptions<A, T, R>) -> Self {
        Self { options }
    }

    /// Produce the localized authoring translation handler.
    #[must_use]
    pub fn translate_localized_content(&self) -> RouteHandler<AuthoringHandlerError<A, T>, R> {
        new_localized_authoring_translation_handler(clone_localized_options(&self.options))
    }
}

impl<A, T, R> AuthoringService<A, T, R> for LocalizedAuthoringService<A, T, R>
where
    A: Send + 'static,
    T: std::fmt::Display + Send + 'static,
    R: HasClock + HasLogger + HasTaskSpawner + Send + Sync + 'static,
{
    fn translate_localized_content(&self) -> RouteHandler<AuthoringHandlerError<A, T>, R> {
        LocalizedAuthoringService::translate_localized_content(self)
    }
}

/// Clone the localized options without requiring `A`, `T` or `R` to be `Clone`.
fn clone_localized_options<A, T, R>(
    options: &LocalizedAuthoringTranslationHandlerOptions<A, T, R>,
) -> LocalizedAuthoringTranslationHandlerOptions<A, T, R>
where
    A: Send + 'static,
    T: Send + 'static,
    R: Send + Sync + 'static,
{
    LocalizedAuthoringTranslationHandlerOptions {
        translator: options.translator.clone(),
        timeout: options.timeout,
        require_access: options.require_access.clone(),
        translate_key: options.translate_key.clone(),
        normalize_language: options.normalize_language.clone(),
        internal_error_status_code: options.internal_error_status_code,
        deliver_patch: options.deliver_patch.clone(),
        form_policy: options.form_policy,
        form: options.form.clone(),
    }
}

/// Build the localized patch options.
#[must_use]
pub fn localized_authoring_translation_patch_options(
    request: &Request,
    signals: &LocalizedAuthoringSignals,
    defaults: &LocalizedAuthoringTranslationPatchDefaults,
) -> AuthoringTranslationPatchOptions {
    let field_normalize = defaults.normalize_language.clone();
    let format_normalize = defaults.normalize_language.clone();
    let content_normalize = defaults.normalize_language.clone();
    AuthoringTranslationPatchOptions {
        signals: authoring_translation_signals_from_localized(signals),
        normalize_language: defaults.normalize_language.clone(),
        field_signal_name: Some(Arc::new(move |lang: &InputBytes, field: &InputBytes| {
            let language=normalize_authoring_language(lang,field_normalize.as_deref());
            localized_field_signal_name(&crate::byte_text::encoding_text(language.as_bytes()), &crate::byte_text::encoding_text(field.as_bytes()), None)
        })),
        field_format_signal_name: Some(Arc::new(move |lang: &InputBytes, field: &InputBytes| {
            let language=normalize_authoring_language(lang,format_normalize.as_deref());
            localized_format_signal_name(&crate::byte_text::encoding_text(language.as_bytes()), &crate::byte_text::encoding_text(field.as_bytes()), None)
        })),
        has_content_signal_name: Some(Arc::new(move |widget: &InputBytes, lang: &InputBytes| {
            let language=normalize_authoring_language(lang,content_normalize.as_deref());
            localized_has_content_signal_name(&crate::byte_text::encoding_text(widget.as_bytes()), &crate::byte_text::encoding_text(language.as_bytes()), None)
        })),
        ready_status_text: translate_key(
            defaults.translate_key.as_ref(),
            request,
            AUTHORING_TRANSLATION_READY_LABEL_KEY,
        ),
        done_status_text: translate_key(
            defaults.translate_key.as_ref(),
            request,
            AUTHORING_TRANSLATION_DONE_LABEL_KEY,
        ),
        failed_status_text: translate_key(
            defaults.translate_key.as_ref(),
            request,
            AUTHORING_TRANSLATION_FAILED_LABEL_KEY,
        ),
        submit_error_open_signal: "locSubmitErrorOpen".to_owned(),
        submit_error_text_signal: "locSubmitErrorText".to_owned(),
    }
}

/// Build the localized authoring translation handler.
#[must_use]
pub fn new_localized_authoring_translation_handler<A, T, R>(
    options: LocalizedAuthoringTranslationHandlerOptions<A, T, R>,
) -> RouteHandler<AuthoringHandlerError<A, T>, R>
where
    A: Send + 'static,
    T: std::fmt::Display + Send + 'static,
    R: HasClock + HasLogger + HasTaskSpawner + Send + Sync + 'static,
{
    let timeout = if options.timeout.is_zero() {
        DEFAULT_AUTHORING_TRANSLATION_TIMEOUT
    } else {
        options.timeout
    };
    let translate_key_port = options.translate_key.clone();
    let normalize_language = options.normalize_language.clone();
    let patch_key = translate_key_port.clone();
    let patch_normalize = normalize_language.clone();
    let unavailable_key = translate_key_port.clone();
    let failed_key = translate_key_port;
    new_authoring_translation_handler(AuthoringTranslationHandlerOptions {
        translator: options.translator,
        timeout,
        require_access: options.require_access,
        widget_key_field_name: LOCALIZED_TRANSLATION_WIDGET_KEY_FIELD_NAME.to_owned(),
        signals_for_widget: Some(Arc::new(|widget_key: &InputBytes| {
            localized_authoring_signals_for_widget(&crate::byte_text::encoding_text(widget_key.as_bytes()))
        })),
        patch_options: Some(Arc::new(move |request: &Request, signals| {
            localized_authoring_translation_patch_options(
                request,
                signals,
                &LocalizedAuthoringTranslationPatchDefaults {
                    translate_key: patch_key.clone(),
                    normalize_language: patch_normalize.clone(),
                },
            )
        })),
        unavailable_status_text: Some(Arc::new(move |request: &Request| {
            translate_key(
                unavailable_key.as_ref(),
                request,
                AUTHORING_TRANSLATION_UNAVAILABLE_LABEL_KEY,
            )
        })),
        failed_status_text: Some(Arc::new(move |request: &Request| {
            translate_key(
                failed_key.as_ref(),
                request,
                AUTHORING_TRANSLATION_FAILED_LABEL_KEY,
            )
        })),
        internal_error_status_code: options.internal_error_status_code,
        deliver_patch: options.deliver_patch.clone(),
        form_policy: options.form_policy,
        form: options.form.clone(),
    })
}

/// Build the authoring translation handler.
#[must_use]
pub fn new_authoring_translation_handler<A, T, R>(
    options: AuthoringTranslationHandlerOptions<A, T, R>,
) -> RouteHandler<AuthoringHandlerError<A, T>, R>
where
    A: Send + 'static,
    T: std::fmt::Display + Send + 'static,
    R: HasClock + HasLogger + HasTaskSpawner + Send + Sync + 'static,
{
    let options = Arc::new(options);
    Arc::new(move |request: &Request| handler_effect(Arc::clone(&options), request.clone()))
}

/// The complete handler effect.
fn handler_effect<A, T, R>(
    options: Arc<AuthoringTranslationHandlerOptions<A, T, R>>,
    request: Request,
) -> Effect<Reply<AuthoringHandlerError<A, T>, R>, AuthoringHandlerError<A, T>, R>
where
    A: Send + 'static,
    T: std::fmt::Display + Send + 'static,
    R: HasClock + HasLogger + HasTaskSpawner + Send + Sync + 'static,
{
    effect! {
        let options=options.clone();let request=request.clone();
        let denied=match options.require_access.as_ref() {
            Some(check)=>check(&request)?, None=>None,
        };
        match denied {
            Some(reply)=>reply,
            None=>after_access(options.clone(),request.clone())?,
        }
    }
}

/// Read the form and continue the handler.
fn after_access<A, T, R>(
    options: Arc<AuthoringTranslationHandlerOptions<A, T, R>>,
    request: Request,
) -> Effect<Reply<AuthoringHandlerError<A, T>, R>, AuthoringHandlerError<A, T>, R>
where
    A: Send + 'static,
    T: std::fmt::Display + Send + 'static,
    R: HasClock + HasLogger + HasTaskSpawner + Send + Sync + 'static,
{
    let form = read_authoring_form::<AuthoringHandlerError<A, T>, R>(
        request.clone(),
        options.form_policy,
        options.form.clone(),
        if options.translator.is_none() { InitializeForm::FormValue } else { InitializeForm::ParseForm },
    );
    let options_for_body = Arc::clone(&options);
    let request_for_body = request;
    effect! {
        bind;
        program: form,
        body: move |form| after_form(options_for_body.clone(), request_for_body.clone(), form),
    }
}

/// Continue the handler after the form read.
fn after_form<A, T, R>(
    options: Arc<AuthoringTranslationHandlerOptions<A, T, R>>,
    request: Request,
    form: AuthoringForm,
) -> Effect<Reply<AuthoringHandlerError<A, T>, R>, AuthoringHandlerError<A, T>, R>
where
    A: Send + 'static,
    T: std::fmt::Display + Send + 'static,
    R: HasClock + HasLogger + HasTaskSpawner + Send + Sync + 'static,
{
    let signals = localized_signals_for_widget(
        &form.first(&options.widget_key_field_name),
        options.signals_for_widget.as_ref(),
    );

    if options.translator.is_none() {
        let translation_signals = authoring_translation_signals_from_localized(&signals);
        let unavailable_text =
            resolved_status_text(options.unavailable_status_text.as_ref(), &request);
        let patch = authoring_translation_unavailable_patch(
            &translation_signals,
            unavailable_text.as_str(),
        );
        return deliver_patch(options,request,patch,translation_signals);
    }

    if form.error().is_some() {
        let translation_signals = authoring_translation_signals_from_localized(&signals);
        let status=resolved_status_text(options.failed_status_text.as_ref(), &request);
        let patch = authoring_translation_status_patch(&translation_signals, &status);
        return deliver_patch(options,request,patch,translation_signals);
    }

    let translation_request = parse_authoring_translation_request(
        &form,
        options.widget_key_field_name.as_str(),
    );
    let mut patch_options = match options.patch_options.as_ref() {
        Some(resolve) => resolve(&request, &signals),
        None => AuthoringTranslationPatchOptions::default(),
    };
    if patch_options.signals == AuthoringTranslationSignals::default() {
        patch_options.signals = authoring_translation_signals_from_localized(&signals);
    }
    let failure_options = patch_options.clone();
    let failure_signals = patch_options.signals.clone();
    let start: Effect<Duration, AuthoringHandlerError<A, T>, R> =
        service! { env: |env: &R| env.clock().now(), };
    let request_for_translation = request;
    let handler_options=options.clone();
    let translator_for_translation = options.translator.clone();
    let timeout = options.timeout;
    effect! {
        bind;
        program: start,
        body: move |started: Duration| {
            run_authoring_translation::<A, T, R>(
                request_for_translation.clone(),
                translator_for_translation.clone(),
                timeout,
                translation_request.clone(),
                patch_options.clone(),
                failure_options.clone(),
                failure_signals.clone(),
                handler_options.clone(),
                started,
            )
        },
    }
}

/// Run the translation and render its signal patch.
///
/// The failure branch emits the source's failure diagnostic through the environment
/// logger before rendering the failure patch; the start instant came from the same
/// injected clock.
fn run_authoring_translation<A, T, R>(
    request: Request,
    translator: Option<AuthoringTranslator<T, R>>,
    timeout: Duration,
    translation_request: AuthoringTranslationRequest,
    patch_options: AuthoringTranslationPatchOptions,
    failure_options: AuthoringTranslationPatchOptions,
    failure_signals: AuthoringTranslationSignals,
    handler_options: Arc<AuthoringTranslationHandlerOptions<A,T,R>>,
    started: Duration,
) -> Effect<Reply<AuthoringHandlerError<A, T>, R>, AuthoringHandlerError<A, T>, R>
where
    A: Send + 'static,
    T: std::fmt::Display + Send + 'static,
    R: HasClock + HasLogger + HasTaskSpawner + Send + Sync + 'static,
{
    let build = build_authoring_translation_patch::<T, R>(
        &request,
        translator,
        timeout,
        translation_request.clone(),
        patch_options,
    );
    let materialized: Effect<
        Result<AuthoringTranslationPatch, AuthoringTranslationError<T>>,
        AuthoringHandlerError<A, T>,
        R,
    > = effect_map_error! {
        program: build.either(),
        map: AuthoringHandlerError::<A,T>::Translation,
    };
    effect! {
        bind;
        program: materialized,
        body: move |exit| match exit {
            Ok(patch) => deliver_patch(handler_options.clone(),request.clone(),patch,failure_signals.clone()),
            Err(failure) => {
                let log = log_authoring_translation_failure::<A, T, R>(
                    &translation_request,
                    &failure_options,
                    &failure,
                    started,
                );
                let patch = authoring_translation_failure_patch(&failure_options);
                let signals = failure_signals.clone();
                let handler_options=handler_options.clone();let request=request.clone();
                effect! {
                    bind;
                    program: log,
                    body: move |_| {
                        deliver_patch(handler_options.clone(),request.clone(),patch.clone(),signals.clone())
                    },
                }
            }
        },
    }
}

/// Resolve fallback status at the write boundary and preserve all native exits.
fn deliver_patch<A,T,R>(options:Arc<AuthoringTranslationHandlerOptions<A,T,R>>,request:Request,
    patch:AuthoringTranslationPatch,signals:AuthoringTranslationSignals)
    ->Effect<Reply<AuthoringHandlerError<A,T>,R>,AuthoringHandlerError<A,T>,R>
where A:Send+'static,T:Send+'static,R:Send+Sync+'static {
    let fallback=resolved_status_text(options.failed_status_text.as_ref(),&request);
    let reply=write_authoring_translation_signal_patch(&patch,&signals,&fallback);
    let Some(deliver)=&options.deliver_patch else { return effect_sync! {once;body:move || reply}; };
    let delivery=effect_map_error! { program:deliver(&request,reply),map:AuthoringHandlerError::<A,T>::Delivery };
    let status=options.internal_error_status_code;
    effect_recover!{program:delivery,recover:move |_failure|effect_sync!{once;body:move ||authoring_write_failure_reply(status)}}

}

/// Precommit fallback for a consumer write adapter; postcommit delivery stays native.
pub fn authoring_write_failure_reply<E:Send+'static,R:Send+Sync+'static>(status:u16)->Reply<E,R> {
    let code=http::StatusCode::from_u16(if status==0 {500} else {status}).unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
    let status=BodyStatus::new(code).unwrap_or(BodyStatus::Ok);
    reply! {status(status);header(http::header::X_CONTENT_TYPE_OPTIONS,http::HeaderValue::from_static("nosniff"));text("\n".to_owned());}
}

/// Emit the source's translation-failure diagnostic through the environment logger.
fn log_authoring_translation_failure<A, T, R>(
    request: &AuthoringTranslationRequest,
    options: &AuthoringTranslationPatchOptions,
    failure: &AuthoringTranslationError<T>,
    started: Duration,
) -> Effect<(), AuthoringHandlerError<A, T>, R>
where
    A: Send + 'static,
    T: std::fmt::Display + Send + 'static,
    R: HasClock + HasLogger + Send + Sync + 'static,
{
    let source_language = normalize_authoring_language(
        &request.source_language,
        options.normalize_language.as_deref(),
    );
    let target_languages = normalize_target_languages(
        &source_language,
        &request.target_languages,
        options.normalize_language.as_deref(),
    );
    let fields = normalize_authoring_fields(&request.fields);
    let field_bytes = authoring_translation_field_byte_summary(&fields);
    let scope = crate::byte_text::encoding_text(request.scope.trim().as_bytes());
    let widget_key = crate::byte_text::encoding_text(request.widget_key.trim().as_bytes());
    let target_text = target_languages.iter().map(|v|crate::byte_text::encoding_text(v.as_bytes())).collect::<Vec<_>>().join(",");
    let field_count = fields.len();
    let target_count = target_languages.len();
    let error_text = failure.to_string();
    let source_language=crate::byte_text::encoding_text(source_language.as_bytes());
    let fields = [ ("error",error_text),("scope",scope),("widget_key",widget_key),
        ("source_language",source_language),("target_languages",target_text),("field_bytes",field_bytes),
        ("field_count",field_count.to_string()),("target_count",target_count.to_string()) ];
    let annotations:effect::Annotations=fields.into_iter().map(|(k,v)|(Arc::<str>::from(k),Arc::<str>::from(v))).collect();
    effect! {
        let mut annotations = annotations.clone();
        let now = service! { env: |env: &R| env.clock().now(), }?;
        let elapsed = now.saturating_sub(started);
        annotations.insert(Arc::from("duration_ms"),Arc::from(elapsed.as_millis().to_string()));
        span!{annotations:annotations,effect:Effect::<(), AuthoringHandlerError<A, T>, R>::log(LogLevel::Error,"contentmgmt authoring translation failed")}?;
    }
}

/// Summarize each field's byte length in request order.
fn authoring_translation_field_byte_summary(fields: &[AuthoringTranslationField]) -> String {
    let mut parts = Vec::with_capacity(fields.len());
    for field in fields {
        let name = field.name.trim();
        let name = if name.is_empty() { "field".to_owned() } else { crate::byte_text::encoding_text(name.as_bytes()) };
        parts.push(format!("{name}:{}", field.value.len()));
    }
    parts.join(",")
}

/// Resolve the localized authoring signals for one widget key.
fn localized_signals_for_widget(
    widget_key: &InputBytes,
    resolve: Option<&SignalsForWidgetFunc>,
) -> LocalizedAuthoringSignals {
    match resolve {
        Some(resolve) => resolve(&widget_key.trim()),
        None => LocalizedAuthoringSignals::default(),
    }
}

/// Resolve one optional status text.
fn resolved_status_text(
    resolve: Option<&AuthoringStatusTextFunc>,
    request: &Request,
) -> String {
    match resolve {
        Some(resolve) => resolve(request),
        None => String::new(),
    }
}

