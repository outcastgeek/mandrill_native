//! Native atom graph transitions over finite regional working sets.

use super::{AtomId, AtomKind, AtomRegistryError};
use crate::PubSubShutdown;
use crate::memory::{
    Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use crate::reactivity::{PreparedInvalidationError, PreparedInvalidationRegistry};
use crate::stream::{PreparedChunkPool, PreparedChunkValues};
use std::alloc::Layout;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex, Weak};

/// Physical capacities for the selected atom graph and its retained outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedAtomRegistryCapacity {
    /// Live registrations.
    pub atoms: NonZeroUsize,
    /// Total dependency edges across registrations.
    pub edges: NonZeroUsize,
    /// Maximum distinct dependencies of one registration or evaluation.
    pub dependencies: NonZeroUsize,
    /// Simultaneous graph traversal and change-set reservations.
    pub operations: NonZeroUsize,
    /// Concurrent evaluations retaining their first observed versions.
    pub evaluations: NonZeroUsize,
    /// Simultaneously retained dependency snapshots.
    pub snapshots: NonZeroUsize,
    /// Maximum existing stable identity bytes retained per key.
    pub bytes_per_identity: Bytes,
}

type NodeRecord = (AtomId, PreparedNode);
type Observed = (AtomId, NonZeroU64);
type Observations = FixedVector<'static, Observed>;

impl PreparedAtomRegistryCapacity {
    /// Regional inline records and actual graph/snapshot working sets.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let nodes = layout::<NodeRecord>(self.atoms.get())?;
        let edges = layout::<Edge>(self.edges.get())?;
        let work = layout::<AtomId>(self.atoms.get())?
            .checked_add(layout::<AtomId>(self.dependencies.get())?)?;
        nodes
            .checked_add(edges)?
            .checked_add(SlotPool::<Work>::storage_bytes(self.operations.get())?)?
            .checked_add(work.checked_mul(self.operations.get())?)?
            .checked_add(SlotPool::<Observations>::storage_bytes(
                self.evaluations.get(),
            )?)?
            .checked_add(
                layout::<Observed>(self.dependencies.get())?.checked_mul(self.evaluations.get())?,
            )?
            .checked_add(PreparedChunkPool::<AtomId>::storage_bytes(
                self.snapshots,
                self.dependencies,
            )?)
    }
}

fn layout<T>(count: usize) -> Result<Bytes, MemoryError> {
    Layout::array::<T>(count)
        .map(|layout| Bytes::new(layout.size()))
        .map_err(|_| MemoryError::InvalidLayout)
}

struct PreparedNode {
    kind: AtomKind,
    owner: NonZeroU64,
    version: NonZeroU64,
    evaluation: Option<NonZeroU64>,
    invalidated: bool,
}

struct Edge {
    dependency: AtomId,
    dependent: AtomId,
}

struct PreparedGraph {
    disposed: bool,
    next_stamp: Option<NonZeroU64>,
    nodes: FixedVector<'static, NodeRecord>,
    edges: FixedVector<'static, Edge>,
}

struct Work {
    walk: FixedVector<'static, AtomId>,
    proposed: FixedVector<'static, AtomId>,
}

struct Workspace(SlotLease<'static, Work>);

impl Drop for Workspace {
    fn drop(&mut self) {
        self.0.walk.clear();
        self.0.proposed.clear();
    }
}

/// An exact registration fence. It cannot authorize a replacement with the same ID.
#[derive(Clone)]
pub struct PreparedAtomOwner {
    graph: Weak<Mutex<PreparedGraph>>,
    id: AtomId,
    stamp: NonZeroU64,
}

impl PreparedAtomOwner {
    #[must_use]
    pub fn id(&self) -> &AtomId {
        &self.id
    }
}

/// A current evaluation and its actual bounded observation records.
pub struct PreparedAtomEvaluation {
    owner: PreparedAtomOwner,
    generation: NonZeroU64,
    observed: SlotLease<'static, Observations>,
}

impl Drop for PreparedAtomEvaluation {
    fn drop(&mut self) {
        self.observed.clear();
    }
}

impl PreparedGraph {
    fn position(&self, id: &AtomId) -> Result<usize, usize> {
        self.nodes
            .as_slice()
            .binary_search_by(|(key, _)| key.cmp(id))
    }

    fn node(&self, id: &AtomId) -> Option<&PreparedNode> {
        let position = self.position(id).ok()?;
        self.nodes.as_slice().get(position).map(|(_, node)| node)
    }

    fn node_mut(&mut self, id: &AtomId) -> Option<&mut PreparedNode> {
        let position = self.position(id).ok()?;
        self.nodes
            .as_mut_slice()
            .get_mut(position)
            .map(|(_, node)| node)
    }

    fn stamp(&self, id: &AtomId) -> Result<NonZeroU64, AtomRegistryError> {
        self.next_stamp
            .ok_or_else(|| AtomRegistryError::GenerationExhausted(id.clone()))
    }

    fn advance_stamp(&mut self, stamp: NonZeroU64) {
        self.next_stamp = stamp.get().checked_add(1).and_then(NonZeroU64::new);
    }

    fn ensure_live(&self) -> Result<(), AtomRegistryError> {
        if self.disposed {
            Err(AtomRegistryError::Disposed)
        } else {
            Ok(())
        }
    }

    fn owned(&self, owner: &PreparedAtomOwner) -> Result<&PreparedNode, AtomRegistryError> {
        self.ensure_live()?;
        let node = self
            .node(&owner.id)
            .ok_or_else(|| AtomRegistryError::Missing(owner.id.clone()))?;
        if node.owner != owner.stamp {
            return Err(AtomRegistryError::StaleOwner(owner.id.clone()));
        }
        Ok(node)
    }

    fn validate_edges(
        &self,
        id: &AtomId,
        proposed: &[AtomId],
        walk: &mut FixedVector<'static, AtomId>,
    ) -> Result<(), PreparedAtomRegistryError> {
        for dependency in proposed {
            if self.node(dependency).is_none() {
                return Err(AtomRegistryError::Missing(dependency.clone()).into());
            }
            if dependency != id {
                self.walk(dependency, WalkDirection::Dependencies, walk);
            }
            if dependency == id || walk.as_slice().contains(id) {
                return Err(AtomRegistryError::DependencyCycle {
                    atom: id.clone(),
                    through: dependency.clone(),
                }
                .into());
            }
        }
        let retained = self
            .edges
            .as_slice()
            .iter()
            .filter(|edge| &edge.dependent != id)
            .count();
        if proposed.len() > self.edges.capacity() - retained {
            return Err(MemoryError::CollectionReservationFailed.into());
        }
        Ok(())
    }

    fn install_edges(&mut self, id: &AtomId, proposed: &[AtomId]) {
        self.edges.retain(|edge| &edge.dependent != id);
        for dependency in proposed {
            self.edges.push_reserved(Edge {
                dependency: dependency.clone(),
                dependent: id.clone(),
            });
        }
    }

    // Every traversed endpoint is a live node. A key is added only once, so
    // the node-width workspace never needs to allocate or reject mid-walk.
    fn walk(
        &self,
        start: &AtomId,
        direction: WalkDirection,
        work: &mut FixedVector<'static, AtomId>,
    ) {
        work.clear();
        work.push_reserved(start.clone());
        let mut position = 0;
        while let Some(current) = work.as_slice().get(position).cloned() {
            for edge in self.edges.as_slice() {
                let (from, to) = match direction {
                    WalkDirection::Dependencies => (&edge.dependent, &edge.dependency),
                    WalkDirection::Dependents => (&edge.dependency, &edge.dependent),
                };
                if from == &current && !work.as_slice().contains(to) {
                    work.push_reserved(to.clone());
                }
            }
            position += 1;
        }
        work.as_mut_slice().sort_unstable();
    }

    fn mark_dependents(&mut self, id: &AtomId, affected: &[AtomId]) {
        for dependent in affected.iter().filter(|dependent| *dependent != id) {
            if let Some(node) = self.node_mut(dependent) {
                node.invalidated = true;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum WalkDirection {
    Dependencies,
    Dependents,
}

fn remember<T>(values: &mut FixedVector<'static, T>, value: T) -> Result<(), MemoryError> {
    values
        .push(value)
        .map_err(|_| MemoryError::CollectionReservationFailed)
}

/// Validation, graph storage and notification refusal retain their own typed detail.
#[derive(Debug)]
pub enum PreparedAtomRegistryError {
    Registry(AtomRegistryError),
    Memory(MemoryError),
    Invalidation(PreparedInvalidationError),
}

impl From<AtomRegistryError> for PreparedAtomRegistryError {
    fn from(error: AtomRegistryError) -> Self {
        Self::Registry(error)
    }
}

impl From<MemoryError> for PreparedAtomRegistryError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<PreparedInvalidationError> for PreparedAtomRegistryError {
    fn from(error: PreparedInvalidationError) -> Self {
        Self::Invalidation(error)
    }
}

/// Selected native graph storage for atom registration, dependency tracking and invalidation.
pub struct PreparedAtomRegistry {
    graph: Arc<Mutex<PreparedGraph>>,
    operations: SlotPool<'static, Work>,
    evaluations: SlotPool<'static, Observations>,
    snapshots: PreparedChunkPool<AtomId>,
    invalidations: Arc<PreparedInvalidationRegistry>,
    identity_limit: Bytes,
}

impl PreparedAtomRegistry {
    /// Provision the actual node/edge table, traversal space, evaluations and snapshots.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedAtomRegistryCapacity,
        invalidations: Arc<PreparedInvalidationRegistry>,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        let graph = PreparedGraph {
            disposed: false,
            next_stamp: Some(NonZeroU64::MIN),
            nodes: FixedVector::prepare(memory.allocator(region)?, capacity.atoms.get())?,
            edges: FixedVector::prepare(memory.allocator(region)?, capacity.edges.get())?,
        };
        Ok(Self {
            graph: Arc::new(Mutex::new(graph)),
            operations: memory.slots(region, capacity.operations.get(), |_| {
                Ok(Work {
                    walk: FixedVector::prepare(memory.allocator(region)?, capacity.atoms.get())?,
                    proposed: FixedVector::prepare(
                        memory.allocator(region)?,
                        capacity.dependencies.get(),
                    )?,
                })
            })?,
            evaluations: memory.slots(region, capacity.evaluations.get(), |_| {
                FixedVector::prepare(memory.allocator(region)?, capacity.dependencies.get())
            })?,
            snapshots: PreparedChunkPool::prepare(
                memory,
                region,
                capacity.snapshots,
                capacity.dependencies,
            )?,
            invalidations,
            identity_limit: capacity.bytes_per_identity,
        })
    }

    #[must_use]
    pub fn invalidations(&self) -> &PreparedInvalidationRegistry {
        &self.invalidations
    }

    fn check_owner(&self, owner: &PreparedAtomOwner) -> Result<(), AtomRegistryError> {
        if !owner.graph.ptr_eq(&Arc::downgrade(&self.graph)) {
            return Err(AtomRegistryError::StaleOwner(owner.id.clone()));
        }
        Ok(())
    }

    fn check_identity(&self, id: &AtomId) -> Result<(), MemoryError> {
        let requested = Bytes::new(id.as_bytes().len());
        if requested > self.identity_limit {
            return Err(MemoryError::ValueTooLarge {
                requested,
                capacity: self.identity_limit,
            });
        }
        Ok(())
    }

    /// Publish a fresh generation only after node capacity and identity size have been checked.
    pub fn register(
        &self,
        id: AtomId,
        kind: AtomKind,
    ) -> Result<PreparedAtomOwner, PreparedAtomRegistryError> {
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        graph.ensure_live()?;
        let position = graph
            .position(&id)
            .err()
            .ok_or_else(|| AtomRegistryError::Duplicate(id.clone()))?;
        self.check_identity(&id)?;
        let stamp = graph.stamp(&id)?;
        remember(
            &mut graph.nodes,
            (
                id.clone(),
                PreparedNode {
                    kind,
                    owner: stamp,
                    version: stamp,
                    evaluation: None,
                    invalidated: false,
                },
            ),
        )?;
        if let Some(suffix) = graph.nodes.as_mut_slice().get_mut(position..) {
            suffix.rotate_right(1);
        }
        graph.advance_stamp(stamp);
        Ok(PreparedAtomOwner {
            graph: Arc::downgrade(&self.graph),
            id,
            stamp,
        })
    }

    /// Replace outgoing dependency edges after validating the whole proposed set.
    /// Caller iteration runs outside the graph gate.
    pub fn replace_dependencies(
        &self,
        owner: &PreparedAtomOwner,
        dependencies: impl IntoIterator<Item = AtomId>,
    ) -> Result<(), PreparedAtomRegistryError> {
        self.check_owner(owner)?;
        let mut work = Workspace(self.operations.try_checkout()?);
        for dependency in dependencies {
            self.check_identity(&dependency)?;
            if !work.0.proposed.as_slice().contains(&dependency) {
                remember(&mut work.0.proposed, dependency)?;
            }
        }
        let work = &mut *work.0;
        work.proposed.as_mut_slice().sort_unstable();
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.owned(owner)?.kind != AtomKind::Derived {
            return Err(AtomRegistryError::NotDerived(owner.id.clone()).into());
        }
        graph.validate_edges(&owner.id, work.proposed.as_slice(), &mut work.walk)?;
        graph.install_edges(&owner.id, work.proposed.as_slice());
        Ok(())
    }

    /// Retain a stable copy of one dependency set in a reusable snapshot slot.
    pub fn dependencies_of(
        &self,
        owner: &PreparedAtomOwner,
    ) -> Result<PreparedChunkValues<AtomId>, PreparedAtomRegistryError> {
        self.check_owner(owner)?;
        let graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        graph.owned(owner)?;
        let mut snapshot = self.snapshots.try_checkout()?;
        for edge in graph
            .edges
            .as_slice()
            .iter()
            .filter(|edge| edge.dependent == owner.id)
        {
            snapshot.push_reserved(edge.dependency.clone());
        }
        Ok(snapshot)
    }

    /// Admit observation storage before beginning a new derived generation.
    pub fn begin_evaluation(
        &self,
        owner: &PreparedAtomOwner,
    ) -> Result<PreparedAtomEvaluation, PreparedAtomRegistryError> {
        self.check_owner(owner)?;
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if graph.owned(owner)?.kind != AtomKind::Derived {
            return Err(AtomRegistryError::NotDerived(owner.id.clone()).into());
        }
        let generation = graph.stamp(&owner.id)?;
        let observed = self.evaluations.try_checkout()?;
        let node = graph
            .node_mut(&owner.id)
            .ok_or_else(|| AtomRegistryError::Missing(owner.id.clone()))?;
        node.evaluation = Some(generation);
        node.invalidated = false;
        graph.advance_stamp(generation);
        Ok(PreparedAtomEvaluation {
            owner: owner.clone(),
            generation,
            observed,
        })
    }

    /// Capture the first observed version before reading a dependency value.
    pub fn observe(
        &self,
        evaluation: &mut PreparedAtomEvaluation,
        dependency: &PreparedAtomOwner,
    ) -> Result<(), PreparedAtomRegistryError> {
        self.check_owner(&evaluation.owner)?;
        self.check_owner(dependency)?;
        let graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let version = graph.owned(dependency)?.version;
        if !evaluation
            .observed
            .as_slice()
            .iter()
            .any(|(id, _)| id == &dependency.id)
        {
            remember(&mut evaluation.observed, (dependency.id.clone(), version))?;
        }
        Ok(())
    }

    /// Check the exact generation without replacing any dependency or value.
    #[must_use]
    pub fn is_current(&self, evaluation: &PreparedAtomEvaluation) -> bool {
        if self.check_owner(&evaluation.owner).is_err() {
            return false;
        }
        let graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        graph
            .owned(&evaluation.owner)
            .is_ok_and(|node| !node.invalidated && node.evaluation == Some(evaluation.generation))
    }

    /// Compare the generation and observations, then publish the selected edge transition.
    /// A raced evaluation installs tracked edges, marks itself invalidated and returns false.
    pub fn commit_evaluation(
        &self,
        evaluation: PreparedAtomEvaluation,
    ) -> Result<bool, PreparedAtomRegistryError> {
        self.commit_evaluation_with(evaluation, true)
    }

    /// Publish a current failure or other terminal outcome while preserving
    /// the previously installed dependencies. Observations from the failed
    /// evaluation are returned with its slot.
    pub fn commit_evaluation_without_dependencies(
        &self,
        evaluation: PreparedAtomEvaluation,
    ) -> Result<bool, PreparedAtomRegistryError> {
        self.commit_evaluation_with(evaluation, false)
    }

    fn commit_evaluation_with(
        &self,
        evaluation: PreparedAtomEvaluation,
        install_dependencies: bool,
    ) -> Result<bool, PreparedAtomRegistryError> {
        self.check_owner(&evaluation.owner)?;
        let id = &evaluation.owner.id;
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        graph.ensure_live()?;
        let node = graph
            .node(id)
            .ok_or_else(|| AtomRegistryError::Missing(id.clone()))?;
        if node.owner != evaluation.owner.stamp
            || node.evaluation != Some(evaluation.generation)
            || node.invalidated
        {
            return Ok(false);
        }
        let mut work = Workspace(self.operations.try_checkout()?);
        let work = &mut *work.0;
        if install_dependencies {
            for (dependency, _) in evaluation.observed.as_slice() {
                work.proposed.push_reserved(dependency.clone());
            }
            work.proposed.as_mut_slice().sort_unstable();
            graph.validate_edges(id, work.proposed.as_slice(), &mut work.walk)?;
        }
        let proposed = work.proposed.as_slice();
        let current = !install_dependencies
            || evaluation
                .observed
                .as_slice()
                .iter()
                .all(|(dependency, version)| {
                    graph
                        .node(dependency)
                        .is_some_and(|node| node.version == *version)
                });
        if !current {
            self.invalidations.publish_guarded(
                proposed.iter().map(AtomId::invalidation_key),
                move || {
                    graph.install_edges(id, proposed);
                    if let Some(node) = graph.node_mut(id) {
                        node.invalidated = true;
                    }
                    drop(graph);
                },
            )?;
            return Ok(false);
        }
        let stamp = graph.stamp(id)?;
        graph.walk(id, WalkDirection::Dependents, &mut work.walk);
        let affected = work.walk.as_slice();
        self.invalidations.publish_guarded(
            affected.iter().map(AtomId::invalidation_key),
            move || {
                if install_dependencies {
                    graph.install_edges(id, proposed);
                }
                mark_change(&mut graph, id, stamp, affected);
                drop(graph);
            },
        )?;
        Ok(true)
    }

    /// Advance one owner and mark all transitive dependents after reserving fanout.
    pub fn changed(&self, owner: &PreparedAtomOwner) -> Result<(), PreparedAtomRegistryError> {
        self.check_owner(owner)?;
        let id = &owner.id;
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        graph.owned(owner)?;
        let stamp = graph.stamp(id)?;
        let mut work = Workspace(self.operations.try_checkout()?);
        graph.walk(id, WalkDirection::Dependents, &mut work.0.walk);
        let affected = work.0.walk.as_slice();
        self.invalidations.publish_guarded(
            affected.iter().map(AtomId::invalidation_key),
            move || {
                mark_change(&mut graph, id, stamp, affected);
                drop(graph);
            },
        )?;
        Ok(())
    }

    /// Remove only this live registration, preserving replacement owners and notification order.
    pub fn remove(&self, owner: &PreparedAtomOwner) -> Result<bool, PreparedAtomRegistryError> {
        self.check_owner(owner)?;
        let id = &owner.id;
        let mut graph = self
            .graph
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        graph.ensure_live()?;
        let Ok(position) = graph.position(id) else {
            return Ok(false);
        };
        if graph.node(id).is_none_or(|node| node.owner != owner.stamp) {
            return Ok(false);
        }
        let mut work = Workspace(self.operations.try_checkout()?);
        graph.walk(id, WalkDirection::Dependents, &mut work.0.walk);
        let affected = work.0.walk.as_slice();
        self.invalidations.publish_guarded(
            affected.iter().map(AtomId::invalidation_key),
            move || {
                graph.mark_dependents(id, affected);
                graph
                    .edges
                    .retain(|edge| &edge.dependency != id && &edge.dependent != id);
                drop(graph.nodes.remove_at(position));
                drop(graph);
            },
        )?;
        Ok(true)
    }

    /// Stop new graph work and settle selected invalidation delivery outside the graph gate.
    pub fn dispose(&self, mode: PubSubShutdown) -> Result<bool, PreparedAtomRegistryError> {
        {
            let mut graph = self
                .graph
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if graph.disposed {
                return Ok(false);
            }
            graph.disposed = true;
            graph.edges.clear();
            graph.nodes.clear();
        }
        self.invalidations.shutdown_now(mode)?;
        Ok(true)
    }
}

fn mark_change(graph: &mut PreparedGraph, id: &AtomId, stamp: NonZeroU64, affected: &[AtomId]) {
    if let Some(node) = graph.node_mut(id) {
        node.version = stamp;
    }
    graph.advance_stamp(stamp);
    graph.mark_dependents(id, affected);
}
