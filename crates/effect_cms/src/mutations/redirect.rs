//! The resource mutation redirect workflow.
//!
//! This owner binds the W7 mutation progression to the routing owner's mutation-result
//! redirect. It owns no validation, no command execution and no redirect target rule: it
//! runs the supplied mutation (or the caller's deferred attempt), applies the recovery
//! callbacks, and produces the redirect response for an unhandled outcome.
//!
//! # Sequence
//!
//! 1. A supplied attempt runs instead of the standard mutation progression; without one,
//!    the progression validates the request, runs the optional before step and invokes the
//!    supplied command executor.
//! 2. The outcome is classified and recovered through [`RecoveryOptions`]: a rejection
//!    reports its validation result to the validation-result callback, else its message to
//!    the message callback; a failure whose validation result is valid reports its cause to
//!    the command-error callback; an applied outcome is not handled.
//! 3. A handled outcome produces no redirect at all. An unhandled outcome runs the
//!    routing owner's see-other redirect for this request, resolved from the applied
//!    result when one exists and with an empty target when none does. The response is
//!    produced inside the lazy effect, so its status and location are available to the
//!    caller as the run's transport result.
//!
//! # Optional services
//!
//! A supplied attempt bypasses the standard progression, so the validation workflow and the
//! command executor are optional and unused in that case. Selecting the progression without
//! a validation service is a programming defect. An absent command service becomes a
//! defect only after validation succeeds; a refusal is recovered first.
//!
//! # Effects
//!
//! The returned description is lazy: the attempt, validators, before callback, executor,
//! recovery callbacks and the redirect response all run only when it runs. The redirect
//! reads the result's shared parameter map during that run, and its full exit is
//! preserved.

use effect::{Defect, Effect, effect, effect_die};
use effect_server::{Reply, Request};

use crate::commands::{CommandService, MutationResult};
use crate::definitions::{ResourceAction, ResourceKind};
use crate::routing::{MutationResultRoutePathBuilder, redirect_mutation_result};
use crate::validation::ActionValidationService;

use super::execution::execute_with_optional_commands;

use super::{
    Attempt, MutationFailure, RecoveryOptions,
    execute_resource_mutation_with_recovery,
};

/// The disposition of one resource mutation redirect run.
pub enum MutationRedirectOutcome<E: Send + 'static, R: Send + Sync + 'static> {
    /// The see-other redirect response for the applied mutation.
    Redirect(Reply<E, R>),
    /// A supplied recovery callback observed the outcome and no redirect is produced.
    Recovered,
}

/// Preserves the mutation/response binding for one resource mutation.
pub struct ResourceMutationRedirectOptions<E: Send + 'static, R: Send + Sync + 'static> {
    /// The resource kind.
    pub kind: ResourceKind,
    /// The requested action.
    pub action: ResourceAction,
    /// The validation workflow used by the standard progression.
    pub validation: Option<ActionValidationService<E, R>>,
    /// The supplied command executor used by the standard progression.
    pub commands: Option<CommandService<E, R>>,
    /// The language used when the result names none.
    pub fallback_language: String,
    /// The path builder used to resolve the result's redirect target.
    pub route_path: Option<MutationResultRoutePathBuilder>,
    /// An alternative deferred attempt.
    pub attempt: Option<Attempt<E, R>>,
    /// The recovery configuration.
    pub recovery: RecoveryOptions<E>,
}

impl<E: Send + 'static, R: Send + Sync + 'static> ResourceMutationRedirectOptions<E, R> {
    /// Construct the options for the standard mutation progression.
    #[must_use]
    pub fn new(
        kind: ResourceKind,
        action: ResourceAction,
        validation: ActionValidationService<E, R>,
        commands: CommandService<E, R>,
        recovery: RecoveryOptions<E>,
    ) -> Self {
        Self {
            kind,
            action,
            validation: Some(validation),
            commands: Some(commands),
            fallback_language: String::new(),
            route_path: None,
            attempt: None,
            recovery,
        }
    }

    /// Construct the options for a supplied deferred attempt.
    ///
    /// The standard progression and its services are not used.
    #[must_use]
    pub fn with_attempt(
        kind: ResourceKind,
        action: ResourceAction,
        attempt: Attempt<E, R>,
        recovery: RecoveryOptions<E>,
    ) -> Self {
        Self {
            kind,
            action,
            validation: None,
            commands: None,
            fallback_language: String::new(),
            route_path: None,
            attempt: Some(attempt),
            recovery,
        }
    }
}

/// Run one resource mutation and produce its redirect response when the outcome is
/// unhandled.
///
/// See the module documentation for the sequence and the handled/redirect split.
/// The redirect response shares the mutation failure error family.
pub fn handle_resource_mutation_redirect<E, R>(
    request: &Request,
    options: ResourceMutationRedirectOptions<E, R>,
) -> Effect<MutationRedirectOutcome<MutationFailure<E>, R>, MutationFailure<E>, R>
where
    E: std::error::Error + Send + 'static,
    R: Send + Sync + 'static,
{
    let request = request.clone();
    effect! {
        let request = request.clone();
        let mutation_request = request.clone();
        let kind = options.kind.clone();
        let action = options.action.clone();
        let attempt = options.attempt.clone();
        let recovery = options.recovery.clone();
        let fallback_language = options.fallback_language.clone();
        let route_path = options.route_path.clone();
        let validation = options.validation.clone();
        let commands = options.commands.clone();
        let (result, handled) = match attempt {
            Some(attempt) => {
                execute_resource_mutation_with_recovery::<E, R>(attempt, recovery.clone())?
            }
            None => match validation {
                Some(validation) => {
                    let outcome = execute_with_optional_commands::<E, R>(
                        &mutation_request,
                        &kind,
                        &action,
                        &validation,
                        commands.as_ref(),
                        None,
                    )
                    .either()?;
                    match outcome {
                        Ok(applied) => applied.recover(recovery.clone()),
                        Err(failure) => failure.recover(recovery),
                    }
                }
                _ => missing_standard_progress::<(Option<MutationResult>, bool), MutationFailure<E>, R>()?,
            },
        };
        if handled {
            MutationRedirectOutcome::Recovered
        } else {
            let redirect = redirect_mutation_result::<MutationFailure<E>, R>(
                result.as_ref(),
                fallback_language.as_str(),
                route_path,
            );
            let response = redirect(&request)?;
            MutationRedirectOutcome::Redirect(response)
        }
    }
}

/// The programming defect for a standard progression without its services.
fn missing_standard_progress<A, E, R>() -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    effect_die! {
        defect: Defect::new(
            "contentmgmt resource mutation redirect requires the validation workflow when no attempt is supplied",
        )
    }
}
