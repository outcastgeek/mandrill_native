//! The authoring owner's translation contracts.
//!
//! This module holds the values shared by the translation workflow, the provider
//! adapter and the HTTP handler: the translation field/input/result records, the
//! decoded request, the authoring signal/patch configuration, and the callback
//! ports through which consumers supply their translator and access decision.
//!
//! Every callback is stored and reused, so it keeps the workspace callback
//! contract (`Fn + Send + Sync + 'static`). The translator and access ports keep
//! their own error families; no `Clone`, `Sync` or additional bound is imposed on
//! those error types. The access port returns the handler's complete error type
//! because its response is returned unchanged by that handler.

use std::fmt;
use std::time::Duration;
use std::sync::Arc;

use effect::Effect;
use effect_server::{Reply, Request};
use serde::{Deserialize, Serialize};

use crate::forms::LocalizedAuthoringSignals;
use crate::definitions::{SharedList,SharedMap};
use crate::localization::TranslateKeyFunc;
use crate::commands::InputBytes;

/// Byte-preserving authoring language policy.
pub type AuthoringLanguageNormalizer = Arc<dyn Fn(&InputBytes) -> InputBytes + Send + Sync + 'static>;
/// Borrowed authoring language policy.
pub type AuthoringLanguageNormalizerRef<'a> = &'a (dyn Fn(&InputBytes) -> InputBytes + Send + Sync);
use crate::routing::AccessControl;

/// The source's management access-control record, promoted through this owner.
pub type ManagementAccessControl<E, R> = AccessControl<E, R>;

/// One authoring field to translate.
///
/// Serialization keeps the provider field shape: `name` and `value` are always
/// present, while an empty `format` is omitted.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthoringTranslationField {
    /// The provider field name.
    pub name: InputBytes,
    /// The exact field value; the workflow never trims this value.
    pub value: InputBytes,
    /// The field format, or empty when the request supplied none.
    #[serde(default, skip_serializing_if = "InputBytes::is_empty")]
    pub format: InputBytes,
}

/// One translation request for a single target language.
#[derive(Clone, Debug, Default)]
pub struct AuthoringTranslationInput {
    /// The authoring scope.
    pub scope: InputBytes,
    /// The normalized source language.
    pub source_language: InputBytes,
    /// The normalized target language.
    pub target_language: InputBytes,
    /// The normalized fields.
    pub fields: SharedList<AuthoringTranslationField>,
}

/// One translation result.
#[derive(Clone, Debug, Default)]
pub struct AuthoringTranslationResult {
    /// The target language the provider reported.
    pub target_language: InputBytes,
    /// The translated values keyed by field name.
    pub fields: SharedMap<InputBytes, InputBytes>,
}

/// One decoded authoring translation request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthoringTranslationRequest {
    /// The trimmed authoring scope.
    pub scope: InputBytes,
    /// The trimmed translation widget key.
    pub widget_key: InputBytes,
    /// The trimmed source language.
    pub source_language: InputBytes,
    /// The raw repeated target languages.
    pub target_languages: Vec<InputBytes>,
    /// The raw requested fields, in request order.
    pub fields: Vec<AuthoringTranslationField>,
}

/// The translation signal names scoped to one authoring widget.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthoringTranslationSignals {
    /// The normalized widget key.
    pub widget_key: String,
    /// The translation-available signal.
    pub translation_available: String,
    /// The translating signal.
    pub translating: String,
    /// The status-text signal.
    pub status_text: String,
}

/// Resolves the authoring signals for one widget key.
pub type SignalsForWidgetFunc =
    Arc<dyn Fn(&InputBytes) -> LocalizedAuthoringSignals + Send + Sync + 'static>;

/// Resolves the patch configuration for one request and its signals.
pub type PatchOptionsFunc = Arc<
    dyn Fn(&Request, &LocalizedAuthoringSignals) -> AuthoringTranslationPatchOptions
        + Send
        + Sync
        + 'static,
>;

/// Resolves one status text for a request.
pub type AuthoringStatusTextFunc = Arc<dyn Fn(&Request) -> String + Send + Sync + 'static>;

/// Resolves one field-value signal name.
pub type FieldSignalNameFunc = Arc<dyn Fn(&InputBytes, &InputBytes) -> String + Send + Sync + 'static>;

/// Resolves one field-format signal name.
pub type FieldFormatSignalNameFunc = Arc<dyn Fn(&InputBytes, &InputBytes) -> String + Send + Sync + 'static>;

/// Resolves one has-content signal name.
pub type HasContentSignalNameFunc = Arc<dyn Fn(&InputBytes, &InputBytes) -> String + Send + Sync + 'static>;

/// Configures the translation signal patch.
#[derive(Clone, Default)]
pub struct AuthoringTranslationPatchOptions {
    /// The patched signal names.
    pub signals: AuthoringTranslationSignals,
    /// The caller's language normalizer.
    pub normalize_language: Option<AuthoringLanguageNormalizer>,
    /// The field-value signal-name policy.
    pub field_signal_name: Option<FieldSignalNameFunc>,
    /// The field-format signal-name policy.
    pub field_format_signal_name: Option<FieldFormatSignalNameFunc>,
    /// The has-content signal-name policy.
    pub has_content_signal_name: Option<HasContentSignalNameFunc>,
    /// The ready status text.
    pub ready_status_text: String,
    /// The done status text.
    pub done_status_text: String,
    /// The failed status text.
    pub failed_status_text: String,
    /// The submit-error-open signal, or empty when absent.
    pub submit_error_open_signal: String,
    /// The submit-error-text signal, or empty when absent.
    pub submit_error_text_signal: String,
}

impl fmt::Debug for AuthoringTranslationPatchOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthoringTranslationPatchOptions")
            .field("signals", &self.signals)
            .field("ready_status_text", &self.ready_status_text)
            .field("done_status_text", &self.done_status_text)
            .field("failed_status_text", &self.failed_status_text)
            .field("submit_error_open_signal", &self.submit_error_open_signal)
            .field("submit_error_text_signal", &self.submit_error_text_signal)
            .finish_non_exhaustive()
    }
}

/// The localized patch defaults supplied by the localized handler.
#[derive(Clone, Default)]
pub struct LocalizedAuthoringTranslationPatchDefaults {
    /// The translation-key port.
    pub translate_key: Option<TranslateKeyFunc<Request>>,
    /// The caller's language normalizer.
    pub normalize_language: Option<AuthoringLanguageNormalizer>,
}

impl fmt::Debug for LocalizedAuthoringTranslationPatchDefaults {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalizedAuthoringTranslationPatchDefaults")
            .field("normalize_language", &self.normalize_language.is_some())
            .finish_non_exhaustive()
    }
}

/// The consumer's translation callback.
///
/// The request is the caller's request-scoped context; the callback returns its
/// own error family and runs only when the returned Effect runs.
pub type AuthoringTranslator<T, R> = Arc<
    dyn Fn(&Request, Duration, AuthoringTranslationInput) -> Effect<AuthoringTranslationResult, T, R>
        + Send
        + Sync
        + 'static,
>;

/// The consumer's access decision for one request.
///
/// `None` continues the translation workflow; `Some(reply)` is returned unchanged,
/// which is the source's `RequireAccess` branch that writes its own response and
/// stops the handler. The callback's error family is the handler's complete error
/// type because a failed decision is returned from the same handler.
pub type AuthoringAccessCheck<A, T, R> = Arc<
    dyn Fn(&Request) -> Effect<Option<Reply<AuthoringHandlerError<A, T>, R>>, AuthoringHandlerError<A, T>, R>
        + Send
        + Sync
        + 'static,
>;

/// A translation failure.
///
/// The consumer's translator error keeps its own identity in [`Self::Translator`];
/// a per-target deadline is [`Self::Timeout`].
#[derive(Debug)]
pub enum AuthoringTranslationError<T> {
    /// The consumer's translator failed.
    Translator(T),
    /// The per-target deadline elapsed and its work was cancelled.
    Timeout,
}

impl<T: fmt::Display> fmt::Display for AuthoringTranslationError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Translator(error) => fmt::Display::fmt(error, formatter),
            Self::Timeout => formatter.write_str("context deadline exceeded"),
        }
    }
}

impl<T: fmt::Debug + fmt::Display + std::error::Error + 'static> std::error::Error
    for AuthoringTranslationError<T>
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Translator(error) => Some(error),
            Self::Timeout => None,
        }
    }
}

/// The translation handler's error family.
///
/// The consumer's access failure keeps its own identity; an expected translation
/// failure can only reach this channel through a failed finalization, because a
/// normal translation failure is rendered as a failed signal patch.
#[derive(Debug)]
pub enum AuthoringHandlerError<A, T> {
    /// The consumer's access decision failed.
    Access(A),
    /// Native response-write adapter failure retained by cleanup exits.
    Delivery(effect_server::ServerError),
    /// Translation cleanup retained an expected translation failure.
    Translation(AuthoringTranslationError<T>),
}

impl<A: fmt::Display, T: fmt::Display> fmt::Display for AuthoringHandlerError<A, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Access(error) => fmt::Display::fmt(error, formatter),
            Self::Delivery(error) => fmt::Display::fmt(error, formatter),
            Self::Translation(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl<
        A: fmt::Debug + fmt::Display + std::error::Error + 'static,
        T: fmt::Debug + fmt::Display + std::error::Error + 'static,
    > std::error::Error for AuthoringHandlerError<A, T>
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Access(error) => Some(error),
            Self::Delivery(error) => Some(error),
            Self::Translation(error) => Some(error),
        }
    }
}

impl PartialEq for AuthoringTranslationInput{fn eq(&self,other:&Self)->bool{self.scope==other.scope&&self.source_language==other.source_language&&self.target_language==other.target_language&&self.fields.to_vec()==other.fields.to_vec()}}
impl Eq for AuthoringTranslationInput{}
impl PartialEq for AuthoringTranslationResult{fn eq(&self,other:&Self)->bool{self.target_language==other.target_language&&self.fields.with(Clone::clone)==other.fields.with(Clone::clone)}}
impl Eq for AuthoringTranslationResult{}
