//! Validation: issue contracts, path bindings, results, issue localization and the
//! W6 action-validation workflow.
//!
//! Validation owns:
//!
//! - the validation issue code/source/scope vocabularies, the normalized issue
//!   record, the localized issue view model and the `ValidationError` wrapper with
//!   its issue extraction helpers;
//! - the consumer-supplied path binding catalog that maps a validated path to the
//!   form field identity and message key the presentation renders;
//! - the validation result record with the preserved invalid-form projection and
//!   its error-derived constructors;
//! - issue localization: message keys, localized messages and field labels, the
//!   validation summary and its title;
//! - the supplied binder/validator algebra: the resource/action validator map, the
//!   binder contract consumers implement, and the command-input and result validators
//!   the W6 workflow runs around the bound resource validator;
//! - the W6 workflow: action support, command-input parsing, list-query validation,
//!   detail-target decision, binding resolution and invalid/stopped/accepted outcome
//!   projection;
//! - the native builtin contract selectors and validation entry points for command,
//!   validation/mutation result, form/list/detail/dashboard page, SEO, sitemap and
//!   sitemap-index documents over the shared schema codec.
//!
//! # Capabilities and data
//!
//! Nothing here reads a database, a catalog or a global translator. Translation is
//! the caller's context capability ([`ValidationTranslation`] for issue and summary
//! messages and [`crate::localization::TranslateKeyFunc`] for field labels), the form
//! definition used for labels is supplied data, and every
//! resource/action validator is a consumer-supplied callback stored in [`Bindings`].
//! The CMS owns no validator implementation for a resource and no command input
//! type. Caller contracts are supplied as native `PureCodec` selector maps, with
//! explicit schema policies and invocation contexts. Prepared handles preserve
//! setup failures; routing selects the platform or project set before validation.
//! Typed preparation uses a supplied codec, while canonical command and result
//! document adapters preserve source field names at the JSON boundary.
//!
//! Borrowed, owned and serialized validation carry the invocation context through
//! nested schemas. Native all-of validation aggregates presence and value failures
//! under the configured error mode. Relation validation consumes the caller's
//! prepared rows; resource validation and persistence remain consumer-owned.
//!
//! # Private implementation
//!
//! `issues`, `paths`, `result`, `localization`, `bindings`, `action_validation` and
//! `contracts` are private implementation modules of this owner: they hold the
//! declarations and the mechanics, while this module is the owner's deliberate public
//! surface.

mod action_validation;
mod bindings;
mod contracts;
mod native_contracts;
mod issues;
mod localization;
mod paths;
mod prepared_result;
mod relation;
mod result;

pub(crate) use self::action_validation::trim_unicode_whitespace;
pub use self::action_validation::{
    build_command_input_from_form, form_values_from_form, ActionValidationAccepted, ActionValidationCause, ActionValidationError,
    ActionValidationOutcome, ActionValidationRejected, ActionValidationService,
    ActionValidationStopped, PreparedActionValidationError, PreparedActionValidationOutcome,
    build_command_input, form_values,
};
pub(crate) use self::action_validation::PreparedValidationInput;
pub use self::bindings::{
    ActionValidator, Binder, Bindings, CommandInputValidator, Options, ValidationResultValidator,
    Validators, new_bindings, new_bindings_from,
};
pub use self::contracts::{
    BUILTIN_CONTRACT_SELECTORS, BuiltinContractError, SELECTOR_COMMAND_INPUT,
    SELECTOR_MUTATION_RESULT, SELECTOR_RESOURCE_DASHBOARD_PAGE, SELECTOR_RESOURCE_DETAIL_PAGE,
    SELECTOR_RESOURCE_FORM_PAGE, SELECTOR_RESOURCE_LIST_PAGE, SELECTOR_SEO,
    SELECTOR_SITEMAP_DOCUMENT, SELECTOR_SITEMAP_INDEX, SELECTOR_VALIDATION_RESULT,
    builtin_contract, builtin_contract_decode_effect, builtin_contract_issues,
    builtin_contract_selectors, builtin_validation_effect, is_builtin_contract_selector,
    validate_builtin_contract, validate_builtin_contract_in, validate_builtin_contract_serializable,
    validate_builtin_contract_serialized,
};
pub use self::issues::{
    SCOPE_FIELD, SCOPE_FORM, VALIDATION_ISSUE_DUPLICATE, VALIDATION_ISSUE_INVALID_DATETIME,
    VALIDATION_ISSUE_INVALID_DURATION, VALIDATION_ISSUE_INVALID_FORMAT,
    VALIDATION_ISSUE_INVALID_OPTION, VALIDATION_ISSUE_INVALID_TIMEZONE,
    VALIDATION_ISSUE_INVALID_URL, VALIDATION_ISSUE_MAX_LENGTH, VALIDATION_ISSUE_MIN_LENGTH,
    VALIDATION_ISSUE_REQUIRED, VALIDATION_ISSUE_SCHEMA, VALIDATION_ISSUE_SCOPE_FIELD,
    VALIDATION_ISSUE_SCOPE_FORM, VALIDATION_ISSUE_SOURCE_CUE, VALIDATION_ISSUE_SOURCE_MANUAL,
    VALIDATION_ISSUE_SOURCE_SYSTEM, VALIDATION_ISSUE_UNKNOWN, ValidationArgument, ValidationError,
    ValidationIssue, ValidationIssueCode, ValidationIssueScope, ValidationIssueSource,
    ValidationIssueVM, clone_validation_issue, clone_validation_issue_vms,
    first_validation_issue_message, normalize, validation_issues_from_error,
};
pub use self::localization::{
    BackgroundValidationTranslation, VALIDATION_SUMMARY_TITLE_KEY, ValidationLocalizationOptions,
    ValidationTranslation, localize_validation_issue, localize_validation_issues,
    validation_issue_message_key, validation_summary_message, validation_summary_title,
};
pub use self::paths::{Binding, Catalog, LookupOptions};
pub use self::prepared_result::{
    PreparedValidationArgument, PreparedValidationIssue, PreparedValidationResult,
    PreparedValidationResults, ValidationResultCapacity, ValidationResultReservation,
};
pub use self::relation::{
    RelationRowInput, RelationRowIssue, RelationRowsApplicationResult,
    RelationRowsApplicationValidateFunc, RelationRowsApplicationValidator, RelationRowsCheckFunc,
    RelationRowsClaim, RelationRowsDraft, RelationRowsObservation, RelationRowsRejected,
    RelationValidationBinding, RelationValidationCause, RelationValidationContractError,
    RelationValidationInput, RelationValidationOutcome, RelationValidationService,
    ValidatedRelationRows, new_relation_validation_binding, new_relation_validation_service,
    observe_relation_rows_rejected, observe_relation_rows_validated,
};
pub use self::result::{
    CODE_INVALID_QUERY, CODE_INVALID_REQUEST, CODE_UNSUPPORTED_ACTION, CODE_VALID, ValidationCode,
    ValidationResult, invalid_validation_result, invalid_validation_result_from_error,
    invalid_validation_result_with_issues, success_validation_result,
};

pub use native_contracts::{validate_native_value, validate_native_command_input, NativeContractValidator, NativeContractHandle, NativeContractError, NativeContractDocument, route_native_contract, native_command_input_document, native_mutation_result_document, native_validation_result_document};
pub use native_contracts::{PreparedNativeContractError, PreparedNativeContractRejection};
