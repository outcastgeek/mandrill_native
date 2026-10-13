//! Projection folding, checkpointing, and reconstruction.

use super::event_journal::validate_journal_read;
use super::{
    EventGroupId, EventJournal, EventTag, EventVersion, HasEventJournal, JournalCursor,
    JournalEntry, JournalReadOutcome, JournalReadViolation, JournalStream,
};
use crate::{Effect, KeyDomain, StableKey};
use crate::{effect, effect_map_error, effect_provide_some, effect_sync, effect_try_sync, service};
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;

/// Projection code is a process-local callback dispatched through a Rust
/// vtable; the journal stores entries and checkpoints, not this callback.
///
/// # Type parameters
/// - `S`: Service or storage type.
/// - `E`: Domain-error type.
type ProjectionFold<S, E> = dyn Fn(&S, &JournalEntry) -> Result<S, E> + Send + Sync;
/// Type alias for projection error in eventlog projection.
///
/// # Type parameters
/// - `R`: Environment type.
/// - `S`: Service or storage type.
/// - `E`: Domain-error type.
type ProjectionError<R, S, E> = ProjectionAdvanceError<
    <<R as HasEventJournal>::Journal as EventJournal>::Error,
    <<R as HasAtomicProjectionStore<S>>::Store as AtomicProjectionStore<S>>::Error,
    E,
>;
/// Type alias for projection effect in eventlog projection.
///
/// # Type parameters
/// - `R`: Environment type.
/// - `S`: Service or storage type.
/// - `E`: Domain-error type.
type ProjectionEffect<R, S, E> = Effect<ProjectionAdvance<S>, ProjectionError<R, S, E>, R>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by eventlog projection operations.
pub struct ProjectionId(Arc<[u8]>);

impl ProjectionId {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// The validated version of a registered projection definition.
pub struct ProjectionVersion(NonZeroU32);

impl ProjectionVersion {
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable identity used by eventlog projection operations.
pub struct ProjectionKey {
    /// Stable identity of this record. Type: `ProjectionId`.
    pub id: ProjectionId,
    /// Stable identity of the ordered stream. Type: `JournalStream`.
    pub stream: JournalStream,
}

/// A projection’s state and journal checkpoint read atomically together.
///
/// # Type parameters
/// - `S`: Service or storage type.
pub struct ProjectionSnapshot<S> {
    /// Key used to address the stored value. Type: `ProjectionKey`.
    pub key: ProjectionKey,
    /// The declared compatibility version. Type: `ProjectionVersion`.
    pub version: ProjectionVersion,
    /// Journal cursor whose events are reflected in this state. Type: `JournalCursor`.
    pub checkpoint: JournalCursor,
    /// Recorded lifecycle state. Type: `Arc<S>`.
    pub state: Arc<S>,
}

impl<S> Clone for ProjectionSnapshot<S> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            version: self.version,
            checkpoint: self.checkpoint,
            state: Arc::clone(&self.state),
        }
    }
}

/// The replacement state and checkpoint submitted as one atomic update.
///
/// # Type parameters
/// - `S`: Service or storage type.
pub enum ProjectionWrite<S> {
    /// The state change was committed and is now authoritative. Contains `ProjectionSnapshot<S>`.
    Committed(ProjectionSnapshot<S>),
    /// The requested state conflicts with the recorded state. Carries `Option<ProjectionSnapshot<S>>`.
    Conflict(Option<ProjectionSnapshot<S>>),
    /// The version conflicts with recorded state. Contains `ProjectionVersion`.
    VersionConflict(ProjectionVersion),
}

/// This stronger store contract atomically publishes both state and cursor.
/// A weaker provider must expose its own duplicate/replay obligations instead
/// of claiming these compare-and-set semantics.
pub trait AtomicProjectionStore<S: Send + Sync + 'static>: Send + Sync + 'static {
    /// Provider-specific failure type returned by this capability.
    type Error: Send + 'static;
    /// # Type parameters
    /// - `S`: Service or store type.
    ///
    /// # Parameters
    /// - `key` (`ProjectionKey`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<Option<ProjectionSnapshot<S>>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn load(&self, key: ProjectionKey) -> Effect<Option<ProjectionSnapshot<S>>, Self::Error>;
    /// # Type parameters
    /// - `S`: Service or store type.
    ///
    /// # Parameters
    /// - `key` (`ProjectionKey`): Stable key used to locate or identify the selected value.
    /// - `expected` (`Option<JournalCursor>`): Value or revision required for the proposed state transition.
    /// - `next` (`ProjectionSnapshot<S>`): Next value or state produced by the transition.
    ///
    /// # Returns
    /// Type: `Effect<ProjectionWrite<S>, Self::Error>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn compare_exchange(
        &self,
        key: ProjectionKey,
        expected: Option<JournalCursor>,
        next: ProjectionSnapshot<S>,
    ) -> Effect<ProjectionWrite<S>, Self::Error>;
}

/// Selects a provider that commits projection state and checkpoint atomically.
pub trait HasAtomicProjectionStore<S: Send + Sync + 'static> {
    /// Store selected by this environment; its trait bound defines the required guarantees.
    type Store: AtomicProjectionStore<S>;
    /// # Type parameters
    /// - `S`: Service or storage type. Bound: `Send + Sync + 'static`.
    ///
    /// # Returns
    /// Type: `Arc<Self::Store>`.
    fn atomic_projection_store(&self) -> Arc<Self::Store>;
}

/// The new checkpoint and state resulting from folding journal entries.
///
/// # Type parameters
/// - `S`: Service or storage type.
pub enum ProjectionAdvance<S> {
    /// The supplied position matches the recorded current position. Contains `ProjectionSnapshot<S>`.
    Current(ProjectionSnapshot<S>),
    /// The recorded cursor moved forward.
    Advanced {
        /// State and checkpoint submitted as one atomic projection update. Type: `ProjectionSnapshot<S>`.
        snapshot: ProjectionSnapshot<S>,
        /// Whether more entries remain after this batch. Type: `bool`.
        has_more: bool,
    },
    /// The requested state conflicts with the recorded state. Carries `Option<ProjectionSnapshot<S>>`.
    Conflict(Option<ProjectionSnapshot<S>>),
}

#[derive(Debug)]
/// A history, codec, fold, or store failure during projection advancement.
///
/// # Type parameters
/// - `JournalError`: Error returned by the event journal.
/// - `StoreError`: Error returned by the backing store.
/// - `FoldError`: Error produced while folding a projection.
pub enum ProjectionAdvanceError<JournalError, StoreError, FoldError> {
    /// The authoritative event journal rejected or failed the operation. Retains the `JournalError` detail.
    Journal(JournalError),
    /// The selected store rejected or failed the operation. Retains the `StoreError` detail.
    Store(StoreError),
    /// Applying the projection fold failed; its checkpoint was not advanced. Retains the `FoldError` detail.
    Fold(FoldError),
    /// Required history was compacted before this replay cursor.
    HistoryTruncated {
        /// Earliest position still retained for replay. Type: `JournalCursor`.
        first_available: JournalCursor,
    },
    /// The supplied cursor is ahead of the journal’s current head.
    CursorAhead {
        /// Current head position of the ordered stream. Type: `JournalCursor`.
        head: JournalCursor,
    },
    /// The journal read failed validation. Carries `JournalReadViolation`.
    InvalidJournalRead(JournalReadViolation),
    /// The projection key differs from the required or recorded value.
    ProjectionKeyMismatch {
        /// The value required by the requested condition. Type: `ProjectionKey`.
        expected: ProjectionKey,
        /// The value observed when the requested condition failed. Type: `ProjectionKey`.
        actual: ProjectionKey,
    },
    /// The received event is invalid in the current protocol state.
    UnexpectedEvent {
        /// Stable event or request group identity. Type: `EventGroupId`.
        group: EventGroupId,
        /// The event or protocol tag. Type: `EventTag`.
        tag: EventTag,
    },
    /// The event version differs from the required or recorded value.
    EventVersionMismatch {
        /// Stable event or request group identity. Type: `EventGroupId`.
        group: EventGroupId,
        /// The event or protocol tag. Type: `EventTag`.
        tag: EventTag,
        /// The value required by the requested condition. Type: `EventVersion`.
        expected: EventVersion,
        /// The value observed when the requested condition failed. Type: `EventVersion`.
        actual: EventVersion,
    },
    /// The projection version differs from the required or recorded value.
    ProjectionVersionMismatch {
        /// The value required by the requested condition. Type: `ProjectionVersion`.
        expected: ProjectionVersion,
        /// The value observed when the requested condition failed. Type: `ProjectionVersion`.
        actual: ProjectionVersion,
    },
}

/// Pure fold over encoded journal entries. The application declares every
/// accepted group/tag/version, so replay never decodes a different version as
/// the current shape. Effects that perform external I/O remain outside fold.
///
/// # Type parameters
/// - `S`: Service or storage type.
/// - `FoldError`: Error produced while folding a projection.
pub struct Projection<S, FoldError> {
    /// Key used to address the stored value. Type: `ProjectionKey`.
    pub key: ProjectionKey,
    /// The declared compatibility version. Type: `ProjectionVersion`.
    pub version: ProjectionVersion,
    accepted: Arc<BTreeMap<(EventGroupId, EventTag), EventVersion>>,
    initial: Arc<dyn Fn() -> S + Send + Sync>,
    fold: Arc<ProjectionFold<S, FoldError>>,
}

impl<S, FoldError> Clone for Projection<S, FoldError> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            version: self.version,
            accepted: Arc::clone(&self.accepted),
            initial: Arc::clone(&self.initial),
            fold: Arc::clone(&self.fold),
        }
    }
}

impl<S: Send + Sync + 'static, FoldError: Send + 'static> Projection<S, FoldError> {
    #[must_use]
    /// Construct a `Projection` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `FoldError`: Error produced while folding a projection. Bound: `Send + 'static`.
    /// - `S`: Service or store type.
    ///
    /// # Parameters
    /// - `key` (`ProjectionKey`): Stable key used to locate or identify the selected value.
    /// - `version` (`ProjectionVersion`): Registered schema, event, or workflow compatibility version.
    /// - `accepted` (`BTreeMap<(EventGroupId, EventTag), EventVersion>`): The accepted supplied to this operation.
    /// - `initial` (`impl Fn() -> S + Send + Sync + 'static`): Initial value used before any state transition.
    /// - `fold` (`impl Fn(&S, &JournalEntry) -> Result<S, FoldError> + Send + Sync + 'static`): The fold supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        key: ProjectionKey,
        version: ProjectionVersion,
        accepted: BTreeMap<(EventGroupId, EventTag), EventVersion>,
        initial: impl Fn() -> S + Send + Sync + 'static,
        fold: impl Fn(&S, &JournalEntry) -> Result<S, FoldError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            key,
            version,
            accepted: Arc::new(accepted),
            initial: Arc::new(initial),
            fold: Arc::new(fold),
        }
    }

    /// Move the recorded position forward after validating the intervening state.
    ///
    /// # Type parameters
    /// - `FoldError`: Error produced while folding a projection. Bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasEventJournal + HasAtomicProjectionStore<S> + Send + Sync + 'static`.
    /// - `S`: Service or store type.
    ///
    /// # Parameters
    /// - `limit` (`NonZeroUsize`): The caller-supplied limit.
    ///
    /// # Returns
    /// Type: `ProjectionEffect<R, S, FoldError>`.
    pub fn advance<R>(&self, limit: NonZeroUsize) -> ProjectionEffect<R, S, FoldError>
    where
        R: HasEventJournal + HasAtomicProjectionStore<S> + Send + Sync + 'static,
    {
        let projection = self.clone();
        {
            let __dsl_program = service! { env: |env: &R| env.atomic_projection_store(), };
            effect! {
                       let __dsl_program = __dsl_program.clone(); let projection = projection.clone(); let store = __dsl_program.clone()?; {
                       let projection = projection.clone();
                       {
                           let __dsl_program = effect_provide_some! {
                           program: effect_map_error! {
                           program: store
                           .load(projection.key.clone()), map: ProjectionAdvanceError::Store, }, project: |_: &R| (), }; effect! {
                               let __dsl_program = __dsl_program.clone(); let projection = projection.clone(); let store = store.clone(); let current = __dsl_program.clone()?; (move || {
                               let projection = projection.clone();
                               let store = Arc::clone(&store);
                               if let Some(snapshot) = &current {
                                   if snapshot.key != projection.key {
                                       return effect_try_sync! {
                                           once; body: {
                                           let __dsl_result = Err(
                                           ProjectionAdvanceError::ProjectionKeyMismatch {
                                               expected: projection.key.clone(),
                                               actual: snapshot.key.clone(),
                                           },
                                       ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                   }
                                   if snapshot.version != projection.version {
                                       return effect_try_sync! {
                                           once; body: {
                                           let __dsl_result = Err(
                                           ProjectionAdvanceError::ProjectionVersionMismatch {
                                               expected: projection.version,
                                               actual: snapshot.version,
                                           },
                                       ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                   }
                               }
                               let stream = projection.key.stream.clone();
                               let after = current
                                   .as_ref()
                                   .map_or(JournalCursor::new(0), |snapshot| snapshot.checkpoint);
                               {
                                   let __dsl_program = {
                                   let __dsl_program = service! { env: |env: &R| env.event_journal(), }; effect! {
                                   let stream = stream.clone(); let journal = __dsl_program.clone()?; {
                                       effect_provide_some! {
                                           program: effect_map_error! {
                                           program: journal
                                           .read(stream.clone(), after, limit), map: ProjectionAdvanceError::Journal, }, project: |_: &R| (), }
                                   }? } }; effect! {
                                       let __dsl_program = __dsl_program.clone(); let current = current.clone(); let projection = projection.clone(); let store = store.clone(); let read = __dsl_program.clone()?; (move || {
                                       let current = current.clone();
                                       let read = match read {
                                           JournalReadOutcome::Complete(read) => read,
                                           JournalReadOutcome::HistoryTruncated { first_available } => {
                                               return effect_try_sync! {
                                                   once; body: {
                                                   let __dsl_result = Err(
                                                   ProjectionAdvanceError::HistoryTruncated {
                                                       first_available,
                                                   },
                                               ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                           }
                                           JournalReadOutcome::CursorAhead { head } => {
                                               return effect_try_sync! {
                                                   once; body: {
                                                   let __dsl_result = Err(
                                                   ProjectionAdvanceError::CursorAhead { head },
                                               ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                           }
                                       };
                                       if let Err(issue) =
                                           validate_journal_read(&read, &projection.key.stream, after, limit)
                                       {
                                           return effect_try_sync! {
                                               once; body: {
                                               let __dsl_result = Err(
                                               ProjectionAdvanceError::InvalidJournalRead(issue),
                                           ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                       }
                                       if read.entries.is_empty() {
                                           let snapshot = current.unwrap_or_else(|| ProjectionSnapshot {
                                               key: projection.key.clone(),
                                               version: projection.version,
                                               checkpoint: after,
                                               state: Arc::new((projection.initial)()),
                                           });
                                           return effect_sync! {
                                               once; body: move || {
                                               ProjectionAdvance::Current(snapshot)
                                           }, };
                                       }
                                       let mut state = current.as_ref().map_or_else(
                                           || Arc::new((projection.initial)()),
                                           |snapshot| Arc::clone(&snapshot.state),
                                       );
                                       for entry in &read.entries {
                                           let key = (entry.event.group.clone(), entry.event.tag.clone());
                                           let Some(expected) = projection.accepted.get(&key) else {
                                               return effect_try_sync! {
                                                   once; body: {
                                                   let __dsl_result = Err(
                                                   ProjectionAdvanceError::UnexpectedEvent {
                                                       group: key.0,
                                                       tag: key.1,
                                                   },
                                               ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                           };
                                           if *expected != entry.event.version {
                                               return effect_try_sync! {
                                                   once; body: {
                                                   let __dsl_result = Err(
                                                   ProjectionAdvanceError::EventVersionMismatch {
                                                       group: key.0,
                                                       tag: key.1,
                                                       expected: *expected,
                                                       actual: entry.event.version,
                                                   },
                                               ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                           }
                                           state = match (projection.fold)(&state, entry) {
                                               Ok(next) => Arc::new(next),
                                               Err(error) => {
                                                   return effect_try_sync! {
                                                       once; body: {
                                                       let __dsl_result = Err(
                                                       ProjectionAdvanceError::Fold(error),
                                                   ); move || __dsl_result }, on_error: ::core::convert::identity, };
                                               }
                                           };
                                       }
                                       let next = ProjectionSnapshot {
                                           key: projection.key.clone(),
                                           version: projection.version,
                                           checkpoint: read.next,
                                           state,
                                       };
                                       effect! {
            bind;
            program: effect_provide_some! {
                program: effect_map_error! {
                program: store
                                           .compare_exchange(
                                               projection.key.clone(),
                                               current.map(|value| value.checkpoint),
                                               next,
                                           ), map: ProjectionAdvanceError::Store, }, project: |_: &R| (), },
            body: {
                                               let version = projection.version;
                                               let has_more = read.has_more;
                                               move |outcome| match outcome {
                                                   ProjectionWrite::Committed(snapshot) => {
                                                       effect_sync! {
                                                           once; body: move || ProjectionAdvance::Advanced {
                                                           snapshot,
                                                           has_more,
                                                       }, }
                                                   }
                                                   ProjectionWrite::Conflict(current) => {
                                                       effect_sync! {
                                                           once; body: move || {
                                                           ProjectionAdvance::Conflict(current)
                                                       }, }
                                                   }
                                                   ProjectionWrite::VersionConflict(actual) => {
                                                       effect_try_sync! {
                                                           once; body: {
                                                           let __dsl_result = Err(
                                                           ProjectionAdvanceError::ProjectionVersionMismatch {
                                                               expected: version,
                                                               actual,
                                                           },
                                                       ); move || __dsl_result }, on_error: ::core::convert::identity, }
                                                   }
                                               }
                                           },
            }
                                   })()? } }
                           })()? } }
                   }? }
        }
    }
}
