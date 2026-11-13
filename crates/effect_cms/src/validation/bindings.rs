//! Consumer-supplied validation binding and validator algebra.
//!
//! The validation bindings keyed by resource kind and action, the binder contract
//! consumers implement to register their validators, and the command-input and
//! validation-result validators the workflow runs around the bound resource
//! validator.
//!
//! # Why the input type is a parameter
//!
//! The command owner is implemented separately, so this owner states the algebra
//! over the supplied input type instead of duplicating the command contract: a
//! consumer instantiates `Bindings<CommandInput, E>` with the command owner's input
//! type, and no command input type, request parser or query service is declared
//! here.
//!
//! # Ownership
//!
//! Every callback is required to be `Send + Sync + 'static` because it is stored in
//! a value the effectful workflow will carry across its steps. An absent validator
//! is ignored by `set`, and `get` reports the missing binding, so the workflow's
//! "no validator" branch stays explicit.

use std::sync::Arc;

use effect::{AtomicRef, Effect};
use effect_server::Request;

use crate::definitions::{ResourceAction, ResourceKind, SharedMap};

use super::paths::Catalog;
use super::result::ValidationResult;

/// Validates and may update one parsed command input for its resource and action.
pub type ActionValidator<I, E, R> =
    Arc<dyn Fn(&Request, AtomicRef<I>) -> Effect<(), E, R> + Send + Sync + 'static>;

/// Validates and may update one command input before the resource validator runs.
pub type CommandInputValidator<I, E> =
    Arc<dyn Fn(&mut I) -> Result<(), E> + Send + Sync + 'static>;

/// Validates one produced validation result.
pub type ValidationResultValidator<E> =
    Arc<dyn Fn(&ValidationResult) -> Result<(), E> + Send + Sync + 'static>;

/// A consumer that registers its own resource validators.
///
/// Each binder adds the kinds and actions it owns, and nothing about the map is
/// global.
pub trait Binder<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// Register this binder's validators.
    fn bind_resource_validation(&self, bindings: &mut Bindings<I, E, R>);
}

/// The resource/action validator map.
///
/// Cloning copies the map structure and shares the validator callbacks, exactly as
/// copying a shared map handle shares its contents.
pub struct Bindings<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    entries: SharedMap<ResourceKind, SharedMap<ResourceAction, ActionValidator<I, E, R>>>, 
}

impl<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for Bindings<I, E, R> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}

impl<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Default for Bindings<I, E, R> {
    fn default() -> Self {
        Self {
            entries: SharedMap::default(),
        }
    }
}

impl<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Bindings<I, E, R> {
    /// Register one validator; an absent validator is ignored.
    ///
    /// The receiver is a shared map handle, so registration is visible through every
    /// copy and to a service already holding one.
    pub fn set(
        &self,
        kind: ResourceKind,
        action: ResourceAction,
        validator: Option<ActionValidator<I, E, R>>,
    ) {
        let Some(validator) = validator else {
            return;
        };
        self.entries.with_mut(|entries| {
            entries.entry(kind).or_default().with_mut(|actions| {
                actions.insert(action, validator);
            });
        });
    }

    /// The validator registered for one kind and action, if any.
    #[must_use]
    pub fn get(
        &self,
        kind: &ResourceKind,
        action: &ResourceAction,
    ) -> Option<ActionValidator<I, E, R>> {
        self.entries.with(|entries| {
            entries
                .get(kind)
                .and_then(|actions| actions.get(action))
        })
    }
}

/// Collect bindings by invoking one binder callback per index.
///
/// Collect bindings by invoking one binder callback per index; the public slice
/// form below delegates here.
#[must_use]
pub fn new_bindings_from<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    count: usize,
    bind: impl Fn(usize, &mut Bindings<I, E, R>),
) -> Bindings<I, E, R> {
    let mut bindings = Bindings::default();
    for index in 0..count {
        bind(index, &mut bindings);
    }
    bindings
}

/// Collect bindings from the supplied binders.
///
/// Every binder registers its own validators into one shared map. An absent binder
/// has no Rust spelling in a slice of trait references, so the nil-binder guard is
/// unrepresentable here rather than omitted.
#[must_use]
pub fn new_bindings<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(binders: &[&dyn Binder<I, E, R>]) -> Bindings<I, E, R> {
    new_bindings_from(binders.len(), |index, bindings| {
        if let Some(binder) = binders.get(index) {
            binder.bind_resource_validation(bindings);
        }
    })
}

/// The command-input and validation-result validators run around the bound
/// validator. The input validators receive the mutable input identity; the result
/// validator keeps its own value-copy scalar behavior.
pub struct Validators<I, E> {
    /// Validates one parsed command input before the resource validator runs.
    pub command_input: Option<CommandInputValidator<I, E>>,
    /// Validates one produced validation result.
    pub validation_result: Option<ValidationResultValidator<E>>,
}

impl<I, E> Clone for Validators<I, E> {
    fn clone(&self) -> Self {
        Self {
            command_input: self.command_input.clone(),
            validation_result: self.validation_result.clone(),
        }
    }
}

impl<I, E> Default for Validators<I, E> {
    fn default() -> Self {
        Self {
            command_input: None,
            validation_result: None,
        }
    }
}

/// The supplied validation options.
pub struct Options<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> {
    /// The resource/action validator map.
    pub bindings: Bindings<I, E, R>,
    /// The command-input and result validators.
    pub validators: Validators<I, E>,
    /// The path binding catalog.
    pub path_catalog: Catalog,
}

impl<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Clone for Options<I, E, R> {
    fn clone(&self) -> Self {
        Self {
            bindings: self.bindings.clone(),
            validators: self.validators.clone(),
            path_catalog: self.path_catalog.clone(),
        }
    }
}

impl<I: Send + 'static, E: Send + 'static, R: Send + Sync + 'static> Default for Options<I, E, R> {
    fn default() -> Self {
        Self {
            bindings: Bindings::default(),
            validators: Validators::default(),
            path_catalog: Catalog::default(),
        }
    }
}
