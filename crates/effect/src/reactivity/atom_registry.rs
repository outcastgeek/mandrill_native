//! Scoped atom evaluation, dependency tracking, and disposal.

use super::{InvalidationError, InvalidationKey, InvalidationRegistry};
use crate::{Effect, KeyDomain, PubSubOptions, PubSubShutdown, StableKey};
use crate::{
    effect, effect_map, effect_map_error, effect_sync, effect_try_sync, effect_uninterruptible,
};
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
    /// The handle belongs to an earlier registration of this ID; it cannot mutate or read dependency metadata of a replacement.
    StaleOwner(AtomId),
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

/// Graph metadata only: owner fences replacement handles; version detects dependency changes independently of evaluation generation.
struct Node {
    kind: AtomKind,
    dependencies: BTreeSet<AtomId>,
    generation: Option<std::num::NonZeroU64>,
    invalidated: bool,
    owner: Arc<()>,
    version: Arc<()>,
}

/// Typed graph used by reactivity atom registry.
struct Graph {
    disposed: bool,
    next_generation: Option<std::num::NonZeroU64>,
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
                next_generation: Some(std::num::NonZeroU64::MIN),
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

    /// Register a currently absent source/derived ID with fresh owner and version identities; duplicate or disposed registration fails.
    pub fn register(&self, id: AtomId, kind: AtomKind) -> Result<(), AtomRegistryError> {
        self.register_owned(id, kind).map(|_| ())
    }

    pub(crate) fn register_owned(
        &self,
        id: AtomId,
        kind: AtomKind,
    ) -> Result<Arc<()>, AtomRegistryError> {
        let owner = Arc::new(());
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
                owner: Arc::clone(&owner),
                version: Arc::new(()),
            },
        );
        graph.dependents.entry(id).or_default();
        Ok(owner)
    }

    /// Read a dependency version only when the handle still owns this registration, under the same graph gate.
    pub(crate) fn dependency_version(
        &self,
        id: &AtomId,
        owner: &Arc<()>,
    ) -> Result<Arc<()>, AtomRegistryError> {
        let graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.disposed {
            return Err(AtomRegistryError::Disposed);
        }
        let node = graph
            .nodes
            .get(id)
            .ok_or_else(|| AtomRegistryError::Missing(id.clone()))?;
        if !Arc::ptr_eq(&node.owner, owner) {
            return Err(AtomRegistryError::StaleOwner(id.clone()));
        }
        Ok(Arc::clone(&node.version))
    }
    /// Replace the current derived ID's dependency set only after existence and cycle checks.
    ///
    /// This registry-authorized operation addresses the current ID. A failed check
    /// leaves prior edges intact; evaluators use generation/version-checked commit
    /// instead to prevent stale builds replacing dependencies.
    pub fn replace_dependencies(
        &self,
        id: &AtomId,
        dependencies: impl IntoIterator<Item = AtomId>,
    ) -> Result<(), AtomRegistryError> {
        self.replace_dependencies_checked(id, None, dependencies, None)
            .map(|_| ())
    }
    fn replace_dependencies_checked(
        &self,
        id: &AtomId,
        generation: Option<std::num::NonZeroU64>,
        dependencies: impl IntoIterator<Item = AtomId>,
        observed: Option<&BTreeMap<AtomId, Arc<()>>>,
    ) -> Result<bool, AtomRegistryError> {
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
        if generation
            .is_some_and(|generation| node.invalidated || node.generation != Some(generation))
        {
            return Ok(false);
        }
        if observed.is_some_and(|observed| {
            observed.iter().any(|(id, version)| {
                graph
                    .nodes
                    .get(id)
                    .is_none_or(|node| !Arc::ptr_eq(&node.version, version))
            })
        }) {
            return Ok(false);
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
        Ok(true)
    }

    pub(crate) fn owns_evaluation(&self, id: &AtomId, generation: std::num::NonZeroU64) -> bool {
        let graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !graph.disposed
            && graph
                .nodes
                .get(id)
                .is_some_and(|node| node.generation == Some(generation))
    }
    // Called by a trusted publication guard after the PubSub capacity decision.
    // No application callback or await occurs while the graph gate is held.
    /// Atomically compare generation and observed dependency versions, then install
    /// observed edges and commit or mark the build invalidated. Returns notification
    /// keys for publication outside the graph gate; no application callbacks run here.
    pub(crate) fn commit_evaluation(
        &self,
        id: &AtomId,
        generation: std::num::NonZeroU64,
        observed: Option<&BTreeMap<AtomId, Arc<()>>>,
    ) -> Result<(bool, Vec<InvalidationKey>), AtomRegistryError> {
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.disposed {
            return Err(AtomRegistryError::Disposed);
        }
        let node = graph
            .nodes
            .get(id)
            .ok_or_else(|| AtomRegistryError::Missing(id.clone()))?;
        if node.generation != Some(generation) || node.invalidated {
            return Ok((false, Vec::new()));
        }
        let current = observed.is_none_or(|observed| {
            observed.iter().all(|(id, version)| {
                graph
                    .nodes
                    .get(id)
                    .is_some_and(|node| Arc::ptr_eq(version, &node.version))
            })
        });
        if let Some(observed) = observed {
            let proposed = observed.keys().cloned().collect::<BTreeSet<_>>();
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
            let node = graph
                .nodes
                .get_mut(id)
                .ok_or_else(|| AtomRegistryError::Missing(id.clone()))?;
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
            if !current {
                if let Some(node) = graph.nodes.get_mut(id) {
                    node.invalidated = true;
                }
                return Ok((
                    false,
                    proposed
                        .into_iter()
                        .map(|id| id.invalidation_key())
                        .collect(),
                ));
            }
        }
        Ok((true, commit_change(&mut graph, id)?))
    }
    /// Check source owner identity and advance its version/invalidation metadata atomically before guarded value publication.
    pub(crate) fn commit_source(
        &self,
        id: &AtomId,
        owner: &Arc<()>,
    ) -> Result<Vec<InvalidationKey>, AtomRegistryError> {
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.disposed {
            return Err(AtomRegistryError::Disposed);
        }
        let node = graph
            .nodes
            .get(id)
            .ok_or_else(|| AtomRegistryError::Missing(id.clone()))?;
        if !Arc::ptr_eq(&node.owner, owner) {
            return Err(AtomRegistryError::StaleOwner(id.clone()));
        }
        commit_change(&mut graph, id)
    }
    pub(crate) fn notify<R: Send + Sync + 'static>(
        &self,
        keys: Vec<InvalidationKey>,
    ) -> Effect<(), AtomRegistryError, R> {
        effect_map! { program: effect_map_error! { program: self.invalidations
        .invalidate(keys), map: AtomRegistryError::Invalidation, }, map: |_| (), }
    }
    /// Allocate a globally fresh evaluation generation for a registered derived ID.
    ///
    /// This registry-authorized operation addresses the current registration by ID.
    /// Derived handles additionally check their owner identity. Missing, disposed,
    /// non-derived or exhausted registrations fail; completion must compare the token
    /// before publishing state or dependency changes.
    pub fn begin_evaluation(&self, id: &AtomId) -> Result<std::num::NonZeroU64, AtomRegistryError> {
        self.begin_evaluation_for(id, None)
    }
    pub(crate) fn begin_evaluation_owned(
        &self,
        id: &AtomId,
        owner: &Arc<()>,
    ) -> Result<std::num::NonZeroU64, AtomRegistryError> {
        self.begin_evaluation_for(id, Some(owner))
    }
    fn begin_evaluation_for(
        &self,
        id: &AtomId,
        owner: Option<&Arc<()>>,
    ) -> Result<std::num::NonZeroU64, AtomRegistryError> {
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.disposed {
            return Err(AtomRegistryError::Disposed);
        }
        let node = graph
            .nodes
            .get(id)
            .ok_or_else(|| AtomRegistryError::Missing(id.clone()))?;
        if owner.is_some_and(|owner| !Arc::ptr_eq(&node.owner, owner)) {
            return Err(AtomRegistryError::StaleOwner(id.clone()));
        }
        let generation = graph
            .next_generation
            .ok_or_else(|| AtomRegistryError::GenerationExhausted(id.clone()))?;
        graph.next_generation = generation
            .get()
            .checked_add(1)
            .and_then(std::num::NonZeroU64::new);
        let node = graph
            .nodes
            .get_mut(id)
            .ok_or_else(|| AtomRegistryError::Missing(id.clone()))?;
        if node.kind != AtomKind::Derived {
            return Err(AtomRegistryError::NotDerived(id.clone()));
        }
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

    /// Lazily advance an ID's version and invalidate its transitive dependents.
    ///
    /// This registry-authorized ID operation addresses the current registration.
    /// Graph changes commit under the gate; subscriber notification occurs afterward
    /// and may fail separately from the committed metadata update.
    pub fn changed<R: Send + Sync + 'static>(
        &self,
        id: AtomId,
    ) -> Effect<(), AtomRegistryError, R> {
        let registry = self.clone();
        effect_uninterruptible! { program: { let __dsl_program = effect_try_sync! { body: move || {
            let mut graph = registry
                .graph
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if graph.disposed {
                return Err(AtomRegistryError::Disposed);
            }
            commit_change(&mut graph, &id)
        }, on_error: std::convert::identity, }; let invalidations = self.invalidations.clone(); effect! { let __dsl_program = __dsl_program.clone(); let invalidations = invalidations.clone(); let keys = __dsl_program?; {
            effect_map! { program: effect_map_error! { program: invalidations
                .invalidate(keys), map: AtomRegistryError::Invalidation, }, map: |_| (), }
        }? } }, }
    }

    /// Lazily remove the current registration by ID and invalidate its dependents.
    ///
    /// Handles use separate owner-checked removal so an old disposer cannot remove a
    /// replacement. Removal may commit before invalidation publication fails; that
    /// case is reported as RemovalCommittedWithoutInvalidation.
    pub fn remove<R: Send + Sync + 'static>(
        &self,
        id: AtomId,
    ) -> Effect<bool, AtomRegistryError, R> {
        self.remove_owned(id, None)
    }
    pub(crate) fn remove_owned<R: Send + Sync + 'static>(
        &self,
        id: AtomId,
        owner: Option<Arc<()>>,
    ) -> Effect<bool, AtomRegistryError, R> {
        let registry = self.clone();
        effect_uninterruptible! { program: { let __dsl_program = effect_try_sync! { body: move || {
            let mut graph = registry
                .graph
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if graph.disposed {
                return Err(AtomRegistryError::Disposed);
            }
            if owner.as_ref().is_some_and(|owner| {
                graph
                    .nodes
                    .get(&id)
                    .is_none_or(|node| !Arc::ptr_eq(owner, &node.owner))
            }) {
                return Ok(None);
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
        }, on_error: std::convert::identity, }; let invalidations = self.invalidations.clone(); effect! { let __dsl_program = __dsl_program.clone(); let invalidations = invalidations.clone(); let affected = __dsl_program?; match affected {
            None => effect_sync! { body: || false, },
            Some(keys) => effect_map! { program: effect_map_error! { program: invalidations
                .invalidate(keys), map: AtomRegistryError::RemovalCommittedWithoutInvalidation, }, map: |_| true, },
        }? } }, }
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
        {
            let __dsl_program = effect_sync! { body: {
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
            }, };
            effect! { let registry = registry.clone(); let first = __dsl_program.clone()?; {
                if first {
                    effect_map! { program: effect_map_error! { program: registry
                        .invalidations
                        .shutdown(mode), map: AtomRegistryError::Invalidation, }, map: |_| true, }
                } else {
                    effect_sync! { body: || false, }
                }
            }? }
        }
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

fn commit_change(
    graph: &mut Graph,
    id: &AtomId,
) -> Result<Vec<InvalidationKey>, AtomRegistryError> {
    let node = graph
        .nodes
        .get_mut(id)
        .ok_or_else(|| AtomRegistryError::Missing(id.clone()))?;
    node.version = Arc::new(());
    let mut affected = BTreeSet::new();
    let mut pending = VecDeque::from([id.clone()]);
    while let Some(current) = pending.pop_front() {
        if affected.insert(current.clone())
            && let Some(dependents) = graph.dependents.get(&current)
        {
            pending.extend(dependents.iter().cloned());
        }
    }
    for dependent in affected.iter().filter(|candidate| *candidate != id) {
        if let Some(node) = graph.nodes.get_mut(dependent) {
            node.invalidated = true;
        }
    }
    Ok(affected
        .into_iter()
        .map(|id| id.invalidation_key())
        .collect())
}
