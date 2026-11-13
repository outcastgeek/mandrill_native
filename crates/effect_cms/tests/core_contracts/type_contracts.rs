//! Negative contracts for native ownership and private publication.
//!
//! Include from the crate root only under cfg(doctest); these are ordinary Rustdoc
//! compile-fail tests, not a scanner or a separate compile-test framework.
//!
//! A denial response cannot be projected twice from the same outcome.
//! ```compile_fail,E0382
//! use effect_cms::access::AccessOutcome;
//! fn publish_twice(outcome: AccessOutcome<std::fmt::Error, ()>) {
//!     let _first = outcome.into_response();
//!     let _second = outcome.into_response();
//! }
//! ```
//!
//! Access disposition cannot be read or substituted through its private state.
//! ```compile_fail,E0616
//! use effect_cms::access::AccessOutcome;
//! fn substitute(outcome: AccessOutcome<std::fmt::Error, ()>) {
//!     let _ = outcome.state;
//! }
//! ```
//!
//! Command resolution implementation is not a public stage/factory boundary.
//! ```compile_fail,E0603
//! use effect_cms::commands::service;
//! fn main() {}
//! ```
//!
//! List admission implementation cannot be entered as a consumer-visible stage.
//! ```compile_fail,E0603
//! use effect_cms::queries::list_query;
//! fn main() {}
//! ```
//!
//! Detail admission implementation cannot be entered as a consumer-visible stage.
//! ```compile_fail,E0603
//! use effect_cms::queries::detail_query;
//! fn main() {}
//! ```
//!
//! Accepted validation inputs can only be published by the validation owner.
//! ```compile_fail,E0451
//! use effect_cms::{commands::{CommandInput, CommandInputHandle},
//!     validation::{ActionValidationAccepted, ValidationResult}};
//! fn main() {
//!     let _forged = ActionValidationAccepted {
//!         input: CommandInputHandle::new(CommandInput::default()),
//!         result: ValidationResult::default(),
//!     };
//! }
//! ```
//!
//! The generic ready mutation stage cannot be named outside its owner.
//! ```compile_fail,E0603
//! use effect_cms::mutations::execution::ReadyMutation;
//! fn main() {}
//! ```
//!
//! An applied mutation cannot be forged from a raw consumer result.
//! ```compile_fail,E0451
//! use effect_cms::{commands::MutationResult,
//!     mutations::AppliedMutation, validation::ValidationResult};
//! fn main() {
//!     let _forged = AppliedMutation {
//!         result: Some(MutationResult::default()),
//!         validation: ValidationResult::default(),
//!     };
//! }
//! ```
//!
//! A relation descriptor's checked declaration cannot be replaced by raw data.
//! ```compile_fail,E0451
//! use effect_cms::definitions::ResourceRelationDescriptor;
//! fn main() {
//!     let _forged = ResourceRelationDescriptor { value: None };
//! }
//! ```
//!
//! A relation selection cannot be fabricated by replacing its owner-held payload.
//! ```compile_fail,E0451
//! use effect_cms::queries::RelationSelection;
//! fn main() {
//!     let _forged = RelationSelection { value: None };
//! }
//! ```
//!
//! Validated relation rows cannot be fabricated from raw rows or a zero claim.
//! ```compile_fail,E0451
//! use effect_cms::validation::ValidatedRelationRows;
//! fn main() {
//!     let _forged = ValidatedRelationRows { value: None, execution: None };
//! }
//! ```
//!
//! A raw transport row vector cannot stand in for validated rows at mutation entry.
//! ```compile_fail,E0308
//! use effect_cms::{mutations::RelationMutationInput, validation::RelationRowInput};
//! fn main() {
//!     let _input = RelationMutationInput {
//!         rows: Vec::<RelationRowInput>::new(),
//!         ..RelationMutationInput::default()
//!     };
//! }
//! ```
