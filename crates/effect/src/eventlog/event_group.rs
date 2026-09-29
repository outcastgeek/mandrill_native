use super::{EventDefinition, EventGroupId, EventTag, EventVersion};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventGroupError {
    GroupMismatch,
    DuplicateTag {
        tag: EventTag,
        version: EventVersion,
    },
}

/// A checked registry of application-owned event tags and revisions.
pub struct EventGroup {
    id: EventGroupId,
    events: BTreeMap<(EventTag, EventVersion), ()>,
}

impl EventGroup {
    fn empty(id: EventGroupId) -> Self {
        Self {
            id,
            events: BTreeMap::new(),
        }
    }

    pub fn from_event<P, A, E>(
        id: EventGroupId,
        first: &EventDefinition<P, A, E>,
    ) -> Result<Self, EventGroupError> {
        let mut group = Self::empty(id);
        group.register(first)?;
        Ok(group)
    }

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
    pub fn id(&self) -> &EventGroupId {
        &self.id
    }

    #[must_use]
    pub fn contains(&self, tag: &EventTag, version: EventVersion) -> bool {
        self.events.contains_key(&(tag.clone(), version))
    }
}
