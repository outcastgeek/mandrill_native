//! Projection replay over leased journal pages and encoded snapshots.

use super::event_journal::validate_journal_positions;
use super::{
    EventGroupId, EventTag, EventVersion, JournalCursor, PreparedJournalEntry,
    PreparedJournalError, PreparedJournalReadOutcome, PreparedMemoryEventJournal,
    PreparedMemoryProjectionStore, PreparedProjectionError, PreparedProjectionReservation,
    PreparedProjectionSnapshot, PreparedProjectionWrite, ProjectionAdvanceError, ProjectionKey,
    ProjectionVersion,
};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

pub enum PreparedProjectionAdvance {
    Current(PreparedProjectionSnapshot),
    Advanced {
        snapshot: PreparedProjectionSnapshot,
        has_more: bool,
    },
    Conflict(Option<PreparedProjectionSnapshot>),
}

pub type PreparedProjectionAdvanceError<E> =
    ProjectionAdvanceError<PreparedJournalError, PreparedProjectionError, E>;

type Initial<E> = dyn Fn(&mut PreparedProjectionReservation) -> Result<(), E> + Send + Sync;
type Fold<E> = dyn Fn(&[u8], &PreparedJournalEntry, &mut PreparedProjectionReservation) -> Result<(), E>
    + Send
    + Sync;

/// An ordered byte-state projection. The next state is written into a reserved
/// snapshot, separate from the current state and journal page.
pub struct PreparedProjection<E> {
    key: ProjectionKey,
    version: ProjectionVersion,
    accepted: BTreeMap<(EventGroupId, EventTag), EventVersion>,
    initial: Arc<Initial<E>>,
    fold: Arc<Fold<E>>,
}

impl<E> PreparedProjection<E> {
    #[must_use]
    pub fn new(
        key: ProjectionKey,
        version: ProjectionVersion,
        accepted: BTreeMap<(EventGroupId, EventTag), EventVersion>,
        initial: impl Fn(&mut PreparedProjectionReservation) -> Result<(), E> + Send + Sync + 'static,
        fold: impl Fn(&[u8], &PreparedJournalEntry, &mut PreparedProjectionReservation) -> Result<(), E>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            key,
            version,
            accepted,
            initial: Arc::new(initial),
            fold: Arc::new(fold),
        }
    }

    /// Fold one validated page and atomically publish its checkpoint and bytes.
    /// The page and both scratch snapshots exist before the first fold.
    pub fn advance(
        &self,
        journal: &PreparedMemoryEventJournal,
        store: &PreparedMemoryProjectionStore,
        limit: NonZeroUsize,
    ) -> Result<PreparedProjectionAdvance, PreparedProjectionAdvanceError<E>> {
        let current = store
            .load(&self.key)
            .map_err(ProjectionAdvanceError::Store)?;
        if let Some(snapshot) = &current
            && snapshot.version != self.version
        {
            return Err(ProjectionAdvanceError::ProjectionVersionMismatch {
                expected: self.version,
                actual: snapshot.version,
            });
        }
        let expected = current.as_ref().map(|snapshot| snapshot.checkpoint);
        let after = expected.unwrap_or(JournalCursor::new(0));
        let page = match journal
            .read_prepared(&self.key.stream, after, limit)
            .map_err(ProjectionAdvanceError::Journal)?
        {
            PreparedJournalReadOutcome::Complete(page) => page,
            PreparedJournalReadOutcome::HistoryTruncated { first_available } => {
                return Err(ProjectionAdvanceError::HistoryTruncated { first_available });
            }
            PreparedJournalReadOutcome::CursorAhead { head } => {
                return Err(ProjectionAdvanceError::CursorAhead { head });
            }
        };
        let entries = page.entries();
        validate_journal_positions(
            entries.iter().map(|entry| {
                (
                    entry.sequence,
                    entry.header.journal == self.key.stream.journal
                        && entry.header.aggregate == self.key.stream.aggregate,
                )
            }),
            page.next,
            page.has_more,
            after,
            limit,
        )
        .map_err(ProjectionAdvanceError::InvalidJournalRead)?;
        if entries.is_empty() {
            if let Some(snapshot) = current {
                return Ok(PreparedProjectionAdvance::Current(snapshot));
            }
            let mut state = store.reserve_snapshot().map_err(prepared_memory)?;
            (self.initial)(&mut state).map_err(ProjectionAdvanceError::Fold)?;
            let snapshot = store
                .finish_snapshot(self.key.clone(), self.version, after, state)
                .map_err(ProjectionAdvanceError::Store)?;
            return Ok(PreparedProjectionAdvance::Current(snapshot));
        }
        for entry in entries {
            let key = (entry.header.group.clone(), entry.header.tag.clone());
            let Some(&expected) = self.accepted.get(&key) else {
                return Err(ProjectionAdvanceError::UnexpectedEvent {
                    group: key.0,
                    tag: key.1,
                });
            };
            if expected != entry.header.version {
                return Err(ProjectionAdvanceError::EventVersionMismatch {
                    group: key.0,
                    tag: key.1,
                    expected,
                    actual: entry.header.version,
                });
            }
        }
        let mut state = store.reserve_snapshot().map_err(prepared_memory)?;
        let mut next = store.reserve_snapshot().map_err(prepared_memory)?;
        if let Some(snapshot) = current {
            state
                .extend_from_slice(snapshot.state.as_ref())
                .map_err(prepared_memory)?;
        } else {
            (self.initial)(&mut state).map_err(ProjectionAdvanceError::Fold)?;
        }
        for entry in entries {
            next.clear();
            (self.fold)(state.as_slice(), entry, &mut next)
                .map_err(ProjectionAdvanceError::Fold)?;
            std::mem::swap(&mut state, &mut next);
        }
        match store
            .compare_exchange(self.key.clone(), expected, self.version, page.next, state)
            .map_err(ProjectionAdvanceError::Store)?
        {
            PreparedProjectionWrite::Committed(snapshot) => {
                Ok(PreparedProjectionAdvance::Advanced {
                    snapshot,
                    has_more: page.has_more,
                })
            }
            PreparedProjectionWrite::Conflict(snapshot) => {
                Ok(PreparedProjectionAdvance::Conflict(snapshot))
            }
            PreparedProjectionWrite::VersionConflict(actual) => {
                Err(ProjectionAdvanceError::ProjectionVersionMismatch {
                    expected: self.version,
                    actual,
                })
            }
        }
    }
}

fn prepared_memory<E>(error: crate::memory::MemoryError) -> PreparedProjectionAdvanceError<E> {
    ProjectionAdvanceError::Store(PreparedProjectionError::Memory(error))
}
