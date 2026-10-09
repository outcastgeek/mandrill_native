//! Registration of distinct event tags and versions.

use super::{EventDefinition, EventGroupId, EventTag, EventVersion};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
/// An invalid stable group identity or incompatible group definition.
pub enum EventGroupError {
    /// The group differs from the required or recorded value.
    GroupMismatch,
    /// The tag already exists or occurred twice.
    DuplicateTag {
        /// The event or protocol tag. Type: `EventTag`.
        tag: EventTag,
        /// The declared compatibility version. Type: `EventVersion`.
        version: EventVersion,
    },
}

/// A checked registry of application-owned event tags and revisions.
pub struct EventGroup {
    id: EventGroupId,
    events: BTreeMap<(EventTag, EventVersion), ()>,
}

impl EventGroup {
    ///
    /// # Parameters
    /// - `id` (`EventGroupId`): Stable identity of the selected record or operation.
    ///
    /// # Returns
    /// Type: `Self`.
    fn empty(id: EventGroupId) -> Self {
        Self {
            id,
            events: BTreeMap::new(),
        }
    }

    /// Derive the stable group identity from a registered event definition.
    ///
    /// # Type parameters
    /// - `P`: Payload type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `id` (`EventGroupId`): Stable identity of the selected record or operation.
    /// - `first` (`&EventDefinition<P, A, E>`): First value or branch in the ordered composition.
    ///
    /// # Returns
    /// Type: `Result<Self, EventGroupError>`.
    pub fn from_event<P, A, E>(
        id: EventGroupId,
        first: &EventDefinition<P, A, E>,
    ) -> Result<Self, EventGroupError> {
        let mut group = Self::empty(id);
        group.register(first)?;
        Ok(group)
    }

    /// Register the supplied definition, rejecting an identity or compatibility conflict.
    ///
    /// # Type parameters
    /// - `P`: Payload type.
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `definition` (`&EventDefinition<P, A, E>`): The typed definition used by this operation.
    ///
    /// # Returns
    /// Type: `Result<(), EventGroupError>`.
    pub fn register<P, A, E>(
        &mut self,
        definition: &EventDefinition<P, A, E>,
    ) -> Result<(), EventGroupError> {
        if definition.group != self.id {
            return Err(EventGroupError::GroupMismatch);
        }
        let key = (definition.tag.clone(), definition.version);
        if self.events.insert(key.clone(), ()).is_some() {
            return Err(EventGroupError::DuplicateTag {
                tag: key.0,
                version: key.1,
            });
        }
        Ok(())
    }

    #[must_use]
    /// Return the stable identity associated with this value.
    ///
    /// # Returns
    /// Type: `&EventGroupId`.
    pub fn id(&self) -> &EventGroupId {
        &self.id
    }

    #[must_use]
    /// Check whether the selected identity or value is present.
    ///
    /// # Parameters
    /// - `tag` (`&EventTag`): The tag supplied to this operation.
    /// - `version` (`EventVersion`): Registered schema, event, or workflow compatibility version.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub fn contains(&self, tag: &EventTag, version: EventVersion) -> bool {
        self.events.contains_key(&(tag.clone(), version))
    }
}

impl EventGroup {
    /// Start a checked group from an ordinary, aggregate-bound or handled definition view.
    /// Handler access stays on the original typed value; grouping stores only accepted identities.
    pub fn from_definition<D: super::EventDefinitionView>(
        group: EventGroupId,
        definition: &D,
    ) -> Result<Self, EventGroupError> {
        Self::from_event(group, definition.definition())
    }
    /// Register the authoritative definition view, retaining current duplicate/group validation.
    pub fn register_definition<D: super::EventDefinitionView>(
        &mut self,
        definition: &D,
    ) -> Result<(), EventGroupError> {
        self.register(definition.definition())
    }
}
