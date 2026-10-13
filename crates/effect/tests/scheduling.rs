use effect::cluster::{ClusterScheduleId, ConcurrentOccurrencePolicy, ScheduleDisposition};
use effect::cluster_schedule;
use effect::{
    CalendarTransitionPolicy, Cron, CronSearchBudget, DayMatch, FixedOffsetZone, GapPolicy,
    KeyDomain, KeyNamespace, KeySegment, MissedOccurrencePolicy, OverlapPolicy, StableKey,
    UtcMillis,
};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "schedule-boundary-test";
}

#[test]
fn skip_ignores_missed_work_but_keeps_the_occurrence_due_now() -> Result<(), String> {
    let cron = Cron::parse("0 * * * * *", DayMatch::Both)
        .map_err(|error| format!("minute schedule: {error:?}"))?;
    let zone =
        FixedOffsetZone::east_seconds(0).map_err(|error| format!("UTC offset: {error:?}"))?;
    let transition = CalendarTransitionPolicy {
        gap: GapPolicy::Skip,
        overlap: OverlapPolicy::Earlier,
    };
    let budget = CronSearchBudget {
        days: NonZeroUsize::MAX,
        candidates: NonZeroUsize::MAX,
    };
    let after = UtcMillis::new(0);
    let due_at = UtcMillis::new(60_000);
    let late = UtcMillis::new(90_000);

    let due = cron
        .due(
            after,
            due_at,
            &zone,
            transition,
            MissedOccurrencePolicy::Skip,
            budget,
        )
        .map_err(|error| format!("due occurrence: {error:?}"))?;
    assert_eq!(due.len(), 1);
    assert_eq!(due.first().ok_or("missing due occurrence")?.instant, due_at);
    assert!(
        cron.due(
            after,
            late,
            &zone,
            transition,
            MissedOccurrencePolicy::Skip,
            budget,
        )
        .map_err(|error| format!("missed occurrence: {error:?}"))?
        .is_empty()
    );

    let namespace = KeyNamespace::<Domain>::new("schedules")
        .map_err(|error| format!("schedule namespace: {error:?}"))?;
    let identity = StableKey::new(&namespace, [KeySegment::Text(Arc::from("one"))])
        .map_err(|error| format!("schedule identity: {error:?}"))?;
    let schedule = cluster_schedule! { id: ClusterScheduleId::from_stable(identity), cron: cron, transitions: transition, catch_up: MissedOccurrencePolicy::Skip, concurrent: ConcurrentOccurrencePolicy::Allow, deadline_after: Duration::ZERO,  };
    let planned = schedule
        .plan_due(after, due_at, &zone, budget, false)
        .map_err(|error| format!("planned occurrence: {error:?}"))?;
    assert_eq!(planned.len(), 1);
    let planned = planned.first().ok_or("missing planned occurrence")?;
    assert_eq!(planned.id.instant, due_at);
    assert_eq!(planned.disposition, ScheduleDisposition::Dispatch);
    Ok(())
}
