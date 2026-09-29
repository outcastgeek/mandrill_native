//! Typed event definitions and stable event identities.

use crate::{KeyDomain, StableKey, UtcMillis};
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// An invalid stable event identity, tag, group, or version.
pub enum EventIdentityError {
    /// No value is present.
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by eventlog event operations.
pub struct EventGroupId(Arc<str>);

impl EventGroupId {
    /// Construct a `EventGroupId` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `value` (`impl Into<Arc<str>>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, EventIdentityError>`.
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, EventIdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(EventIdentityError::Empty)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    /// Borrow the textual representation without allocating a new string.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// A validated stable discriminator for a typed event.
pub struct EventTag(Arc<str>);

impl EventTag {
    /// Construct a `EventTag` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `value` (`impl Into<Arc<str>>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, EventIdentityError>`.
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, EventIdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(EventIdentityError::Empty)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    /// Borrow the textual representation without allocating a new string.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! event_key {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        /// Stable EventLog identity encoded as validated, namespaced bytes.
        pub struct $name(Arc<[u8]>);
        impl $name {
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
                Self(key.into_bytes())
            }
            #[must_use]
            /// Borrow the stable encoded bytes without transferring ownership.
            ///
            /// # Returns
            /// Type: `&[u8]`.
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
/// The validated encoded version of an event definition.
pub struct EventVersion(NonZeroU32);

impl EventVersion {
    #[must_use]
    /// # Parameters
    /// - `value` (`NonZeroU32`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(value: NonZeroU32) -> Self {
        Self(value)
    }
    #[must_use]
    /// # Returns
    /// Type: `NonZeroU32`.
    pub const fn value(self) -> NonZeroU32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The ordered stream position used to fence writers and resume readers.
pub struct JournalCursor(u64);

impl JournalCursor {
    #[must_use]
    /// # Parameters
    /// - `value` (`u64`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    #[must_use]
    /// # Returns
    /// Type: `u64`.
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// An event identity, tag, version, and encoded payload ready for append.
pub struct EventEnvelope {
    /// Journal that owns the authoritative records. Type: `EventJournalId`.
    pub journal: EventJournalId,
    /// Aggregate identity or accumulated metric data associated with this record. Type: `AggregateId`.
    pub aggregate: AggregateId,
    /// Stable identity of this record. Type: `EventId`.
    pub id: EventId,
    /// Stable event or request group identity. Type: `EventGroupId`.
    pub group: EventGroupId,
    /// The event or protocol tag. Type: `EventTag`.
    pub tag: EventTag,
    /// The declared compatibility version. Type: `EventVersion`.
    pub version: EventVersion,
    /// The payload carried by this request or record. Type: `Arc<[u8]>`.
    pub payload: Arc<[u8]>,
    /// The recorded outcome of this operation. Type: `Option<EncodedEventOutcome>`.
    pub outcome: Option<EncodedEventOutcome>,
    /// The time at which the occurred occurred. Type: `UtcMillis`.
    pub occurred_at: UtcMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The terminal category encoded for an event invocation.
pub enum EventOutcomeKind {
    /// The operation completed successfully.
    Succeeded,
    /// The operation completed with a failure.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A stored event result that can be decoded using its registered codec.
pub struct EncodedEventOutcome {
    /// Typed discriminator for this state. Type: `EventOutcomeKind`.
    pub kind: EventOutcomeKind,
    /// The payload carried by this request or record. Type: `Arc<[u8]>`.
    pub payload: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// An accepted event envelope together with its assigned stream position.
pub struct JournalEntry {
    /// The ordered sequence position. Type: `NonZeroU64`.
    pub sequence: NonZeroU64,
    /// Event associated with this record. Type: `EventEnvelope`.
    pub event: EventEnvelope,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The stable identity of an ordered event journal stream.
pub struct JournalStream {
    /// Journal that owns the authoritative records. Type: `EventJournalId`.
    pub journal: EventJournalId,
    /// Aggregate identity or accumulated metric data associated with this record. Type: `AggregateId`.
    pub aggregate: AggregateId,
}

impl EventEnvelope {
    #[must_use]
    /// Return the stream associated with this value.
    ///
    /// # Returns
    /// Type: `JournalStream`.
    pub fn stream(&self) -> JournalStream {
        JournalStream {
            journal: self.journal.clone(),
            aggregate: self.aggregate.clone(),
        }
    }
}
