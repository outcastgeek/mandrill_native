//! Relation mutation (workflow W21).
//!
//! This child owner provides:
//!
//! - the authorizing input carrying the owner-issued validated rows;
//! - the constructor-bound before-hook and executor capabilities;
//! - the private request/readiness stages, one-shot claim consumption, the
//!   before-hook observation and the executor observation;
//! - the applied/rejected/failed/unresolved outcomes, their detached drafts,
//!   ordered issues and retained consumer causes;
//! - the unconstructed zero-service outcome and the owner sequence: identity
//!   admission, claim, before hook, executor, classification.
//!
//! # Capabilities and data
//!
//! The definition identity comes from the owner-issued validated draft; the
//! consumer supplies the before hook and the actual executor. The CMS owns the
//! prescribed sequencing, the request identity and the closed outcome publication.
//! It owns no database, SQL, schema, migration, pool or backend; the executor
//! rechecks current facts in its own transaction.
//!
//! # Effects
//!
//! [`RelationMutationService::execute`] is a lazy [`effect::Effect`]. The before
//! hook and executor are effectful consumer capabilities and run only when the
//! description runs. Expected callback failures become classified outcomes with the
//! consumer's cause; cancellation at the owner checkpoints takes precedence;
//! defects, interruption and finalization failures keep their own effect channels.
//! The request/ready stages are private and consumed once.
//!
//! # Private implementation
//!
//! `bindings`, `observation` and `execution` are private implementation modules of
//! this child owner; this module is its deliberate public surface, re-exported by
//! the mutations owner.

mod bindings;
mod execution;
mod observation;

pub use self::bindings::{
    RelationBefore, RelationBeforeFunc, RelationExecuteFunc, RelationMutationBinding,
    RelationMutationInput, RelationMutationService, new_relation_mutation_binding,
    new_relation_mutation_service,
};
pub use self::observation::{
    RelationBeforeObservation, RelationMutationApplied, RelationMutationCause,
    RelationMutationContractError, RelationMutationFailed, RelationMutationObservation,
    RelationMutationOutcome, RelationMutationRejected, RelationMutationRequest,
    RelationMutationUnresolved, observe_relation_before_accepted, observe_relation_before_failed,
    observe_relation_before_rejected, observe_relation_mutation_applied,
    observe_relation_mutation_failed, observe_relation_mutation_rejected,
    observe_relation_mutation_unresolved,
};

/// Compile-time witness that the child owner keeps its capability, effect and
/// private-publication shapes.
const _: fn() = || {
    use effect::Effect;

    fn witness<E, R>()
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let _: fn(
            RelationMutationBinding<E, R>,
        ) -> Result<RelationMutationService<E, R>, RelationMutationContractError> =
            new_relation_mutation_service::<E, R>;
        let _: fn(
            &RelationMutationService<E, R>,
            RelationMutationInput,
        ) -> Effect<Option<RelationMutationOutcome<E>>, E, R> =
            RelationMutationService::execute;
        let _: fn(&RelationMutationRequest, String) -> RelationMutationObservation<E> =
            observe_relation_mutation_applied::<E>;
    }
    witness::<std::fmt::Error, ()>();
};
