//! Relation row validation (workflow W20).
//!
//! This child owner provides:
//!
//! - the decoded row input, the ordered row issue and the closed outcome;
//! - the detached `RelationRowsDraft`, the private validated rows record, its
//!   one-shot claim and the rejected rows/issue/cause projection;
//! - the constructor-bound preparation observation and application validator
//!   binding, the observation constructors and the relation-validation service;
//! - decoding: request presence and cancellation, registered source definition,
//!   named relation resolution, constructor-bound source/relation identity,
//!   declared operation, declared save mode, distinct raw row keys, typed target
//!   identity versus explicit related-record creation, and reorder prerequisites;
//! - the validation sequence: preparation observation, classification, application
//!   validation, cancellation precedence, and acceptance or ordered rejection.
//!
//! # Capabilities and data
//!
//! The definition registry, the preparation observation callback and the application
//! validator are required constructor/capability inputs. The consumer performs its
//! own lookups and reconciliation inside those callbacks; this owner owns only the
//! prescribed decode/validate sequence, the ordered issues and the private accepted
//! representation. The CMS owns no database, SQL, schema, migration, pool or
//! backend.
//!
//! # Effects
//!
//! [`RelationValidationService::validate`] is a lazy [`effect::Effect`]: decoding,
//! the preparation observation and the application validator run only when the
//! description runs. A consumer callback failure becomes a rejected outcome with
//! its own cause; defects, interruption and finalization failures keep their own
//! effect channels. The accepted value is published only through the owner's
//! factories and can be claimed once. An unconstructed default service publishes
//! the zero outcome (`None`) and runs no callback.
//!
//! # Private implementation
//!
//! `rows`, `decoding`, `bindings` and `workflow` are private implementation modules
//! of this child owner; this module is its deliberate public surface, re-exported by
//! the validation owner.

mod bindings;
mod decoding;
mod rows;
mod workflow;

pub use self::bindings::{
    RelationRowsApplicationResult, RelationRowsApplicationValidateFunc,
    RelationRowsApplicationValidator, RelationRowsCheckFunc, RelationRowsObservation,
    RelationValidationBinding, RelationValidationService, new_relation_validation_binding,
    new_relation_validation_service, observe_relation_rows_rejected,
    observe_relation_rows_validated,
};
pub use self::rows::{
    RelationRowInput, RelationRowIssue, RelationRowsClaim, RelationRowsDraft,
    RelationRowsRejected, RelationValidationCause, RelationValidationContractError,
    RelationValidationInput, RelationValidationOutcome, ValidatedRelationRows,
};

/// Compile-time witness that the child owner keeps its capability, effect and
/// private-publication shapes.
const _: fn() = || {
    use std::sync::Arc;

    use effect::Effect;

    use crate::definitions::Registry;

    fn witness<E, R>()
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
    {
        let _: fn(
            Arc<dyn Registry + Send + Sync>,
            RelationValidationBinding<E, R>,
        ) -> Result<RelationValidationService<E, R>, RelationValidationContractError> =
            new_relation_validation_service::<E, R>;
        let _: fn(
            &RelationValidationService<E, R>,
            RelationValidationInput,
        ) -> Effect<Option<RelationValidationOutcome<E>>, E, R> =
            RelationValidationService::validate;
        let _: fn(&ValidatedRelationRows) -> Option<RelationRowsClaim> =
            ValidatedRelationRows::claim;
    }
    witness::<std::fmt::Error, ()>();
};
