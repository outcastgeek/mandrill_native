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
    pub fn from_stable<D: KeyDomain>(key: StableKey<D>) -> Self {
        Self(key.into_bytes())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A logical firing identity. The instant distinguishes both sides of a
/// daylight-saving overlap; replay of the same occurrence keeps this ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterOccurrenceId {
    pub schedule: ClusterScheduleId,
    pub instant: UtcMillis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConcurrentOccurrencePolicy {
    Allow,
    Skip,
    Defer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleDisposition {
    Dispatch,
    MissedDeadline,
    SkippedWhileActive,
    DeferredWhileActive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedOccurrence {
    pub id: ClusterOccurrenceId,
    pub calendar: CalendarOccurrence,
    pub deadline: UtcMillis,
    pub disposition: ScheduleDisposition,
}

#[derive(Debug)]
pub enum ClusterScheduleError<ZoneError> {
    Calendar(CronEvalError<ZoneError>),
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
    pub fn id(&self) -> &ClusterScheduleId {
        &self.id
    }

    /// Enumerate the complete bounded window before selecting catch-up work.
    /// A budget failure is returned rather than silently dropping firings.
    /// The caller persists its own cursor and occurrence ledger atomically
    /// with submission according to its chosen store contract.
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
