//! Native contract validation for the content document selectors.
//!
//! Ten builtin selectors name the closed documents the content owner publishes:
//! command input, validation result, mutation result, form page, list page,
//! detail page, dashboard page, SEO record, sitemap document and sitemap index.
//! Each contract reuses the shared record, list, map, enum and scalar codecs; it
//! checks declared field presence, closed property sets, wildcard and enum-keyed
//! maps, recursive list children, Unicode scalar-length minima, integral and
//! inclusive numeric bounds, and declared literal values.
//!
//! Validation accepts already parsed JSON, serialized JSON text, or any value
//! implementing [`Serialize`]. Failures become the owner's [`ValidationIssue`]
//! values with selector-relative field paths, typed arguments and the underlying
//! [`SchemaIssue`] retained as the error cause. An unknown selector, or a contract
//! whose definition cannot be built, is an explicit error; it is never treated as
//! a pass. Caller-defined contract files, overlays and selector lookup are outside
//! this builtin surface.
//!
//! Every entry point takes the caller's traversal policy. The policy's collection
//! limit bounds each object's property count and each array's element count, and
//! its issue limit bounds reported issues; no hidden default is supplied.
//!
//! Wrapper codecs forward the invocation context: required-field presence and value
//! validation are composed with an all-of codec, so a caller's [`ParseContext`]
//! reaches presence checks, nested maps, arrays and recursive children. The owned
//! effect entry accepts an existing context; the serialized effect entry creates one
//! from the supplied schema policy. Borrowed and serialized convenience entries also
//! carry explicit contexts through the native codec boundary.

mod codec;
mod command;
mod pages;
mod shared;
mod sitemap;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use effect::{
    Effect, IssueSegment, JsonValue, ParseContext, ParsePolicy, PureCodec, SchemaIssue,
    SchemaIssueKind, SchemaPolicy, effect_schema_parse,
};
use serde::Serialize;

use super::issues::{
    VALIDATION_ISSUE_DUPLICATE, VALIDATION_ISSUE_INVALID_FORMAT, VALIDATION_ISSUE_INVALID_OPTION,
    VALIDATION_ISSUE_MAX_LENGTH, VALIDATION_ISSUE_MIN_LENGTH, VALIDATION_ISSUE_REQUIRED,
    VALIDATION_ISSUE_SCHEMA, VALIDATION_ISSUE_SCOPE_FIELD, VALIDATION_ISSUE_SCOPE_FORM,
    VALIDATION_ISSUE_SOURCE_CUE, ValidationArgument, ValidationError, ValidationIssue,
    ValidationIssueCode, ValidationIssueScope, ValidationIssueSource, normalize,
};

/// The command-input document selector.
pub const SELECTOR_COMMAND_INPUT: &str = "#CommandInputVM";
/// The validation-result document selector.
pub const SELECTOR_VALIDATION_RESULT: &str = "#ValidationResultVM";
/// The mutation-result document selector.
pub const SELECTOR_MUTATION_RESULT: &str = "#MutationResultVM";
/// The form-page document selector.
pub const SELECTOR_RESOURCE_FORM_PAGE: &str = "#ResourceFormPageVM";
/// The list-page document selector.
pub const SELECTOR_RESOURCE_LIST_PAGE: &str = "#ResourceListPageVM";
/// The detail-page document selector.
pub const SELECTOR_RESOURCE_DETAIL_PAGE: &str = "#ResourceDetailPageVM";
/// The dashboard-page document selector.
pub const SELECTOR_RESOURCE_DASHBOARD_PAGE: &str = "#ResourceDashboardPageVM";
/// The SEO document selector.
pub const SELECTOR_SEO: &str = "#SEOVM";
/// The sitemap-document selector.
pub const SELECTOR_SITEMAP_DOCUMENT: &str = "#SitemapDocumentVM";
/// The sitemap-index selector.
pub const SELECTOR_SITEMAP_INDEX: &str = "#SitemapIndexVM";

/// Every builtin selector, in declaration order.
pub const BUILTIN_CONTRACT_SELECTORS: [&str; 10] = [
    SELECTOR_COMMAND_INPUT,
    SELECTOR_VALIDATION_RESULT,
    SELECTOR_MUTATION_RESULT,
    SELECTOR_RESOURCE_FORM_PAGE,
    SELECTOR_RESOURCE_LIST_PAGE,
    SELECTOR_RESOURCE_DETAIL_PAGE,
    SELECTOR_RESOURCE_DASHBOARD_PAGE,
    SELECTOR_SEO,
    SELECTOR_SITEMAP_DOCUMENT,
    SELECTOR_SITEMAP_INDEX,
];

/// A contract failure that is not a document violation.
#[derive(Debug)]
pub enum BuiltinContractError {
    /// No builtin contract is registered under the selector.
    UnknownSelector(String),
    /// The selector is registered but its contract could not be built.
    Unavailable {
        /// The selector whose contract is unavailable.
        selector: String,
        /// The definition failure.
        cause: Box<SchemaIssue>,
    },
    /// The document violated its contract.
    Invalid {
        /// The selector whose contract was applied.
        selector: String,
        /// The normalized issues derived from the structured schema failure.
        error: ValidationError,
    },
}

impl BuiltinContractError {
    /// The selector this error belongs to.
    #[must_use]
    pub fn selector(&self) -> &str {
        match self {
            Self::UnknownSelector(selector) => selector,
            Self::Unavailable { selector, .. } => selector,
            Self::Invalid { selector, .. } => selector,
        }
    }

    /// The normalized document issues; non-violation failures carry none.
    #[must_use]
    pub fn issues(&self) -> &[ValidationIssue] {
        match self {
            Self::Invalid { error, .. } => error.issues(),
            Self::UnknownSelector(_) | Self::Unavailable { .. } => &[],
        }
    }

    /// The retained violation wrapper for the invalid variant.
    #[must_use]
    pub fn validation_error(&self) -> Option<&ValidationError> {
        match self {
            Self::Invalid { error, .. } => Some(error),
            Self::UnknownSelector(_) | Self::Unavailable { .. } => None,
        }
    }

    pub(super) fn invalid(selector: &str, issue: SchemaIssue) -> Self {
        let issues = builtin_contract_issues(&issue);
        match ValidationError::wrap(Some(Box::new(issue.clone())), issues) {
            Some(error) => Self::Invalid {
                selector: selector.to_owned(),
                error,
            },
            None => Self::Unavailable {
                selector: selector.to_owned(),
                cause: Box::new(issue),
            },
        }
    }

    fn serialization(selector: &str, error: serde_json::Error) -> Self {
        let detail = format!("value could not be serialized as JSON: {error}");
        let issue = ValidationIssue {
            code: ValidationIssueCode::from(VALIDATION_ISSUE_INVALID_FORMAT),
            source: ValidationIssueSource::from(VALIDATION_ISSUE_SOURCE_CUE),
            scope: ValidationIssueScope::from(VALIDATION_ISSUE_SCOPE_FORM),
            message: detail.clone(),
            ..ValidationIssue::default()
        };
        match ValidationError::wrap(Some(Box::new(error)), vec![issue]) {
            Some(error) => Self::Invalid {
                selector: selector.to_owned(),
                error,
            },
            None => Self::Unavailable {
                selector: selector.to_owned(),
                cause: Box::new(SchemaIssue::new(SchemaIssueKind::JsonEncoding {
                    detail: detail.into(),
                })),
            },
        }
    }
}

impl fmt::Display for BuiltinContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSelector(selector) => {
                write!(formatter, "unknown builtin contract selector: {selector}")
            }
            Self::Unavailable { selector, cause } => {
                write!(
                    formatter,
                    "builtin contract {selector} is unavailable: {cause}"
                )
            }
            Self::Invalid { error, .. } => fmt::Display::fmt(error, formatter),
        }
    }
}

impl Error for BuiltinContractError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnknownSelector(_) => None,
            Self::Unavailable { cause, .. } => Some(cause.as_ref() as &(dyn Error + 'static)),
            Self::Invalid { error, .. } => Some(error as &(dyn Error + 'static)),
        }
    }
}

/// Whether the selector is one of the builtin contracts.
#[must_use]
pub fn is_builtin_contract_selector(selector: &str) -> bool {
    BUILTIN_CONTRACT_SELECTORS.contains(&selector)
}

/// All builtin selectors.
#[must_use]
pub fn builtin_contract_selectors() -> &'static [&'static str] {
    &BUILTIN_CONTRACT_SELECTORS
}

/// Build the codec for one builtin selector under the supplied policy.
///
/// The decode direction validates a document and forwards any supplied
/// [`ParseContext`]. The encode direction performs the same declared checks while
/// writing the value back to the same JSON shape; it is not a general-purpose
/// encoder for application models. Records with required fields compose two
/// validation branches, so the policy's collection limit must allow at least two.
pub fn builtin_contract(
    selector: &str,
    policy: ParsePolicy,
) -> Result<PureCodec<JsonValue, JsonValue>, BuiltinContractError> {
    let contract = match selector {
        SELECTOR_COMMAND_INPUT => command::command_input(policy),
        SELECTOR_VALIDATION_RESULT => command::validation_result(policy),
        SELECTOR_MUTATION_RESULT => command::mutation_result(policy),
        SELECTOR_RESOURCE_FORM_PAGE => pages::resource_form_page(policy),
        SELECTOR_RESOURCE_LIST_PAGE => pages::resource_list_page(policy),
        SELECTOR_RESOURCE_DETAIL_PAGE => pages::resource_detail_page(policy),
        SELECTOR_RESOURCE_DASHBOARD_PAGE => pages::resource_dashboard_page(policy),
        SELECTOR_SEO => shared::seo(policy),
        SELECTOR_SITEMAP_DOCUMENT => sitemap::sitemap_document(policy),
        SELECTOR_SITEMAP_INDEX => sitemap::sitemap_index(policy),
        other => return Err(BuiltinContractError::UnknownSelector(other.to_owned())),
    };
    contract.map_err(|cause| BuiltinContractError::Unavailable {
        selector: selector.to_owned(),
        cause: Box::new(cause),
    })
}

/// Validate a borrowed document with a fresh explicit traversal policy.
pub fn validate_builtin_contract(
    selector: &str,
    value: &JsonValue,
    policy: SchemaPolicy,
) -> Result<(), BuiltinContractError> {
    validate_builtin_contract_in(selector, value, &ParseContext::new(policy))
}

/// Continue validation with the caller's existing paths and traversal budgets.
pub fn validate_builtin_contract_in(
    selector: &str,
    value: &JsonValue,
    context: &ParseContext,
) -> Result<(), BuiltinContractError> {
    let contract = builtin_contract(selector, context.policy().parse)?;
    context
        .check_json(value)
        .and_then(|()| contract.decode_ref_in(value, context).map(|_| ()))
        .map_err(|issue| BuiltinContractError::invalid(selector, issue))
}

/// Parse serialized JSON and validate using one fresh traversal context.
pub fn validate_builtin_contract_serialized(
    selector: &str,
    text: &str,
    policy: SchemaPolicy,
) -> Result<(), BuiltinContractError> {
    let contract = builtin_contract(selector, policy.parse)?;
    contract
        .parse_str_in(text, &ParseContext::new(policy))
        .map(|_| ())
        .map_err(|issue| BuiltinContractError::invalid(selector, issue))
}

/// Serialize a DTO and validate its representation with the supplied policy.
pub fn validate_builtin_contract_serializable<T: Serialize>(
    selector: &str,
    value: &T,
    policy: SchemaPolicy,
) -> Result<(), BuiltinContractError> {
    let value = serde_json::to_value(value)
        .map_err(|error| BuiltinContractError::serialization(selector, error))?;
    validate_builtin_contract(selector, &value, policy)
}

/// Build a lazy configured parse that validates serialized text against a builtin
/// selector with the caller's schema policy applied to parsing and representation
/// checks.
///
/// A fresh context is created per execution from the supplied policy, the parsed
/// JSON is representation-checked, and that same context is forwarded through the
/// contract's nested codecs. The returned effect fails with the structured
/// [`SchemaIssue`] on either a representation or contract violation. Use
/// [`builtin_contract_issues`] to map that issue to the owner's validation issues,
/// or [`builtin_contract_decode_effect`] to continue an existing context after
/// parsing elsewhere.
pub fn builtin_validation_effect(
    selector: &str,
    text: impl Into<Arc<str>>,
    policy: SchemaPolicy,
) -> Result<Effect<JsonValue, SchemaIssue, ()>, BuiltinContractError> {
    let contract = builtin_contract(selector, policy.parse)?;
    let configured = contract
        .into_effect::<(), ()>()
        .configured(policy)
        .map_err(|cause| BuiltinContractError::Unavailable {
            selector: selector.to_owned(),
            cause: Box::new(cause),
        })?;
    Ok(effect_schema_parse! {
        codec: configured,
        text: text,
    })
}

/// Decode and validate an owned document while forwarding the caller's existing
/// traversal context through the contract codecs.
///
/// The context's policy also supplies this contract's construction policy, so
/// widths, issue budgets and reported paths follow the caller's state rather than
/// a fresh one. The returned effect is lazy; the caller selects the executor.
pub fn builtin_contract_decode_effect(
    selector: &str,
    value: JsonValue,
    context: ParseContext,
) -> Result<Effect<JsonValue, SchemaIssue, ()>, BuiltinContractError> {
    let policy = context.policy().parse;
    let contract = builtin_contract(selector, policy)?;
    Ok(contract
        .into_effect::<(), ()>()
        .decode_in(value, Some(context)))
}

/// Map a structured schema failure to the owner's normalized validation issues.
///
/// Nested issue groups are flattened into one issue per leaf. Each issue keeps the
/// selector-relative path (alternatives do not contribute a path segment), a typed
/// argument map for length and numeric bounds, and a readable message.
#[must_use]
pub fn builtin_contract_issues(issue: &SchemaIssue) -> Vec<ValidationIssue> {
    let mut output = Vec::new();
    collect_issues(issue, &mut output);
    if output.is_empty() {
        output.push(map_issue(issue));
    }
    output
}

fn collect_issues(issue: &SchemaIssue, output: &mut Vec<ValidationIssue>) {
    if matches!(issue.kind(), SchemaIssueKind::Multiple) && !issue.children().is_empty() {
        for child in issue.children() {
            collect_issues(child, output);
        }
        return;
    }
    output.push(map_issue(issue));
}

fn map_issue(issue: &SchemaIssue) -> ValidationIssue {
    let path = issue_path(issue);
    let (code, args, message) = issue_diagnostic(issue.kind());
    let scope = if path.is_empty() {
        VALIDATION_ISSUE_SCOPE_FORM
    } else {
        VALIDATION_ISSUE_SCOPE_FIELD
    };
    normalize(ValidationIssue {
        code,
        source: ValidationIssueSource::from(VALIDATION_ISSUE_SOURCE_CUE),
        scope: ValidationIssueScope::from(scope),
        path,
        args,
        message,
        ..ValidationIssue::default()
    })
}

fn issue_path(issue: &SchemaIssue) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(issue.path().len());
    for segment in issue.path() {
        if let Some(name) = segment.text() {
            parts.push(name.to_owned());
        } else if let IssueSegment::Index(index) = segment {
            parts.push(index.to_string());
        }
    }
    let path = parts.join(".");
    let unknown = if let SchemaIssueKind::UnknownField(name) = issue.kind() {
        Some(name.as_ref())
    } else if let SchemaIssueKind::PreparedUnknownField(name) = issue.kind() {
        Some(name.as_str())
    } else {
        None
    };
    match unknown {
        Some(name) if path.is_empty() => name.to_owned(),
        Some(name) => format!("{path}.{name}"),
        None => path,
    }
}

pub(super) type DiagnosticLengths = [Option<(&'static str, usize)>; 2];

fn issue_diagnostic(
    kind: &SchemaIssueKind,
) -> (ValidationIssueCode, BTreeMap<String, ValidationArgument>, String) {
    with_issue_diagnostic(kind, |code, message, lengths| {
        let args = lengths.into_iter().flatten()
            .map(|(name, value)| (name.to_owned(), length_argument(value)))
            .collect();
        (ValidationIssueCode::from(code), args, message.to_string())
    })
}

pub(super) fn with_issue_diagnostic<R>(
    kind: &SchemaIssueKind,
    visit: impl FnOnce(&'static str, fmt::Arguments<'_>, DiagnosticLengths) -> R,
) -> R {
    macro_rules! diagnostic {
        ($code:expr, $($message:tt)+) => {
            visit($code, format_args!($($message)+), [None, None])
        };
    }
    match kind {
        SchemaIssueKind::Memory(error) => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "validation storage refused: {error}",
        ),
        SchemaIssueKind::MissingField(name) => diagnostic!(
            VALIDATION_ISSUE_REQUIRED, "missing required field: {name}",
        ),
        SchemaIssueKind::UnknownField(name) => diagnostic!(
            VALIDATION_ISSUE_INVALID_OPTION, "unknown field: {name}",
        ),
        SchemaIssueKind::PreparedUnknownField(name) => diagnostic!(
            VALIDATION_ISSUE_INVALID_OPTION, "unknown field: {}", name.as_str(),
        ),
        SchemaIssueKind::DuplicateField(name) => diagnostic!(
            VALIDATION_ISSUE_DUPLICATE, "duplicate field: {name}",
        ),
        SchemaIssueKind::PreparedDuplicateField(name) => diagnostic!(
            VALIDATION_ISSUE_DUPLICATE, "duplicate field: {}", name.as_str(),
        ),
        SchemaIssueKind::LiteralMismatch
        | SchemaIssueKind::InvalidTag
        | SchemaIssueKind::NoAlternative
        | SchemaIssueKind::AmbiguousAlternatives { .. } => diagnostic!(
            VALIDATION_ISSUE_INVALID_OPTION, "value is not one of the permitted values",
        ),
        SchemaIssueKind::PatternMismatch => diagnostic!(
            VALIDATION_ISSUE_INVALID_FORMAT, "value does not match the required format",
        ),
        SchemaIssueKind::TypeMismatch { expected, actual } => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "expected {expected:?} but received {actual:?}",
        ),
        SchemaIssueKind::VersionMismatch { expected, actual } => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "expected version {expected} but received {actual:?}",
        ),
        SchemaIssueKind::LengthMismatch { expected, actual } => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "expected length {expected} but received {actual}",
        ),
        SchemaIssueKind::OutOfRange => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "numeric value is outside the permitted range",
        ),
        SchemaIssueKind::NonFinite => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "numeric value must be finite",
        ),
        SchemaIssueKind::InvalidValue { code } => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "invalid value: {code}",
        ),
        SchemaIssueKind::Constraint { name, operand } => with_constraint_diagnostic(name, operand, visit),
        SchemaIssueKind::JsonSyntax { detail } => diagnostic!(
            VALIDATION_ISSUE_INVALID_FORMAT, "invalid JSON: {detail}",
        ),
        SchemaIssueKind::PreparedJsonSyntax { detail } => diagnostic!(
            VALIDATION_ISSUE_INVALID_FORMAT, "invalid JSON: {}", detail.as_str(),
        ),
        SchemaIssueKind::JsonEncoding { detail } => diagnostic!(
            VALIDATION_ISSUE_INVALID_FORMAT, "value could not be encoded as JSON: {detail}",
        ),
        SchemaIssueKind::PreparedJsonEncoding { detail } => diagnostic!(
            VALIDATION_ISSUE_INVALID_FORMAT, "value could not be encoded as JSON: {}", detail.as_str(),
        ),
        SchemaIssueKind::UnsupportedRepresentation { kind } => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "unsupported representation: {kind}",
        ),
        SchemaIssueKind::LimitExceeded { resource } => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "validation limit exceeded: {resource:?}",
        ),
        SchemaIssueKind::ResourceDemand { resource, limit, observed } => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "validation demand {observed} for {resource:?} exceeds limit {limit}",
        ),
        SchemaIssueKind::PatternBudget => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "validation pattern budget exceeded",
        ),
        SchemaIssueKind::InvalidDefinition => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "contract definition is invalid",
        ),
        SchemaIssueKind::MissingValidator => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "contract validation path is unavailable",
        ),
        SchemaIssueKind::MissingWriter => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "contract encoding path is unavailable",
        ),
        SchemaIssueKind::InputConsumed => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "validation input was already consumed",
        ),
        SchemaIssueKind::MergeConflict => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "encoded values conflict",
        ),
        SchemaIssueKind::Multiple => diagnostic!(
            VALIDATION_ISSUE_SCHEMA, "multiple contract issues",
        ),
    }
}

fn with_constraint_diagnostic<R>(
    name: &str,
    operand: &str,
    visit: impl FnOnce(&'static str, fmt::Arguments<'_>, DiagnosticLengths) -> R,
) -> R {
    match name {
        "stringLength" => match string_length_limits(operand) {
            Some((minimum, _)) if minimum > 0 => visit(
                VALIDATION_ISSUE_MIN_LENGTH,
                format_args!("value must contain at least {minimum} rune(s)"),
                [Some(("min", minimum)), None],
            ),
            Some((_, maximum)) => visit(
                VALIDATION_ISSUE_MAX_LENGTH,
                format_args!("value must contain at most {maximum} rune(s)"),
                [Some(("max", maximum)), None],
            ),
            None => visit(
                VALIDATION_ISSUE_SCHEMA,
                format_args!("string length constraint failed: {operand}"),
                [None, None],
            ),
        },
        "minimum" | "exclusiveMinimum" => visit(
            VALIDATION_ISSUE_SCHEMA,
            format_args!("value must be at least {operand}"),
            [None, None],
        ),
        "maximum" | "exclusiveMaximum" => visit(
            VALIDATION_ISSUE_SCHEMA,
            format_args!("value must be at most {operand}"),
            [None, None],
        ),
        "arrayLength" | "propertyCount" => {
            let lengths = string_length_limits(operand)
                .map_or([None, None], |(minimum, maximum)| [
                    Some(("max", maximum)), Some(("min", minimum)),
                ]);
            visit(
                VALIDATION_ISSUE_SCHEMA,
                format_args!("{name} constraint failed: {operand}"),
                lengths,
            )
        }
        _ => visit(
            VALIDATION_ISSUE_SCHEMA,
            format_args!("{name} constraint failed: {operand}"),
            [None, None],
        ),
    }
}

fn string_length_limits(operand: &str) -> Option<(usize, usize)> {
    let (minimum, maximum) = operand.split_once("..=")?;
    Some((minimum.parse().ok()?, maximum.parse().ok()?))
}

fn length_argument(value: usize) -> ValidationArgument {
    match i64::try_from(value) {
        Ok(length) => ValidationArgument::Integer(length),
        Err(_) => ValidationArgument::Text(value.to_string()),
    }
}
