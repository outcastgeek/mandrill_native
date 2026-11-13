//! Validation results: the W6 terminal projection.
//!
//! Validation result types: the outcome code vocabulary, the validation result
//! record and the invalid/success constructors the command and mutation owners
//! consume.
//!
//! The workflow that produces a result from a request belongs to the command
//! integration and is not modelled here; these are the result contracts it publishes
//! and the pure constructors that preserve the invalid-form defaults
//! (`SubmitDisabled` and `RequiresRehydrate`).

use std::error::Error;
use std::fmt;

use crate::definitions::ResourceAction;

use super::issues::{
    VALIDATION_ISSUE_SCOPE_FORM, VALIDATION_ISSUE_SOURCE_SYSTEM, VALIDATION_ISSUE_UNKNOWN,
    ValidationIssue, ValidationIssueCode, ValidationIssueScope, ValidationIssueSource,
    clone_validation_issue, validation_issues_from_error,
};

/// The `valid` outcome code.
pub const CODE_VALID: &str = "valid";
/// The `invalid-request` outcome code.
pub const CODE_INVALID_REQUEST: &str = "invalid-request";
/// The `invalid-query` outcome code.
pub const CODE_INVALID_QUERY: &str = "invalid-query";
/// The `unsupported-action` outcome code.
pub const CODE_UNSUPPORTED_ACTION: &str = "unsupported-action";

/// One open validation outcome code vocabulary value.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ValidationCode(String);

impl ValidationCode {
    /// Admit the caller-supplied value exactly as written.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value exactly as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for ValidationCode {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ValidationCode {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl AsRef<str> for ValidationCode {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for ValidationCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.as_str())
    }
}

/// The projection of the W6 terminal validation outcome.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValidationResult {
    /// The action the input belongs to.
    pub action: ResourceAction,
    /// The outcome class.
    pub code: ValidationCode,
    /// The projection message.
    pub message: String,
    /// The issues behind an invalid outcome.
    pub issues: Vec<ValidationIssue>,
    /// Whether the presentation disables submission.
    pub submit_disabled: bool,
    /// Whether the presentation must rehydrate the form.
    pub requires_rehydrate: bool,
}

/// The accepted result for one action.
#[must_use]
pub fn success_validation_result(action: ResourceAction) -> ValidationResult {
    ValidationResult {
        action,
        code: ValidationCode::from(CODE_VALID),
        ..ValidationResult::default()
    }
}

/// The preserved invalid-form projection.
#[must_use]
pub fn invalid_validation_result(
    action: ResourceAction,
    code: ValidationCode,
    message: impl AsRef<str>,
) -> ValidationResult {
    ValidationResult {
        action,
        code,
        message: message.as_ref().trim().to_owned(),
        issues: Vec::new(),
        submit_disabled: true,
        requires_rehydrate: true,
    }
}

/// The preserved invalid-form projection carrying detached issues.
#[must_use]
pub fn invalid_validation_result_with_issues(
    action: ResourceAction,
    code: ValidationCode,
    message: impl AsRef<str>,
    source: &[ValidationIssue],
) -> ValidationResult {
    let mut result = invalid_validation_result(action, code, message);
    if !source.is_empty() {
        result.issues = source.iter().cloned().map(clone_validation_issue).collect();
    }
    if result.message.is_empty() {
        for issue in &result.issues {
            if !issue.message.is_empty() {
                result.message = issue.message.clone();
                break;
            }
        }
    }
    result
}

/// The invalid-form projection derived from an error.
///
/// The error's own validation issues are preserved; when it carries none and has
/// text, one form-level `unknown`/`system` issue keeps the message visible.
#[must_use]
pub fn invalid_validation_result_from_error(
    action: ResourceAction,
    code: ValidationCode,
    error: Option<&(dyn Error + 'static)>,
) -> ValidationResult {
    let message = error
        .map(|error| error.to_string().trim().to_owned())
        .unwrap_or_default();
    let mut source = error.map(validation_issues_from_error).unwrap_or_default();
    if source.is_empty() && !message.is_empty() {
        source = vec![ValidationIssue {
            code: ValidationIssueCode::from(VALIDATION_ISSUE_UNKNOWN),
            source: ValidationIssueSource::from(VALIDATION_ISSUE_SOURCE_SYSTEM),
            scope: ValidationIssueScope::from(VALIDATION_ISSUE_SCOPE_FORM),
            message: message.clone(),
            ..ValidationIssue::default()
        }];
    }
    invalid_validation_result_with_issues(action, code, message, &source)
}
