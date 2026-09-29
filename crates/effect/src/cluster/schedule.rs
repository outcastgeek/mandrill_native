//! Logical recurring entity execution.

use crate::{
    CalendarOccurrence, CalendarTransitionPolicy, CalendarZone, Cron, CronEvalError,
    CronSearchBudget, KeyDomain, MissedOccurrencePolicy, StableKey, TimeError, UtcMillis,
};
use std::sync::Arc;
use std::time::Duration;

/// Stable identity supplied by the application, independent of a process or
/// scheduler implementation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClusterScheduleId(Arc<[u8]>);

impl ClusterScheduleId {
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

/// A logical firing identity. The instant distinguishes both sides of a
/// daylight-saving overlap; replay of the same occurrence keeps this ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterOccurrenceId {
    /// Stable identity of the scheduled cluster operation. Type: `ClusterScheduleId`.
    pub schedule: ClusterScheduleId,
    /// Resolved absolute instant. Type: `UtcMillis`.
    pub instant: UtcMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The decision when a scheduled occurrence overlaps work already running.
pub enum ConcurrentOccurrencePolicy {
    /// Permit the proposed operation.
    Allow,
    /// Leave the occurrence or value unchanged and continue.
    Skip,
    /// Delay this occurrence until the active work settles.
    Defer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The scheduling action selected for a cluster occurrence.
pub enum ScheduleDisposition {
    /// Send the accepted message to its registered handler.
    Dispatch,
    /// The scheduled deadline passed before work could start.
    MissedDeadline,
    /// The occurrence was skipped because work remained active.
    SkippedWhileActive,
    /// The occurrence was postponed because work is still active.
    DeferredWhileActive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A resolved scheduled instant together with its cluster execution identity.
pub struct PlannedOccurrence {
    /// Stable identity of this record. Type: `ClusterOccurrenceId`.
    pub id: ClusterOccurrenceId,
    /// Resolved calendar occurrence for this scheduled operation. Type: `CalendarOccurrence`.
    pub calendar: CalendarOccurrence,
    /// The absolute deadline for this operation. Type: `UtcMillis`.
    pub deadline: UtcMillis,
    /// Recorded delivery status of this message. Type: `ScheduleDisposition`.
    pub disposition: ScheduleDisposition,
}

#[derive(Debug)]
/// A calendar, identity, or store failure while planning cluster work.
///
/// # Type parameters
/// - `ZoneError`: Error returned by the calendar-zone provider.
pub enum ClusterScheduleError<ZoneError> {
    /// Calendar resolution failed under the selected policy. Retains the `CronEvalError<ZoneError>` detail.
    Calendar(CronEvalError<ZoneError>),
    /// The selected clock could not schedule the deadline. Retains the `TimeError` detail.
    Deadline(TimeError),
}

/// Pure recurring schedule description. A caller supplies zone, search
/// limits, the wall-clock reading, and whether earlier work is still active.
/// No timer, executor, process-local clock, or implicit catch-up rule is used.
pub struct ClusterSchedule {
    id: ClusterScheduleId,
    cron: Cron,
    transitions: CalendarTransitionPolicy,
    catch_up: MissedOccurrencePolicy,
    concurrent: ConcurrentOccurrencePolicy,
    deadline_after: Duration,
}

impl ClusterSchedule {
    #[must_use]
    /// Construct a `ClusterSchedule` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `id` (`ClusterScheduleId`): Stable identity of the selected record or operation.
    /// - `cron` (`Cron`): The cron supplied to this operation.
    /// - `transitions` (`CalendarTransitionPolicy`): The transitions supplied to this operation.
    /// - `catch_up` (`MissedOccurrencePolicy`): The catch up supplied to this operation.
    /// - `concurrent` (`ConcurrentOccurrencePolicy`): The concurrent supplied to this operation.
    /// - `deadline_after` (`Duration`): The deadline after supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(
        id: ClusterScheduleId,
        cron: Cron,
        transitions: CalendarTransitionPolicy,
        catch_up: MissedOccurrencePolicy,
        concurrent: ConcurrentOccurrencePolicy,
        deadline_after: Duration,
    ) -> Self {
        Self {
            id,
            cron,
            transitions,
            catch_up,
            concurrent,
            deadline_after,
        }
    }

    #[must_use]
    /// Return the stable identity associated with this value.
    ///
    /// # Returns
    /// Type: `&ClusterScheduleId`.
    pub fn id(&self) -> &ClusterScheduleId {
        &self.id
    }

    /// Enumerate the complete bounded window before selecting catch-up work.
    /// A budget failure is returned rather than silently dropping firings.
    /// The caller persists its own cursor and occurrence ledger atomically
    /// with submission according to its chosen store contract.
    ///
    /// # Type parameters
    /// - `Zone`: Bounds: `CalendarZone`.
    ///
    /// # Parameters
    /// - `after` (`UtcMillis`): Cursor or instant after which results are selected.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `zone` (`&Zone`): The zone supplied to this operation.
    /// - `budget` (`CronSearchBudget`): The budget supplied to this operation.
    /// - `active` (`bool`): The active supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Vec<PlannedOccurrence>, ClusterScheduleError<Zone::Error>>`.
    pub fn plan_due<Zone: CalendarZone>(
        &self,
        after: UtcMillis,
        now: UtcMillis,
        zone: &Zone,
        budget: CronSearchBudget,
        active: bool,
    ) -> Result<Vec<PlannedOccurrence>, ClusterScheduleError<Zone::Error>> {
        let selected = self
            .cron
            .due(after, now, zone, self.transitions, self.catch_up, budget)
            .map_err(ClusterScheduleError::Calendar)?;
        selected
            .into_iter()
            .map(|calendar| {
                let deadline = calendar
                    .instant
                    .checked_add(self.deadline_after)
                    .map_err(ClusterScheduleError::Deadline)?;
                let disposition = if now > deadline {
                    ScheduleDisposition::MissedDeadline
                } else if active {
                    match self.concurrent {
                        ConcurrentOccurrencePolicy::Allow => ScheduleDisposition::Dispatch,
                        ConcurrentOccurrencePolicy::Skip => ScheduleDisposition::SkippedWhileActive,
                        ConcurrentOccurrencePolicy::Defer => {
                            ScheduleDisposition::DeferredWhileActive
                        }
                    }
                } else {
                    ScheduleDisposition::Dispatch
                };
                Ok(PlannedOccurrence {
                    id: ClusterOccurrenceId {
                        schedule: self.id.clone(),
                        instant: calendar.instant,
                    },
                    calendar,
                    deadline,
                    disposition,
                })
            })
            .collect()
    }
}
