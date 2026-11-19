//! The W6 action-validation workflow: action support, target decision, command-input
//! parsing, list-query validation, supplied binder/validator sequencing and invalid
//! result projection.
//!
//! The workflow consumes the command owner's `CommandInput`, the query owner's
//! admitted list/detail records and query service, and this owner's binding, issue,
//! path and result contracts. All executors and data access remain consumer-owned; this
//! owner performs validation and orchestration only.
//!
//! # Sequence
//!
//! 1. The definition's declared actions decide support. `translate` is supported when
//!    widgets are declared; otherwise the action must be listed.
//! 2. A supported action builds the command input from the request: merged body/query
//!    form values, the admitted list query and the admitted detail query.
//! 3. The admitted list query is validated against the definition's filters.
//! 4. Update, delete, publish and draft require a detail target (identifier, slug or
//!    parent); an absent target is an invalid request.
//! 5. The supplied command-input validator runs with the mutable input and may update
//!    it.
//! 6. The resource/action binding is resolved **after** that validator, so a validator
//!    that changes the binding map is observed.
//! 7. The resource validator runs with the same mutable input, then the accepted
//!    result passes the supplied result validator.
//! 8. Every failure is projected as an invalid form result; a failure of the result
//!    validator stops the outcome without publishing the rejection.
//!
//! Construction performs no work. The returned [`Effect`] reads the request form,
//! admits queries and invokes validators only when it runs. An absent request yields
//! the invalid-request outcome, and an absent query service yields zero list/detail
//! queries.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use effect::memory::MemoryError;
use effect::{Effect, effect, effect_map_error, effect_ref, effect_try_future, effect_try_sync};
use effect_server::{ByteFormData, ByteFormPolicy, Request, ServerError, decode_url_encoded_pairs};
use http::Method;

use crate::commands::{
    CommandInput, CommandInputHandle, ERR_INVALID_REQUEST, ERR_UNSUPPORTED_ACTION, InputBytes,
};
use crate::definitions::{
    RESOURCE_ACTION_DELETE, RESOURCE_ACTION_DRAFT, RESOURCE_ACTION_PUBLISH,
    RESOURCE_ACTION_TRANSLATE, RESOURCE_ACTION_UPDATE, RESOURCE_FILTER_SORT, Registry,
    ResourceAction, ResourceDefinition, ResourceFilterDefinition, ResourceFilterKey, ResourceKind,
    UnknownResourceKind,
};
use crate::forms::PreparedMergedForms;
use crate::queries::{PreparedDetailLimits, PreparedListLimits, QueryService, ResourceDetailQuery, ResourceListQuery, check_request_bytes};

use super::bindings::Options;
use super::result::{
    CODE_INVALID_QUERY, CODE_INVALID_REQUEST, CODE_UNSUPPORTED_ACTION, ValidationCode,
    ValidationResult, invalid_validation_result_from_error, success_validation_result,
};

/// A validation workflow contract failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionValidationError {
    /// The resource kind is not registered.
    UnknownResource(UnknownResourceKind),
    /// A malformed or incomplete request.
    InvalidRequest(String),
    /// An action the definition does not declare.
    UnsupportedAction(ResourceAction),
    /// A list-query filter key the definition does not declare.
    UnsupportedFilter(ResourceFilterKey),
    /// A list-query value outside the declared allowed values.
    InvalidFilter(String),
}

impl ActionValidationError {
    fn invalid_request(detail: impl Into<String>) -> Self {
        Self::InvalidRequest(detail.into())
    }
}

impl fmt::Display for ActionValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownResource(error) => fmt::Display::fmt(error, formatter),
            Self::InvalidRequest(detail) => write!(formatter, "{ERR_INVALID_REQUEST}: {detail}"),
            Self::UnsupportedAction(action) => {
                write!(formatter, "{ERR_UNSUPPORTED_ACTION}: {action}")
            }
            Self::UnsupportedFilter(key) => {
                write!(formatter, "contentmgmt: unsupported filter: {key}")
            }
            Self::InvalidFilter(detail) => {
                write!(formatter, "contentmgmt: invalid filter value: {detail}")
            }
        }
    }
}

impl Error for ActionValidationError {}

/// The cause of a rejected validation outcome.
#[derive(Clone, Debug)]
pub enum ActionValidationCause<E> {
    /// A workflow contract failure.
    Contract(ActionValidationError),
    /// A consumer-supplied validator failure.
    Consumer(E),
}

impl<E> fmt::Display for ActionValidationCause<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => fmt::Display::fmt(error, formatter),
            Self::Consumer(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl<E> Error for ActionValidationCause<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error),
            Self::Consumer(error) => Some(error),
        }
    }
}

/// An accepted action input.
#[derive(Clone)]
pub struct ActionValidationAccepted {
    input: CommandInputHandle,
    result: ValidationResult,
}

impl ActionValidationAccepted {
    /// The parsed command-input identity.
    #[must_use]
    pub fn input(&self) -> CommandInputHandle {
        self.input.clone()
    }

    /// The accepted result.
    #[must_use]
    pub fn result(&self) -> &ValidationResult {
        &self.result
    }

    fn into_parts(self) -> (CommandInputHandle, ValidationResult) {
        (self.input, self.result)
    }
}

/// A rejected action input.
#[derive(Clone)]
pub struct ActionValidationRejected<E> {
    input: Option<CommandInputHandle>,
    result: ValidationResult,
    cause: ActionValidationCause<E>,
}

impl<E> ActionValidationRejected<E> {
    /// The parsed command-input identity, when parsing completed before the rejection.
    #[must_use]
    pub fn input(&self) -> Option<CommandInputHandle> {
        self.input.clone()
    }

    /// The invalid-form result.
    #[must_use]
    pub fn result(&self) -> &ValidationResult {
        &self.result
    }

    /// The rejection cause.
    #[must_use]
    pub fn cause(&self) -> &ActionValidationCause<E> {
        &self.cause
    }
}

/// An outcome stopped by the result validator.
#[derive(Clone, Debug)]
pub struct ActionValidationStopped<E> {
    result: ValidationResult,
    cause: E,
}

impl<E> ActionValidationStopped<E> {
    /// The result that failed validation.
    #[must_use]
    pub fn result(&self) -> &ValidationResult {
        &self.result
    }

    /// The result-validator failure.
    #[must_use]
    pub fn cause(&self) -> &E {
        &self.cause
    }
}

/// The closed validation outcome.
#[derive(Clone)]
pub enum ActionValidationOutcome<E> {
    /// The action input was accepted.
    Accepted(ActionValidationAccepted),
    /// The action input was rejected with an invalid-form result.
    Rejected(ActionValidationRejected<E>),
    /// The result validator stopped publication.
    Stopped(ActionValidationStopped<E>),
}

impl<E> ActionValidationOutcome<E> {
    /// The projected validation result.
    #[must_use]
    pub fn result(&self) -> &ValidationResult {
        match self {
            Self::Accepted(accepted) => accepted.result(),
            Self::Rejected(rejected) => rejected.result(),
            Self::Stopped(stopped) => stopped.result(),
        }
    }

    /// The parsed command-input identity, when one is retained by the outcome.
    #[must_use]
    pub fn input(&self) -> Option<CommandInputHandle> {
        match self {
            Self::Accepted(accepted) => Some(accepted.input()),
            Self::Rejected(rejected) => rejected.input(),
            Self::Stopped(_) => None,
        }
    }

    /// Consume the outcome into input identity, result and cause.
    #[must_use]
    pub fn into_projection(
        self,
    ) -> (
        Option<CommandInputHandle>,
        ValidationResult,
        Option<ActionValidationCause<E>>,
    ) {
        match self {
            Self::Accepted(accepted) => {
                let (input, result) = accepted.into_parts();
                (Some(input), result, None)
            }
            Self::Rejected(rejected) => (rejected.input, rejected.result, Some(rejected.cause)),
            Self::Stopped(stopped) => (
                None,
                stopped.result,
                Some(ActionValidationCause::Consumer(stopped.cause)),
            ),
        }
    }
}

/// Action validation with its report held by prepared result backing.
pub enum PreparedActionValidationOutcome<E> {
    /// Accepted input and its validation report.
    Accepted {
        input: CommandInputHandle,
        result: super::PreparedValidationResult,
    },
    /// Rejected input, report and original rejection cause.
    Rejected {
        input: Option<CommandInputHandle>,
        result: super::PreparedValidationResult,
        cause: ActionValidationCause<E>,
    },
    /// The result validator stopped publication.
    Stopped {
        result: super::PreparedValidationResult,
        cause: E,
    },
}

/// Failure before publishing a prepared action validation outcome.
#[derive(Debug)]
pub enum PreparedActionValidationError<E> {
    /// Admission or result storage refused the operation.
    Memory(MemoryError),
    /// The existing validation workflow failed.
    Validation(ActionValidationCause<E>),
}

fn prepare_action_outcome<E>(
    outcome: ActionValidationOutcome<E>,
    reservation: super::ValidationResultReservation,
) -> Result<PreparedActionValidationOutcome<E>, MemoryError> {
    Ok(match outcome {
        ActionValidationOutcome::Accepted(accepted) => {
            let (input, result) = accepted.into_parts();
            PreparedActionValidationOutcome::Accepted { input, result: reservation.publish(result)? }
        }
        ActionValidationOutcome::Rejected(rejected) => PreparedActionValidationOutcome::Rejected {
            input: rejected.input,
            result: reservation.publish(rejected.result)?,
            cause: rejected.cause,
        },
        ActionValidationOutcome::Stopped(stopped) => PreparedActionValidationOutcome::Stopped {
            result: reservation.publish(stopped.result)?,
            cause: stopped.cause,
        },
    })
}

/// The W6 action-validation service.
pub struct ActionValidationService<E: Send + 'static, R: Send + Sync + 'static> {
    resources: Arc<dyn Registry + Send + Sync>,
    queries: Option<QueryService>,
    form_policy: ByteFormPolicy,
    options: Options<CommandInput, E, R>,
    prepared_input: Option<Arc<PreparedValidationInput<E>>>,
}

pub(crate) struct PreparedValidationInput<E> {
    pub(crate) storage: Arc<PreparedMergedForms>,
    pub(crate) list: PreparedListLimits,
    pub(crate) detail: PreparedDetailLimits,
    pub(crate) on_capacity: Arc<dyn Fn(ServerError) -> E + Send + Sync>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> Clone for ActionValidationService<E, R> {
    fn clone(&self) -> Self {
        Self {
            resources: Arc::clone(&self.resources),
            queries: self.queries.clone(),
            form_policy: self.form_policy,
            options: self.options.clone(),
            prepared_input: self.prepared_input.clone(),
        }
    }
}

impl<E, R> ActionValidationService<E, R>
where
    E: Error + Send + 'static,
    R: Send + Sync + 'static,
{
    /// Construct the service.
    ///
    /// The definition registry is required. The optional query service preserves the
    /// absent-query branch: when absent, the command input carries zero queries.
    #[must_use]
    pub fn new(
        resources: Arc<dyn Registry + Send + Sync>,
        queries: Option<QueryService>,
        form_policy: ByteFormPolicy,
        options: Options<CommandInput, E, R>,
    ) -> Self {
        Self {
            resources,
            queries,
            form_policy,
            options,
            prepared_input: None,
        }
    }

    pub(crate) fn with_prepared_input(mut self, prepared: Arc<PreparedValidationInput<E>>) -> Self {
        self.prepared_input = Some(prepared);
        self
    }

    /// Validate that the definition declares an action.
    ///
    /// `translate` is accepted when widgets are declared or when the definition
    /// lists it; every other action is accepted exactly when the definition lists
    /// it.
    pub fn validate_action(
        &self,
        kind: &ResourceKind,
        action: &ResourceAction,
    ) -> Result<(), ActionValidationError> {
        let definition = self
            .resources
            .definition(kind)
            .map_err(ActionValidationError::UnknownResource)?;
        if action.as_str() == RESOURCE_ACTION_TRANSLATE && !definition.widgets.is_empty() {
            return Ok(());
        }
        if definition.actions.with(|actions| actions.contains(action)) {
            Ok(())
        } else {
            Err(ActionValidationError::UnsupportedAction(action.clone()))
        }
    }

    /// Validate an admitted list query against the definition's filters.
    pub fn validate_list_query(
        &self,
        kind: &ResourceKind,
        query: &ResourceListQuery,
    ) -> Result<(), ActionValidationError> {
        let definition = self
            .resources
            .definition(kind)
            .map_err(ActionValidationError::UnknownResource)?;
        validate_list_query_definition(&definition, query)
    }

    /// Validate a create request.
    pub fn validate_create(
        &self,
        request: Option<&Request>,
        kind: &ResourceKind,
    ) -> Effect<ActionValidationOutcome<E>, ActionValidationCause<E>, R>
    where
        R: Send + Sync + 'static,
    {
        self.validate_action_input(
            request,
            kind,
            &ResourceAction::new(crate::definitions::RESOURCE_ACTION_CREATE),
        )
    }

    /// Validate an update request.
    pub fn validate_update(
        &self,
        request: Option<&Request>,
        kind: &ResourceKind,
    ) -> Effect<ActionValidationOutcome<E>, ActionValidationCause<E>, R>
    where
        R: Send + Sync + 'static,
    {
        self.validate_action_input(request, kind, &ResourceAction::new(RESOURCE_ACTION_UPDATE))
    }

    /// Run the workflow for one action and project its outcome.
    pub fn validate_action_input(
        &self,
        request: Option<&Request>,
        kind: &ResourceKind,
        action: &ResourceAction,
    ) -> Effect<ActionValidationOutcome<E>, ActionValidationCause<E>, R>
    where
        R: Send + Sync + 'static,
    {
        let service = self.clone();
        let request = request.cloned();
        let kind = kind.clone();
        let action = action.clone();
        effect! {
            let service = service.clone();
            let request = request.clone();
            let kind = kind.clone();
            let action = action.clone();
            match service.validate_action(&kind, &action) {
                Err(error) => service.rejected(
                    None,
                    &action,
                    ValidationCode::from(CODE_UNSUPPORTED_ACTION),
                    ActionValidationCause::Contract(error),
                ),
                Ok(()) => {
                    let built = service.input_effect(request.as_ref(), &kind, &action).either()?;
                    match built {
                        Err(error @ ActionValidationCause::Consumer(_)) => effect_try_sync! {
                            once;
                            body: move || Err(error),
                        }?,
                        Err(error) => service.rejected(
                            None,
                            &action,
                            ValidationCode::from(CODE_INVALID_REQUEST),
                            error,
                        ),
                        Ok(input) => {
                            let input = CommandInputHandle::new(input);
                            service.finish_effect(input, request.as_ref(), &kind, &action)?
                        }
                    }
                }
            }
        }
    }

    /// Reserve issue, argument and byte storage before invoking action validation.
    ///
    /// The existing binder and validator contracts produce the original typed
    /// outcome. Its projection publishes into the held cell before mutation callers
    /// can consume it. Caller-owned validator payloads retain their normal ownership.
    pub fn validate_action_input_prepared(
        &self,
        request: Option<&Request>,
        kind: &ResourceKind,
        action: &ResourceAction,
        results: Arc<super::PreparedValidationResults>,
    ) -> Effect<PreparedActionValidationOutcome<E>, PreparedActionValidationError<E>, R> {
        let service = self.clone();
        let request = request.cloned();
        let kind = kind.clone();
        let action = action.clone();
        effect! {
            let service = service.clone();
            let request = request.clone();
            let kind = kind.clone();
            let action = action.clone();
            let results = results.clone();
            let reservation = effect_try_sync! {
                body { results.reserve() }, on_error: PreparedActionValidationError::Memory,
            }?;
            let outcome = effect_map_error! {
                program: service.validate_action_input(request.as_ref(), &kind, &action),
                map: PreparedActionValidationError::Validation,
            }?;
            effect_try_sync! {
                once;
                body: move || prepare_action_outcome(outcome, reservation),
                on_error: PreparedActionValidationError::Memory,
            }?
        }
    }

    fn input_effect(
        &self,
        request: Option<&Request>,
        kind: &ResourceKind,
        action: &ResourceAction,
    ) -> Effect<CommandInput, ActionValidationCause<E>, R>
    where
        R: Send + Sync + 'static,
    {
        match &self.prepared_input {
            Some(prepared) => build_prepared_command_input(
                self.queries.clone(), request, kind, action, self.form_policy, prepared.clone()
            ),
            None => effect_map_error! {
                program: build_command_input(self.queries.clone(), request, kind, action, self.form_policy),
                map: ActionValidationCause::Contract,
            },
        }
    }

    fn finish_effect(
        &self,
        input: CommandInputHandle,
        request: Option<&Request>,
        kind: &ResourceKind,
        action: &ResourceAction,
    ) -> Effect<ActionValidationOutcome<E>, ActionValidationCause<E>, R>
    where
        R: Send + Sync + 'static,
    {
        let service = self.clone();
        let input_handle = input.clone();
        let request = request.cloned();
        let kind = kind.clone();
        let action = action.clone();
        effect! {
            let service = service.clone();
            let input = input.clone();
            let input_handle = input_handle.clone();
            let request = request.clone();
            let kind = kind.clone();
            let action = action.clone();
            let guard_service = service.clone();
            let guard_input = input_handle.clone();
            let guard_kind = kind.clone();
            let guard_action = action.clone();
            let early = effect_ref! {
                in_place;
                reference: input.clone(),
                modify: move |value| guard_service.finish_in_place(value, guard_input.clone(), &guard_kind, &guard_action),
            }?;
            if let Some(outcome) = early {
                outcome
            } else if let Some(validator) = service.options.bindings.get(&kind, &action) {
                if let Some(request) = request.as_ref() {
                    let result = effect_map_error! {
                        program: validator(request, input_handle.clone()),
                        map: ActionValidationCause::Consumer,
                    }.either()?;
                    match result {
                        Ok(()) => service.accepted(input_handle, &action),
                        Err(cause) => service.rejected(Some(input_handle), &action, ValidationCode::from(CODE_INVALID_REQUEST), cause),
                    }
                } else {
                    service.rejected(Some(input_handle), &action, ValidationCode::from(CODE_INVALID_REQUEST), ActionValidationCause::Contract(ActionValidationError::invalid_request("request is nil")))
                }
            } else {
                service.accepted(input_handle, &action)
            }
        }
    }

    fn finish_in_place(
        &self,
        input: &mut CommandInput,
        input_handle: CommandInputHandle,
        kind: &ResourceKind,
        action: &ResourceAction,
    ) -> Option<ActionValidationOutcome<E>> {
        if let Err(error) = self.validate_list_query(kind, &input.list_query) {
            return Some(self.rejected(
                Some(input_handle),
                action,
                ValidationCode::from(CODE_INVALID_QUERY),
                ActionValidationCause::Contract(error),
            ));
        }
        if missing_detail_target(action, &input.detail_query) {
            return Some(self.rejected(
                Some(input_handle),
                action,
                ValidationCode::from(CODE_INVALID_REQUEST),
                ActionValidationCause::Contract(ActionValidationError::invalid_request(
                    "missing detail target",
                )),
            ));
        }
        if let Some(validate) = self.options.validators.command_input.as_ref() {
            if let Err(error) = validate(input) {
                return Some(self.rejected(
                    Some(input_handle),
                    action,
                    ValidationCode::from(CODE_INVALID_REQUEST),
                    ActionValidationCause::Consumer(error),
                ));
            }
        }

        None
    }

    fn accepted(
        &self,
        input_handle: CommandInputHandle,
        action: &ResourceAction,
    ) -> ActionValidationOutcome<E> {
        let result = success_validation_result(action.clone());
        if let Some(validate) = self.options.validators.validation_result.as_ref() {
            if let Err(error) = validate(&result) {
                return ActionValidationOutcome::Stopped(ActionValidationStopped {
                    result,
                    cause: error,
                });
            }
        }
        ActionValidationOutcome::Accepted(ActionValidationAccepted {
            input: input_handle,
            result,
        })
    }

    fn rejected(
        &self,
        input: Option<CommandInputHandle>,
        action: &ResourceAction,
        code: ValidationCode,
        cause: ActionValidationCause<E>,
    ) -> ActionValidationOutcome<E> {
        let result = match &cause {
            ActionValidationCause::Contract(error) => {
                invalid_validation_result_from_error(action.clone(), code, Some(error))
            }
            ActionValidationCause::Consumer(error) => {
                invalid_validation_result_from_error(action.clone(), code, Some(error))
            }
        };
        if let Some(validate) = self.options.validators.validation_result.as_ref() {
            if let Err(error) = validate(&result) {
                return ActionValidationOutcome::Stopped(ActionValidationStopped {
                    result,
                    cause: error,
                });
            }
        }
        ActionValidationOutcome::Rejected(ActionValidationRejected {
            input,
            result,
            cause,
        })
    }
}

fn validate_list_query_definition(
    definition: &ResourceDefinition,
    query: &ResourceListQuery,
) -> Result<(), ActionValidationError> {
    if query.page < 1 || query.page_size < 1 {
        return Err(ActionValidationError::InvalidFilter(
            "invalid pagination".to_owned(),
        ));
    }
    let filter_definitions: BTreeMap<ResourceFilterKey, ResourceFilterDefinition> = definition
        .filters
        .to_vec()
        .into_iter()
        .map(|filter| (filter.key.clone(), filter))
        .collect();

    if !query.sort.is_empty() {
        let filter = filter_definitions
            .get(&ResourceFilterKey::new(RESOURCE_FILTER_SORT))
            .cloned()
            .unwrap_or_default();
        if !query.sort.trim().is_empty()
            && !filter.allowed.is_empty()
            && !filter.allowed.with(|allowed| {
                allowed
                    .iter()
                    .any(|allowed| allowed.as_str().as_bytes() == query.sort.as_bytes())
            })
        {
            return Err(ActionValidationError::InvalidFilter(format!(
                "{}={}",
                filter.key,
                crate::byte_text::encoding_text(query.sort.as_bytes())
            )));
        }
    }
    for (key, value) in &query.filters {
        let Some(filter) = filter_definitions.get(key) else {
            return Err(ActionValidationError::UnsupportedFilter(key.clone()));
        };
        if !value.trim().is_empty()
            && !filter.allowed.is_empty()
            && !filter.allowed.with(|allowed| {
                allowed
                    .iter()
                    .any(|allowed| allowed.as_str().as_bytes() == value.as_bytes())
            })
        {
            return Err(ActionValidationError::InvalidFilter(format!(
                "{}={}",
                filter.key,
                crate::byte_text::encoding_text(value.as_bytes())
            )));
        }
    }
    for (key, values) in &query.filter_values {
        let Some(filter) = filter_definitions.get(key) else {
            return Err(ActionValidationError::UnsupportedFilter(key.clone()));
        };
        for value in values {
            let value = value.trim();
            if !value.is_empty()
                && !filter.allowed.is_empty()
                && !filter.allowed.with(|allowed| {
                    allowed
                        .iter()
                        .any(|allowed| allowed.as_str().as_bytes() == value.as_bytes())
                })
            {
                return Err(ActionValidationError::InvalidFilter(format!(
                    "{}={}",
                    filter.key,
                    crate::byte_text::encoding_text(value.as_bytes())
                )));
            }
        }
    }
    Ok(())
}

fn missing_detail_target(action: &ResourceAction, detail: &ResourceDetailQuery) -> bool {
    let target_action = action.as_str() == RESOURCE_ACTION_UPDATE
        || action.as_str() == RESOURCE_ACTION_DELETE
        || action.as_str() == RESOURCE_ACTION_PUBLISH
        || action.as_str() == RESOURCE_ACTION_DRAFT;
    target_action
        && detail.id.trim().is_empty()
        && detail.slug.trim().is_empty()
        && detail.parent.trim().is_empty()
}

/// Build a command input from one request.
///
/// The request's merged form map supplies the command form values as exact bytes
/// (body values first by byte key, then query values, first value per key, Unicode
/// whitespace trimmed at the value boundaries). The optional query service supplies
/// admitted list/detail queries; absent, both are zero values. A body or query parse
/// failure is an invalid request: a raw semicolon, malformed escape or byte/count
/// bound violation is never admitted, while valid decoded non-UTF8 bytes are data.
pub fn build_command_input<R>(
    query_service: Option<QueryService>,
    request: Option<&Request>,
    kind: &ResourceKind,
    action: &ResourceAction,
    form_policy: ByteFormPolicy,
) -> Effect<CommandInput, ActionValidationError, R>
where
    R: Send + Sync + 'static,
{
    build_command_input_from_form(query_service, request, kind, action, form_policy, None)
}

/// Build with an optional already initialized merged form owned by the caller.
/// A supplied map is authoritative, including mutations and prior partial results;
/// neither body nor query form parsing is repeated. Query admission is unchanged.
pub fn build_command_input_from_form<R>(
    query_service: Option<QueryService>,
    request: Option<&Request>,
    kind: &ResourceKind,
    action: &ResourceAction,
    form_policy: ByteFormPolicy,
    initialized_form: Option<ByteFormData>,
) -> Effect<CommandInput, ActionValidationError, R>
where
    R: Send + Sync + 'static,
{
    let initialized_form = initialized_form.map(Arc::new);
    let request = request.cloned();
    let kind = kind.clone();
    let action = action.clone();
    effect! {
        let initialized_form = initialized_form.clone();
        let request = request.clone();
        let kind = kind.clone();
        let action = action.clone();
        let query_service = query_service.clone();
        let request = match request {
            Some(request) => request,
            None => failed_value::<Request, R>("request is nil")?,
        };
        let form_request = request.clone();
        let (body, query_values) = match initialized_form.as_ref() {
            Some(form) => (Some(ByteFormData { values: form.values.clone() }), BTreeMap::new()),
            None => {
                let body_request = form_request.clone();
                let body = effect_try_future! {
                    once;
                    body: move |_| async move { read_validation_body(&body_request, form_policy).await },
                    on_error: |error| ActionValidationError::invalid_request(format!("parse form: {error}")),
                }?;
                let query_request = form_request.clone();
                let query_values = effect_try_sync! {
                    once;
                    body: move || query_form_values(&query_request),
                }?;
                (body, query_values)
            }
        };
        let list_query = match query_service.as_ref() {
            Some(service) => service.list_query::<ActionValidationError, R>(&request, &kind)?,
            None => ResourceListQuery::default(),
        };
        let detail_query = match query_service.as_ref() {
            Some(service) => service.detail_query::<ActionValidationError, R>(&request, &kind)?,
            None => ResourceDetailQuery::default(),
        };
        CommandInput {
            kind,
            action,
            list_query,
            detail_query,
            form_values: merged_form_values(body.as_ref(), &query_values),
        }
    }
}

fn build_prepared_command_input<E, R>(
    queries: Option<QueryService>,
    request: Option<&Request>,
    kind: &ResourceKind,
    action: &ResourceAction,
    form_policy: ByteFormPolicy,
    prepared: Arc<PreparedValidationInput<E>>,
) -> Effect<CommandInput, ActionValidationCause<E>, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let request = request.cloned();
    let kind = kind.clone();
    let action = action.clone();
    let capacity = prepared.on_capacity.clone();
    effect_try_future! {
        body: move |_| {
            let prepared = prepared.clone();
            let request = request.clone();
            let kind = kind.clone();
            let action = action.clone();
            let queries = queries.clone();
            async move {
                let request = request.ok_or_else(|| ServerError::Invalid("request is nil".into()))?;
                check_request_bytes(&request, prepared.list.request.min(prepared.detail.request))?;
                let form = prepared.storage.read_validation(&request, form_policy).await?;
                if let Some(diagnostic) = form.validation_diagnostic() {
                    return Err(ServerError::Invalid(diagnostic.to_string()));
                }
                let (list_query, detail_query) = match queries {
                    Some(queries) => queries.admit_prepared(
                        &request, &kind, &form, prepared.list, prepared.detail
                    )?,
                    None => (ResourceListQuery::default(), ResourceDetailQuery::default()),
                };
                let mut form_values = BTreeMap::new();
                let mut prior = None;
                for (key, value) in form.validation_pairs() {
                    if prior == Some(key) {
                        continue;
                    }
                    prior = Some(key);
                    form_values.insert(InputBytes::from(key), InputBytes::from(trim_unicode_whitespace(value)));
                }
                Ok(CommandInput { kind, action, list_query, detail_query, form_values })
            }
        },
        on_error: move |error| {
            if matches!(&error, ServerError::Memory(_) | ServerError::Limit(_)) {
                ActionValidationCause::Consumer(capacity(error))
            } else {
                ActionValidationCause::Contract(ActionValidationError::invalid_request(format!("parse form: {error}")))
            }
        },
    }
}

/// Read first values from a caller-owned initialized merged form without reparsing.
#[must_use]
pub fn form_values_from_form(form: &ByteFormData) -> BTreeMap<InputBytes, InputBytes> {
    untrimmed_form_values(Some(form), &BTreeMap::new())
}

/// Read the request's untrimmed merged form values as exact bytes.
///
/// An absent request or any body/query parse failure yields no values.
pub fn form_values<R>(
    request: Option<&Request>,
    form_policy: ByteFormPolicy,
) -> Effect<BTreeMap<InputBytes, InputBytes>, ActionValidationError, R>
where
    R: Send + Sync + 'static,
{
    let request = request.cloned();
    effect! {
        let request = request.clone();
        match request {
            None => BTreeMap::new(),
            Some(request) => {
                let body_request = request.clone();
                let body = effect_try_future! {
                    once;
                    body: move |_| async move { read_validation_body(&body_request, form_policy).await },
                    on_error: |error| ActionValidationError::invalid_request(format!("parse form: {error}")),
                }.either()?;
                match body {
                    Ok(body) => match query_form_values(&request) {
                        Ok(query_values) => untrimmed_form_values(body.as_ref(), &query_values),
                        Err(_) => BTreeMap::new(),
                    },
                    Err(_) => BTreeMap::new(),
                }
            }
        }
    }
}

fn query_form_values(
    request: &Request,
) -> Result<BTreeMap<InputBytes, Vec<InputBytes>>, ActionValidationError> {
    if request
        .uri()
        .query()
        .unwrap_or_default()
        .as_bytes()
        .split(|b| *b == b'&')
        .count()
        > 10_000
    {
        return Err(ActionValidationError::invalid_request(
            "parse form: form segments",
        ));
    }
    let decoded =
        decode_url_encoded_pairs(request.uri().query().unwrap_or_default().as_bytes(), true)
            .map_err(|error| {
                ActionValidationError::invalid_request(format!("parse form: {error}"))
            })?;
    Ok(decoded
        .into_iter()
        .map(|(key, values)| {
            (
                InputBytes::from(key),
                values.into_iter().map(InputBytes::from).collect(),
            )
        })
        .collect())
}

fn failed_value<A, R>(detail: &str) -> Effect<A, ActionValidationError, R>
where
    A: Send + 'static,
    R: Send + Sync + 'static,
{
    let error = ActionValidationError::invalid_request(detail);
    effect_try_sync! {
        once;
        body: move || {
            let value: Result<A, ActionValidationError> = Err(error);
            value
        },
    }
}

fn reads_body_form(request: &Request) -> bool {
    matches!(
        request.method(),
        &Method::POST | &Method::PUT | &Method::PATCH
    )
}

async fn read_validation_body(
    request: &Request,
    policy: ByteFormPolicy,
) -> Result<Option<ByteFormData>, ServerError> {
    if let Some(values) = request.peek_completed_form_fields() {
        return Ok(Some(values));
    }
    if !reads_body_form(request) {
        return Ok(None);
    }
    let raw = request
        .headers()
        .get(http::header::CONTENT_TYPE)
        .map_or(&b""[..], |v| v.as_bytes());
    let (urlencoded, diagnostic) = crate::form_media::form_media_type(raw);
    let values = if urlencoded {
        let body = match request
            .read_partial_byte_form_with_segment_limit(policy, std::num::NonZeroUsize::new(10_000))
            .await
        {
            Ok(body) => body,
            Err(error) => {
                return Err(match (diagnostic, &error) {
                    (_, ServerError::Limit(_)) => error,
                    (Some(diagnostic), _) => ServerError::Invalid(diagnostic.into()),
                    _ => error,
                });
            }
        };
        if let Some(error) = diagnostic {
            return Err(ServerError::Invalid(error.into()));
        }
        if let Some(error) = &body.diagnostic {
            return Err(ServerError::Invalid(format!("parse form: {error}")));
        }
        Some(ByteFormData {
            values: body.values.clone(),
        })
    } else {
        None
    };
    if let Some(error) = diagnostic {
        return Err(ServerError::Invalid(error.into()));
    }
    Ok(values)
}

fn merged_form_values(
    body: Option<&ByteFormData>,
    query: &BTreeMap<InputBytes, Vec<InputBytes>>,
) -> BTreeMap<InputBytes, InputBytes> {
    let merged = merged_values(body, query);
    merged
        .into_iter()
        .map(|(key, values)| {
            let value = values
                .first()
                .map(|value| InputBytes::from(trim_unicode_whitespace(value.as_bytes())))
                .unwrap_or_default();
            (key, value)
        })
        .collect()
}

fn untrimmed_form_values(
    body: Option<&ByteFormData>,
    query: &BTreeMap<InputBytes, Vec<InputBytes>>,
) -> BTreeMap<InputBytes, InputBytes> {
    merged_values(body, query)
        .into_iter()
        .map(|(key, values)| (key, values.first().cloned().unwrap_or_default()))
        .collect()
}

fn merged_values(
    body: Option<&ByteFormData>,
    query: &BTreeMap<InputBytes, Vec<InputBytes>>,
) -> BTreeMap<InputBytes, Vec<InputBytes>> {
    let mut values: BTreeMap<InputBytes, Vec<InputBytes>> = BTreeMap::new();
    if let Some(body) = body {
        for (key, body_values) in &body.values {
            values.insert(
                InputBytes::from(key.clone()),
                body_values.iter().cloned().map(InputBytes::from).collect(),
            );
        }
    }
    for (key, query_values) in query {
        values
            .entry(key.clone())
            .or_default()
            .extend(query_values.iter().cloned());
    }
    values
}

/// Trim Unicode whitespace at both byte boundaries without converting the data.
///
/// Only a valid text rune that is whitespace is removed. An invalid byte sequence
/// is preserved as a non-whitespace boundary.
pub(crate) fn trim_unicode_whitespace(bytes: &[u8]) -> &[u8] {
    let mut start = 0_usize;
    while let Some((character, size)) = first_text_rune(&bytes[start..]) {
        if character.is_whitespace() {
            start += size;
        } else {
            break;
        }
    }
    let mut end = bytes.len();
    while end > start {
        match last_text_rune(&bytes[start..end]) {
            Some((character, size)) if character.is_whitespace() => end -= size,
            _ => break,
        }
    }
    &bytes[start..end]
}

fn first_text_rune(bytes: &[u8]) -> Option<(char, usize)> {
    let first = *bytes.first()?;
    if first < 0x80 {
        return Some((char::from(first), 1));
    }
    for width in 2..=4 {
        if bytes.len() < width {
            break;
        }
        if let Ok(text) = std::str::from_utf8(&bytes[..width]) {
            let mut characters = text.chars();
            if let (Some(character), None) = (characters.next(), characters.next()) {
                return Some((character, width));
            }
        }
    }
    None
}

fn last_text_rune(bytes: &[u8]) -> Option<(char, usize)> {
    let last = *bytes.last()?;
    if last < 0x80 {
        return Some((char::from(last), 1));
    }
    for width in 2..=4 {
        if bytes.len() < width {
            break;
        }
        if let Ok(text) = std::str::from_utf8(&bytes[bytes.len() - width..]) {
            let mut characters = text.chars();
            if let (Some(character), None) = (characters.next(), characters.next()) {
                return Some((character, width));
            }
        }
    }
    None
}
