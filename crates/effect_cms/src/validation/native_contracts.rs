//! Native caller contracts and typed DTO preparation. Consumers supply codecs;
//! this owner routes selectors and reports issues without a foreign schema runtime.
use super::{
    BUILTIN_CONTRACT_SELECTORS, BuiltinContractError, CODE_INVALID_REQUEST,
    PreparedValidationResult, PreparedValidationResults, ValidationArgument, ValidationResult,
    builtin_contract, is_builtin_contract_selector,
};
use effect::memory::MemoryError;
use effect::{
    Effect, JsonValue, ParseContext, PureCodec, SchemaIssue, SchemaPolicy, effect_try_sync,
};
use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};
use std::num::NonZeroU8;

/// One immutable native selector set, prepared by its owning application.
#[derive(Clone, Default)]
pub struct NativeContractValidator {
    contracts: BTreeMap<String, PureCodec<JsonValue, JsonValue>>,
}
impl NativeContractValidator {
    /// Supply caller-native contracts without interpreting filesystem schema text.
    pub fn new(contracts: BTreeMap<String, PureCodec<JsonValue, JsonValue>>) -> Self {
        Self { contracts }
    }
    /// Prepare the builtin set under the application's explicit construction policy.
    pub fn builtin(policy: effect::ParsePolicy) -> Result<Self, BuiltinContractError> {
        BUILTIN_CONTRACT_SELECTORS
            .iter()
            .map(|selector| {
                builtin_contract(selector, policy).map(|codec| ((*selector).to_owned(), codec))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(Self::new)
    }
    /// Validate a document with shared invocation paths and budgets.
    pub fn validate(
        &self,
        selector: &str,
        document: &JsonValue,
        context: &ParseContext,
    ) -> Result<(), NativeContractError> {
        let codec = self
            .contracts
            .get(selector)
            .ok_or_else(|| NativeContractError::MissingSelector(selector.to_owned()))?;
        context
            .check_json(document)
            .and_then(|()| codec.decode_ref_in(document, context).map(|_| ()))
            .map_err(|issue| {
                NativeContractError::Contract(BuiltinContractError::invalid(selector, issue))
            })
    }

    /// Validate a borrowed document with prepared issue backing and publication.
    /// A rejected document retains both the original schema issue and the prepared
    /// normalized projection. The caller owns the supplied document representation.
    pub fn validate_prepared(
        &self,
        selector: &str,
        document: &JsonValue,
        context: &ParseContext,
        outputs: &PreparedValidationResults,
        group_depth: NonZeroU8,
    ) -> Result<Option<PreparedNativeContractRejection>, PreparedNativeContractError> {
        let codec = self.contracts.get(selector)
            .ok_or(PreparedNativeContractError::MissingSelector)?;
        if context.issue_memory().is_none() {
            return Err(PreparedNativeContractError::Memory(MemoryError::UnsupportedRunContract));
        }
        let output = outputs.reserve().map_err(PreparedNativeContractError::Memory)?;
        let validated = context.check_json(document)
            .and_then(|()| codec.decode_ref_in(document, context).map(|_| ()));
        match validated {
            Ok(()) => Ok(None),
            Err(cause) => {
                let report = output.publish_schema_rejection("", CODE_INVALID_REQUEST, &cause, group_depth)
                    .map_err(PreparedNativeContractError::Memory)?;
                Ok(Some(PreparedNativeContractRejection { cause, report }))
            }
        }
    }
    /// Parse serialized input and validate using one supplied context.
    pub fn validate_serialized(
        &self,
        selector: &str,
        text: &str,
        context: &ParseContext,
    ) -> Result<(), NativeContractError> {
        let codec = self
            .contracts
            .get(selector)
            .ok_or_else(|| NativeContractError::MissingSelector(selector.to_owned()))?;
        codec
            .parse_str_in(text, context)
            .map(|_| ())
            .map_err(|issue| {
                NativeContractError::Contract(BuiltinContractError::invalid(selector, issue))
            })
    }
    /// Prepare a typed value through its declared native codec and validate it.
    pub fn validate_typed<A: Send + 'static>(
        &self,
        selector: &str,
        value: &A,
        preparation: &NativeContractDocument<A>,
        context: &ParseContext,
    ) -> Result<(), NativeContractError> {
        let document = preparation
            .codec
            .encode_ref_in(value, context)
            .map_err(|issue| {
                NativeContractError::Contract(BuiltinContractError::invalid(selector, issue))
            })?;
        self.validate(selector, &document, context)
    }
    /// Build a reusable lazy validation; each execution has fresh counters.
    pub fn validate_effect<R: Send + Sync + 'static>(
        &self,
        selector: String,
        document: JsonValue,
        policy: SchemaPolicy,
    ) -> Effect<(), NativeContractError, R> {
        let validator = self.clone();
        effect_try_sync! { body: move || validator.validate(&selector, &document, &ParseContext::new(policy)), }
    }
}

/// A rejected native contract with its original schema cause and bounded CMS report.
pub struct PreparedNativeContractRejection {
    pub cause: SchemaIssue,
    pub report: PreparedValidationResult,
}

/// Failure to resolve or run the prepared native contract.
#[derive(Debug)]
pub enum PreparedNativeContractError {
    MissingSelector,
    Memory(MemoryError),
}

impl fmt::Display for PreparedNativeContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSelector => formatter.write_str("validate schema: missing selector"),
            Self::Memory(error) => error.fmt(formatter),
        }
    }
}

impl Error for PreparedNativeContractError {}
/// Typed preparation uses the application's real codec, not reflection or guessed fields.
pub struct NativeContractDocument<A: Send + 'static> {
    codec: PureCodec<A, JsonValue>,
}
impl<A: Send + 'static> NativeContractDocument<A> {
    pub fn new(codec: PureCodec<A, JsonValue>) -> Self {
        Self { codec }
    }
}
/// A prepared validator and its original setup error. Error takes precedence.
#[derive(Clone, Default)]
pub struct NativeContractHandle {
    pub validator: Option<Arc<NativeContractValidator>>,
    pub error: Option<Arc<dyn Error + Send + Sync>>,
}
#[derive(Debug)]
pub enum NativeContractError {
    Setup(Arc<dyn Error + Send + Sync>),
    MissingInput,
    MissingValidator(String),
    MissingSelector(String),
    Contract(BuiltinContractError),
}
impl fmt::Display for NativeContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInput => f.write_str("validate command input: nil input"),
            Self::Setup(error) => error.fmt(f),
            Self::MissingValidator(selector) => {
                write!(f, "validate contract {selector}: validator unavailable")
            }
            Self::MissingSelector(selector) => {
                write!(f, "validate schema: missing selector {selector}")
            }
            Self::Contract(error) => error.fmt(f),
        }
    }
}
impl Error for NativeContractError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Setup(error) => Some(error.as_ref()),
            Self::Contract(error) => Some(error),
            _ => None,
        }
    }
}
impl NativeContractError {
    pub fn issues(&self) -> Vec<super::ValidationIssue> {
        if let Self::Contract(error) = self {
            return error.issues().to_vec();
        }
        vec![super::ValidationIssue {
            code: super::VALIDATION_ISSUE_SCHEMA.into(),
            source: super::VALIDATION_ISSUE_SOURCE_CUE.into(),
            scope: super::VALIDATION_ISSUE_SCOPE_FORM.into(),
            message: self.to_string().trim().to_owned(),
            ..Default::default()
        }]
    }
}
/// Select platform/project by the supplied predicate (builtin selectors by default),
/// then preserve setup failure before checking availability or validating.
pub fn route_native_contract(
    selector: &str,
    document: &JsonValue,
    platform: &NativeContractHandle,
    project: &NativeContractHandle,
    is_platform: Option<&dyn Fn(&str) -> bool>,
    context: &ParseContext,
) -> Result<(), NativeContractError> {
    let platform_selected = is_platform.map_or_else(
        || is_builtin_contract_selector(selector),
        |select| select(selector),
    );
    let handle = if platform_selected { platform } else { project };
    if let Some(error) = &handle.error {
        return Err(NativeContractError::Setup(error.clone()));
    }
    let validator = handle
        .validator
        .as_ref()
        .ok_or_else(|| NativeContractError::MissingValidator(selector.to_owned()))?;
    validator.validate(selector, document, context)
}
/// An omitted optional validator accepts the supplied document.
pub fn validate_native_value(
    validator: Option<&NativeContractValidator>,
    selector: &str,
    document: &JsonValue,
    context: &ParseContext,
) -> Result<(), NativeContractError> {
    match validator {
        Some(validator) => validator.validate(selector, document, context),
        None => Ok(()),
    }
}
/// Missing command input fails even when the optional validator is absent.
pub fn validate_native_command_input(
    validator: Option<&NativeContractValidator>,
    input: Option<&crate::commands::CommandInput>,
    context: &ParseContext,
) -> Result<(), NativeContractError> {
    let input = input.ok_or(NativeContractError::MissingInput)?;
    if let Some(validator) = validator {
        validator.validate(
            super::SELECTOR_COMMAND_INPUT,
            &native_command_input_document(input),
            context,
        )?;
    }
    Ok(())
}
/// Canonical command-input DTO fields; submitted bytes convert only at JSON encoding.
pub fn native_command_input_document(value: &crate::commands::CommandInput) -> JsonValue {
    let list = &value.list_query;
    let detail = &value.detail_query;
    let filters: BTreeMap<_, _> = list
        .filters
        .iter()
        .map(|(key, value)| (key.as_str(), value))
        .collect();
    let filter_values: BTreeMap<_, _> = list
        .filter_values
        .iter()
        .map(|(key, value)| (key.as_str(), value))
        .collect();
    serde_json::json!({"Kind":value.kind.as_str(),"Action":value.action.as_str(),"ListQuery":{"Page":list.page,"PageSize":list.page_size,"Search":list.search,"Sort":list.sort,"Filters":filters,"FilterValues":filter_values},"DetailQuery":{"ID":detail.id,"Slug":detail.slug,"Parent":detail.parent,"Locale":detail.locale},"FormValues":value.form_values})
}
/// Canonical mutation result document, reading the shared redirect map at invocation.
pub fn native_mutation_result_document(value: &crate::commands::MutationResult) -> JsonValue {
    value.redirect_params.with(|params| serde_json::json!({"Kind":value.kind.as_str(),"Action":value.action.as_str(),"RedirectRoute":value.redirect_route,"RedirectLanguage":value.redirect_language,"RedirectParams":params}))
}
/// Canonical validation result document. Non-finite issue arguments are encoding errors.
pub fn native_validation_result_document(
    value: &ValidationResult,
) -> Result<JsonValue, SchemaIssue> {
    let mut issues = Vec::with_capacity(value.issues.len());
    for issue in &value.issues {
        let mut args = BTreeMap::new();
        for (key, value) in &issue.args {
            let value = match value {
                ValidationArgument::Integer(value) => serde_json::json!(value),
                ValidationArgument::Text(value) => serde_json::json!(value),
                ValidationArgument::Number(value) => {
                    if !value.is_finite() {
                        return Err(SchemaIssue::new(effect::SchemaIssueKind::JsonEncoding {
                            detail: "non-finite validation argument".into(),
                        }));
                    }
                    serde_json::json!(value)
                }
            };
            args.insert(key, value);
        }
        issues.push(serde_json::json!({"Code":issue.code.as_str(),"Source":issue.source.as_str(),"Scope":issue.scope.as_str(),"Path":issue.path,"Field":issue.field,"FieldID":issue.field_id,"FieldKey":issue.field_key,"SectionKey":issue.section_key,"MessageKey":issue.message_key,"Args":args,"Message":issue.message}));
    }
    Ok(
        serde_json::json!({"Action":value.action.as_str(),"Code":value.code.as_str(),"Message":value.message,"Issues":issues,"SubmitDisabled":value.submit_disabled,"RequiresRehydrate":value.requires_rehydrate}),
    )
}
