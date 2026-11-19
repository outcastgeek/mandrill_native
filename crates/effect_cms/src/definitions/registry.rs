//! The concrete resource definition registry.
//!
//! Registration applies these rules: an empty kind is skipped, every
//! other definition is normalized by the builder owner, a repeated kind replaces
//! the stored definition while the kind keeps appearing in insertion order, and
//! every lookup returns an independent outer copy whose relation declarations are
//! detached and whose model collections stay shared.
//!
//! Registration performs no I/O, opens no store and holds no capability: a
//! definition is caller-supplied data. Consumers that need a lookup inside an
//! effectful workflow lift these pure functions at their own boundary.

use effect::memory::{Bytes, FixedVector, MemoryError, PreparedMemory, RegionId};
use std::alloc::Layout;
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::ControlFlow;
use std::sync::Arc;

use super::Registry;
use super::builders::{normalize_owned_resource_definition, normalize_resource_definition};
use super::demand::DefinitionDemand;
use super::model::ResourceDefinition;
use super::vocabulary::ResourceKind;

/// A lookup of a kind that was never registered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownResourceKind {
    kind: ResourceKind,
}

impl UnknownResourceKind {
    pub(crate) fn new(kind: ResourceKind) -> Self {
        Self { kind }
    }

    /// The kind that was not registered.
    #[must_use]
    pub fn kind(&self) -> &ResourceKind {
        &self.kind
    }
}

impl fmt::Display for UnknownResourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "contentmgmt: unknown resource kind: {}",
            self.kind.as_str()
        )
    }
}

impl std::error::Error for UnknownResourceKind {}

/// The concrete definition registry produced by this owner.
///
/// There is no public field, no setter and no `Default`: the registry exists only
/// through its constructor, which registers the complete supplied definition set.
#[derive(Clone, Debug)]
pub struct DefinitionRegistry {
    registrations: Registrations,
}

/// Startup capacities for registration records and declared resource metadata.
#[derive(Clone, Copy, Debug)]
pub struct PreparedDefinitionCapacity {
    /// Maximum declarations, including repeated registrations.
    pub registrations: NonZeroUsize,
    /// Cumulative admitted bytes of declared metadata and child descriptors.
    pub metadata: NonZeroUsize,
}

#[derive(Clone, Debug)]
enum Registrations {
    Ordinary {
        definitions: BTreeMap<ResourceKind, ResourceDefinition>,
        order: Vec<ResourceKind>,
    },
    Prepared(Arc<PreparedRegistrations>),
}

struct PreparedRegistrations {
    definitions: FixedVector<'static, ResourceDefinition>,
    order: FixedVector<'static, usize>,
}

impl fmt::Debug for PreparedRegistrations {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("PreparedRegistrations")
            .field("definitions", &self.definitions.as_slice())
            .field("order", &self.order.as_slice())
            .finish()
    }
}

impl DefinitionRegistry {
    /// Register the supplied definitions.
    ///
    /// An empty kind is skipped without being normalized. A repeated kind keeps its
    /// insertion position and resolves to the last stored definition.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = ResourceDefinition>) -> Self {
        let mut definitions = BTreeMap::new();
        let mut order = Vec::new();
        for definition in values {
            if definition.kind.as_str().is_empty() {
                continue;
            }
            let definition = normalize_resource_definition(definition);
            let kind = definition.kind.clone();
            definitions.insert(kind.clone(), definition);
            order.push(kind);
        }
        Self {
            registrations: Registrations::Ordinary { definitions, order },
        }
    }

    /// Region payload for the fixed definition and registration-order tables.
    pub fn prepared_storage_bytes(
        capacity: PreparedDefinitionCapacity,
    ) -> Result<Bytes, MemoryError> {
        let definitions = Layout::array::<ResourceDefinition>(capacity.registrations.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let order = Layout::array::<usize>(capacity.registrations.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        Bytes::new(definitions.size()).checked_add(Bytes::new(order.size()))
    }

    /// Move the complete registration into fixed tables in the selected startup
    /// region. Caller-owned definition payloads move into the table. Borrowed
    /// visits reuse that storage and compatibility lookups return detached copies.
    /// Empty kinds are skipped and repeated registrations count toward both capacities.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedDefinitionCapacity,
        values: impl IntoIterator<Item = ResourceDefinition>,
    ) -> Result<Self, MemoryError> {
        let mut definitions: FixedVector<'static, ResourceDefinition> =
            FixedVector::prepare(memory.allocator(region)?, capacity.registrations.get())?;
        let mut order =
            FixedVector::prepare(memory.allocator(region)?, capacity.registrations.get())?;
        let mut demand = DefinitionDemand::new(capacity.metadata.get());
        for definition in values {
            if definition.kind.as_str().is_empty() {
                continue;
            }
            demand.account(&definition)?;
            let position = definitions
                .as_slice()
                .iter()
                .position(|current| current.kind == definition.kind);
            let index = position.unwrap_or(definitions.len());
            order.push(index).map_err(|_| MemoryError::NoSlot)?;
            let definition = normalize_owned_resource_definition(definition);
            if let Some(current) = definitions.as_mut_slice().get_mut(index) {
                *current = definition;
            } else {
                definitions
                    .push(definition)
                    .map_err(|_| MemoryError::NoSlot)?;
            }
        }
        Ok(Self {
            registrations: Registrations::Prepared(Arc::new(PreparedRegistrations {
                definitions,
                order,
            })),
        })
    }

    fn find(&self, kind: &ResourceKind) -> Option<&ResourceDefinition> {
        match &self.registrations {
            Registrations::Ordinary { definitions, .. } => definitions.get(kind),
            Registrations::Prepared(registry) => registry
                .definitions
                .as_slice()
                .iter()
                .find(|definition| definition.kind == *kind),
        }
    }
}

impl Registry for DefinitionRegistry {
    fn definition(&self, kind: &ResourceKind) -> Result<ResourceDefinition, UnknownResourceKind> {
        match self.find(kind) {
            Some(definition) => Ok(lookup_copy(definition)),
            None => Err(UnknownResourceKind::new(kind.clone())),
        }
    }

    fn definitions(&self) -> Vec<ResourceDefinition> {
        let count = match &self.registrations {
            Registrations::Ordinary { order, .. } => order.len(),
            Registrations::Prepared(registry) => registry.order.len(),
        };
        let mut definitions = Vec::with_capacity(count);
        let _ = self.visit_definitions(&mut |definition| {
            definitions.push(lookup_copy(definition));
            ControlFlow::Continue(())
        });
        definitions
    }

    fn visit_definition(
        &self,
        kind: &ResourceKind,
        visit: &mut dyn FnMut(&ResourceDefinition),
    ) -> Result<(), UnknownResourceKind> {
        let definition = self
            .find(kind)
            .ok_or_else(|| UnknownResourceKind::new(kind.clone()))?;
        visit(definition);
        Ok(())
    }

    fn visit_definitions(
        &self,
        visit: &mut dyn FnMut(&ResourceDefinition) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        match &self.registrations {
            Registrations::Ordinary { definitions, order } => {
                for definition in order.iter().filter_map(|kind| definitions.get(kind)) {
                    visit(definition)?;
                }
            }
            Registrations::Prepared(registry) => {
                for definition in registry
                    .order
                    .as_slice()
                    .iter()
                    .filter_map(|index| registry.definitions.as_slice().get(*index))
                {
                    visit(definition)?;
                }
            }
        }
        ControlFlow::Continue(())
    }

    fn visit_prepared_definitions(
        &self,
        visit: &mut dyn FnMut(&ResourceDefinition) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, MemoryError> {
        Ok(self.visit_definitions(visit))
    }
}

/// The outer copy a lookup returns.
///
/// A value copy detaches the relation declarations and copies the model's shared
/// collection headers (list backing storage with this copy's own header, shared
/// maps). Replacing outer fields does not alter the stored definition. Appending
/// changes only this header's length, but an in-capacity append can overwrite shared
/// backing elements visible through other headers. In-place element and map mutations
/// are observable in both directions.
fn lookup_copy(definition: &ResourceDefinition) -> ResourceDefinition {
    definition.clone()
}
