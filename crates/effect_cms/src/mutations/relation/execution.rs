//! The relation-mutation progression.
//!
//! The owner keeps this order: validate the service/input identity, consume
//! the owner-issued validation claim once, run the before hook, classify its
//! observation, then run the bound executor and classify its observation. The
//! request/ready stages are private values, so a forged or replayed stage is
//! unrepresentable through the public contract. Expected callback failures become
//! classified outcomes with their consumer causes; cancellation at the owner
//! checkpoints takes precedence. Defects, interruption and finalization failures
//! keep their own effect channels.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use effect::{Effect, effect, effect_sync};

use crate::validation::{RelationRowsClaim, RelationRowsDraft};
use crate::validation::RelationRowIssue;

use super::bindings::{
    RelationMutationBindingValue, RelationMutationInput, RelationMutationService,
};
use super::observation::{
    RelationBeforeObservationValue, RelationMutationApplied, RelationMutationCause,
    RelationMutationFailed, RelationMutationObservation, RelationMutationObservationValue,
    RelationMutationOutcome,
    RelationMutationRejected, RelationMutationRequest, RelationMutationUnresolved,
};

/// One private execution owner for a validated relation mutation.
pub(super) struct RelationMutationExecution {
    pub(super) before_started: AtomicBool,
    pub(super) execute_started: AtomicBool,
}

/// The private readiness shared by the request and the applied publication.
pub(super) struct RelationMutationReadiness {
    pub(super) claim: RelationRowsClaim,
    pub(super) execution: Arc<RelationMutationExecution>,
}

/// A prepared mutation that has not run its before hook.
struct PreparedMutation<E: Send + 'static, R: Send + Sync + 'static> {
    readiness: Arc<RelationMutationReadiness>,
    binding: Arc<RelationMutationBindingValue<E, R>>,
}

/// A mutation whose before hook accepted the draft.
struct ReadyMutation<E: Send + 'static, R: Send + Sync + 'static> {
    readiness: Arc<RelationMutationReadiness>,
    binding: Arc<RelationMutationBindingValue<E, R>>,
}

impl<E, R> RelationMutationService<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Execute one relation mutation and publish its closed outcome.
    ///
    /// The returned effect is lazy. An unconstructed zero service publishes the zero
    /// outcome (`None`) and invokes no callback.
    #[must_use]
    pub fn execute(
        &self,
        input: RelationMutationInput,
    ) -> Effect<Option<RelationMutationOutcome<E>>, E, R> {
        if self.binding_value().is_none() {
            return effect_sync! { body: move || None };
        }
        let service = self.clone();
        effect! {
            let service = service.clone();
            let input = input.clone();
            let prepared = prepare_relation_mutation(&service, input);
            match prepared {
                Err(outcome) => Some(outcome),
                Ok(prepared) => {
                    let ready = before_relation_mutation(prepared)?;
                    match ready {
                        Err(outcome) => Some(outcome),
                        Ok(ready) => Some(execute_relation_mutation(ready)?),
                    }
                }
            }
        }
    }
}

/// Build a rejection outcome.
fn rejected<E>(
    draft: RelationRowsDraft,
    issues: Vec<RelationRowIssue>,
    cause: Option<RelationMutationCause<E>>,
) -> RelationMutationOutcome<E> {
    RelationMutationOutcome::Rejected(RelationMutationRejected::new(
        draft, issues, cause,
    ))
}

/// Build a failure outcome.
fn failed<E>(
    draft: RelationRowsDraft,
    cause: RelationMutationCause<E>,
) -> RelationMutationOutcome<E> {
    RelationMutationOutcome::Failed(RelationMutationFailed::new(draft, cause))
}

/// Build an unresolved outcome.
fn unresolved<E>(
    draft: RelationRowsDraft,
    cause: RelationMutationCause<E>,
) -> RelationMutationOutcome<E> {
    RelationMutationOutcome::Unresolved(RelationMutationUnresolved::new(draft, cause))
}

/// Whether the draft's request cancellation is requested.
fn cancellation_requested(draft: &RelationRowsDraft) -> bool {
    draft
        .request()
        .is_some_and(|request| request.cancellation().is_requested())
}

/// Prepare one mutation from the owner-issued validated rows.
fn prepare_relation_mutation<E, R>(
    service: &RelationMutationService<E, R>,
    input: RelationMutationInput,
) -> Result<PreparedMutation<E, R>, RelationMutationOutcome<E>>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let draft = input.rows.draft();
    let Some(binding) = service.binding_value() else {
        return Err(rejected(
            draft,
            Vec::new(),
            Some(RelationMutationCause::Message(
                "relation mutation requires owner-issued validation and a bound executor"
                    .to_owned(),
            )),
        ));
    };
    if !input.rows.is_valid() {
        return Err(rejected(
            draft,
            Vec::new(),
            Some(RelationMutationCause::Message(
                "relation mutation requires owner-issued validation and a bound executor"
                    .to_owned(),
            )),
        ));
    }
    let descriptor = draft.descriptor();
    let Some(descriptor_source) = descriptor.source_kind() else {
        return Err(rejected(
            draft,
            Vec::new(),
            Some(RelationMutationCause::Message(
                "relation mutation input does not match its validated source/relation/revision"
                    .to_owned(),
            )),
        ));
    };
    let Some(descriptor_definition) = descriptor.definition() else {
        return Err(rejected(
            draft,
            Vec::new(),
            Some(RelationMutationCause::Message(
                "relation mutation input does not match its validated source/relation/revision"
                    .to_owned(),
            )),
        ));
    };
    if &input.source != descriptor_source
        || input.relation.as_str() != descriptor_definition.name.as_str()
        || input.source_id != draft.source_id()
        || input.revision != draft.revision()
        || binding.source() != &input.source
        || binding.relation() != input.relation.as_str()
    {
        return Err(rejected(
            draft,
            Vec::new(),
            Some(RelationMutationCause::Message(
                "relation mutation input does not match its validated source/relation/revision"
                    .to_owned(),
            )),
        ));
    }
    if cancellation_requested(&draft) {
        return Err(failed(draft, RelationMutationCause::Cancelled));
    }
    let Some(claim) = input.rows.claim() else {
        return Err(rejected(
            draft,
            Vec::new(),
            Some(RelationMutationCause::Message(
                "relation validation already belongs to a mutation execution".to_owned(),
            )),
        ));
    };
    let readiness = Arc::new(RelationMutationReadiness {
        claim,
        execution: Arc::new(RelationMutationExecution {
            before_started: AtomicBool::new(false),
            execute_started: AtomicBool::new(false),
        }),
    });
    Ok(PreparedMutation {
        readiness,
        binding: Arc::clone(binding),
    })
}

/// Run the bound before hook and classify its observation.
fn before_relation_mutation<E, R>(
    prepared: PreparedMutation<E, R>,
) -> Effect<Result<ReadyMutation<E, R>, RelationMutationOutcome<E>>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let PreparedMutation {
        readiness,
        binding,
    } = prepared;
    if !readiness.claim.is_valid()
        || readiness
            .execution
            .before_started
            .swap(true, Ordering::SeqCst)
    {
        return effect_sync! {
            once;
            body: move || {
                Err(failed(
                    RelationRowsDraft::default(),
                    RelationMutationCause::Message(
                        "relation mutation: invalid or reused validated stage".to_owned(),
                    ),
                ))
            },
        };
    }
    let draft = readiness.claim.draft();
    if cancellation_requested(&draft) {
        return effect_sync! {
            once;
            body: move || Err(failed(draft, RelationMutationCause::Cancelled)),
        };
    }
    let before = Arc::clone(binding.before());
    effect! {
        let readiness = Arc::clone(&readiness);
        let binding = Arc::clone(&binding);
        let draft = draft.clone();
        let attempted = effect! { either; program: before.before(draft.clone()), }?;
        match attempted {
            Ok(observation) => {
                if cancellation_requested(&draft) {
                    Err(failed(draft, RelationMutationCause::Cancelled))
                } else if !observation.is_matching(&draft) {
                    Err(failed(
                        draft,
                        RelationMutationCause::Message(
                            "relation before hook returned a different input observation"
                                .to_owned(),
                        ),
                    ))
                } else {
                    match observation.into_value() {
                        Some(RelationBeforeObservationValue::Accepted { .. }) => {
                            Ok(ReadyMutation {
                                readiness: Arc::clone(&readiness),
                                binding: Arc::clone(&binding),
                            })
                        }
                        Some(RelationBeforeObservationValue::Rejected {
                            issues, cause, ..
                        }) => Err(rejected(draft, issues, cause)),
                        Some(RelationBeforeObservationValue::Failed { cause, .. }) => {
                            Err(failed(draft, cause))
                        }
                        None => Err(failed(
                            draft,
                            RelationMutationCause::Message(
                                "relation before hook returned a different input observation"
                                    .to_owned(),
                            ),
                        )),
                    }
                }
            }
            Err(error) => {
                if cancellation_requested(&draft) {
                    Err(failed(draft, RelationMutationCause::Cancelled))
                } else {
                    Err(failed(draft, RelationMutationCause::Consumer(error)))
                }
            }
        }
    }
}

/// Run the bound executor and classify its observation.
fn execute_relation_mutation<E, R>(
    ready: ReadyMutation<E, R>,
) -> Effect<RelationMutationOutcome<E>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let ReadyMutation {
        readiness,
        binding,
    } = ready;
    let request = RelationMutationRequest::from_readiness(Arc::clone(&readiness));
    if !request.is_valid()
        || readiness
            .execution
            .execute_started
            .swap(true, Ordering::SeqCst)
    {
        return effect_sync! {
            once;
            body: move || {
                failed(
                    RelationRowsDraft::default(),
                    RelationMutationCause::Message(
                        "relation mutation: invalid or reused ready stage".to_owned(),
                    ),
                )
            },
        };
    }
    let draft = request.draft();
    if cancellation_requested(&draft) {
        return effect_sync! {
            once;
            body: move || failed(draft, RelationMutationCause::Cancelled),
        };
    }
    let execute = Arc::clone(binding.execute());
    effect! {
        let request = request.clone();
        let draft = draft.clone();
        let attempted = effect! { either; program: execute(request.clone()), }?;
        match attempted {
            Ok(observation) => finish_execution(observation, request, draft),
            Err(error) => {
                if cancellation_requested(&draft) {
                    unresolved(draft, RelationMutationCause::Cancelled)
                } else {
                    failed(draft, RelationMutationCause::Consumer(error))
                }
            }
        }
    }
}

/// Finish one executor observation against the request identity.
fn finish_execution<E>(
    observation: RelationMutationObservation<E>,
    request: RelationMutationRequest,
    draft: RelationRowsDraft,
) -> RelationMutationOutcome<E> {
    let expected = request.readiness().cloned();
    let Some(value) = observation.into_value() else {
        return unresolved_mismatch(&draft);
    };
    let observed = match &value {
        RelationMutationObservationValue::Applied { readiness, .. }
        | RelationMutationObservationValue::Rejected { readiness, .. }
        | RelationMutationObservationValue::Failed { readiness, .. }
        | RelationMutationObservationValue::Unresolved { readiness, .. } => readiness,
    };
    if !expected.as_ref().is_some_and(|expected| Arc::ptr_eq(expected, observed)) {
        return unresolved_mismatch(&draft);
    }
    match value {
        RelationMutationObservationValue::Applied { revision, .. } => {
            RelationMutationOutcome::Applied(RelationMutationApplied::new(request, revision))
        }
        RelationMutationObservationValue::Rejected { issues, cause, .. } => {
            rejected(draft, issues, cause)
        }
        RelationMutationObservationValue::Failed { cause, .. } => failed(draft, cause),
        RelationMutationObservationValue::Unresolved { cause, .. } => {
            unresolved(draft, cause)
        }
    }
}

/// Build the unresolved outcome for a missing or mismatched observation.
fn unresolved_mismatch<E>(draft: &RelationRowsDraft) -> RelationMutationOutcome<E> {
    let cause = if cancellation_requested(draft) {
        RelationMutationCause::Cancelled
    } else {
        RelationMutationCause::Message(
            "relation executor returned no matching classified observation".to_owned(),
        )
    };
    unresolved(draft.clone(), cause)
}
