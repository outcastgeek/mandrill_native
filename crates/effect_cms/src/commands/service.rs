//! The command service: input/result contracts, action availability/support and
//! consumer executor invocation.
//!
//! The command service owns `CommandInput`/`MutationResult`, the error identities and
//! the option fields; the admitted query records come from the queries owner instead
//! of being redeclared.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use effect::{AtomicRef, Effect, effect, effect_map_error, effect_ref, effect_try_sync};
use effect_server::Request;

use crate::definitions::{
    RESOURCE_ACTION_TRANSLATE, Registry, ResourceAction, ResourceDefinition, ResourceKind, SharedMap,
    UnknownResourceKind,
};
use crate::queries::{ResourceDetailQuery, ResourceListQuery};

use super::actions::{ActionLabelResolver, SubmitActionVM};
use super::bindings::ResourceCommandBindings;

/// The invalid-request message.
pub const ERR_INVALID_REQUEST: &str = "contentmgmt: invalid request";

/// The unsupported-action message.
pub const ERR_UNSUPPORTED_ACTION: &str = "contentmgmt: unsupported action";

/// One owned raw input key or value.
///
/// Ordering and equality are bytewise over the owned bytes. Text access is
/// explicitly fallible; no display or lossy conversion is implied.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InputBytes(Vec<u8>);

impl PartialEq<str> for InputBytes {
    fn eq(&self, other: &str) -> bool { self.as_bytes() == other.as_bytes() }
}
impl PartialEq<&str> for InputBytes {
    fn eq(&self, other: &&str) -> bool { self.as_bytes() == other.as_bytes() }
}
impl PartialEq<String> for InputBytes {
    fn eq(&self, other: &String) -> bool { self.as_bytes() == other.as_bytes() }
}

impl<'de> serde::Deserialize<'de> for InputBytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <String as serde::Deserialize>::deserialize(deserializer).map(Self::from)
    }
}

impl serde::Serialize for InputBytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&crate::byte_text::encoding_text(self.as_bytes()))
    }
}

impl InputBytes {
    /// Own the supplied bytes.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// Trim Unicode whitespace while preserving malformed bytes.
    #[must_use]
    pub fn trim(&self) -> Self {
        Self::from(crate::validation::trim_unicode_whitespace(self.as_bytes()))
    }

    /// Borrow the exact bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Borrow the bytes as text, or report that they are not text.
    pub fn to_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.0)
    }

    /// Whether the value contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The number of owned bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Take the owned bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl From<Vec<u8>> for InputBytes {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<&[u8]> for InputBytes {
    fn from(value: &[u8]) -> Self {
        Self(value.to_vec())
    }
}

impl From<String> for InputBytes {
    fn from(value: String) -> Self {
        Self(value.into_bytes())
    }
}

impl From<&str> for InputBytes {
    fn from(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
}

impl AsRef<[u8]> for InputBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl std::borrow::Borrow<[u8]> for InputBytes {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// One validated command payload.
///
/// The list and detail queries are the query owner's admitted records, and the form
/// values preserve their supplied bytes; the request builder trims value boundaries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandInput {
    /// Resource kind.
    pub kind: ResourceKind,
    /// Requested action.
    pub action: ResourceAction,
    /// Admitted list query.
    pub list_query: ResourceListQuery,
    /// Admitted detail query.
    pub detail_query: ResourceDetailQuery,
    /// Request form values as exact bytes.
    pub form_values: BTreeMap<InputBytes, InputBytes>,
}

/// The mutable command-input identity shared through one execution.
///
/// Cloning the handle shares the same input value. Reads use [`AtomicRef::with`],
/// and mutations use [`AtomicRef::modify_in_place`], so a deferred callback can
/// observe and update the same per-run input before and after its own work.
pub type CommandInputHandle = AtomicRef<CommandInput>;

/// The observed result of an applied command.
#[derive(Clone, Debug, Default)]
pub struct MutationResult {
    /// Resource kind; an empty value is filled from the input.
    pub kind: ResourceKind,
    /// Applied action; an empty value is filled from the input.
    pub action: ResourceAction,
    /// Redirect route.
    pub redirect_route: String,
    /// Redirect language.
    pub redirect_language: String,
    /// Redirect parameters.
    ///
    /// The handle aliases its map contents through a value copy, so a result
    /// validator's map edits reach the published result while its scalar-field
    /// replacements remain local.
    pub redirect_params: SharedMap<String, String>,
}

impl PartialEq for MutationResult {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.action == other.action
            && self.redirect_route == other.redirect_route
            && self.redirect_language == other.redirect_language
            && self
                .redirect_params
                .with(|left| other.redirect_params.with(|right| left == right))
    }
}

impl Eq for MutationResult {}

/// A CMS-owned command failure.
///
/// These are typed variants so callers match them without parsing display text. A
/// consumer
/// validator/executor failure keeps its own expected error value in
/// [`CommandError::Consumer`] and is never flattened into a CMS message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandError<E> {
    /// A malformed or incomplete command request.
    InvalidRequest(String),
    /// An action the definition does not declare.
    UnsupportedAction(ResourceAction),
    /// The consumer's expected validation/execution failure.
    Consumer(E),
}

impl<E> CommandError<E> {
    /// Build an invalid-request failure with detail text.
    #[must_use]
    pub fn invalid_request(detail: impl Into<String>) -> Self {
        Self::InvalidRequest(detail.into())
    }

    /// Build an unsupported-action failure.
    #[must_use]
    pub fn unsupported_action(action: ResourceAction) -> Self {
        Self::UnsupportedAction(action)
    }

    /// The consumer's expected failure, when this is a consumer failure.
    #[must_use]
    pub fn consumer_cause(&self) -> Option<&E> {
        match self {
            Self::InvalidRequest(_) | Self::UnsupportedAction(_) => None,
            Self::Consumer(cause) => Some(cause),
        }
    }
}

impl<E> fmt::Display for CommandError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(detail) => {
                write!(formatter, "{ERR_INVALID_REQUEST}: {detail}")
            }
            Self::UnsupportedAction(action) => {
                write!(formatter, "{ERR_UNSUPPORTED_ACTION}: {action}")
            }
            Self::Consumer(cause) => fmt::Display::fmt(cause, formatter),
        }
    }
}

impl<E> std::error::Error for CommandError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Consumer(cause) => Some(cause),
            Self::InvalidRequest(_) | Self::UnsupportedAction(_) => None,
        }
    }
}

/// Validates and may update one command input.
pub type CommandInputValidator<E> =
    Arc<dyn Fn(&mut CommandInput) -> Result<(), E> + Send + Sync + 'static>;

/// Validates one mutation result through a value copy.
///
/// Scalar changes on the copy stay local. Clone the result's redirect-parameter
/// handle and mutate its contents to make those contents visible to the published
/// result.
pub type MutationResultValidator<E> =
    Arc<dyn Fn(MutationResult) -> Result<(), E> + Send + Sync + 'static>;

/// Executes one validated command through the shared input identity.
pub type Executor<E, R> = Arc<
    dyn Fn(&Request, CommandInputHandle) -> Effect<MutationResult, E, R> + Send + Sync + 'static,
>;

/// Reports whether an action is available for a resource kind.
pub trait ActionResolver {
    /// Whether the action is available.
    fn supports(&self, kind: &ResourceKind, action: &ResourceAction) -> bool;
}

/// Configures the command service.
pub struct Options<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Resolves the localized action label.
    pub resolve_action_label: Option<ActionLabelResolver>,
    /// Validates the command input before support and executor resolution.
    pub validate_command_input: Option<CommandInputValidator<E>>,
    /// Validates the observed mutation result.
    pub validate_mutation_result: Option<MutationResultValidator<E>>,
    /// The consumer-supplied executor bindings.
    pub executors: ResourceCommandBindings<E, R>,
}

impl<E, R> Default for Options<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            resolve_action_label: None,
            validate_command_input: None,
            validate_mutation_result: None,
            executors: ResourceCommandBindings::default(),
        }
    }
}

impl<E, R> Clone for Options<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            resolve_action_label: self.resolve_action_label.clone(),
            validate_command_input: self.validate_command_input.clone(),
            validate_mutation_result: self.validate_mutation_result.clone(),
            executors: self.executors.clone(),
        }
    }
}

/// The command service.
///
/// The definition registry and every consumer capability are required constructor
/// inputs; there is no absent-registry, setter or runtime lookup branch.
pub struct CommandService<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    resources: Arc<dyn Registry + Send + Sync>,
    options: Options<E, R>,
}

impl<E, R> Clone for CommandService<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            resources: Arc::clone(&self.resources),
            options: self.options.clone(),
        }
    }
}

impl<E, R> CommandService<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Construct the command service.
    #[must_use]
    pub fn new(resources: Arc<dyn Registry + Send + Sync>, options: Options<E, R>) -> Self {
        Self { resources, options }
    }

    /// Return the actions offered for a kind, in declaration order.
    ///
    /// A definition declaring widgets appends the translation action once. The
    /// operation is a lazy effect; the registry lookup and label callbacks run when
    /// the effect runs. An unknown kind carries the registry's own lookup error.
    pub fn actions(
        &self,
        request: &Request,
        kind: &ResourceKind,
    ) -> Effect<Vec<SubmitActionVM>, UnknownResourceKind, R> {
        let resources = Arc::clone(&self.resources);
        let label = self.options.resolve_action_label.clone();
        let request = request.clone();
        let kind = kind.clone();
        effect_try_sync! {
            body {
                let definition = resources.definition(&kind)?;
                Ok(resolve_available_actions(
                    &definition,
                    &request,
                    &kind,
                    label.as_ref(),
                ))
            },
        }
    }

    /// Report whether an action is available for a kind.
    ///
    /// An unknown kind supports no action. `translate` is supported exactly when
    /// the definition declares widgets; every other action is supported exactly
    /// when the definition lists it.
    #[must_use]
    pub fn supports(&self, kind: &ResourceKind, action: &ResourceAction) -> bool {
        let Ok(definition) = self.resources.definition(kind) else {
            return false;
        };
        decide_action_support(&definition, action)
    }

    /// Execute the bound executor for a validated input.
    ///
    /// An absent input is represented by `None`. The same mutable handle is used for
    /// input validation, support and executor selection (read after validation),
    /// executor invocation, and missing result-field defaults (read after the
    /// executor). An executor error stops the sequence before result validation, and
    /// no work runs when the returned description is constructed. The result
    /// validator receives a value copy whose redirect-parameter handle aliases the
    /// published result's map contents; its scalar changes remain local.
    pub fn execute(
        &self,
        request: &Request,
        input: Option<CommandInputHandle>,
    ) -> Effect<MutationResult, CommandError<E>, R> {
        let service = self.clone();
        let request = request.clone();
        effect! {
            let service = service.clone();
            let request = request.clone();
            let input = input.clone();
            let input = effect_try_sync! {
                once;
                body: move || {
                    input.ok_or_else(|| CommandError::invalid_request("nil command input"))
                },
            }?;
            let service_for_validation = service.clone();
            let validation_input = input.clone();
            let validation = effect_ref! {
                in_place;
                reference: validation_input,
                modify: move |value| {
                    match service_for_validation
                        .options
                        .validate_command_input
                        .as_ref()
                    {
                        Some(validate) => validate(value),
                        None => Ok(()),
                    }
                },
            }?;
            effect_try_sync! {
                once;
                body: move || validation.map_err(CommandError::Consumer),
            }?;
            let service_for_prepare = service.clone();
            let prepare_input = input.clone();
            let selection = prepare_input
                .with::<Result<Executor<E, R>, CommandError<E>>, CommandError<E>, R>(move |value| {
                    if !service_for_prepare.supports(&value.kind, &value.action) {
                        return Err(CommandError::unsupported_action(value.action.clone()));
                    }
                    service_for_prepare
                        .options
                        .executors
                        .resolve(&value.kind, &value.action)
                        .ok_or_else(|| {
                            if service_for_prepare
                                .options
                                .executors
                                .has_kind(&value.kind)
                            {
                                CommandError::invalid_request(format!(
                                    "missing executor for {}/{}",
                                    value.kind, value.action
                                ))
                            } else {
                                CommandError::invalid_request(format!(
                                    "missing command binding for {}",
                                    value.kind
                                ))
                            }
                        })
                })?;
            let executor = effect_try_sync! {
                once;
                body: move || selection,
            }?;
            let executor_input = input.clone();
            let mut result = effect_map_error! {
                program: executor(&request, executor_input),
                map |cause| { CommandError::Consumer(cause) },
            }?;
            let defaults_input = input.clone();
            let (kind, action) = defaults_input
                .with::<(ResourceKind, ResourceAction), CommandError<E>, R>(|value| {
                    (value.kind.clone(), value.action.clone())
                })?;
            if result.kind.as_str().is_empty() {
                result.kind = kind;
            }
            if result.action.as_str().is_empty() {
                result.action = action;
            }
            let service_for_result = service.clone();
            let result_for_validation = result.clone();
            effect_try_sync! {
                once;
                body: move || {
                    if let Some(validate) = service_for_result
                        .options
                        .validate_mutation_result
                        .as_ref()
                    {
                        validate(result_for_validation).map_err(CommandError::Consumer)?;
                    }
                    Ok(())
                },
            }?;
            result
        }
    }
}

impl<E, R> ActionResolver for CommandService<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn supports(&self, kind: &ResourceKind, action: &ResourceAction) -> bool {
        CommandService::supports(self, kind, action)
    }
}

/// Construct the command service.
#[must_use]
pub fn new_service<E, R>(
    resources: Arc<dyn Registry + Send + Sync>,
    options: Options<E, R>,
) -> CommandService<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    CommandService::new(resources, options)
}

/// Project the definition's declared actions and the conditional translation action.
fn resolve_available_actions(
    definition: &ResourceDefinition,
    request: &Request,
    kind: &ResourceKind,
    label: Option<&ActionLabelResolver>,
) -> Vec<SubmitActionVM> {
    let action_count = definition.actions.len();
    let mut out = Vec::with_capacity(action_count + 1);
    for index in 0..action_count {
        // Capture the current element after the preceding callback. Release the
        // shared-list read lock before invoking consumer code.
        let action = definition.actions.with(|actions| actions[index].clone());
        out.push(SubmitActionVM {
            label: resolve_action_label(label, request, kind, &action),
            action,
        });
    }
    if !definition.widgets.is_empty() {
        let action = ResourceAction::new(RESOURCE_ACTION_TRANSLATE);
        out.push(SubmitActionVM {
            label: resolve_action_label(label, request, kind, &action),
            action,
        });
    }
    out
}

/// Resolve one label with the action-name fallback.
fn resolve_action_label(
    label: Option<&ActionLabelResolver>,
    request: &Request,
    kind: &ResourceKind,
    action: &ResourceAction,
) -> String {
    match label {
        Some(resolve) => resolve(request, kind, action),
        None => action.as_str().to_owned(),
    }
}

/// Decide action support for one resolved definition.
fn decide_action_support(definition: &ResourceDefinition, action: &ResourceAction) -> bool {
    if action.as_str() == RESOURCE_ACTION_TRANSLATE {
        return !definition.widgets.is_empty();
    }
    definition.actions.with(|actions| actions.contains(action))
}
