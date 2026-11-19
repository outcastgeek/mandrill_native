//! Forms: the W5 form-surface workflow and its localized authoring surfaces.
//!
//! Forms owns:
//!
//! - the pure mode decision (`new` → create, `edit` → update, every other surface →
//!   create while keeping the surface) and the form/submit action filters;
//! - the form page service: registered-definition lookup, the command owner's offered
//!   actions, the consumer's validation-path binding application, title/breadcrumb/
//!   route resolution, the localized status and the consumer's page validator;
//! - the form page source/frame/binding and page assembly, including the preserved
//!   error-message projection, the action/cancel targets, the caller's form id and the
//!   section building/marking;
//! - the form field/section/page view models and the canonical state key;
//! - the localized tabs child owner: the tab/draft projection, the label and format
//!   policies, the authoring signals/attributes and the media-tools mount, together
//!   with the Maud rendering of the widget and its alerts;
//! - the relation form child owner: the preparation/admission rules, the private
//!   validated state and rows, the selection encoding and the Maud relation control;
//! - the consumer-facing draft render state and the slug update-target resolution.
//!
//! # Capabilities and data
//!
//! The definition registry, the command service, the query service, the route/path
//! builders, the translation ports and the page validator are supplied capabilities;
//! every form page, tab set and relation control is assembled from typed values and
//! consumer callbacks. The CMS owns no database, SQL, schema, migration, pool,
//! backend, scanner, bridge or host. Consumer CRUD and executors stay consumer-owned;
//! this owner performs its source orchestration through the supplied capabilities.
//!
//! Captured callbacks are `Send + Sync + 'static` because they are carried by the
//! lazy effects and rendered values. No extra `Clone`, `Sync` or effect bounds are
//! imposed on the error or environment families; defects, interruption and
//! finalization failures keep their own effect channels.
//!
//! # Effects
//!
//! The page service, the frame resolution, the page assembly and the draft render
//! state return lazy [`effect::Effect`] values built with the workspace macros. No
//! source, parser, encoder or consumer callback runs when a description is
//! constructed. The pure projections (mode decision, section building, status
//! projection, tab building, relation preparation and markup) are ordinary functions
//! lifted by their effectful callers.
//!
//! # HTML
//!
//! Every HTML-producing function in this owner is authored with Maud; the governed
//! controls reuse the workspace component library's published class/property
//! contracts and icon markup instead of a second style contract. No HTML is manually
//! concatenated.
//!
//! # Private implementation
//!
//! `mode`, `service`, `page`, `sections`, `draft` and the `localized_tabs` and
//! `relation_form` children hold the implementation; `localized_tabs` and
//! `relation_form` are genuine child owners with their own vocabulary, rules and
//! rendering consumers. This module is the owner's deliberate public surface.

mod draft;
mod localized_tabs;
mod mode;
mod page;
mod prepared_input;
mod relation_form;
mod sections;
mod service;

pub use self::draft::{
    DraftBuildFunc, DraftExtraFunc, DraftFlowError, DraftMediaFunc, DraftRowFunc, DraftsFunc,
    NoResourceDraftRenderExtra, ResourceDraftRenderState, ResourceDraftRenderStateOptions,
    build_resource_draft_render_state, resolve_parent_slug_update_target,
    resolve_slug_update_target,
};
pub use self::localized_tabs::{
    LOCALIZED_TAB_LABEL_LANGUAGE_NAME, LOCALIZED_TAB_LABEL_TRANSLATED,
    LOCALIZED_TAB_LABEL_TRANSLATED_WITH_LANGUAGE_NAME,
    LOCALIZED_TRANSLATION_WIDGET_KEY_FIELD_NAME, LocalizedAuthoringSignals,
    LocalizedContentTabSetOptions, LocalizedContentTabsOptions,
    LocalizedFormatOption, LocalizedFormatOptionDefinition, LocalizedFormatSelectConfig,
    LocalizedFormatSelectProps, LocalizedFormatSelectVM, LocalizedLanguageNameFunc,
    LocalizedMediaToolsOptions, LocalizedMediaToolsProps, LocalizedTab, LocalizedTabLabelMode,
    LocalizedTabLabelOptions, LocalizedTabsAuthoringConfig, LocalizedTabsAuthoringOptions,
    LocalizedTabsAuthoringProps, LocalizedTabsMediaToolsConfig, LocalizedTabsProps,
    LocalizedTabsPropsOptions, LocalizedTabsRuntime, LocalizedTextAreaPanelVM,
    LocalizedTextTabSetOptions, LocalizedTextTabsOptions, MEDIA_SHORTCODE_TOOLBAR_HELP_KEY,
    build_localized_content_description_tab_set, build_localized_content_description_tabs,
    build_localized_content_tab_set, build_localized_content_tabs,
    build_localized_text_tab_set, build_localized_text_tabs, language_name_tab_label,
    localized_authoring_form_attributes, localized_authoring_form_attributes_for_widgets,
    localized_authoring_signals_for_widget, localized_editor_text_area_class,
    localized_field_attributes, localized_field_id_func, localized_field_name_func,
    localized_field_signal_name, localized_form_error_alert, localized_form_error_alert_class,
    localized_format_attributes, localized_format_options, localized_format_select,
    localized_format_signal_name, localized_has_content_signal_name,
    localized_live_submit_alerts, localized_live_submit_blocked_alert,
    localized_live_submit_error_alert, localized_media_insert_helper, localized_media_open_url,
    localized_normalize_language, localized_submit_control_attributes,
    localized_tab_error_message, localized_tabs, localized_tabs_has_media_tools,
    localized_text_area_panel, new_default_localized_tabs_authoring_config,
    new_default_localized_tabs_media_tools_config, new_localized_format_select_helper,
    new_localized_media_tools_props, new_localized_tabs_authoring_props,
    new_localized_tabs_props, normalize_localized_signal_part,
    set_localized_authoring_availability, translated_language_name_tab_label,
    translated_tab_label,
};
pub use self::mode::{
    Action, ModeDecision, decide_mode, filter_form_actions, filter_submit_actions, hidden_action,
};
pub use self::prepared_input::{MergedFormCapacity, PreparedMergedForm, PreparedMergedForms};
pub use crate::form_media::{InitializeForm, MergedFormPolicy};
pub use self::page::{
    EditPageFunc, FormPageBinding, FormPageSource, NewPageFunc, PageFunc, RehydratePageFunc,
    ResourceFormFrame, bind_form_page_source, build_resource_form_page,
    resolve_resource_form_frame, resolve_resource_form_frame_with_validation,
    resource_form_frame_validation,
};
pub use self::relation_form::{
    RelationFormActionInput, RelationFormError, RelationFormInput, RelationFormIssue,
    RelationFormLabels, RelationFormRow, RelationFormState, RelationRejectedSelectionDraft,
    RelationRowDraft, RelationRowRenderFunc, RelationSelectionValueFunc, prepare_relation_form,
    relation_control, relation_request_action,
};
pub use self::sections::{
    BreadcrumbVM, FormStatusVM, ResourceFieldOptionVM, ResourceFieldVM, ResourceFormFieldState,
    ResourceFormPageVM, ResourceFormSectionVM, ResourceFormState,
    attach_validation_issues_to_form_sections, build_resource_form_sections,
    build_resource_form_sections_for_kind, resource_form_field_state_key,
};
pub use self::service::{
    FormPageError, FormPageValidator, Options, ResourceModeTitleResolver, Service,
    localized_form_status, new_service,
};

/// Compile-time witness that the owner keeps its required effect/callback shapes.
///
/// The page service and frame resolution stay lazy effects under the caller's error
/// and environment families, and the relation encoder keeps its consumer error.
const _: fn() = || {
    #[allow(clippy::type_complexity)]
    fn witness<C, E, R>()
    where
        C: crate::validation::ValidationTranslation + Send + Sync + 'static,
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let _: fn(
            &Service<C, E, R>,
            std::sync::Arc<C>,
            &effect_server::Request,
            &crate::definitions::ResourceKind,
        ) -> effect::Effect<ResourceFormPageVM, FormPageError<E>, R> = Service::new_page;
        let _: fn(
            &Service<C, E, R>,
            std::sync::Arc<C>,
            &effect_server::Request,
            &crate::definitions::ResourceKind,
        ) -> effect::Effect<ResourceFormPageVM, FormPageError<E>, R> = Service::edit_page;
    }
    witness::<
        crate::validation::BackgroundValidationTranslation,
        (),
        (),
    >();
};
