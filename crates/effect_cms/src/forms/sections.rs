//! Form section, field and page view models and their validation attachment.
//!
//! The form state is a map keyed by the field's canonical state key (the trimmed
//! declared name, else the declared key). Building sections resolves each declared
//! field against the state's zero value, translates the declared label/placeholder/
//! help/description keys, trims the submitted names and ids, and copies the state's
//! values, attributes, options and localized data.
//!
//! Validation attachment copies the outer section list and each section's field list
//! before marking fields and localized tabs with their matching issues. A field
//! matches an issue when one of its candidates (input id, name, key) equals or
//! prefixes the issue candidates (field id, field, field key, path and the
//! underscore-normalized path). The first matched issue supplies the field-level
//! message only when the field has none.

use std::collections::BTreeMap;
use std::sync::Arc;

use effect_server::Request;
use maud::Markup;
use serde::ser::{SerializeSeq, SerializeStruct};
use serde::{Serialize, Serializer};

use crate::attachments::{MediaAttachmentFormItem, MediaAttachmentManagerProps, MediaSurfaceMounts};
use crate::definitions::{
    Registry, ResourceFieldKey, ResourceFieldKind, ResourceFormDefinition,
    ResourceFormFieldDefinition, ResourceRouteSet, ResourceSectionKey, find_resource_definition,
    resource_form_field_state_key as canonical_form_field_state_key,
};
use crate::localization::TranslateKeyFunc;
use crate::routing::{ManagementBreadcrumbVM, translate};
use crate::validation::{
    ValidationIssueVM, clone_validation_issue_vms,
};

use super::localized_tabs::{
    LocalizedMediaToolsProps, LocalizedTab, LocalizedTabsAuthoringProps, LocalizedTabsProps,
};
use super::relation_form::RelationFormState;

/// One rendered form field's state.
#[derive(Clone, Debug, Default)]
pub struct ResourceFormFieldState {
    /// Whether the control is disabled.
    pub disabled: bool,
    /// Help text that overrides the declared help-text key when non-empty.
    pub help_text: String,
    /// A prepared editor component.
    pub prepared_editor: Option<Markup>,
    /// The relation editor state.
    pub relation: RelationFormState,
    /// The submitted value.
    pub value: crate::commands::InputBytes,
    /// The display value.
    pub display_value: String,
    /// The secondary value.
    pub secondary_value: String,
    /// Additional element attributes.
    pub attrs: BTreeMap<String, String>,
    /// Selectable options.
    pub options: Vec<ResourceFieldOptionVM>,
    /// The localized tabs attached to this field.
    pub localized_tabs: Option<Arc<LocalizedTabsProps>>,
    /// The media attachment manager attached to this field.
    pub media_manager: Option<Arc<MediaAttachmentManagerProps>>,
    /// Whether the field is invalid.
    pub error: bool,
    /// The field-level message.
    pub error_message: String,
    /// The field's localized issues.
    pub errors: Vec<ValidationIssueVM>,
}

/// The form state keyed by the canonical field state key.
pub type ResourceFormState = BTreeMap<String, ResourceFormFieldState>;

/// One rendered field option.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceFieldOptionVM {
    /// The submitted value.
    pub value: String,
    /// The localized label.
    pub label: String,
    /// Whether the option is selected.
    pub selected: bool,
    /// Additional option attributes.
    pub attrs: BTreeMap<String, String>,
}

/// One rendered form field.
#[derive(Clone, Debug, Default)]
pub struct ResourceFieldVM {
    /// A prepared editor component.
    pub prepared_editor: Option<Markup>,
    /// The relation editor state.
    pub relation: RelationFormState,
    /// The declared field key.
    pub key: ResourceFieldKey,
    /// The declared field kind.
    pub kind: ResourceFieldKind,
    /// The trimmed submitted field name.
    pub name: String,
    /// The trimmed input element id.
    pub input_id: String,
    /// The localized label.
    pub label: String,
    /// The value.
    pub value: crate::commands::InputBytes,
    /// The display value.
    pub display_value: String,
    /// The localized placeholder.
    pub placeholder: String,
    /// The localized help text.
    pub help_text: String,
    /// The localized description.
    pub description: String,
    /// Whether the field is required.
    pub required: bool,
    /// Whether the control is disabled.
    pub disabled: bool,
    /// Declared text-area rows.
    pub rows: i64,
    /// The trimmed secondary submitted name.
    pub secondary_name: String,
    /// The trimmed secondary input id.
    pub secondary_id: String,
    /// The trimmed secondary value.
    pub secondary_value: String,
    /// Additional element attributes.
    pub attrs: BTreeMap<String, String>,
    /// Selectable options.
    pub options: Vec<ResourceFieldOptionVM>,
    /// The localized tabs attached to this field.
    pub localized_tabs: Option<Arc<LocalizedTabsProps>>,
    /// The media attachment manager attached to this field.
    pub media_manager: Option<Arc<MediaAttachmentManagerProps>>,
    /// Whether the field is invalid.
    pub error: bool,
    /// The field-level message.
    pub error_message: String,
    /// The field's localized issues.
    pub errors: Vec<ValidationIssueVM>,
}

/// One rendered form section.
#[derive(Clone, Debug, Default)]
pub struct ResourceFormSectionVM {
    /// The declared section key.
    pub key: ResourceSectionKey,
    /// The localized title.
    pub title: String,
    /// The localized description.
    pub description: String,
    /// The section's fields.
    pub fields: Vec<ResourceFieldVM>,
}

/// The rendered form status.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormStatusVM {
    /// The outcome code.
    pub code: crate::validation::ValidationCode,
    /// The summary message.
    pub message: String,
    /// The localized issues.
    pub issues: Vec<ValidationIssueVM>,
    /// Whether the presentation disables submission.
    pub submit_disabled: bool,
    /// Whether the presentation must rehydrate the form.
    pub requires_rehydrate: bool,
}

/// The rendered form page.
#[derive(Clone, Debug, Default)]
pub struct ResourceFormPageVM {
    /// The resource kind.
    pub kind: crate::definitions::ResourceKind,
    /// The form surface.
    pub mode: crate::definitions::ResourceViewMode,
    /// The localized title.
    pub title: String,
    /// The management breadcrumb.
    pub breadcrumb: BreadcrumbVM,
    /// The resolved route set.
    pub routes: ResourceRouteSet,
    /// The form element id.
    pub form_id: crate::commands::InputBytes,
    /// The submission target.
    pub action_href: String,
    /// The cancel target.
    pub cancel_href: String,
    /// The offered actions.
    pub actions: Vec<crate::commands::SubmitActionVM>,
    /// The definition's localized authoring widgets.
    pub widgets: Vec<crate::definitions::LocalizedWidgetVM>,
    /// The form status.
    pub status: FormStatusVM,
    /// The rendered sections.
    pub sections: Vec<ResourceFormSectionVM>,
}

/// The rendered management breadcrumb.
pub type BreadcrumbVM = ManagementBreadcrumbVM;

// ---------------------------------------------------------------------------
// Published `#ResourceFormPageVM` contract serialization
// ---------------------------------------------------------------------------
//
// The page and its nested contract values serialize with the published contract
// field names, including the acronym fields (`FormID`, `ActionHref`, `CancelHref`,
// `InputID`, `SecondaryID`, `FieldID`, `FieldKey`, `FieldName`).
// The absent-issue lists serialize as `null`, matching the contract's `default_null`
// presence rule and an absent collection.
//
// `PreparedEditor` and `Relation` are render values, not contract fields. The
// optional `LocalizedTabs` and `MediaManager` values serialize as their source-shaped
// nested objects when present and are omitted when absent; their render-only
// components (panel/after-helpers, normalizer, request) are not contract data.

/// Serialize one declared open-vocabulary value as its exact text.
struct VocabularyText<'a, T: AsRef<str>>(&'a T);

impl<T: AsRef<str>> Serialize for VocabularyText<'_, T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.0.as_ref())
    }
}

/// Serialize the management breadcrumb contract value.
struct BreadcrumbContract<'a>(&'a ManagementBreadcrumbVM);

impl Serialize for BreadcrumbContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ManagementBreadcrumbVM", 4)?;
        state.serialize_field("DashboardRoute", &self.0.dashboard_route)?;
        state.serialize_field("DashboardLabel", &self.0.dashboard_label)?;
        state.serialize_field("CurrentLabel", &self.0.current_label)?;
        state.serialize_field("CurrentSubtitle", &self.0.current_subtitle)?;
        state.end()
    }
}

/// Serialize the resource route-set contract value.
struct RouteSetContract<'a>(&'a ResourceRouteSet);

impl Serialize for RouteSetContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ResourceRouteSet", 9)?;
        state.serialize_field("Dashboard", &self.0.dashboard)?;
        state.serialize_field("List", &self.0.list)?;
        state.serialize_field("Detail", &self.0.detail)?;
        state.serialize_field("New", &self.0.new)?;
        state.serialize_field("NewPost", &self.0.new_post)?;
        state.serialize_field("Edit", &self.0.edit)?;
        state.serialize_field("EditPost", &self.0.edit_post)?;
        state.serialize_field("Delete", &self.0.delete)?;
        let aux: BTreeMap<String, String> = self.0.aux.with(|entries| {
            entries
                .iter()
                .map(|(key, value)| (key.as_str().to_owned(), value.clone()))
                .collect()
        });
        state.serialize_field("Aux", &aux)?;
        state.end()
    }
}

/// Serialize the offered submit-action contract values.
struct SubmitActionsContract<'a>(&'a [crate::commands::SubmitActionVM]);

impl Serialize for SubmitActionsContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for action in self.0 {
            sequence.serialize_element(&SubmitActionContract(action))?;
        }
        sequence.end()
    }
}

/// Serialize one submit-action contract value.
struct SubmitActionContract<'a>(&'a crate::commands::SubmitActionVM);

impl Serialize for SubmitActionContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("SubmitActionVM", 2)?;
        state.serialize_field("Action", &VocabularyText(&self.0.action))?;
        state.serialize_field("Label", &self.0.label)?;
        state.end()
    }
}

/// Serialize the localized-widget contract values.
struct WidgetsContract<'a>(&'a [crate::definitions::LocalizedWidgetVM]);

impl Serialize for WidgetsContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for widget in self.0 {
            sequence.serialize_element(&WidgetContract(widget))?;
        }
        sequence.end()
    }
}

/// Serialize one localized-widget contract value.
struct WidgetContract<'a>(&'a crate::definitions::LocalizedWidgetVM);

impl Serialize for WidgetContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("LocalizedWidgetVM", 2)?;
        state.serialize_field("WidgetKey", &self.0.widget_key)?;
        state.serialize_field("Scope", &self.0.scope)?;
        state.end()
    }
}

/// Serialize a validation-issue list, using the contract's explicit null absence.
struct ValidationIssuesContract<'a>(&'a [ValidationIssueVM]);

impl Serialize for ValidationIssuesContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.0.is_empty() {
            return serializer.serialize_none();
        }
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for issue in self.0 {
            sequence.serialize_element(&ValidationIssueContract(issue))?;
        }
        sequence.end()
    }
}

/// Serialize one localized validation-issue contract value.
struct ValidationIssueContract<'a>(&'a ValidationIssueVM);

impl Serialize for ValidationIssueContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ValidationIssueVM", 7)?;
        state.serialize_field("Code", &self.0.code)?;
        state.serialize_field("Path", &self.0.path)?;
        state.serialize_field("Field", &self.0.field)?;
        state.serialize_field("FieldID", &self.0.field_id)?;
        state.serialize_field("FieldKey", &self.0.field_key)?;
        state.serialize_field("FieldName", &self.0.field_name)?;
        state.serialize_field("Message", &self.0.message)?;
        state.end()
    }
}

/// Serialize one localized-tabs contract value.
///
/// The source-shaped keys keep every data member. The normalizer function and the
/// panel/after-helper components are render-only values with no contract data, so they
/// are omitted; a populated authoring/media-tools value and a non-empty tab list are
/// encoded as nested objects/arrays.
struct LocalizedTabsContract<'a>(&'a LocalizedTabsProps);

impl Serialize for LocalizedTabsContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("LocalizedTabsProps", 10)?;
        state.serialize_field("DefaultValue", &self.0.default_value)?;
        state.serialize_field("AriaLabel", &self.0.aria_label)?;
        state.serialize_field("DefaultBadgeText", &self.0.default_badge_text)?;
        state.serialize_field("Class", &self.0.class)?;
        state.serialize_field("ListClass", &self.0.list_class)?;
        state.serialize_field("PanelCardClass", &self.0.panel_card_class)?;
        state.serialize_field("PanelContentClass", &self.0.panel_content_class)?;
        if let Some(authoring) = self.0.authoring.as_deref() {
            state.serialize_field("Authoring", &AuthoringPropsContract(authoring))?;
        }
        if let Some(media_tools) = self.0.media_tools.as_deref() {
            state.serialize_field("MediaTools", &MediaToolsContract(media_tools))?;
        }
        if !self.0.tabs.is_empty() {
            state.serialize_field("Tabs", &LocalizedTabsListContract(&self.0.tabs))?;
        }
        state.end()
    }
}

/// Serialize one localized-tabs authoring contract value.
struct AuthoringPropsContract<'a>(&'a LocalizedTabsAuthoringProps);

impl Serialize for AuthoringPropsContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("LocalizedTabsAuthoringProps", 24)?;
        state.serialize_field("Enabled", &self.0.enabled)?;
        state.serialize_field("TranslationAvailable", &self.0.translation_available)?;
        state.serialize_field("Scope", &self.0.scope)?;
        state.serialize_field("WidgetKey", &self.0.widget_key)?;
        state.serialize_field("StreamURL", &self.0.stream_url)?;
        state.serialize_field("CheckboxID", &self.0.checkbox_id)?;
        state.serialize_field("CheckboxName", &self.0.checkbox_name)?;
        state.serialize_field("CheckboxChecked", &self.0.checkbox_checked)?;
        state.serialize_field("AutoTranslateLabel", &self.0.auto_translate_label)?;
        state.serialize_field("AutoTranslateHelp", &self.0.auto_translate_help)?;
        state.serialize_field("StatusIdleText", &self.0.status_idle_text)?;
        state.serialize_field("StatusOffText", &self.0.status_off_text)?;
        state.serialize_field("StatusWorkingText", &self.0.status_working_text)?;
        state.serialize_field("StatusDoneText", &self.0.status_done_text)?;
        state.serialize_field("StatusErrorText", &self.0.status_error_text)?;
        state.serialize_field("StatusUnavailableText", &self.0.status_unavailable_text)?;
        state.serialize_field("TabMissingText", &self.0.tab_missing_text)?;
        state.serialize_field("TabCompleteText", &self.0.tab_complete_text)?;
        state.serialize_field("TabEmptyText", &self.0.tab_empty_text)?;
        state.serialize_field("SubmitBlockedText", &self.0.submit_blocked_text)?;
        state.serialize_field("SubmitTranslatingText", &self.0.submit_translating_text)?;
        state.serialize_field("SubmitErrorText", &self.0.submit_error_text)?;
        state.serialize_field("SubmitUnavailableText", &self.0.submit_unavailable_text)?;
        state.serialize_field("SubmitEmptySourceText", &self.0.submit_empty_source_text)?;
        state.end()
    }
}

/// Serialize one localized media-tools contract value.
///
/// The request is a render-only transport value, not contract data, so it is omitted;
/// every string identity and a non-empty attachment list are encoded.
struct MediaToolsContract<'a>(&'a LocalizedMediaToolsProps);

impl Serialize for MediaToolsContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("LocalizedMediaToolsProps", 10)?;
        state.serialize_field("Scope", &self.0.scope)?;
        state.serialize_field("DialogID", &self.0.dialog_id)?;
        state.serialize_field("RootID", &self.0.root_id)?;
        state.serialize_field("OwnerFormID", &self.0.owner_form_id)?;
        state.serialize_field("AttachmentID", &self.0.attachment_id)?;
        state.serialize_field("FieldKey", &self.0.field_key)?;
        state.serialize_field("HelpText", &self.0.help_text)?;
        state.serialize_field("Label", &self.0.label)?;
        state.serialize_field("OpenURL", &self.0.open_url)?;
        if !self.0.attachments.is_empty() {
            state.serialize_field(
                "Attachments",
                &AttachmentItemsContract(&self.0.attachments),
            )?;
        }
        state.end()
    }
}

/// Serialize the localized-tab list.
struct LocalizedTabsListContract<'a>(&'a [LocalizedTab]);

impl Serialize for LocalizedTabsListContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for tab in self.0 {
            sequence.serialize_element(&LocalizedTabContract(tab))?;
        }
        sequence.end()
    }
}

/// Serialize one localized-tab contract value.
struct LocalizedTabContract<'a>(&'a LocalizedTab);

impl Serialize for LocalizedTabContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("LocalizedTab", 12)?;
        state.serialize_field("Value", &self.0.value)?;
        state.serialize_field("Label", &self.0.label)?;
        state.serialize_field("Required", &self.0.required)?;
        state.serialize_field("HasContent", &self.0.has_content)?;
        state.serialize_field("Error", &self.0.error)?;
        state.serialize_field("ErrorMessage", &self.0.error_message)?;
        if !self.0.errors.is_empty() {
            state.serialize_field("Errors", &ValidationIssuesContract(&self.0.errors))?;
        }
        state.serialize_field("FieldKey", &self.0.field_key)?;
        state.serialize_field("FieldValue", &crate::byte_text::encoding_text(self.0.field_value.as_bytes()))?;
        state.serialize_field("FormatValue", &crate::byte_text::encoding_text(self.0.format_value.as_bytes()))?;
        state.serialize_field("FieldID", &self.0.field_id)?;
        state.serialize_field("FieldLabel", &self.0.field_label)?;
        state.end()
    }
}

/// Serialize one media-attachment-manager contract value.
struct MediaManagerContract<'a>(&'a MediaAttachmentManagerProps);

impl Serialize for MediaManagerContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("MediaAttachmentManagerProps", 7)?;
        state.serialize_field("Scope", &self.0.scope)?;
        state.serialize_field("OwnerContentID", &self.0.owner_content_id)?;
        state.serialize_field("Mounts", &SurfaceMountsContract(&self.0.mounts))?;
        state.serialize_field("PickerURL", &self.0.picker_url)?;
        state.serialize_field("CreateURL", &self.0.create_url)?;
        state.serialize_field("MutationURL", &self.0.mutation_url)?;
        if !self.0.attachments.is_empty() {
            state.serialize_field(
                "Attachments",
                &AttachmentItemsContract(&self.0.attachments),
            )?;
        }
        state.end()
    }
}

/// Serialize the media surface mount identities.
struct SurfaceMountsContract<'a>(&'a MediaSurfaceMounts);

impl Serialize for SurfaceMountsContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("MediaSurfaceMounts", 6)?;
        state.serialize_field("SectionID", &self.0.section_id)?;
        state.serialize_field("OwnerFormID", &self.0.owner_form_id)?;
        state.serialize_field("PickerRootID", &self.0.picker_root_id)?;
        state.serialize_field("CreateRootID", &self.0.create_root_id)?;
        state.serialize_field("ShortcodeRootID", &self.0.shortcode_root_id)?;
        state.serialize_field("ShortcodeDialogID", &self.0.shortcode_dialog_id)?;
        state.end()
    }
}

/// Serialize the attachment form-item list.
struct AttachmentItemsContract<'a>(&'a [MediaAttachmentFormItem]);

impl Serialize for AttachmentItemsContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for item in self.0 {
            sequence.serialize_element(&AttachmentItemContract(item))?;
        }
        sequence.end()
    }
}

/// Serialize one media attachment form-item contract value.
struct AttachmentItemContract<'a>(&'a MediaAttachmentFormItem);

impl Serialize for AttachmentItemContract<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("MediaAttachmentFormItem", 19)?;
        state.serialize_field("Key", &self.0.key)?;
        state.serialize_field("Position", &self.0.position)?;
        state.serialize_field("AssetID", &self.0.asset_id)?;
        state.serialize_field("Kind", &self.0.kind)?;
        state.serialize_field("Role", &self.0.role)?;
        state.serialize_field("Status", &self.0.status)?;
        state.serialize_field("Title", &self.0.title)?;
        state.serialize_field("Description", &self.0.description)?;
        state.serialize_field("Provider", &self.0.provider)?;
        state.serialize_field("URL", &self.0.url)?;
        state.serialize_field("ThumbnailURL", &self.0.thumbnail_url)?;
        state.serialize_field("ResolvedURL", &self.0.resolved_url)?;
        state.serialize_field("ResolvedThumbnail", &self.0.resolved_thumbnail)?;
        state.serialize_field("Alt", &self.0.alt)?;
        state.serialize_field("TitleOverride", &self.0.title_override)?;
        state.serialize_field("Caption", &self.0.caption)?;
        state.serialize_field("AltOverride", &self.0.alt_override)?;
        state.serialize_field("Shortcode", &self.0.shortcode)?;
        state.serialize_field("EditURL", &self.0.edit_url)?;
        state.end()
    }
}

/// Serialize the rendered form status as the published contract value.
impl Serialize for FormStatusVM {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FormStatusVM", 5)?;
        state.serialize_field("Code", &VocabularyText(&self.code))?;
        state.serialize_field("Message", &self.message)?;
        state.serialize_field("Issues", &ValidationIssuesContract(&self.issues))?;
        state.serialize_field("SubmitDisabled", &self.submit_disabled)?;
        state.serialize_field("RequiresRehydrate", &self.requires_rehydrate)?;
        state.end()
    }
}

/// Serialize one rendered field option as the published contract value.
impl Serialize for ResourceFieldOptionVM {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ResourceFieldOptionVM", 4)?;
        state.serialize_field("Value", &self.value)?;
        state.serialize_field("Label", &self.label)?;
        state.serialize_field("Selected", &self.selected)?;
        state.serialize_field("Attrs", &self.attrs)?;
        state.end()
    }
}

/// Serialize one rendered field as the published contract value.
impl Serialize for ResourceFieldVM {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ResourceFieldVM", 23)?;
        state.serialize_field("Key", &VocabularyText(&self.key))?;
        state.serialize_field("Kind", &VocabularyText(&self.kind))?;
        state.serialize_field("Name", &self.name)?;
        state.serialize_field("InputID", &self.input_id)?;
        state.serialize_field("Label", &self.label)?;
        state.serialize_field("Value", &self.value)?;
        state.serialize_field("DisplayValue", &self.display_value)?;
        state.serialize_field("Placeholder", &self.placeholder)?;
        state.serialize_field("HelpText", &self.help_text)?;
        state.serialize_field("Description", &self.description)?;
        state.serialize_field("Required", &self.required)?;
        state.serialize_field("Disabled", &self.disabled)?;
        state.serialize_field("Rows", &self.rows)?;
        state.serialize_field("SecondaryName", &self.secondary_name)?;
        state.serialize_field("SecondaryID", &self.secondary_id)?;
        state.serialize_field("SecondaryValue", &self.secondary_value)?;
        state.serialize_field("Attrs", &self.attrs)?;
        state.serialize_field("Options", &self.options)?;
        if let Some(tabs) = self.localized_tabs.as_deref() {
            state.serialize_field("LocalizedTabs", &LocalizedTabsContract(tabs))?;
        }
        if let Some(manager) = self.media_manager.as_deref() {
            state.serialize_field("MediaManager", &MediaManagerContract(manager))?;
        }
        state.serialize_field("Error", &self.error)?;
        state.serialize_field("ErrorMessage", &self.error_message)?;
        state.serialize_field("Errors", &ValidationIssuesContract(&self.errors))?;
        state.end()
    }
}

/// Serialize one rendered section as the published contract value.
impl Serialize for ResourceFormSectionVM {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ResourceFormSectionVM", 4)?;
        state.serialize_field("Key", &VocabularyText(&self.key))?;
        state.serialize_field("Title", &self.title)?;
        state.serialize_field("Description", &self.description)?;
        state.serialize_field("Fields", &self.fields)?;
        state.end()
    }
}

/// Serialize the rendered form page as the published `#ResourceFormPageVM` value.
impl Serialize for ResourceFormPageVM {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ResourceFormPageVM", 12)?;
        state.serialize_field("Kind", &VocabularyText(&self.kind))?;
        state.serialize_field("Mode", &VocabularyText(&self.mode))?;
        state.serialize_field("Title", &self.title)?;
        state.serialize_field("Breadcrumb", &BreadcrumbContract(&self.breadcrumb))?;
        state.serialize_field("Routes", &RouteSetContract(&self.routes))?;
        state.serialize_field("FormID", &self.form_id)?;
        state.serialize_field("ActionHref", &self.action_href)?;
        state.serialize_field("CancelHref", &self.cancel_href)?;
        state.serialize_field("Actions", &SubmitActionsContract(&self.actions))?;
        state.serialize_field("Widgets", &WidgetsContract(&self.widgets))?;
        state.serialize_field("Status", &self.status)?;
        state.serialize_field("Sections", &self.sections)?;
        state.end()
    }
}

/// Build the rendered sections for one form definition.
#[must_use]
pub fn build_resource_form_sections(
    request: &Request,
    definition: &ResourceFormDefinition,
    state: &ResourceFormState,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> Vec<ResourceFormSectionVM> {
    if definition.sections.is_empty() {
        return Vec::new();
    }
    let section_count = definition.sections.len();
    let mut sections = Vec::with_capacity(section_count);
    for section_index in 0..section_count {
        let section_definition = definition.sections.with(|sections| sections[section_index].clone());
        let mut section = ResourceFormSectionVM {
            key: section_definition.key.clone(),
            title: translate(translate_key, request, section_definition.title_key.as_str()),
            description: translate(
                translate_key,
                request,
                section_definition.description_key.as_str(),
            ),
            fields: Vec::with_capacity(section_definition.fields.len()),
        };
        for field_index in 0..section_definition.fields.len() {
            let field_definition = section_definition.fields.with(|fields| fields[field_index].clone());
            let field_state = state
                .get(&canonical_form_field_state_key(&field_definition))
                .cloned()
                .unwrap_or_default();
            let mut field = ResourceFieldVM {
                disabled: field_state.disabled,
                prepared_editor: field_state.prepared_editor.clone(),
                relation: field_state.relation.clone(),
                key: field_definition.key.clone(),
                kind: field_definition.kind.clone(),
                label: translate(translate_key, request, field_definition.label_key.as_str()),
                value: field_state.value.clone(),
                required: field_definition.required,
                name: field_definition.name.trim().to_owned(),
                input_id: field_definition.input_id.trim().to_owned(),
                placeholder: translate(
                    translate_key,
                    request,
                    field_definition.placeholder_key.as_str(),
                ),
                help_text: translate(
                    translate_key,
                    request,
                    field_definition.help_text_key.as_str(),
                ),
                description: translate(
                    translate_key,
                    request,
                    field_definition.description_key.as_str(),
                ),
                rows: field_definition.rows,
                secondary_name: field_definition.secondary_name.trim().to_owned(),
                secondary_id: field_definition.secondary_id.trim().to_owned(),
                secondary_value: field_state.secondary_value.trim().to_owned(),
                display_value: field_state.display_value.trim().to_owned(),
                attrs: clone_string_map(&field_state.attrs),
                options: clone_field_options(&field_state.options),
                localized_tabs: field_state.localized_tabs.clone(),
                media_manager: field_state.media_manager.clone(),
                error: field_state.error,
                error_message: field_state.error_message.trim().to_owned(),
                errors: clone_validation_issue_vms(&field_state.errors),
            };
            if !field_state.help_text.is_empty() {
                field.help_text = field_state.help_text;
            }
            section.fields.push(field);
        }
        sections.push(section);
    }
    sections
}

/// Build the rendered sections for a registered resource kind.
///
/// An unregistered kind yields no sections.
#[must_use]
pub fn build_resource_form_sections_for_kind<R: Registry + ?Sized>(
    request: &Request,
    resources: Option<&R>,
    kind: &crate::definitions::ResourceKind,
    state: &ResourceFormState,
    translate_key: Option<&TranslateKeyFunc<Request>>,
) -> Vec<ResourceFormSectionVM> {
    let Some(definition) = find_resource_definition(resources, kind) else {
        return Vec::new();
    };
    build_resource_form_sections(request, &definition.form, state, translate_key)
}

/// Resolve the canonical state key for a declared field.
#[must_use]
pub fn resource_form_field_state_key(field: &ResourceFormFieldDefinition) -> String {
    canonical_form_field_state_key(field)
}

/// Attach the matching localized issues to the sections and their tabs.
///
/// A field and its localized tabs are marked only when an issue matches; the field's
/// existing messages are kept and the first matched issue fills an empty field or tab
/// message. The outer section list and each section's field list are copied before
/// mutation.
#[must_use]
pub fn attach_validation_issues_to_form_sections(
    sections: Vec<ResourceFormSectionVM>,
    issues: &[ValidationIssueVM],
) -> Vec<ResourceFormSectionVM> {
    if sections.is_empty() || issues.is_empty() {
        return sections;
    }
    let mut sections = sections;
    for section in &mut sections {
        for field in &mut section.fields {
            let matched = validation_issues_for_resource_field(field, issues);
            if matched.is_empty() {
                continue;
            }
            field.error = true;
            field.errors = clone_validation_issue_vms(&field.errors);
            field.errors.extend(matched.iter().cloned());
            if field.error_message.trim().is_empty() {
                field.error_message = matched[0].message.clone();
            }
            if let Some(localized_tabs) = field.localized_tabs.as_ref() {
                let tabs = attach_validation_issues_to_localized_tabs(localized_tabs, field, &matched);
                field.localized_tabs = Some(Arc::new(tabs));
            }
        }
    }
    sections
}

fn validation_issues_for_resource_field(
    field: &ResourceFieldVM,
    issues: &[ValidationIssueVM],
) -> Vec<ValidationIssueVM> {
    let field_candidates = compact_distinct_strings([
        field.input_id.as_str(),
        field.name.as_str(),
        field.key.as_str(),
    ]);
    if field_candidates.is_empty() {
        return Vec::new();
    }
    let mut matched = Vec::new();
    for issue in issues {
        let underscored_path = issue.path.replace('.', "_");
        let issue_candidates = compact_distinct_strings([
            issue.field_id.as_str(),
            issue.field.as_str(),
            issue.field_key.as_str(),
            issue.path.as_str(),
            underscored_path.as_str(),
        ]);
        'outer: for field_candidate in &field_candidates {
            for issue_candidate in &issue_candidates {
                if validation_field_candidate_matches(field_candidate, issue_candidate) {
                    matched.push(issue.clone());
                    break 'outer;
                }
            }
        }
    }
    matched
}

fn attach_validation_issues_to_localized_tabs(
    props: &LocalizedTabsProps,
    field: &ResourceFieldVM,
    issues: &[ValidationIssueVM],
) -> LocalizedTabsProps {
    if props.tabs.is_empty() || issues.is_empty() {
        return props.clone();
    }
    let mut props = props.clone();
    for tab in &mut props.tabs {
        let matched = validation_issues_for_localized_tab(field, tab, issues);
        if matched.is_empty() {
            continue;
        }
        tab.error = true;
        tab.errors = clone_validation_issue_vms(&tab.errors);
        tab.errors.extend(matched.iter().cloned());
        if tab.error_message.trim().is_empty() {
            tab.error_message = matched[0].message.clone();
        }
    }
    props
}

fn validation_issues_for_localized_tab(
    field: &ResourceFieldVM,
    tab: &LocalizedTab,
    issues: &[ValidationIssueVM],
) -> Vec<ValidationIssueVM> {
    if issues.is_empty() {
        return Vec::new();
    }
    issues
        .iter()
        .filter(|issue| validation_issue_matches_localized_tab(field, tab, issue))
        .cloned()
        .collect()
}

fn validation_issue_matches_localized_tab(
    field: &ResourceFieldVM,
    tab: &LocalizedTab,
    issue: &ValidationIssueVM,
) -> bool {
    let tab_candidates = localized_tab_validation_candidates(field, tab);
    let issue_candidates = localized_tab_issue_candidates(issue);
    for tab_candidate in &tab_candidates {
        for issue_candidate in &issue_candidates {
            if localized_validation_candidate_matches(tab_candidate, issue_candidate) {
                return true;
            }
        }
    }
    let lang = tab.value.trim();
    issue_candidates
        .iter()
        .any(|issue_candidate| localized_path_matches_language(issue_candidate, lang))
}

fn localized_tab_validation_candidates(field: &ResourceFieldVM, tab: &LocalizedTab) -> Vec<String> {
    let lang = tab.value.trim();
    let bases = compact_distinct_strings([
        tab.field_key.as_str(),
        field.input_id.as_str(),
        field.name.as_str(),
        field.key.as_str(),
    ]);
    let mut candidates: Vec<String> = compact_distinct_strings([tab.field_id.as_str()]);
    for base in bases {
        candidates.push(format!("{base}.{lang}"));
        candidates.push(format!("{base}_{lang}"));
        candidates.push(format!("{base}-{lang}"));
    }
    compact_distinct_strings(candidates.iter().map(String::as_str))
}

fn localized_tab_issue_candidates(issue: &ValidationIssueVM) -> Vec<String> {
    let mut candidates = compact_distinct_strings([
        issue.field_id.as_str(),
        issue.field.as_str(),
        issue.field_key.as_str(),
        issue.path.as_str(),
    ]);
    if !issue.path.is_empty() {
        candidates.push(issue.path.replace('.', "_"));
        candidates.push(issue.path.replace('.', "-"));
    }
    compact_distinct_strings(candidates.iter().map(String::as_str))
}

fn localized_validation_candidate_matches(tab_candidate: &str, issue_candidate: &str) -> bool {
    let tab_candidate = tab_candidate.trim().to_lowercase();
    let issue_candidate = issue_candidate.trim().to_lowercase();
    if tab_candidate.is_empty() || issue_candidate.is_empty() {
        return false;
    }
    if tab_candidate == issue_candidate {
        return true;
    }
    issue_candidate.starts_with(&format!("{tab_candidate}."))
        || issue_candidate.starts_with(&format!("{tab_candidate}_"))
        || issue_candidate.starts_with(&format!("{tab_candidate}-"))
}

fn localized_path_matches_language(path: &str, lang: &str) -> bool {
    let path = path.trim().to_lowercase();
    let lang = lang.trim().to_lowercase();
    if path.is_empty() || lang.is_empty() {
        return false;
    }
    if path == lang {
        return true;
    }
    for separator in [".", "_", "-"] {
        let token = format!("{separator}{lang}");
        if path.ends_with(&token)
            || path.contains(&format!("{token}."))
            || path.contains(&format!("{token}_"))
            || path.contains(&format!("{token}-"))
        {
            return true;
        }
    }
    for segment in path.split(['.', '_', '[', ']']) {
        let segment = segment.trim();
        if segment == lang || segment.starts_with(&format!("{lang}-")) {
            return true;
        }
    }
    false
}

fn clone_string_map(source: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    source.clone()
}

fn clone_field_options(source: &[ResourceFieldOptionVM]) -> Vec<ResourceFieldOptionVM> {
    if source.is_empty() {
        return Vec::new();
    }
    source
        .iter()
        .map(|option| ResourceFieldOptionVM {
            value: option.value.clone(),
            label: option.label.clone(),
            selected: option.selected,
            attrs: option.attrs.clone(),
        })
        .collect()
}

fn compact_distinct_strings<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if !seen.insert(value.to_owned()) {
            continue;
        }
        result.push(value.to_owned());
    }
    result
}

fn validation_field_candidate_matches(field_candidate: &str, candidate: &str) -> bool {
    let field_candidate = field_candidate.trim();
    let candidate = candidate.trim();
    if field_candidate.is_empty() || candidate.is_empty() {
        return false;
    }
    if field_candidate == candidate {
        return true;
    }
    candidate.starts_with(&format!("{field_candidate}."))
        || candidate.starts_with(&format!("{field_candidate}_"))
}
