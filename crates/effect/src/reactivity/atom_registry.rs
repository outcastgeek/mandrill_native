//! Scoped atom evaluation, dependency tracking, and disposal.

use super::{InvalidationError, InvalidationKey, InvalidationRegistry};
use crate::{Effect, KeyDomain, PubSubOptions, PubSubShutdown, StableKey};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by reactivity atom registry operations.
pub struct AtomId(InvalidationKey);

impl AtomId {
    #[must_use]
    /// Wrap a typed stable key while preserving its identity bytes.
    ///
    /// # Type parameters
    /// - `D`: Identity-domain type. Bounds: `KeyDomain`.
    ///
    /// # Parameters
    /// - `key` (`StableKey<D>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(InvalidationKey::from_stable(key))
    }

    #[must_use]
    /// Return the stable key used to publish invalidations for this atom.
    ///
    /// # Returns
    /// Type: `InvalidationKey`.
    pub fn invalidation_key(&self) -> InvalidationKey {
        self.0.clone()
    }

    #[must_use]
    /// Borrow the stable encoded bytes without transferring ownership.
    ///
    /// # Returns
    /// Type: `&[u8]`.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The source or derived role assigned to an atom in a registry.
pub enum AtomKind {
    /// The error originated in the underlying source.
    Source,
    /// This atom computes its value from registered dependencies.
    Derived,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// An identity conflict or lifecycle failure while registering atoms.
pub enum AtomRegistryError {
    /// The ation failed validation. Carries `InvalidationError`.
    Invalidation(InvalidationError),
    /// The removal committed, but publishing its invalidation failed. Retains the `InvalidationError` detail.
    RemovalCommittedWithoutInvalidation(InvalidationError),
    /// The  already exists or occurred twice. Carries `AtomId`.
    Duplicate(AtomId),
    /// No matching value exists. Carries `AtomId`.
    Missing(AtomId),
    /// The referenced atom is not registered as a derived atom.
    NotDerived(AtomId),
    /// No further generation can be allocated without exceeding its range. Retains the `AtomId` detail.
    GenerationExhausted(AtomId),
    /// The requested dependency would create a reactive graph cycle.
    DependencyCycle {
        /// Atom whose dependency cannot be added. Type: `AtomId`.
        atom: AtomId,
        /// Atom that closes the cycle. Type: `AtomId`.
        through: AtomId,
    },
    /// The resource has already been disposed.
    Disposed,
}

/// Typed node used by reactivity atom registry.
struct Node {
    kind: AtomKind,
    dependencies: BTreeSet<AtomId>,
    generation: Option<std::num::NonZeroU64>,
    invalidated: bool,
}

/// Typed graph used by reactivity atom registry.
struct Graph {
    disposed: bool,
    nodes: BTreeMap<AtomId, Node>,
    dependents: BTreeMap<AtomId, BTreeSet<AtomId>>,
}

/// Graph ownership and publication stay within this registry instance.
/// Nodes only carry identities and dependencies; typed values live in the
/// corresponding AtomRef or derived atom handle, without dyn Any payloads.
pub struct AtomRegistry {
    graph: Arc<Mutex<Graph>>,
    invalidations: InvalidationRegistry,
}

impl Clone for AtomRegistry {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            graph: Arc::clone(&self.graph),
            invalidations: self.invalidations.clone(),
        }
    }
}

impl AtomRegistry {
    /// Construct a `AtomRegistry` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `options` (`PubSubOptions`): Caller-selected bounds and behavior for this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, AtomRegistryError>`.
    pub fn new(options: PubSubOptions) -> Result<Self, AtomRegistryError> {
        Ok(Self {
            graph: Arc::new(Mutex::new(Graph {
                disposed: false,
                nodes: BTreeMap::new(),
                dependents: BTreeMap::new(),
            })),
            invalidations: InvalidationRegistry::new(options)
                .map_err(AtomRegistryError::Invalidation)?,
        })
    }

    #[must_use]
    /// Borrow the invalidation publisher associated with this registry.
    ///
    /// # Returns
    /// Type: `InvalidationRegistry`.
    pub fn invalidations(&self) -> InvalidationRegistry {
        self.invalidations.clone()
    }

    #[must_use]
    /// # Parameters
    /// - `other` (`&Self`): Second value used for comparison or composition.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.graph, &other.graph)
    }

    /// Register the supplied definition, rejecting an identity or compatibility conflict.
    ///
    /// # Parameters
    /// - `id` (`AtomId`): Stable identity of the selected record or operation.
    /// - `kind` (`AtomKind`): The kind supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), AtomRegistryError>`.
    pub fn register(&self, id: AtomId, kind: AtomKind) -> Result<(), AtomRegistryError> {
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.disposed {
            return Err(AtomRegistryError::Disposed);
        }
        if graph.nodes.contains_key(&id) {
            return Err(AtomRegistryError::Duplicate(id));
        }
        graph.nodes.insert(
            id.clone(),
            Node {
                kind,
                dependencies: BTreeSet::new(),
                generation: None,
                invalidated: false,
            },
        );
        graph.dependents.entry(id).or_default();
        Ok(())
    }

    /// Dynamic dependencies replace the previous set only after all targets
    /// exist and the resulting graph is proven acyclic. The failed update
    /// leaves the original edges intact.
    ///
    /// # Parameters
    /// - `id` (`&AtomId`): Stable identity of the selected record or operation.
    /// - `dependencies` (`impl IntoIterator<Item = AtomId>`): The dependencies supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), AtomRegistryError>`.
    pub fn replace_dependencies(
        &self,
        id: &AtomId,
        dependencies: impl IntoIterator<Item = AtomId>,
    ) -> Result<(), AtomRegistryError> {
        let proposed = dependencies.into_iter().collect::<BTreeSet<_>>();
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.disposed {
            return Err(AtomRegistryError::Disposed);
        }
        let Some(node) = graph.nodes.get(id) else {
            return Err(AtomRegistryError::Missing(id.clone()));
        };
        if node.kind != AtomKind::Derived {
            return Err(AtomRegistryError::NotDerived(id.clone()));
        }
        for dependency in &proposed {
            if !graph.nodes.contains_key(dependency) {
                return Err(AtomRegistryError::Missing(dependency.clone()));
            }
            if dependency == id || reaches(&graph, dependency, id) {
                return Err(AtomRegistryError::DependencyCycle {
                    atom: id.clone(),
                    through: dependency.clone(),
                });
            }
        }
        let Some(node) = graph.nodes.get_mut(id) else {
            return Err(AtomRegistryError::Missing(id.clone()));
        };
        let old = std::mem::replace(&mut node.dependencies, proposed.clone());
        for dependency in old.difference(&proposed) {
            if let Some(dependents) = graph.dependents.get_mut(dependency) {
                dependents.remove(id);
            }
        }
        for dependency in proposed.difference(&old) {
            graph
                .dependents
                .entry(dependency.clone())
                .or_default()
                .insert(id.clone());
        }
        Ok(())
    }

    /// Bump a node generation before starting a refresh. Completion must
    /// compare this token; disposal or a later refresh makes it obsolete.
    ///
    /// # Parameters
    /// - `id` (`&AtomId`): Stable identity of the selected record or operation.
    ///
    /// # Returns
    /// Type: `Result<std::num::NonZeroU64, AtomRegistryError>`.
    pub fn begin_evaluation(&self, id: &AtomId) -> Result<std::num::NonZeroU64, AtomRegistryError> {
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.disposed {
            return Err(AtomRegistryError::Disposed);
        }
        let node = graph
            .nodes
            .get_mut(id)
            .ok_or_else(|| AtomRegistryError::Missing(id.clone()))?;
        if node.kind != AtomKind::Derived {
            return Err(AtomRegistryError::NotDerived(id.clone()));
        }
        let generation = match node.generation {
            None => Some(std::num::NonZeroU64::MIN),
            Some(previous) => previous
                .get()
                .checked_add(1)
                .and_then(std::num::NonZeroU64::new),
        }
        .ok_or_else(|| AtomRegistryError::GenerationExhausted(id.clone()))?;
        node.generation = Some(generation);
        node.invalidated = false;
        Ok(generation)
    }

    /// Read the registered dependencies of one derived atom.
    ///
    /// # Parameters
    /// - `id` (`&AtomId`): Stable identity of the selected record or operation.
    ///
    /// # Returns
    /// Type: `Result<BTreeSet<AtomId>, AtomRegistryError>`.
    pub fn dependencies_of(&self, id: &AtomId) -> Result<BTreeSet<AtomId>, AtomRegistryError> {
        let graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.disposed {
            return Err(AtomRegistryError::Disposed);
        }
        graph
            .nodes
            .get(id)
            .map(|node| node.dependencies.clone())
            .ok_or_else(|| AtomRegistryError::Missing(id.clone()))
    }

    #[must_use]
    /// Check whether the recorded revision is still current.
    ///
    /// # Parameters
    /// - `id` (`&AtomId`): Stable identity of the selected record or operation.
    /// - `generation` (`std::num::NonZeroU64`): The generation supplied to this operation.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub fn is_current(&self, id: &AtomId, generation: std::num::NonZeroU64) -> bool {
        let graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !graph.disposed
            && graph
                .nodes
                .get(id)
                .is_some_and(|node| !node.invalidated && node.generation == Some(generation))
    }

    /// A successful source mutation invalidates all transitive dependents in
    /// one batch. Publication happens after releasing the graph lock. Once
    /// mutation starts, cancellation cannot split it from publication;
    /// caller-selected backpressure can still delay completion.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `id` (`AtomId`): Stable identity of the selected record or operation.
    ///
    /// # Returns
    /// Type: `Effect<(), AtomRegistryError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn changed<R: Send + Sync + 'static>(
        &self,
        id: AtomId,
    ) -> Effect<(), AtomRegistryError, R> {
        let registry = self.clone();
        Effect::try_sync(
            move || {
                let mut graph = registry
                    .graph
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if graph.disposed {
                    return Err(AtomRegistryError::Disposed);
                }
                if !graph.nodes.contains_key(&id) {
                    return Err(AtomRegistryError::Missing(id.clone()));
                }
                let mut affected = BTreeSet::new();
                let mut pending = VecDeque::from([id.clone()]);
                while let Some(current) = pending.pop_front() {
                    if affected.insert(current.clone())
                        && let Some(dependents) = graph.dependents.get(&current)
                    {
                        pending.extend(dependents.iter().cloned());
                    }
                }
                for dependent in affected.iter().filter(|candidate| *candidate != &id) {
                    if let Some(node) = graph.nodes.get_mut(dependent) {
                        node.invalidated = true;
                    }
                }
                Ok(affected
                    .into_iter()
                    .map(|id| id.invalidation_key())
                    .collect::<Vec<_>>())
            },
            std::convert::identity,
        )
        .flat_map({
            let invalidations = self.invalidations.clone();
            move |keys| {
                invalidations
                    .invalidate(keys)
                    .map_error(AtomRegistryError::Invalidation)
                    .map(|_| ())
            }
        })
        .uninterruptible()
    }

    /// Remove the node and all graph edges. Handles become stale; an obsolete
    /// evaluation can no longer pass its generation check.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `id` (`AtomId`): Stable identity of the selected record or operation.
    ///
    /// # Returns
    /// Type: `Effect<bool, AtomRegistryError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn remove<R: Send + Sync + 'static>(
        &self,
        id: AtomId,
    ) -> Effect<bool, AtomRegistryError, R> {
        let registry = self.clone();
        Effect::try_sync(
            move || {
                let mut graph = registry
                    .graph
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if graph.disposed {
                    return Err(AtomRegistryError::Disposed);
                }
                let Some(node) = graph.nodes.remove(&id) else {
                    return Ok(None);
                };
                let mut affected = BTreeSet::new();
                let mut pending = VecDeque::from([id.clone()]);
                while let Some(current) = pending.pop_front() {
                    if affected.insert(current.clone())
                        && let Some(dependents) = graph.dependents.get(&current)
                    {
                        pending.extend(dependents.iter().cloned());
                    }
                }
                for dependent in affected.iter().filter(|candidate| *candidate != &id) {
                    if let Some(node) = graph.nodes.get_mut(dependent) {
                        node.invalidated = true;
                    }
                }
                for dependency in node.dependencies {
                    if let Some(dependents) = graph.dependents.get_mut(&dependency) {
                        dependents.remove(&id);
                    }
                }
                if let Some(dependents) = graph.dependents.remove(&id) {
                    for dependent in dependents {
                        if let Some(node) = graph.nodes.get_mut(&dependent) {
                            node.dependencies.remove(&id);
                        }
                    }
                }
                Ok(Some(
                    affected
                        .into_iter()
                        .map(|atom| atom.invalidation_key())
                        .collect::<Vec<_>>(),
                ))
            },
            std::convert::identity,
        )
        .flat_map({
            let invalidations = self.invalidations.clone();
            move |affected| match affected {
                None => Effect::sync(|| false),
                Some(keys) => invalidations
                    .invalidate(keys)
                    .map_error(AtomRegistryError::RemovalCommittedWithoutInvalidation)
                    .map(|_| true),
            }
        })
        .uninterruptible()
    }

    /// Release owned native state and await its disposal where required.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `mode` (`PubSubShutdown`): Selected behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<bool, AtomRegistryError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn dispose<R: Send + Sync + 'static>(
        &self,
        mode: PubSubShutdown,
    ) -> Effect<bool, AtomRegistryError, R> {
        let registry = self.clone();
        Effect::sync({
            let registry = registry.clone();
            move || {
                let mut graph = registry
                    .graph
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if graph.disposed {
                    return false;
                }
                graph.disposed = true;
                graph.nodes.clear();
                graph.dependents.clear();
                true
            }
        })
        .flat_map(move |first| {
            if first {
                registry
                    .invalidations
                    .shutdown(mode)
                    .map_error(AtomRegistryError::Invalidation)
                    .map(|_| true)
            } else {
                Effect::sync(|| false)
            }
        })
    }
}

///
/// # Parameters
/// - `graph` (`&Graph`): The graph supplied to this operation.
/// - `from` (`&AtomId`): The from supplied to this operation.
/// - `target` (`&AtomId`): The target supplied to this operation.
///
/// # Returns
/// Type: `bool`.
/// Returns whether the requested condition holds.
fn reaches(graph: &Graph, from: &AtomId, target: &AtomId) -> bool {
    let mut visited = BTreeSet::new();
    let mut pending = vec![from.clone()];
    while let Some(current) = pending.pop() {
        if &current == target {
            return true;
        }
        if visited.insert(current.clone())
            && let Some(node) = graph.nodes.get(&current)
        {
            pending.extend(node.dependencies.iter().cloned());
        }
    }
    false
}
