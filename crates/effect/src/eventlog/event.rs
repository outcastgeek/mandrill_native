use crate::{KeyDomain, StableKey, UtcMillis};
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventIdentityError {
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventGroupId(Arc<str>);

impl EventGroupId {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, EventIdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(EventIdentityError::Empty)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventTag(Arc<str>);

impl EventTag {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, EventIdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(EventIdentityError::Empty)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! event_key {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Arc<[u8]>);
        impl $name {
            #[must_use]
            pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
                Self(key.into_bytes())
            }
            #[must_use]
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }
        }
    };
}

event_key!(AggregateId);
event_key!(EventId);
event_key!(EventJournalId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventVersion(NonZeroU32);

impl EventVersion {
    #[must_use]
    pub const fn new(value: NonZeroU32) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn value(self) -> NonZeroU32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JournalCursor(u64);

impl JournalCursor {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventEnvelope {
    pub journal: EventJournalId,
    pub aggregate: AggregateId,
    pub id: EventId,
    pub group: EventGroupId,
    pub tag: EventTag,
    pub version: EventVersion,
    pub payload: Arc<[u8]>,
    pub outcome: Option<EncodedEventOutcome>,
    pub occurred_at: UtcMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventOutcomeKind {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedEventOutcome {
    pub kind: EventOutcomeKind,
    pub payload: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub sequence: NonZeroU64,
    pub event: EventEnvelope,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JournalStream {
    pub journal: EventJournalId,
    pub aggregate: AggregateId,
}

impl EventEnvelope {
    #[must_use]
    pub fn stream(&self) -> JournalStream {
        JournalStream {
            journal: self.journal.clone(),
            aggregate: self.aggregate.clone(),
        }
    }
}
