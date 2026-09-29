use effect::cluster::{
    ClusterSchedule, ClusterScheduleId, ConcurrentOccurrencePolicy, ScheduleDisposition,
};
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
fn skip_ignores_missed_work_but_keeps_the_occurrence_due_now() {
    let cron = Cron::parse("0 * * * * *", DayMatch::Both).unwrap();
    let zone = FixedOffsetZone::east_seconds(0).unwrap();
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
        .unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].instant, due_at);
    assert!(
        cron.due(
            after,
            late,
            &zone,
            transition,
            MissedOccurrencePolicy::Skip,
            budget,
        )
        .unwrap()
        .is_empty()
    );

    let namespace = KeyNamespace::<Domain>::new("schedules").unwrap();
    let identity = StableKey::new(&namespace, [KeySegment::Text(Arc::from("one"))]).unwrap();
    let schedule = ClusterSchedule::new(
        ClusterScheduleId::from_stable(identity),
        cron,
        transition,
        MissedOccurrencePolicy::Skip,
        ConcurrentOccurrencePolicy::Allow,
        Duration::ZERO,
    );
    let planned = schedule
        .plan_due(after, due_at, &zone, budget, false)
        .unwrap();
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].id.instant, due_at);
    assert_eq!(planned[0].disposition, ScheduleDisposition::Dispatch);
}
