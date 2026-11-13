//! Validation issue localization.
//!
//! Issue localization: the issue message key, the localized issue view model, the
//! validation summary and the field-label resolution against the caller's form
//! definition.
//!
//! # Two translation ports
//!
//! Issue and summary messages use a context translation capability that returns the
//! key itself when no message is installed, so the owner detects that recovery and
//! applies its documented fallback text. Form field labels use the localization
//! owner's [`TranslateKeyFunc`], which has no format arguments.
//!
//! Both stay caller-supplied capabilities: [`ValidationTranslation`] is the
//! context translation contract (the caller's context type implements it, and
//! [`BackgroundValidationTranslation`] returns every key unchanged), while the label
//! port is the localization owner's [`TranslateKeyFunc`] over the same caller context
//! type. No catalog, translator, culture registry or global lookup is created here.
//!
//! # Fallback text
//!
//! When the translation is absent (the result equals the key), an explicit supplied
//! message wins; otherwise the owner renders the English fallback sentence for the
//! issue code.

use std::collections::BTreeSet;

use crate::definitions::{
    ResourceAction, ResourceFormDefinition, ResourceFormFieldDefinition, ResourceKind,
    resource_form_field_state_key,
};
use crate::localization::TranslateKeyFunc;
use crate::queries::first_non_blank;

use super::issues::{
    VALIDATION_ISSUE_DUPLICATE, VALIDATION_ISSUE_INVALID_OPTION, VALIDATION_ISSUE_INVALID_URL,
    VALIDATION_ISSUE_MAX_LENGTH, VALIDATION_ISSUE_MIN_LENGTH, VALIDATION_ISSUE_REQUIRED,
    VALIDATION_ISSUE_SCOPE_FORM, VALIDATION_ISSUE_UNKNOWN, ValidationArgument, ValidationIssue,
    ValidationIssueVM, normalize,
};

/// The validation summary title key.
pub const VALIDATION_SUMMARY_TITLE_KEY: &str = "contentmgmt.validation.summary.title";
/// The one-issue summary key.
const VALIDATION_SUMMARY_ONE_KEY: &str = "contentmgmt.validation.summary.one";
/// The many-issue summary key.
const VALIDATION_SUMMARY_MANY_KEY: &str = "contentmgmt.validation.summary.many";
/// The fallback field-name key.
const VALIDATION_FIELD_FALLBACK_KEY: &str = "contentmgmt.validation.field.this_field";

/// The caller's translation capability.
///
/// An implementation returns the key itself when it has no message for that key;
/// this owner detects that recovery by comparing the result with the key, so an
/// implementation must not invent a message.
pub trait ValidationTranslation {
    /// Translate one key with typed format arguments, returning the key when absent.
    fn translate(&self, key: &str, args: &[&ValidationArgument]) -> String;
}

/// A background translation: no translator is installed, so every
/// lookup returns the key and the owner's fallback text applies.
#[derive(Clone, Copy, Debug, Default)]
pub struct BackgroundValidationTranslation;

impl ValidationTranslation for BackgroundValidationTranslation {
    fn translate(&self, key: &str, _args: &[&ValidationArgument]) -> String {
        key.to_owned()
    }
}

/// Configures one issue localization pass.
pub struct ValidationLocalizationOptions<C> {
    /// The resource kind being rendered.
    pub resource: ResourceKind,
    /// The action being rendered.
    pub action: ResourceAction,
    /// The caller's form definition, used to resolve field labels.
    pub definition: ResourceFormDefinition,
    /// The consumer's label translation port.
    pub translate_key: Option<TranslateKeyFunc<C>>,
}

/// The message key an issue renders through.
#[must_use]
pub fn validation_issue_message_key(issue: &ValidationIssue) -> String {
    let key = issue.message_key.trim();
    if !key.is_empty() {
        return key.to_owned();
    }
    let code = issue.code.as_str().trim();
    let code = if code.is_empty() {
        VALIDATION_ISSUE_UNKNOWN
    } else {
        code
    };
    format!("contentmgmt.validation.issue.{code}")
}

/// Localize one issue.
#[must_use]
pub fn localize_validation_issue<C: ValidationTranslation>(
    context: &C,
    issue: ValidationIssue,
    options: &ValidationLocalizationOptions<C>,
) -> ValidationIssueVM {
    let issue = normalize(issue);
    let field_name = validation_issue_field_name(context, &issue, options);
    let message = validation_issue_message(context, &issue, &field_name);
    ValidationIssueVM {
        code: issue.code.as_str().to_owned(),
        path: issue.path,
        field: issue.field,
        field_id: issue.field_id,
        field_key: issue.field_key,
        field_name,
        message,
    }
}

/// Localize every issue; an empty source stays an empty list.
#[must_use]
pub fn localize_validation_issues<C: ValidationTranslation>(
    context: &C,
    source: &[ValidationIssue],
    options: &ValidationLocalizationOptions<C>,
) -> Vec<ValidationIssueVM> {
    source
        .iter()
        .cloned()
        .map(|issue| localize_validation_issue(context, issue, options))
        .collect()
}

/// The localized validation summary for the supplied issues.
#[must_use]
pub fn validation_summary_message<C: ValidationTranslation>(
    context: &C,
    source: &[ValidationIssue],
    _options: &ValidationLocalizationOptions<C>,
) -> String {
    if source.is_empty() {
        return String::new();
    }
    let count = validation_issue_field_count(source);
    if count <= 1 {
        return translate_validation(context, VALIDATION_SUMMARY_ONE_KEY, &[]);
    }
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    translate_validation(
        context,
        VALIDATION_SUMMARY_MANY_KEY,
        &[ValidationArgument::Integer(count)],
    )
}

/// The localized validation summary title.
#[must_use]
pub fn validation_summary_title<C: ValidationTranslation>(context: &C) -> String {
    translate_validation(context, VALIDATION_SUMMARY_TITLE_KEY, &[])
}

/// Field-name humanization: separators become spaces and every
/// word starts uppercase.
fn humanize_validation_field(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let replaced = value.replace(['.', '_', '-'], " ");
    let mut parts: Vec<String> = Vec::new();
    for part in replaced.split_whitespace() {
        let mut characters = part.chars();
        let Some(first) = characters.next() else {
            continue;
        };
        let mut text = String::new();
        text.extend(first.to_uppercase());
        text.push_str(characters.as_str());
        parts.push(text);
    }
    parts.join(" ")
}

/// One issue's localized message.
fn validation_issue_message<C: ValidationTranslation>(
    context: &C,
    issue: &ValidationIssue,
    field_name: &str,
) -> String {
    let key = validation_issue_message_key(issue);
    let args = validation_issue_translation_args(issue, field_name);
    let message = translate_validation(context, &key, &args);
    if !message.is_empty() && message != key {
        return message;
    }
    if !issue.message.is_empty() {
        return issue.message.clone();
    }
    fallback_validation_issue_message(issue, field_name)
}

/// The format arguments one message key receives.
fn validation_issue_translation_args(
    issue: &ValidationIssue,
    field_name: &str,
) -> Vec<ValidationArgument> {
    if !issue.message_key.trim().is_empty() {
        return Vec::new();
    }
    match issue.code.as_str() {
        VALIDATION_ISSUE_MIN_LENGTH => vec![
            ValidationArgument::Text(field_name.to_owned()),
            ValidationArgument::Integer(validation_issue_int_arg(issue, "min")),
        ],
        VALIDATION_ISSUE_MAX_LENGTH => vec![
            ValidationArgument::Text(field_name.to_owned()),
            ValidationArgument::Integer(validation_issue_int_arg(issue, "max")),
        ],
        _ => vec![ValidationArgument::Text(field_name.to_owned())],
    }
}

/// One integer issue argument, zero when absent.
fn validation_issue_int_arg(issue: &ValidationIssue, key: &str) -> i64 {
    issue
        .args
        .get(key)
        .map_or(0, ValidationArgument::as_integer)
}

/// The human-readable field name for one issue.
fn validation_issue_field_name<C: ValidationTranslation>(
    context: &C,
    issue: &ValidationIssue,
    options: &ValidationLocalizationOptions<C>,
) -> String {
    let label = validation_issue_field_label(context, issue, options);
    if !label.is_empty() {
        return label;
    }
    if !issue.field.is_empty() {
        return humanize_validation_field(&issue.field);
    }
    if !issue.path.is_empty() {
        return humanize_validation_field(&issue.path);
    }
    translate_validation(context, VALIDATION_FIELD_FALLBACK_KEY, &[])
}

/// The label of the first declared form field the issue identifies.
///
/// A matching field ends the search even when both its translated label and its
/// humanized state key are blank, matching the
/// `for`-loop on the first match.
fn validation_issue_field_label<C: ValidationTranslation>(
    context: &C,
    issue: &ValidationIssue,
    options: &ValidationLocalizationOptions<C>,
) -> String {
    if options.definition.sections.is_empty() {
        return String::new();
    }
    let candidates = validation_issue_field_candidates(issue);
    let mut label = String::new();
    let mut matched = false;
    options.definition.sections.with(|sections| {
        for section in sections {
            section.fields.with(|fields| {
                for field in fields {
                    if !resource_form_field_matches_issue(field, &candidates) {
                        continue;
                    }
                    let translated = translate_builder_key(
                        context,
                        options.translate_key.as_ref(),
                        field.label_key.as_str(),
                    );
                    label = if translated.is_empty() {
                        humanize_validation_field(&resource_form_field_state_key(field))
                    } else {
                        translated
                    };
                    matched = true;
                    break;
                }
            });
            if matched {
                break;
            }
        }
    });
    label
}

/// The identities one issue can be matched by.
fn validation_issue_field_candidates(issue: &ValidationIssue) -> Vec<String> {
    let issue = normalize(issue.clone());
    let mut values = vec![
        issue.field_id,
        issue.field,
        issue.field_key,
        issue.path.clone(),
    ];
    if !issue.path.is_empty() {
        values.push(issue.path.replace('.', "_"));
    }
    compact_distinct_strings(values)
}

/// Whether one declared field answers to any of the issue's identities.
fn resource_form_field_matches_issue(
    field: &ResourceFormFieldDefinition,
    candidates: &[String],
) -> bool {
    let field_candidates = compact_distinct_strings(vec![
        field.input_id.clone(),
        field.name.clone(),
        field.key.as_str().to_owned(),
        resource_form_field_state_key(field),
    ]);
    for field_candidate in &field_candidates {
        for candidate in candidates {
            if validation_field_candidate_matches(field_candidate, candidate) {
                return true;
            }
        }
    }
    false
}

/// Whether one declared identity covers one issue identity.
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

/// The number of distinct fields the issues report, counting form-level issues
/// individually.
fn validation_issue_field_count(source: &[ValidationIssue]) -> usize {
    let mut fields: BTreeSet<String> = BTreeSet::new();
    let mut form_level = 0_usize;
    for issue in source {
        let issue = normalize(issue.clone());
        if issue.scope.as_str() == VALIDATION_ISSUE_SCOPE_FORM {
            form_level += 1;
            continue;
        }
        let key = first_non_blank(&[issue.field_id, issue.field, issue.field_key, issue.path]);
        if key.is_empty() {
            form_level += 1;
            continue;
        }
        fields.insert(key);
    }
    fields.len() + form_level
}

/// Trim the message text of one translation.
fn translate_validation<C: ValidationTranslation>(
    context: &C,
    key: &str,
    args: &[ValidationArgument],
) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    let args: Vec<&ValidationArgument> = args.iter().collect();
    context.translate(key, &args).trim().to_owned()
}

/// Resolve one field-label key through the caller's port, returning the key when
/// the port is absent.
fn translate_builder_key<C>(
    context: &C,
    translate_key: Option<&TranslateKeyFunc<C>>,
    key: &str,
) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    match translate_key {
        Some(translate) => translate(context, key),
        None => key.to_owned(),
    }
}

/// The English fallback sentence for one issue.
fn fallback_validation_issue_message(issue: &ValidationIssue, field_name: &str) -> String {
    let trimmed = field_name.trim();
    let field_name = if trimmed.is_empty() {
        "This field"
    } else {
        trimmed
    };
    match issue.code.as_str() {
        VALIDATION_ISSUE_REQUIRED => format!("{field_name} is required."),
        VALIDATION_ISSUE_MIN_LENGTH => format!(
            "{field_name} must be at least {} characters.",
            validation_issue_int_arg(issue, "min")
        ),
        VALIDATION_ISSUE_MAX_LENGTH => format!(
            "{field_name} must be at most {} characters.",
            validation_issue_int_arg(issue, "max")
        ),
        VALIDATION_ISSUE_INVALID_URL => format!("{field_name} must be a valid URL."),
        VALIDATION_ISSUE_INVALID_OPTION => {
            format!("{field_name} must be one of the available options.")
        }
        VALIDATION_ISSUE_DUPLICATE => format!("{field_name} is already in use."),
        _ => format!("{field_name} is invalid."),
    }
}

/// Trim, drop empty entries and deduplicate, preserving the first occurrence order.
fn compact_distinct_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut result: Vec<String> = Vec::new();
    for value in values {
        let value = value.trim().to_owned();
        if value.is_empty() || !seen.insert(value.clone()) {
            continue;
        }
        result.push(value);
    }
    result
}
