use super::event_journal::validate_journal_read;
use super::{
    EventJournal, HasEventJournal, JournalCursor, JournalEntry, JournalReadOutcome,
    JournalReadViolation, JournalStream,
};
use crate::reactivity::{InvalidationError, InvalidationKey, InvalidationRegistry};
use crate::{Effect, PublishReport};
use std::num::NonZeroUsize;
use std::sync::Arc;

type ObservationKeys = dyn Fn(&JournalEntry) -> Vec<InvalidationKey> + Send + Sync;

pub struct EventObservationReport {
    pub next: JournalCursor,
    pub has_more: bool,
    pub notification: Option<PublishReport>,
}

#[derive(Debug)]
pub enum EventObservationError<JournalError> {
    Journal(JournalError),
    HistoryTruncated {
        first_available: JournalCursor,
    },
    CursorAhead {
        head: JournalCursor,
    },
    InvalidJournalRead {
        replay_after: JournalCursor,
        issue: JournalReadViolation,
    },
    Notification {
        replay_after: JournalCursor,
        source: InvalidationError,
    },
    NotificationDropped {
        replay_after: JournalCursor,
        dropped: NonZeroUsize,
    },
}

/// Reconcile committed journal entries with local reactive invalidation.
/// The returned cursor is advanced only after publication. A lost/dropped
/// notification is recoverable by reading the journal again from the prior
/// cursor; the notification itself is never treated as append authority.
pub struct EventObservation {
    registry: InvalidationRegistry,
    keys: Arc<ObservationKeys>,
}

impl EventObservation {
    #[must_use]
    pub fn new(
        registry: InvalidationRegistry,
        keys: impl Fn(&JournalEntry) -> Vec<InvalidationKey> + Send + Sync + 'static,
    ) -> Self {
        Self {
            registry,
            keys: Arc::new(keys),
        }
    }

    pub fn reconcile<R>(
        &self,
        stream: JournalStream,
        after: JournalCursor,
        limit: NonZeroUsize,
    ) -> Effect<EventObservationReport, EventObservationError<<R::Journal as EventJournal>::Error>, R>
    where
        R: HasEventJournal + Send + Sync + 'static,
    {
        let registry = self.registry.clone();
        let keys = Arc::clone(&self.keys);
        let validation_stream = stream.clone();
        Effect::service(|env: &R| env.event_journal())
            .flat_map(move |journal| {
                journal
                    .read(stream.clone(), after, limit)
                    .map_error(EventObservationError::Journal)
                    .provide_some(|_: &R| ())
            })
            .flat_map(move |read| {
                let read = match read {
                    JournalReadOutcome::Complete(read) => read,
                    JournalReadOutcome::HistoryTruncated { first_available } => {
                        return Effect::from_result_once(Err(
                            EventObservationError::HistoryTruncated { first_available },
                        ));
                    }
                    JournalReadOutcome::CursorAhead { head } => {
                        return Effect::from_result_once(Err(EventObservationError::CursorAhead {
                            head,
                        }));
                    }
                };
                if let Err(issue) = validate_journal_read(&read, &validation_stream, after, limit) {
                    return Effect::from_result_once(Err(
                        EventObservationError::InvalidJournalRead {
                            replay_after: after,
                            issue,
                        },
                    ));
                }
                let mut batch = Vec::new();
                for entry in &read.entries {
                    batch.extend(keys(entry));
                }
                registry
                    .invalidate(batch)
                    .map_error(move |source| EventObservationError::Notification {
                        replay_after: after,
                        source,
                    })
                    .flat_map(move |notification| {
                        if let Some(dropped) =
                            notification.and_then(|report| NonZeroUsize::new(report.dropped))
                        {
                            return Effect::from_result_once(Err(
                                EventObservationError::NotificationDropped {
                                    replay_after: after,
                                    dropped,
                                },
                            ));
                        }
                        Effect::sync_once(move || EventObservationReport {
                            next: read.next,
                            has_more: read.has_more,
                            notification,
                        })
                    })
            })
    }
}
