//! Committed-event integration with reactive observation.

use super::event_journal::validate_journal_read;
use super::{
    EventJournal, HasEventJournal, JournalCursor, JournalEntry, JournalReadOutcome,
    JournalReadViolation, JournalStream,
};
use crate::reactivity::{InvalidationError, InvalidationKey, InvalidationRegistry};
use crate::{Effect, PublishReport};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Type alias for observation keys in eventlog event observation.
type ObservationKeys = dyn Fn(&JournalEntry) -> Vec<InvalidationKey> + Send + Sync;

/// The cursor and invalidations accounted for during event observation.
pub struct EventObservationReport {
    /// Next position after the current record. Type: `JournalCursor`.
    pub next: JournalCursor,
    /// Whether more entries remain after this batch. Type: `bool`.
    pub has_more: bool,
    /// Publication report when invalidation was emitted. Type: `Option<PublishReport>`.
    pub notification: Option<PublishReport>,
}

#[derive(Debug)]
/// An observation failure carrying the cursor from which reconciliation can resume.
///
/// # Type parameters
/// - `JournalError`: Error returned by the event journal.
pub enum EventObservationError<JournalError> {
    /// The authoritative event journal rejected or failed the operation. Retains the `JournalError` detail.
    Journal(JournalError),
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
    /// The journal read failed validation.
    InvalidJournalRead {
        /// Cursor from which reconciliation can safely resume. Type: `JournalCursor`.
        replay_after: JournalCursor,
        /// Structured validation issue explaining this failure. Type: `JournalReadViolation`.
        issue: JournalReadViolation,
    },
    /// No tification is available.
    Notification {
        /// Cursor from which reconciliation can safely resume. Type: `JournalCursor`.
        replay_after: JournalCursor,
        /// The underlying failure or provider cause. Type: `InvalidationError`.
        source: InvalidationError,
    },
    /// No tification dropped is available.
    NotificationDropped {
        /// Cursor from which reconciliation can safely resume. Type: `JournalCursor`.
        replay_after: JournalCursor,
        /// Number of values discarded under the overflow policy. Type: `NonZeroUsize`.
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
    /// Construct a `EventObservation` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `registry` (`InvalidationRegistry`): Registry that owns definitions and rejects identity conflicts.
    /// - `keys` (`impl Fn(&JournalEntry) -> Vec<InvalidationKey> + Send + Sync + 'static`): The keys supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        registry: InvalidationRegistry,
        keys: impl Fn(&JournalEntry) -> Vec<InvalidationKey> + Send + Sync + 'static,
    ) -> Self {
        Self {
            registry,
            keys: Arc::new(keys),
        }
    }

    /// Publish invalidations for committed events and return a replay-safe cursor.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Additional bound: `HasEventJournal + Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `stream` (`JournalStream`): Scoped source of ordered chunks or events.
    /// - `after` (`JournalCursor`): Cursor or instant after which results are selected.
    /// - `limit` (`NonZeroUsize`): The caller-supplied limit.
    ///
    /// # Returns
    /// Type: `Effect<EventObservationReport, EventObservationError<<R::Journal as EventJournal>::Error>, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
