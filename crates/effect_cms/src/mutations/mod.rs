//! Mutations: the W7 resource mutation progression and its classified outcomes.
//!
//! Mutations owns:
//!
//! - the optional before callback contract and its cause-aware result over the
//!   mutable command-input identity;
//! - the mutation cause and the applied/rejected/failed outcome contracts;
//! - the one resource mutation progression: requested-action validation, the
//!   before callback's input updates, the supplied command executor and outcome
//!   classification;
//! - the resource mutation redirect: the same progression (or the caller's deferred
//!   attempt), its recovery, and the see-other redirect capability of an unhandled
//!   outcome.
//!
//! # Consumer-owned data access
//!
//! The CMS implements no create/read/update/delete operation, database, SQL
//! statement, schema, migration, pool or backend. The supplied command executor is
//! the consumer's own effectful capability stored in
//! [`crate::commands::ResourceCommandBindings`]; this owner invokes it through
//! [`crate::commands::CommandService`] and returns its result unchanged. Validation
//! is likewise the validation owner's workflow over caller-supplied validators.
//!
//! # Effects
//!
//! [`execute_resource_mutation`] and [`execute_resource_mutation_with_before`]
//! return lazy [`effect::Effect`] values. Validation refusal, before-callback
//! failure and executor failure become typed [`MutationFailure`] values in the
//! effect's error channel; defects, interruption and finalization failures keep
//! their own channels. [`execute_resource_mutation_with_recovery`] runs a deferred
//! [`Attempt`] and recovers its classified outcome through [`RecoveryOptions`].
//! Construction invokes no callback and performs no work.
//!
//! # Private prerequisites
//!
//! The validated and ready stages are private values; the ready stage is consumed by
//! the executor step. This preserves the request/input identity and makes a forged
//! or replayed stage unrepresentable through ownership rather than a runtime stage
//! registry, and adds no one-shot restriction to the reusable public workflow.
//!
//! # HTML
//!
//! This owner produces no HTML: the mutation-result redirect reuses the routing and
//! access redirect responses.

mod execution;
mod relation;
pub use relation::*;
mod redirect;

pub use self::execution::{
    AppliedMutation, Attempt, AttemptedMutation, BeforeExecute, FailedMutation, MutationCause,
    MutationFailure, RecoveryOptions, RejectedMutation, execute_resource_mutation,
    execute_resource_mutation_with_before, execute_resource_mutation_with_recovery,
};
pub use self::redirect::{
    MutationRedirectOutcome, ResourceMutationRedirectOptions, handle_resource_mutation_redirect,
};

pub(crate) use execution::execute_with_optional_commands;
