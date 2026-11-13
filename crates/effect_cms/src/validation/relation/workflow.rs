//! The relation-validation sequence over a decoded row draft.
//!
//! The owner keeps this order: recheck cancellation, run the constructor-bound
//! preparation observation, recheck cancellation, classify that observation, run the
//! application validator, recheck cancellation, then accept or reject. A closed
//! rejection carries the detached rows, the ordered issues and the optional cause.
//! The accepted record is private and consumable once through its claim.
//!
//! Consumer callback failures arrive as expected failures on the capability boundary
//! and become rejected outcomes with the consumer's cause; cancellation checked at
//! these checkpoints takes precedence over such a cause. Defects,
//! interruption and finalization failures keep their own effect channels.

use std::sync::Arc;

use effect::{Effect, effect, effect_sync};

use super::bindings::{
    RelationRowsApplicationResult, RelationRowsObservationValue, RelationValidationService,
};
use super::decoding::{DecodedRelationRows, decode_relation_rows};
use super::rows::{
    RelationRowsDraft, RelationRowsRejected, RelationValidationCause, RelationValidationInput,
    RelationValidationOutcome, ValidatedRelationRows,
};

impl<E, R> RelationValidationService<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Validate one relation-rows input and publish its closed outcome.
    ///
    /// The returned effect is lazy: decoding, the bound preparation observation and
    /// the application validator all run when the description runs. An unconstructed
    /// zero service publishes the zero outcome and runs no callback.
    #[must_use]
    pub fn validate(
        &self,
        input: RelationValidationInput,
    ) -> Effect<Option<RelationValidationOutcome<E>>, E, R> {
        if self.resources().is_none() || self.binding_value().is_none() {
            return effect_sync! { body: move || None };
        }
        let service = self.clone();
        effect! {
            let service = service.clone();
            let input = input.clone();
            let decoded = decode_relation_rows(&service, input);
            match decoded {
                Ok(decoded) => Some(validate_relation_rows(service, decoded)?),
                Err(rejected) => Some(RelationValidationOutcome::Rejected(rejected)),
            }
        }
    }
}

/// Build one closed rejection from a draft.
fn reject<E>(
    draft: &RelationRowsDraft,
    issues: Vec<super::rows::RelationRowIssue>,
    cause: Option<RelationValidationCause<E>>,
) -> RelationValidationOutcome<E> {
    RelationValidationOutcome::Rejected(RelationRowsRejected::new(
        draft.rows(),
        issues,
        cause,
    ))
}

/// Whether the draft's request cancellation is requested.
fn cancellation_requested(draft: &RelationRowsDraft) -> bool {
    draft
        .request()
        .is_some_and(|request| request.cancellation().is_requested())
}

/// The missing-observation rejection.
fn missing_observation<E>(draft: &RelationRowsDraft) -> RelationValidationOutcome<E> {
    reject(
        draft,
        Vec::new(),
        Some(RelationValidationCause::Message(
            "relation validation: missing or mismatched preparation observation".to_owned(),
        )),
    )
}

/// Run the bound preparation observation.
fn check_step<E, R>(
    service: &RelationValidationService<E, R>,
    draft: RelationRowsDraft,
) -> Effect<Result<super::bindings::RelationRowsObservation<E>, RelationValidationOutcome<E>>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let Some(binding) = service.binding_value() else {
        return effect_sync! {
            once;
            body: move || {
                Err(reject(
                    &draft,
                    Vec::new(),
                    Some(RelationValidationCause::Message(
                        "relation validation requires its request and bound capabilities"
                            .to_owned(),
                    )),
                ))
            },
        };
    };
    let Some(request) = draft.request().cloned() else {
        return effect_sync! {
            once;
            body: move || {
                Err(reject(
                    &draft,
                    Vec::new(),
                    Some(RelationValidationCause::Message(
                        "relation validation requires its request and bound capabilities"
                            .to_owned(),
                    )),
                ))
            },
        };
    };
    let check = Arc::clone(binding.check());
    effect! {
        let draft = draft.clone();
        let request = request.clone();
        let attempted = check(draft.clone()).either()?;
        match attempted {
            Ok(observation) => {
                if request.cancellation().is_requested() {
                    Err(reject(
                        &draft,
                        Vec::new(),
                        Some(RelationValidationCause::Cancelled),
                    ))
                } else if observation.is_valid_for(&draft) {
                    Ok(observation)
                } else {
                    Err(missing_observation(&draft))
                }
            }
            Err(error) => {
                if request.cancellation().is_requested() {
                    Err(reject(
                        &draft,
                        Vec::new(),
                        Some(RelationValidationCause::Cancelled),
                    ))
                } else {
                    Err(reject(
                        &draft,
                        Vec::new(),
                        Some(RelationValidationCause::Consumer(error)),
                    ))
                }
            }
        }
    }
}

/// Run the bound application validator.
fn application_step<E, R>(
    service: &RelationValidationService<E, R>,
    draft: RelationRowsDraft,
) -> Effect<Result<RelationRowsApplicationResult<E>, RelationValidationOutcome<E>>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let Some(binding) = service.binding_value() else {
        return effect_sync! {
            once;
            body: move || {
                Err(reject(
                    &draft,
                    Vec::new(),
                    Some(RelationValidationCause::Message(
                        "relation validation requires its request and bound capabilities"
                            .to_owned(),
                    )),
                ))
            },
        };
    };
    let Some(request) = draft.request().cloned() else {
        return effect_sync! {
            once;
            body: move || {
                Err(reject(
                    &draft,
                    Vec::new(),
                    Some(RelationValidationCause::Message(
                        "relation validation requires its request and bound capabilities"
                            .to_owned(),
                    )),
                ))
            },
        };
    };
    let application = Arc::clone(binding.application());
    effect! {
        let draft = draft.clone();
        let request = request.clone();
        let attempted = application.validate(draft.clone()).either()?;
        match attempted {
            Ok(result) => {
                if request.cancellation().is_requested() {
                    Err(reject(
                        &draft,
                        result.issues,
                        Some(RelationValidationCause::Cancelled),
                    ))
                } else {
                    Ok(result)
                }
            }
            Err(error) => {
                if request.cancellation().is_requested() {
                    Err(reject(
                        &draft,
                        Vec::new(),
                        Some(RelationValidationCause::Cancelled),
                    ))
                } else {
                    Err(reject(
                        &draft,
                        Vec::new(),
                        Some(RelationValidationCause::Consumer(error)),
                    ))
                }
            }
        }
    }
}

/// Validate one decoded relation-rows stage.
fn validate_relation_rows<E, R>(
    service: RelationValidationService<E, R>,
    decoded: DecodedRelationRows,
) -> Effect<RelationValidationOutcome<E>, E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let draft = RelationRowsDraft::from_value(decoded.value);
    if cancellation_requested(&draft) {
        return effect_sync! {
            once;
            body: move || {
                reject(
                    &draft,
                    Vec::new(),
                    Some(RelationValidationCause::Cancelled),
                )
            },
        };
    }
    effect! {
        let service = service.clone();
        let draft = draft.clone();
        let checked = check_step(&service, draft.clone())?;
        match checked {
            Err(outcome) => outcome,
            Ok(observation) => match observation.into_value() {
                Some(RelationRowsObservationValue::Prepared { .. }) => {
                    let applied = application_step(&service, draft.clone())?;
                    match applied {
                        Err(outcome) => outcome,
                        Ok(result) => {
                            if !result.issues.is_empty() || result.cause.is_some() {
                                reject(
                                    &draft,
                                    result.issues,
                                    result.cause.map(RelationValidationCause::Consumer),
                                )
                            } else {
                                match draft.value().cloned() {
                                    Some(value) => RelationValidationOutcome::Accepted(
                                        ValidatedRelationRows::accept(value),
                                    ),
                                    None => RelationValidationOutcome::Rejected(
                                        RelationRowsRejected::new(
                                            draft.rows(),
                                            Vec::new(),
                                            Some(RelationValidationCause::Message(
                                                "relation validation: invalid, mismatched or reused decoded stage"
                                                    .to_owned(),
                                            )),
                                        ),
                                    ),
                                }
                            }
                        }
                    }
                }
                Some(RelationRowsObservationValue::Rejected {
                    issues, cause, ..
                }) => reject(&draft, issues, cause),
                None => missing_observation(&draft),
            },
        }
    }
}
