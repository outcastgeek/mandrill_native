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

use std::collections::BTreeMap;
use std::fmt;

use super::Registry;
use super::builders::normalize_resource_definition;
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
    definitions: BTreeMap<ResourceKind, ResourceDefinition>,
    order: Vec<ResourceKind>,
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
        Self { definitions, order }
    }
}

impl Registry for DefinitionRegistry {
    fn definition(&self, kind: &ResourceKind) -> Result<ResourceDefinition, UnknownResourceKind> {
        match self.definitions.get(kind) {
            Some(definition) => Ok(lookup_copy(definition)),
            None => Err(UnknownResourceKind::new(kind.clone())),
        }
    }

    fn definitions(&self) -> Vec<ResourceDefinition> {
        let mut definitions = Vec::with_capacity(self.order.len());
        for kind in &self.order {
            if let Some(definition) = self.definitions.get(kind) {
                definitions.push(lookup_copy(definition));
            }
        }
        definitions
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
