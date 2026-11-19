//! The W7 resource mutation progression.
//!
//! The workflow consumes the validation owner's accepted input and result and the
//! command owner's supplied executor. It performs no persistence and owns no
//! resource validation or command implementation.
//!
//! # Sequence
//!
//! 1. The requested action selects create validation, update validation or the
//!    general action-input validation.
//! 2. A refused validation outcome stops before the before callback and the
//!    executor; an accepted outcome carries the exact command input and result
//!    forward.
//! 3. The optional before callback runs with that same mutable input identity and
//!    may update its fields. Its validation result is classified only when it
//!    reports a cause; without a cause the mutation continues regardless of the
//!    result it also returned.
//! 4. The supplied command executor runs with the same request and input identity
//!    through the command service, which reads updated fields for selection and
//!    missing-result defaults.
//! 5. The outcome is classified: a cause with a non-valid validation result is a
//!    rejection, a cause with a valid result is a failure, and no cause is an
//!    applied mutation carrying the executor's result and the accepted result.
//!
//! # Private stages
//!
//! The validated and ready stages are private values with private fields, and the
//! ready stage is consumed by the executor step. A forged or replayed stage is
//! therefore unrepresentable outside this module; no runtime stage registry or
//! one-shot restriction is added to a reusable public workflow.
//!
//! # Effects
//!
//! The public operations return lazy [`Effect`] values. Validation rejection,
//! before-callback failure and executor failure become typed [`MutationFailure`]
//! values; defects, interruption and finalization failures keep their own effect
//! channels. Construction runs no validator, callback or executor.
//!
//! [`execute_resource_mutation_with_recovery`] runs a deferred [`Attempt`] and
//! applies [`RecoveryOptions`] to its classified outcome. An applied outcome
//! returns its optional result with `false`; a rejection runs the validation-result
//! callback, else the message callback with the preserved fallback message; a
//! failure runs only the command-error callback. Recovered outcomes return no
//! result with `true`.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use effect::{
    Defect, Effect, effect, effect_die, effect_from_result, effect_map_error, effect_ref,
    effect_sync, effect_try_sync,
};
use effect_server::Request;

use crate::commands::{
    CommandError, CommandInput, CommandInputHandle, CommandService, MutationResult,
};
use crate::definitions::{
    RESOURCE_ACTION_CREATE, RESOURCE_ACTION_UPDATE, ResourceAction, ResourceKind,
};
use crate::validation::{
    ActionValidationCause, ActionValidationError, ActionValidationOutcome, ActionValidationService,
    CODE_VALID, ValidationResult,
};

/// Runs after validation and before the supplied command executor.
///
/// The callback receives the exact validated command-input identity and may update
/// it. It returns a validation result and an optional cause; a reported cause stops
/// the mutation, while a missing cause lets it continue. Both values can be present;
/// only the cause decides.
pub type BeforeExecute<E> =
    Arc<dyn Fn(&mut CommandInput) -> (ValidationResult, Option<E>) + Send + Sync + 'static>;

/// Identifies which supplied step stopped a mutation.
#[derive(Clone, Debug)]
pub enum MutationCause<E> {
    /// The validation workflow reported a refusal or stopped publication.
    Validation(ActionValidationCause<E>),
    /// The before callback reported a failure.
    Before(E),
    /// An attempt reported a failure.
    Attempt(E),
    /// The command service or one of its supplied callbacks failed.
    Command(CommandError<E>),
}

impl<E> fmt::Display for MutationCause<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(cause) => fmt::Display::fmt(cause, formatter),
            Self::Before(cause) => fmt::Display::fmt(cause, formatter),
            Self::Attempt(cause) => fmt::Display::fmt(cause, formatter),
            Self::Command(cause) => fmt::Display::fmt(cause, formatter),
        }
    }
}

impl<E> Error for MutationCause<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Validation(cause) => Some(cause),
            Self::Before(cause) => Some(cause),
            Self::Attempt(cause) => Some(cause),
            Self::Command(cause) => Some(cause),
        }
    }
}

/// The completed mutation.
///
/// The result is optional because an attempt may complete without one and still
/// report no cause; the standard workflow always supplies the executor's result.
#[derive(Clone, Debug)]
pub struct AppliedMutation {
    result: Option<MutationResult>,
    validation: ValidationResult,
}

impl AppliedMutation {
    fn new(result: Option<MutationResult>, validation: ValidationResult) -> Self {
        Self { result, validation }
    }

    /// The supplied executor's result, when one was observed.
    #[must_use]
    pub fn result(&self) -> Option<&MutationResult> {
        self.result.as_ref()
    }

    /// The accepted validation result.
    #[must_use]
    pub fn validation(&self) -> &ValidationResult {
        &self.validation
    }

    /// Consume the applied mutation into its result and accepted validation result.
    #[must_use]
    pub fn project(self) -> (Option<MutationResult>, ValidationResult) {
        (self.result, self.validation)
    }

    /// Recover an applied mutation: return its result unhandled.
    #[must_use]
    pub fn recover<E>(self, _options: RecoveryOptions<E>) -> (Option<MutationResult>, bool) {
        (self.result, false)
    }
}

/// A mutation stopped by a non-valid validation result.
#[derive(Clone)]
pub struct RejectedMutation<E> {
    input: Option<CommandInputHandle>,
    result: Option<MutationResult>,
    validation: ValidationResult,
    cause: MutationCause<E>,
}

impl<E> fmt::Debug for RejectedMutation<E>
where
    E: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RejectedMutation")
            .field("has_input", &self.input.is_some())
            .field("result", &self.result)
            .field("validation", &self.validation)
            .field("cause", &self.cause)
            .finish()
    }
}

impl<E> RejectedMutation<E> {
    fn new(
        input: Option<CommandInputHandle>,
        result: Option<MutationResult>,
        validation: ValidationResult,
        cause: MutationCause<E>,
    ) -> Self {
        Self {
            input,
            result,
            validation,
            cause,
        }
    }

    /// The command-input identity parsed before the refusal, when one exists.
    #[must_use]
    pub fn input(&self) -> Option<CommandInputHandle> {
        self.input.clone()
    }

    /// The result observed with the refusal, when one exists.
    #[must_use]
    pub fn result(&self) -> Option<&MutationResult> {
        self.result.as_ref()
    }

    /// The invalid-form result.
    #[must_use]
    pub fn validation(&self) -> &ValidationResult {
        &self.validation
    }

    /// The cause of the refusal.
    #[must_use]
    pub fn cause(&self) -> &MutationCause<E> {
        &self.cause
    }

    /// Consume the rejection into its result, validation result and cause.
    #[must_use]
    pub fn project(
        self,
    ) -> (Option<MutationResult>, ValidationResult, MutationCause<E>) {
        (self.result, self.validation, self.cause)
    }

    /// Recover a rejection with the validation callbacks.
    #[must_use]
    pub fn recover(self, options: RecoveryOptions<E>) -> (Option<MutationResult>, bool) {
        let mut validation = self.validation;
        if validation.message.trim().is_empty() {
            validation.message = options.validation_fallback.clone();
        }
        if let Some(callback) = options.on_validation_result.as_ref() {
            callback(&validation);
            return (None, true);
        }
        if let Some(callback) = options.on_validation_error.as_ref() {
            let mut message = validation.message.trim().to_owned();
            if message.is_empty() {
                message = options.validation_fallback.trim().to_owned();
            }
            callback(&message);
        }
        (None, true)
    }
}

/// A mutation stopped by a failure with a valid validation result.
#[derive(Clone, Debug)]
pub struct FailedMutation<E> {
    result: Option<MutationResult>,
    validation: ValidationResult,
    cause: MutationCause<E>,
}

impl<E> FailedMutation<E> {
    fn new(
        result: Option<MutationResult>,
        validation: ValidationResult,
        cause: MutationCause<E>,
    ) -> Self {
        Self {
            result,
            validation,
            cause,
        }
    }

    /// The result observed with the failure, when one exists.
    #[must_use]
    pub fn result(&self) -> Option<&MutationResult> {
        self.result.as_ref()
    }

    /// The validation result observed at the failure.
    #[must_use]
    pub fn validation(&self) -> &ValidationResult {
        &self.validation
    }

    /// The cause of the failure.
    #[must_use]
    pub fn cause(&self) -> &MutationCause<E> {
        &self.cause
    }

    /// Consume the failure into its result, validation result and cause.
    #[must_use]
    pub fn project(
        self,
    ) -> (Option<MutationResult>, ValidationResult, MutationCause<E>) {
        (self.result, self.validation, self.cause)
    }

    /// Recover a failure through the command-error callback.
    #[must_use]
    pub fn recover(self, options: RecoveryOptions<E>) -> (Option<MutationResult>, bool) {
        if let Some(callback) = options.on_command_error.as_ref() {
            callback(&self.cause);
        }
        (None, true)
    }
}

/// The classified failure of one mutation.
#[derive(Clone, Debug)]
pub enum MutationFailure<E> {
    /// The validation result marks a refused action input.
    Rejected(RejectedMutation<E>),
    /// The validation result is valid, so the supplied step failed.
    Failed(FailedMutation<E>),
}

impl<E> MutationFailure<E> {
    /// The validation result carried by the failure.
    #[must_use]
    pub fn validation(&self) -> &ValidationResult {
        match self {
            Self::Rejected(rejected) => rejected.validation(),
            Self::Failed(failed) => failed.validation(),
        }
    }

    /// The cause carried by the failure.
    #[must_use]
    pub fn cause(&self) -> &MutationCause<E> {
        match self {
            Self::Rejected(rejected) => rejected.cause(),
            Self::Failed(failed) => failed.cause(),
        }
    }

    /// The result observed with the failure, when one exists.
    #[must_use]
    pub fn result(&self) -> Option<&MutationResult> {
        match self {
            Self::Rejected(rejected) => rejected.result(),
            Self::Failed(failed) => failed.result(),
        }
    }

    /// The command-input identity, when the rejection retains one.
    #[must_use]
    pub fn input(&self) -> Option<CommandInputHandle> {
        match self {
            Self::Rejected(rejected) => rejected.input(),
            Self::Failed(_) => None,
        }
    }

    /// Whether the failure is a validation rejection.
    #[must_use]
    pub fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected(_))
    }

    /// Whether the failure is a step failure with a valid validation result.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    /// Consume the failure into its result, validation result and cause.
    #[must_use]
    pub fn project(
        self,
    ) -> (Option<MutationResult>, ValidationResult, MutationCause<E>) {
        match self {
            Self::Rejected(rejected) => rejected.project(),
            Self::Failed(failed) => failed.project(),
        }
    }

    /// Recover the failure through the branch-appropriate callback.
    #[must_use]
    pub fn recover(self, options: RecoveryOptions<E>) -> (Option<MutationResult>, bool) {
        match self {
            Self::Rejected(rejected) => rejected.recover(options),
            Self::Failed(failed) => failed.recover(options),
        }
    }
}

impl<E> fmt::Display for MutationFailure<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.cause(), formatter)
    }
}

impl<E> Error for MutationFailure<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause())
    }
}

/// The raw projection of one executed attempt.
///
/// The result and cause are independent: an attempt may complete without a result,
/// or may report a result together with a cause. Classification retains both.
#[derive(Clone, Debug)]
pub struct AttemptedMutation<E> {
    result: Option<MutationResult>,
    validation: ValidationResult,
    cause: Option<MutationCause<E>>,
}

impl<E> AttemptedMutation<E> {
    /// Build the projection of one attempt.
    #[must_use]
    pub fn new(
        result: Option<MutationResult>,
        validation: ValidationResult,
        cause: Option<MutationCause<E>>,
    ) -> Self {
        Self {
            result,
            validation,
            cause,
        }
    }

    /// Project an applied mutation into an attempt projection.
    #[must_use]
    pub fn from_applied(applied: AppliedMutation) -> Self {
        let (result, validation) = applied.project();
        Self {
            result,
            validation,
            cause: None,
        }
    }

    /// Project a classified failure into an attempt projection.
    #[must_use]
    pub fn from_failure(failure: MutationFailure<E>) -> Self {
        let (result, validation, cause) = failure.project();
        Self {
            result,
            validation,
            cause: Some(cause),
        }
    }

    /// The result observed by the attempt, when one exists.
    #[must_use]
    pub fn result(&self) -> Option<&MutationResult> {
        self.result.as_ref()
    }

    /// The validation result observed by the attempt.
    #[must_use]
    pub fn validation(&self) -> &ValidationResult {
        &self.validation
    }

    /// The cause observed by the attempt, when one exists.
    #[must_use]
    pub fn cause(&self) -> Option<&MutationCause<E>> {
        self.cause.as_ref()
    }

    /// Consume the projection into its result, validation result and cause.
    #[must_use]
    pub fn project(
        self,
    ) -> (Option<MutationResult>, ValidationResult, Option<MutationCause<E>>) {
        (self.result, self.validation, self.cause)
    }
}

/// One deferred mutation attempt.
///
/// The attempt runs only when the executing effect runs and may be reused across
/// runs. An observed step failure belongs in the returned projection; a classified
/// failure may instead use the effect's error channel, and recovery handles either
/// form without losing its result or validation result.
pub type Attempt<E, R> = Arc<
    dyn Fn() -> Effect<AttemptedMutation<E>, MutationFailure<E>, R> + Send + Sync + 'static,
>;

/// Configures mutation-outcome recovery.
pub struct RecoveryOptions<E> {
    /// Message used when a rejection has no message.
    pub validation_fallback: String,
    /// Receives the rejection message when no validation-result callback is set.
    pub on_validation_error: Option<Arc<dyn Fn(&str) + Send + Sync + 'static>>,
    /// Receives the rejection result; takes precedence over the message callback.
    pub on_validation_result: Option<Arc<dyn Fn(&ValidationResult) + Send + Sync + 'static>>,
    /// Receives the cause of a failure whose validation result is valid.
    pub on_command_error: Option<Arc<dyn Fn(&MutationCause<E>) + Send + Sync + 'static>>,
}

impl<E> Default for RecoveryOptions<E> {
    fn default() -> Self {
        Self {
            validation_fallback: String::new(),
            on_validation_error: None,
            on_validation_result: None,
            on_command_error: None,
        }
    }
}

impl<E> Clone for RecoveryOptions<E> {
    fn clone(&self) -> Self {
        Self {
            validation_fallback: self.validation_fallback.clone(),
            on_validation_error: self.on_validation_error.clone(),
            on_validation_result: self.on_validation_result.clone(),
            on_command_error: self.on_command_error.clone(),
        }
    }
}

/// The exact request, command-input identity and accepted validation result carried
/// by the private validated stage.
#[derive(Clone)]
struct ValidatedMutation {
    request: Request,
    input: CommandInputHandle,
    validation: ValidationResult,
}

/// The private ready stage; only the before step constructs it.
struct ReadyMutation {
    validated: ValidatedMutation,
}

/// Run one resource mutation.
///
/// This is [`execute_resource_mutation_with_before`] without a before callback.
pub fn execute_resource_mutation<E, R>(
    request: &Request,
    kind: &ResourceKind,
    action: &ResourceAction,
    validation: &ActionValidationService<E, R>,
    commands: &CommandService<E, R>,
) -> Effect<AppliedMutation, MutationFailure<E>, R>
where
    E: Error + Send + 'static,
    R: Send + Sync + 'static,
{
    execute_resource_mutation_with_before(request, kind, action, validation, commands, None)
}

/// Run one resource mutation with an optional before callback.
///
/// The returned description is lazy: validation, the before callback and the
/// supplied executor run only when it runs. See the module documentation for the
/// sequence and classification.
pub fn execute_resource_mutation_with_before<E, R>(
    request: &Request,
    kind: &ResourceKind,
    action: &ResourceAction,
    validation: &ActionValidationService<E, R>,
    commands: &CommandService<E, R>,
    before: Option<BeforeExecute<E>>,
) -> Effect<AppliedMutation, MutationFailure<E>, R>
where
    E: Error + Send + 'static,
    R: Send + Sync + 'static,
{
    execute_with_optional_commands(request, kind, action, validation, Some(commands), before)
}

/// Redirect compatibility: an absent command service is observed only if reached.
pub(crate) fn execute_with_optional_commands<E, R>(
    request: &Request,
    kind: &ResourceKind,
    action: &ResourceAction,
    validation: &ActionValidationService<E, R>,
    commands: Option<&CommandService<E, R>>,
    before: Option<BeforeExecute<E>>,
) -> Effect<AppliedMutation, MutationFailure<E>, R>
where
    E: Error + Send + 'static,
    R: Send + Sync + 'static,
{
    let admission_identity = (kind, action);
    let request = request.clone();
    let kind = kind.clone();
    let action = action.clone();
    let validation = validation.clone();
    let commands = commands.cloned();
    let admission = commands.clone();
    let admission_request = request.clone();
    let program = effect! {
        let request = request.clone();
        let kind = kind.clone();
        let action = action.clone();
        let validation = validation.clone();
        let commands = commands.clone();
        let before = before.clone();
        let validation_effect = match action.as_str() {
            RESOURCE_ACTION_CREATE => validation.validate_create(Some(&request), &kind),
            RESOURCE_ACTION_UPDATE => validation.validate_update(Some(&request), &kind),
            _ => validation.validate_action_input(Some(&request), &kind, &action),
        };
        let validation_attempt = effect_map_error! {
            program: effect! { either; program: validation_effect, },
            map |error| {
                classify_failure(
                    None,
                    None,
                    ValidationResult::default(),
                    MutationCause::Validation(error),
                )
            },
        }?;
        let staged = effect_try_sync! {
            once;
            body: move || stage_validation(validation_attempt, request.clone()),
        }?;
        if before.is_some()
            && let Some(preflight) = commands.as_ref().and_then(|commands| commands.preflight_input(&staged.input))
        {
            let validation = staged.validation.clone();
            effect_map_error! {
                program: preflight,
                map: move |cause| classify_failure(None, None, validation.clone(), MutationCause::Command(cause)),
            }?;
        }
        let ready = run_before(staged, before.clone())?;
        let validation = ready.validated.validation.clone();
        let commands = match commands {
            Some(commands) => commands,
            None => effect_die! {
                defect: Defect::new("contentmgmt resource mutation requires a command executor after validation"),
            }?,
        };
        let executed = effect_map_error! {
            program: commands.execute_admitted(
                &ready.validated.request,
                Some(ready.validated.input.clone()),
            ),
            map move |cause| {
                classify_failure(
                    None,
                    None,
                    validation.clone(),
                    MutationCause::Command(cause),
                )
            },
        }?;
        AppliedMutation::new(Some(executed), ready.validated.validation)
    };
    if let Some(commands) = admission {
        return commands.with_work(&admission_request, Some(admission_identity), program, |cause| {
            classify_failure(None, None, ValidationResult::default(), MutationCause::Command(cause))
        });
    }
    program
}

/// Run one deferred attempt and recover its classified outcome.
///
/// The attempt runs only when the returned description runs and may be reused
/// across runs. An attempt's expected failure is classified and recovered; a
/// classified failure in its error channel is recovered directly. Defects,
/// interruption and finalization failures keep their own channels. The result is
/// returned only for an applied outcome, and `false` marks that unhandled case.
pub fn execute_resource_mutation_with_recovery<E, R>(
    attempt: Attempt<E, R>,
    options: RecoveryOptions<E>,
) -> Effect<(Option<MutationResult>, bool), MutationFailure<E>, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    effect! {
        let attempt = attempt.clone();
        let options = options.clone();
        let attempted = effect! { either; program: attempt(), }?;
        recover_mutation_outcome(attempted.and_then(classify_attempt), options)
    }
}

pub(super) fn recover_mutation_outcome<E>(
    outcome: Result<AppliedMutation, MutationFailure<E>>,
    options: RecoveryOptions<E>,
) -> (Option<MutationResult>, bool) {
    match outcome {
        Ok(applied) => applied.recover(options),
        Err(failure) => failure.recover(options),
    }
}

/// Classify a validation outcome and retain its input when it was accepted.
fn stage_validation<E>(
    attempt: Result<ActionValidationOutcome<E>, ActionValidationCause<E>>,
    request: Request,
) -> Result<ValidatedMutation, MutationFailure<E>>
where
    E: Error + Send + 'static,
{
    let outcome = match attempt {
        Ok(outcome) => outcome,
        Err(error) => {
            return Err(classify_failure(
                None,
                None,
                ValidationResult::default(),
                MutationCause::Validation(error),
            ));
        }
    };
    let (input, validation, cause) = outcome.into_projection();
    if let Some(cause) = cause {
        return Err(classify_failure(
            input,
            None,
            validation,
            MutationCause::Validation(cause),
        ));
    }
    let Some(input) = input else {
        return Err(MutationFailure::Failed(FailedMutation::new(
            None,
            validation,
            MutationCause::Validation(ActionValidationCause::Contract(
                ActionValidationError::InvalidRequest(
                    "accepted validation input is absent".to_owned(),
                ),
            )),
        )));
    };
    Ok(ValidatedMutation {
        request,
        input,
        validation,
    })
}

/// Run the optional before callback and construct the ready stage.
fn run_before<E, R>(
    validated: ValidatedMutation,
    before: Option<BeforeExecute<E>>,
) -> Effect<ReadyMutation, MutationFailure<E>, R>
where
    E: Error + Send + 'static,
    R: Send + Sync + 'static,
{
    let Some(before) = before else {
        return effect_sync! {
            once;
            body: move || ReadyMutation { validated },
        };
    };
    effect! {
        let validated = validated.clone();
        let before = before.clone();
        let before_input = validated.input.clone();
        let observation = effect_ref! {
            in_place;
            reference: before_input,
            modify: move |value| before(value),
        }?;
        let checked: Result<(), MutationFailure<E>> = match observation.1 {
            None => Ok(()),
            Some(cause) => Err(classify_failure(
                None,
                None,
                observation.0,
                MutationCause::Before(cause),
            )),
        };
        effect_from_result! { once; result: checked, }?;
        ReadyMutation { validated }
    }
}

/// Classify one stopped step by the validation result it carries.
fn classify_failure<E>(
    input: Option<CommandInputHandle>,
    result: Option<MutationResult>,
    validation: ValidationResult,
    cause: MutationCause<E>,
) -> MutationFailure<E> {
    if validation.code.as_str() == CODE_VALID {
        MutationFailure::Failed(FailedMutation::new(result, validation, cause))
    } else {
        MutationFailure::Rejected(RejectedMutation::new(input, result, validation, cause))
    }
}

/// Classify an attempt projection by its cause and validation result.
fn classify_attempt<E>(
    attempted: AttemptedMutation<E>,
) -> Result<AppliedMutation, MutationFailure<E>> {
    let (result, validation, cause) = attempted.project();
    match cause {
        None => Ok(AppliedMutation::new(result, validation)),
        Some(cause) => Err(classify_failure(None, result, validation, cause)),
    }
}
