//! Authoring: localized form translation, the translator/provider contract and the
//! authoring translation route (workflow W13).
//!
//! The owner provides:
//!
//! - the authoring translation records: the provider-shaped field, the single-target
//!   input, the result, the decoded form request and the authoring signal names;
//! - request/target/field normalization, including source-language exclusion,
//!   first-occurrence target de-duplication, blank-field removal and the exact
//!   first-value form precedence;
//! - the provider field JSON parser with Markdown-fence trimming and invalid-escape
//!   repair;
//! - the per-target concurrent translation workflow, its per-target deadline,
//!   first-failure cancellation and the ready/no-op/failure signal patches;
//! - the consumer translator and access ports and the Gemini provider adapter;
//! - the authoring translation HTTP handler, the localized handler/service and the
//!   managed authoring route registration;
//! - the localized media-tools promotion: the canonical forms records and builders
//!   are re-exported, never copied.
//!
//! # Consumer supplies
//!
//! The translator callback, the access decision, the translation-key port, the
//! language normalizer and the Gemini credential are consumer supplies. The HTTP
//! transport (for Gemini), the clock, the task spawner and the logger (for the
//! source failure diagnostic) are required members of the effect environment; the
//! owner constructs no client, runtime, host, global service or backend. The CMS owns
//! no database, SQL, schema, migration or pool.
//!
//! # Effects
//!
//! Request reading, translation, deadlines, concurrent target traversal and the
//! handler are lazy [`effect::Effect`] values built with the workspace macros. A
//! consumer callback or provider call runs only when the description runs. Expected
//! translator failures are captured into the failed signal patch; the handler's
//! error channel keeps the access failure, and defect, interruption and
//! finalization-failure channels are restored unchanged rather than flattened.
//!
//! # Data and cancellation
//!
//! Request cancellation interrupts the whole program, a per-target deadline cancels
//! and joins only that target, and the first expected target failure cancels the
//! remaining target children. The normalized input and its execution owner are
//! private and consumed once, so the source's runtime anti-forgery checks become
//! Rust ownership instead of a public one-shot restriction.
//!
//! # Private implementation
//!
//! `contracts`, `request`, `translation`, `patch`, `runtime`, `route` and `gemini`
//! are private implementation modules of this owner; this module is the owner's
//! deliberate public surface.

use std::time::Duration;

use effect::Effect;
use effect_server::Request;

mod contracts;
mod gemini;
mod patch;
mod request;
mod route;
mod runtime;
mod translation;

pub use self::contracts::{
    AuthoringAccessCheck, AuthoringHandlerError, AuthoringStatusTextFunc,
    AuthoringLanguageNormalizer, AuthoringLanguageNormalizerRef,
    AuthoringTranslationError, AuthoringTranslationField, AuthoringTranslationInput,
    AuthoringTranslationPatchOptions, AuthoringTranslationRequest, AuthoringTranslationResult,
    AuthoringTranslationSignals, AuthoringTranslator, FieldFormatSignalNameFunc,
    FieldSignalNameFunc, HasContentSignalNameFunc,
    LocalizedAuthoringTranslationPatchDefaults, ManagementAccessControl, PatchOptionsFunc,
    SignalsForWidgetFunc,
};
pub use self::gemini::{
    DEFAULT_GEMINI_AUTHORING_TRANSLATION_MODEL, GEMINI_API_BASE_URL, GeminiProviderError,
    GeminiTranslator, GeminiTranslatorError, GeminiWorkingLimits, PreparedGeminiWorkspace,
    new_gemini_translator, new_gemini_translator_with_model,
};
pub use self::patch::{
    AuthoringTranslationPatch, TranslationSignalValue, authoring_translation_failure_patch,
    authoring_translation_fallback_signals_json, authoring_translation_signals_from_localized,
    authoring_translation_status_patch, authoring_translation_unavailable_patch,
    write_authoring_translation_signal_patch, write_signal_patch,
};
pub use self::request::{
    AuthoringTranslationRequestError, parse_authoring_translation_request_from_form,
    parse_authoring_translation_request_prepared, parse_authoring_translation_request_with_form,
    project_prepared_authoring_request,
};
pub use self::route::{AUTHORING_ROUTE_CATEGORY, AuthoringRouteService, AuthoringRouteServiceOptions};
pub use self::runtime::{
    AUTHORING_TRANSLATION_DONE_LABEL_KEY, AUTHORING_TRANSLATION_FAILED_LABEL_KEY,
    AUTHORING_TRANSLATION_READY_LABEL_KEY, AUTHORING_TRANSLATION_UNAVAILABLE_LABEL_KEY,
    AuthoringPatchDelivery, authoring_write_failure_reply, AuthoringService, AuthoringTranslationHandlerOptions, DEFAULT_AUTHORING_TRANSLATION_TIMEOUT,
    LocalizedAuthoringService, LocalizedAuthoringTranslationHandlerOptions,
    localized_authoring_translation_patch_options, new_authoring_translation_handler,
    new_localized_authoring_translation_handler, new_prepared_authoring_translation_handler,
};
pub use self::translation::{
    authoring_field_format_map, build_authoring_translation_patch, normalize_authoring_fields,
    normalize_authoring_language, normalize_target_languages,
    parse_authoring_translation_fields_json, translate_authoring_fields_with_timeout,
};

pub use crate::forms::{
    LOCALIZED_TRANSLATION_WIDGET_KEY_FIELD_NAME, LocalizedAuthoringSignals, LocalizedMediaToolsOptions,
    LocalizedMediaToolsProps, LocalizedTab, LocalizedTabsProps, localized_editor_text_area_class,
    localized_media_open_url, localized_tabs_has_media_tools, new_localized_media_tools_props,
};

/// The witness clock/spawner environment; it exists only for the compile-time
/// signature proof and performs no execution.
struct WitnessRuntime {
    clock: effect::TestClock,
    tasks: WitnessSpawner,
    logger: effect::Logger,
}

/// A spawner that refuses work; the witness never runs a program.
struct WitnessSpawner;

impl effect::TaskSpawner for WitnessSpawner {
    fn spawn(&self, _task: effect::ScheduledTask) -> Result<(), effect::Defect> {
        Err(effect::Defect::new("authoring witness spawner"))
    }
}

impl effect::HasClock for WitnessRuntime {
    type Clock = effect::TestClock;

    fn clock(&self) -> &Self::Clock {
        &self.clock
    }
}

impl effect::HasTaskSpawner for WitnessRuntime {
    type Tasks = WitnessSpawner;

    fn task_spawner(&self) -> &Self::Tasks {
        &self.tasks
    }
}

impl effect::HasLogger for WitnessRuntime {
    fn logger(&self) -> &effect::Logger {
        &self.logger
    }
}

/// Compile-time witness that the owner's public contracts keep their declared
/// effect, error and environment shapes.
const _: fn() = || {
    fn witness<A, T, R>()
    where
        A: Send + 'static,
        T: std::fmt::Display + Send + 'static,
        R: effect::HasClock + effect::HasLogger + effect::HasTaskSpawner + Send + Sync + 'static,
    {
        let _: fn(
            Request,
            Option<AuthoringTranslator<T, R>>,
            Duration,
            AuthoringTranslationInput,
        ) -> Effect<Option<AuthoringTranslationResult>, AuthoringTranslationError<T>, R> =
            |request, translator, timeout, input| {
                translate_authoring_fields_with_timeout(
                    &request,
                    translator,
                    timeout,
                    input,
                )
            };
        let _: fn(
            Request,
            Option<AuthoringTranslator<T, R>>,
            Duration,
            AuthoringTranslationRequest,
            AuthoringTranslationPatchOptions,
        ) -> Effect<AuthoringTranslationPatch, AuthoringTranslationError<T>, R> =
            |request, translator, timeout, translation_request, options| {
                build_authoring_translation_patch(
                    &request,
                    translator,
                    timeout,
                    translation_request,
                    options,
                )
            };
        let _: fn(
            AuthoringTranslationHandlerOptions<A, T, R>,
        ) -> crate::routing::RouteHandler<AuthoringHandlerError<A, T>, R> =
            new_authoring_translation_handler::<A, T, R>;
    }
    witness::<std::fmt::Error, std::fmt::Error, WitnessRuntime>();
};
