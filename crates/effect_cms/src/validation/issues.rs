//! Validation issues and the owner's issue error contract.
//!
//! The issue code/source/scope vocabularies, the normalized issue record, the
//! localized issue view model and the `ValidationError` wrapper the rest of the
//! platform returns.
//!
//! # Open vocabularies
//!
//! Codes, sources and scopes are open string values: the named constants are
//! spellings, not an admission rule, so a consumer-defined code is as valid as a
//! named one. The
//! normalization fallback is what turns a zero value into `unknown`, `manual` and
//! the derived scope.
//!
//! # Normalization
//!
//! [`normalize`] normalizes every text field: a blank code becomes [`VALIDATION_ISSUE_UNKNOWN`], a blank source becomes
//! [`VALIDATION_ISSUE_SOURCE_MANUAL`], a blank scope is derived from the field
//! identity, the field falls back to the field key and then the path, the field id
//! falls back to the field, and absent arguments become the empty map.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::queries::first_non_blank;

/// The `required` issue code.
pub const VALIDATION_ISSUE_REQUIRED: &str = "required";
/// The `min_length` issue code.
pub const VALIDATION_ISSUE_MIN_LENGTH: &str = "min_length";
/// The `max_length` issue code.
pub const VALIDATION_ISSUE_MAX_LENGTH: &str = "max_length";
/// The `invalid_format` issue code.
pub const VALIDATION_ISSUE_INVALID_FORMAT: &str = "invalid_format";
/// The `invalid_option` issue code.
pub const VALIDATION_ISSUE_INVALID_OPTION: &str = "invalid_option";
/// The `invalid_url` issue code.
pub const VALIDATION_ISSUE_INVALID_URL: &str = "invalid_url";
/// The `invalid_datetime` issue code.
pub const VALIDATION_ISSUE_INVALID_DATETIME: &str = "invalid_datetime";
/// The `invalid_timezone` issue code.
pub const VALIDATION_ISSUE_INVALID_TIMEZONE: &str = "invalid_timezone";
/// The `invalid_duration` issue code.
pub const VALIDATION_ISSUE_INVALID_DURATION: &str = "invalid_duration";
/// The `duplicate` issue code.
pub const VALIDATION_ISSUE_DUPLICATE: &str = "duplicate";
/// The `schema` issue code.
pub const VALIDATION_ISSUE_SCHEMA: &str = "schema";
/// The `unknown` issue code.
pub const VALIDATION_ISSUE_UNKNOWN: &str = "unknown";

/// The `cue` issue source.
pub const VALIDATION_ISSUE_SOURCE_CUE: &str = "cue";
/// The `manual` issue source.
pub const VALIDATION_ISSUE_SOURCE_MANUAL: &str = "manual";
/// The `system` issue source.
pub const VALIDATION_ISSUE_SOURCE_SYSTEM: &str = "system";

/// The `form` issue scope.
pub const VALIDATION_ISSUE_SCOPE_FORM: &str = "form";
/// The `field` issue scope.
pub const VALIDATION_ISSUE_SCOPE_FIELD: &str = "field";

/// The form-scope alias.
pub const SCOPE_FORM: &str = VALIDATION_ISSUE_SCOPE_FORM;
/// The field-scope alias.
pub const SCOPE_FIELD: &str = VALIDATION_ISSUE_SCOPE_FIELD;

/// One open validation issue vocabulary value.
///
/// The value is retained
/// exactly as written and the owner performs no vocabulary rejection.
macro_rules! open_validation_value {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
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

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.0.as_str())
            }
        }
    };
}

open_validation_value! {
    /// One issue code.
    ValidationIssueCode
}

open_validation_value! {
    /// One issue source.
    ValidationIssueSource
}

open_validation_value! {
    /// One issue scope.
    ValidationIssueScope
}

/// One scalar issue argument.
///
/// Arguments are integers (`min`, `max`) plus text handed to the translator. The
/// closed scalar kinds below are the JSON scalar shapes the argument map can carry;
/// an unsupported type is unrepresentable rather than coerced silently.
#[derive(Clone, Debug, PartialEq)]
pub enum ValidationArgument {
    /// An integer argument.
    Integer(i64),
    /// A floating-point argument.
    Number(f64),
    /// A text argument.
    Text(String),
}

impl ValidationArgument {
    /// The integer reading of this argument, matching `validationIssueIntArg`.
    ///
    /// Integers and floats convert directly, and text parses as an integer,
    /// yielding zero on failure or an unsupported kind. Float-to-integer conversion
    /// saturates outside the integer range.
    #[must_use]
    pub fn as_integer(&self) -> i64 {
        match self {
            Self::Integer(value) => *value,
            Self::Number(value) => *value as i64,
            Self::Text(value) => value.trim().parse().unwrap_or(0),
        }
    }
}

/// One validation issue, normalized to the owner's canonical shape.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValidationIssue {
    /// The issue code.
    pub code: ValidationIssueCode,
    /// Where the issue came from.
    pub source: ValidationIssueSource,
    /// Whether the issue belongs to the form or to one field.
    pub scope: ValidationIssueScope,
    /// The dotted path inside the validated value.
    pub path: String,
    /// The submitted field name.
    pub field: String,
    /// The rendered input id.
    pub field_id: String,
    /// The declared field key.
    pub field_key: String,
    /// The declared section key.
    pub section_key: String,
    /// An explicit message key.
    pub message_key: String,
    /// Typed translation arguments.
    pub args: BTreeMap<String, ValidationArgument>,
    /// A source-supplied message, used when no translation is available.
    pub message: String,
}

/// One localized issue.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ValidationIssueVM {
    /// The issue code.
    pub code: String,
    /// The dotted path inside the validated value.
    pub path: String,
    /// The submitted field name.
    pub field: String,
    /// The rendered input id.
    pub field_id: String,
    /// The declared field key.
    pub field_key: String,
    /// The human-readable field label.
    pub field_name: String,
    /// The localized message.
    pub message: String,
}

/// A validation error: normalized issues plus an optional cause.
#[derive(Debug)]
pub struct ValidationError {
    issues: Vec<ValidationIssue>,
    cause: Option<Box<dyn Error + Send + 'static>>,
}

impl ValidationError {
    /// Build the error form of the supplied issues.
    ///
    /// Returns `None` when no issue was supplied, matching an empty-set
    /// constructor that returns no error.
    #[must_use]
    pub fn new(issues: Vec<ValidationIssue>) -> Option<Self> {
        Self::wrap(None, issues)
    }

    /// Build the error form of the supplied cause and issues.
    ///
    /// Returns `None` when neither a cause nor an issue was supplied.
    #[must_use]
    pub fn wrap(
        cause: Option<Box<dyn Error + Send + 'static>>,
        issues: Vec<ValidationIssue>,
    ) -> Option<Self> {
        let issues: Vec<ValidationIssue> = issues.into_iter().map(normalize).collect();
        if issues.is_empty() && cause.is_none() {
            return None;
        }
        Some(Self { issues, cause })
    }

    /// The normalized issues.
    #[must_use]
    pub fn issues(&self) -> &[ValidationIssue] {
        &self.issues
    }

    /// The wrapped cause.
    #[must_use]
    pub fn cause(&self) -> Option<&(dyn Error + 'static)> {
        self.cause
            .as_deref()
            .map(|cause| cause as &(dyn Error + 'static))
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for issue in &self.issues {
            if !issue.message.is_empty() {
                return formatter.write_str(issue.message.as_str());
            }
            if !issue.message_key.is_empty() {
                return formatter.write_str(issue.message_key.as_str());
            }
        }
        if let Some(cause) = self.cause.as_deref() {
            return fmt::Display::fmt(cause, formatter);
        }
        formatter.write_str("validation failed")
    }
}

impl Error for ValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause()
    }
}

/// Normalize one issue to the owner's canonical shape.
#[must_use]
pub fn normalize(issue: ValidationIssue) -> ValidationIssue {
    normalize_validation_issue(issue)
}

/// Clone one issue deeply, including its argument map.
#[must_use]
pub fn clone_validation_issue(issue: ValidationIssue) -> ValidationIssue {
    clone_issue(issue)
}

/// Clone a localized issue list; an empty source stays an empty list.
#[must_use]
pub fn clone_validation_issue_vms(source: &[ValidationIssueVM]) -> Vec<ValidationIssueVM> {
    source.to_vec()
}

/// The validation issues carried by an error in its chain, if any.
///
/// This walks the error's `source()` chain and returns the issues of the first
/// [`ValidationError`] found.
#[must_use]
pub fn validation_issues_from_error(error: &(dyn Error + 'static)) -> Vec<ValidationIssue> {
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(error) = current {
        if let Some(validation) = error.downcast_ref::<ValidationError>() {
            return validation.issues.iter().cloned().map(normalize).collect();
        }
        current = error.source();
    }
    Vec::new()
}

/// The first non-blank issue message of an error, else the error text.
#[must_use]
pub fn first_validation_issue_message(error: Option<&(dyn Error + 'static)>) -> String {
    let Some(error) = error else {
        return String::new();
    };
    for issue in validation_issues_from_error(error) {
        if !issue.message.is_empty() {
            return issue.message;
        }
    }
    error.to_string().trim().to_owned()
}

/// The owner's canonical normalization of one issue.
fn normalize_validation_issue(issue: ValidationIssue) -> ValidationIssue {
    let mut issue = issue;
    issue.path = issue.path.trim().to_owned();
    issue.field = issue.field.trim().to_owned();
    issue.field_id = issue.field_id.trim().to_owned();
    issue.field_key = issue.field_key.trim().to_owned();
    issue.section_key = issue.section_key.trim().to_owned();
    issue.message_key = issue.message_key.trim().to_owned();
    issue.message = issue.message.trim().to_owned();
    if issue.code.as_str().is_empty() {
        issue.code = ValidationIssueCode::from(VALIDATION_ISSUE_UNKNOWN);
    }
    if issue.source.as_str().is_empty() {
        issue.source = ValidationIssueSource::from(VALIDATION_ISSUE_SOURCE_MANUAL);
    }
    if issue.scope.as_str().is_empty() {
        issue.scope = if issue.path.is_empty()
            && issue.field.is_empty()
            && issue.field_id.is_empty()
            && issue.field_key.is_empty()
        {
            ValidationIssueScope::from(VALIDATION_ISSUE_SCOPE_FORM)
        } else {
            ValidationIssueScope::from(VALIDATION_ISSUE_SCOPE_FIELD)
        };
    }
    if issue.field.is_empty() {
        issue.field = first_non_blank(&[issue.field_key.clone(), issue.path.clone()]);
    }
    if issue.field_id.is_empty() {
        issue.field_id = issue.field.clone();
    }
    issue
}

/// Clone one issue, normalizing it and copying its argument map.
fn clone_issue(issue: ValidationIssue) -> ValidationIssue {
    let mut issue = normalize_validation_issue(issue);
    if !issue.args.is_empty() {
        issue.args = issue.args.clone();
    }
    issue
}
