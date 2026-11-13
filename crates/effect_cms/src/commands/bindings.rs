//! Consumer-supplied command executor bindings.
//!
//! The bindings map keyed by resource kind and action, the ordered collector and
//! live resolution. The executor is the consumer's own effectful capability; the CMS
//! stores it and invokes it, and the CMS owns no command implementation.
//!
//! A bindings handle shares its maps with every copy: a consumer that registers an
//! executor after the service was constructed changes what the service resolves.

use crate::definitions::{ResourceAction, ResourceKind, SharedMap};

use super::service::Executor;

/// A resource command executor.
pub type ResourceCommandExecutor<E, R> = Executor<E, R>;

/// A consumer that registers its own command executors.
///
/// Each binder adds the kinds and actions it owns, and nothing about the map is
/// global.
pub trait ResourceCommandBinder<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Register this binder's executors.
    fn bind_resource_commands(&self, bindings: &ResourceCommandBindings<E, R>);
}

/// Maps resource kind and action to a consumer-supplied executor.
///
/// Cloning copies the map handle and shares the entries. Registering through any
/// handle is visible to every other handle and to a service holding one.
pub struct ResourceCommandBindings<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    entries: SharedMap<ResourceKind, SharedMap<ResourceAction, ResourceCommandExecutor<E, R>>>,
}

impl<E, R> Clone for ResourceCommandBindings<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}

impl<E, R> Default for ResourceCommandBindings<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            entries: SharedMap::default(),
        }
    }
}

impl<E, R> ResourceCommandBindings<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    /// Register one executor; an absent executor is ignored.
    ///
    /// The receiver is a shared map handle, so this assignment is observable through
    /// every copy.
    pub fn set(
        &self,
        kind: ResourceKind,
        action: ResourceAction,
        executor: Option<ResourceCommandExecutor<E, R>>,
    ) {
        let Some(executor) = executor else {
            return;
        };
        self.entries.with_mut(|entries| {
            entries.entry(kind).or_default().with_mut(|actions| {
                actions.insert(action, executor);
            });
        });
    }

    /// Whether the map holds a kind entry, even when the action is absent.
    #[must_use]
    pub fn has_kind(&self, kind: &ResourceKind) -> bool {
        self.entries.with(|entries| entries.contains_key(kind))
    }

    /// The executor registered for one kind and action, if any.
    #[must_use]
    pub fn resolve(
        &self,
        kind: &ResourceKind,
        action: &ResourceAction,
    ) -> Option<ResourceCommandExecutor<E, R>> {
        self.entries
            .with(|entries| entries.get(kind).and_then(|actions| actions.get(action)))
    }
}

/// Collect bindings from the supplied binders in their original order.
///
/// Collect every binder's executors. A nil binder has no Rust spelling in a slice of
/// trait references, so the nil-binder guard is unrepresentable here rather than
/// omitted.
#[must_use]
pub fn new_resource_command_bindings<E, R>(
    binders: &[&dyn ResourceCommandBinder<E, R>],
) -> ResourceCommandBindings<E, R>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
{
    let bindings = ResourceCommandBindings::default();
    for binder in binders {
        binder.bind_resource_commands(&bindings);
    }
    bindings
}
