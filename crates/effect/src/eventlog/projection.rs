use super::event_journal::validate_journal_read;
use super::{
    EventGroupId, EventJournal, EventTag, EventVersion, HasEventJournal, JournalCursor,
    JournalEntry, JournalReadOutcome, JournalReadViolation, JournalStream,
};
use crate::{Effect, KeyDomain, StableKey};
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;

type ProjectionFold<S, E> = dyn Fn(&S, &JournalEntry) -> Result<S, E> + Send + Sync;
type ProjectionError<R, S, E> = ProjectionAdvanceError<
    <<R as HasEventJournal>::Journal as EventJournal>::Error,
    <<R as HasAtomicProjectionStore<S>>::Store as AtomicProjectionStore<S>>::Error,
    E,
>;
type ProjectionEffect<R, S, E> = Effect<ProjectionAdvance<S>, ProjectionError<R, S, E>, R>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectionId(Arc<[u8]>);

impl ProjectionId {
    #[must_use]
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectionVersion(NonZeroU32);

impl ProjectionVersion {
    #[must_use]
    pub const fn new(value: NonZeroU32) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn value(self) -> NonZeroU32 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectionKey {
    pub id: ProjectionId,
    pub stream: JournalStream,
}

pub struct ProjectionSnapshot<S> {
    pub key: ProjectionKey,
    pub version: ProjectionVersion,
    pub checkpoint: JournalCursor,
    pub state: Arc<S>,
}

impl<S> Clone for ProjectionSnapshot<S> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            version: self.version,
            checkpoint: self.checkpoint,
            state: Arc::clone(&self.state),
        }
    }
}

pub enum ProjectionWrite<S> {
    Committed(ProjectionSnapshot<S>),
    Conflict(Option<ProjectionSnapshot<S>>),
    VersionConflict(ProjectionVersion),
}

/// This stronger store contract atomically publishes both state and cursor.
/// A weaker provider must expose its own duplicate/replay obligations instead
/// of claiming these compare-and-set semantics.
pub trait AtomicProjectionStore<S: Send + Sync + 'static>: Send + Sync + 'static {
    type Error: Send + 'static;
    fn load(&self, key: ProjectionKey) -> Effect<Option<ProjectionSnapshot<S>>, Self::Error>;
    fn compare_exchange(
        &self,
        key: ProjectionKey,
        expected: Option<JournalCursor>,
        next: ProjectionSnapshot<S>,
    ) -> Effect<ProjectionWrite<S>, Self::Error>;
}

pub trait HasAtomicProjectionStore<S: Send + Sync + 'static> {
    type Store: AtomicProjectionStore<S>;
    fn atomic_projection_store(&self) -> Arc<Self::Store>;
}

pub enum ProjectionAdvance<S> {
    Current(ProjectionSnapshot<S>),
    Advanced {
        snapshot: ProjectionSnapshot<S>,
        has_more: bool,
    },
    Conflict(Option<ProjectionSnapshot<S>>),
}

#[derive(Debug)]
pub enum ProjectionAdvanceError<JournalError, StoreError, FoldError> {
    Journal(JournalError),
    Store(StoreError),
    Fold(FoldError),
    HistoryTruncated {
        first_available: JournalCursor,
    },
    CursorAhead {
        head: JournalCursor,
    },
    InvalidJournalRead(JournalReadViolation),
    ProjectionKeyMismatch {
        expected: ProjectionKey,
        actual: ProjectionKey,
    },
    UnexpectedEvent {
        group: EventGroupId,
        tag: EventTag,
    },
    EventVersionMismatch {
        group: EventGroupId,
        tag: EventTag,
        expected: EventVersion,
        actual: EventVersion,
    },
    ProjectionVersionMismatch {
        expected: ProjectionVersion,
        actual: ProjectionVersion,
    },
}

/// Pure fold over encoded journal entries. The application declares every
/// accepted group/tag/version, so replay never decodes a different version as
/// the current shape. Effects that perform external I/O remain outside fold.
pub struct Projection<S, FoldError> {
    pub key: ProjectionKey,
    pub version: ProjectionVersion,
    accepted: Arc<BTreeMap<(EventGroupId, EventTag), EventVersion>>,
    initial: Arc<dyn Fn() -> S + Send + Sync>,
    fold: Arc<ProjectionFold<S, FoldError>>,
}

impl<S, FoldError> Clone for Projection<S, FoldError> {
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

    pub fn advance<R>(&self, limit: NonZeroUsize) -> ProjectionEffect<R, S, FoldError>
    where
        R: HasEventJournal + HasAtomicProjectionStore<S> + Send + Sync + 'static,
    {
        let projection = self.clone();
        Effect::service(|env: &R| env.atomic_projection_store()).flat_map(move |store| {
            let projection = projection.clone();
            store
                .load(projection.key.clone())
                .map_error(ProjectionAdvanceError::Store)
                .provide_some(|_: &R| ())
                .flat_map(move |current| {
                    let projection = projection.clone();
                    let store = Arc::clone(&store);
                    if let Some(snapshot) = &current {
                        if snapshot.key != projection.key {
                            return Effect::from_result_once(Err(
                                ProjectionAdvanceError::ProjectionKeyMismatch {
                                    expected: projection.key.clone(),
                                    actual: snapshot.key.clone(),
                                },
                            ));
                        }
                        if snapshot.version != projection.version {
                            return Effect::from_result_once(Err(
                                ProjectionAdvanceError::ProjectionVersionMismatch {
                                    expected: projection.version,
                                    actual: snapshot.version,
                                },
                            ));
                        }
                    }
                    let stream = projection.key.stream.clone();
                    let after = current
                        .as_ref()
                        .map_or(JournalCursor::new(0), |snapshot| snapshot.checkpoint);
                    Effect::service(|env: &R| env.event_journal())
                        .flat_map(move |journal| {
                            journal
                                .read(stream.clone(), after, limit)
                                .map_error(ProjectionAdvanceError::Journal)
                                .provide_some(|_: &R| ())
                        })
                        .flat_map(move |read| {
                            let current = current.clone();
                            let read = match read {
                                JournalReadOutcome::Complete(read) => read,
                                JournalReadOutcome::HistoryTruncated { first_available } => {
                                    return Effect::from_result_once(Err(
                                        ProjectionAdvanceError::HistoryTruncated {
                                            first_available,
                                        },
                                    ));
                                }
                                JournalReadOutcome::CursorAhead { head } => {
                                    return Effect::from_result_once(Err(
                                        ProjectionAdvanceError::CursorAhead { head },
                                    ));
                                }
                            };
                            if let Err(issue) =
                                validate_journal_read(&read, &projection.key.stream, after, limit)
                            {
                                return Effect::from_result_once(Err(
                                    ProjectionAdvanceError::InvalidJournalRead(issue),
                                ));
                            }
                            if read.entries.is_empty() {
                                let snapshot = current.unwrap_or_else(|| ProjectionSnapshot {
                                    key: projection.key.clone(),
                                    version: projection.version,
                                    checkpoint: after,
                                    state: Arc::new((projection.initial)()),
                                });
                                return Effect::sync_once(move || {
                                    ProjectionAdvance::Current(snapshot)
                                });
                            }
                            let mut state = current.as_ref().map_or_else(
                                || Arc::new((projection.initial)()),
                                |snapshot| Arc::clone(&snapshot.state),
                            );
                            for entry in &read.entries {
                                let key = (entry.event.group.clone(), entry.event.tag.clone());
                                let Some(expected) = projection.accepted.get(&key) else {
                                    return Effect::from_result_once(Err(
                                        ProjectionAdvanceError::UnexpectedEvent {
                                            group: key.0,
                                            tag: key.1,
                                        },
                                    ));
                                };
                                if *expected != entry.event.version {
                                    return Effect::from_result_once(Err(
                                        ProjectionAdvanceError::EventVersionMismatch {
                                            group: key.0,
                                            tag: key.1,
                                            expected: *expected,
                                            actual: entry.event.version,
                                        },
                                    ));
                                }
                                state = match (projection.fold)(&state, entry) {
                                    Ok(next) => Arc::new(next),
                                    Err(error) => {
                                        return Effect::from_result_once(Err(
                                            ProjectionAdvanceError::Fold(error),
                                        ));
                                    }
                                };
                            }
                            let next = ProjectionSnapshot {
                                key: projection.key.clone(),
                                version: projection.version,
                                checkpoint: read.next,
                                state,
                            };
                            store
                                .compare_exchange(
                                    projection.key.clone(),
                                    current.map(|value| value.checkpoint),
                                    next,
                                )
                                .map_error(ProjectionAdvanceError::Store)
                                .provide_some(|_: &R| ())
                                .flat_map({
                                    let version = projection.version;
                                    move |outcome| match outcome {
                                        ProjectionWrite::Committed(snapshot) => {
                                            Effect::sync_once(move || ProjectionAdvance::Advanced {
                                                snapshot,
                                                has_more: read.has_more,
                                            })
                                        }
                                        ProjectionWrite::Conflict(current) => {
                                            Effect::sync_once(move || {
                                                ProjectionAdvance::Conflict(current)
                                            })
                                        }
                                        ProjectionWrite::VersionConflict(actual) => {
                                            Effect::from_result_once(Err(
                                                ProjectionAdvanceError::ProjectionVersionMismatch {
                                                    expected: version,
                                                    actual,
                                                },
                                            ))
                                        }
                                    }
                                })
                        })
                })
        })
    }
}
