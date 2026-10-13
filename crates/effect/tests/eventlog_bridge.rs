use effect::eventlog::{
    AggregateId, AppendDecision, AppendReceipt, AtomicProjectionStore, EventEnvelope, EventGroupId,
    EventId, EventJournal, EventJournalId, EventObservation, EventTag, EventVersion,
    HasAtomicProjectionStore, HasEventJournal, JournalCursor, JournalStream, MemoryEventJournal,
    MemoryJournalCapacity, MemoryProjectionStore, ProjectionAdvance, ProjectionId, ProjectionKey,
    ProjectionVersion,
};
use effect::projection;
use effect::reactivity::{InvalidationKey, InvalidationRegistry};
use effect::{
    Exit, KeyDomain, KeyNamespace, KeySegment, PubSubOptions, PubSubOverflow, PubSubShutdown,
    ReplayPolicy, StableKey, SubscriptionEvent, UtcMillis,
};
use futures_executor::block_on;
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "eventlog-bridge-test";
}

struct Environment {
    journal: Arc<MemoryEventJournal>,
    projections: Arc<MemoryProjectionStore<usize>>,
}

impl HasEventJournal for Environment {
    type Journal = MemoryEventJournal;
    fn event_journal(&self) -> Arc<Self::Journal> {
        Arc::clone(&self.journal)
    }
}

impl HasAtomicProjectionStore<usize> for Environment {
    type Store = MemoryProjectionStore<usize>;
    fn atomic_projection_store(&self) -> Arc<Self::Store> {
        Arc::clone(&self.projections)
    }
}

#[test]
fn append_projects_and_notifies_without_moving_authority_from_journal() -> Result<(), String> {
    let limit = NonZeroUsize::MIN.saturating_add(1);
    let environment = Arc::new(Environment {
        journal: Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: limit,
            entries_per_stream: limit,
            tombstones_per_stream: limit,
        })),
        projections: Arc::new(MemoryProjectionStore::new(limit)),
    });
    let namespace = KeyNamespace::<Domain>::new("events")
        .map_err(|error| format!("event namespace: {error:?}"))?;
    let key = |text: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(text))])
            .map_err(|error| format!("{text} identity: {error:?}"))
    };
    let group = EventGroupId::new("orders").map_err(|error| format!("event group: {error:?}"))?;
    let tag = EventTag::new("placed").map_err(|error| format!("event tag: {error:?}"))?;
    let version = EventVersion::new(NonZeroU32::MIN);
    let stream = JournalStream {
        journal: EventJournalId::from_stable(key("journal")?),
        aggregate: AggregateId::from_stable(key("aggregate")?),
    };
    let event = EventEnvelope {
        journal: stream.journal.clone(),
        aggregate: stream.aggregate.clone(),
        id: EventId::from_stable(key("event")?),
        group: group.clone(),
        tag: tag.clone(),
        version,
        payload: Arc::from(&b"order"[..]),
        outcome: None,
        occurred_at: UtcMillis::new(0),
    };
    let accepted = BTreeMap::from([((group, tag), version)]);
    let projection = projection! { key: ProjectionKey {
        id: ProjectionId::from_stable(key("projection")?),
        stream: stream.clone(),
    }, version: ProjectionVersion::new(NonZeroU32::MIN), accepted: accepted.clone(), initial: || 0usize, fold: |count, _| Ok::<usize, ()>(count + 1),  };
    let invalidation = InvalidationKey::from_stable(key("changed")?);
    let registry = InvalidationRegistry::new(PubSubOptions {
        capacity: limit,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    })
    .map_err(|error| format!("invalidation registry: {error:?}"))?;
    let Exit::Success(subscriber) = block_on(
        registry
            .subscribe::<Environment>()
            .run(Arc::clone(&environment)),
    ) else {
        return Err(String::from("invalidation subscription must succeed"));
    };

    let appended = block_on(
        environment
            .journal
            .append(event, JournalCursor::new(0))
            .run(Arc::new(())),
    );
    assert!(matches!(
        appended,
        Exit::Success(AppendReceipt::Decided(AppendDecision::Committed(_)))
    ));
    let failing = projection! { key: projection.key.clone(), version: projection.version, accepted: accepted, initial: || 0usize, fold: |_, _| Err::<usize, &'static str>("fold failed"),  };
    let failed = block_on(failing.advance(limit).run(Arc::clone(&environment)));
    assert!(matches!(
        failed,
        Exit::Fail(effect::eventlog::ProjectionAdvanceError::Fold(
            "fold failed"
        ))
    ));
    let unsaved = block_on(
        environment
            .projections
            .load(projection.key.clone())
            .run(Arc::new(())),
    );
    assert!(matches!(unsaved, Exit::Success(None)));
    let projected = block_on(projection.advance(limit).run(Arc::clone(&environment)));
    assert!(
        matches!(projected, Exit::Success(ProjectionAdvance::Advanced { snapshot, .. }) if *snapshot.state == 1 && snapshot.checkpoint.value() == 1)
    );

    let observation = EventObservation::new(registry.clone(), {
        let invalidation = invalidation.clone();
        move |_| vec![invalidation.clone()]
    });
    let observed = block_on(
        observation
            .reconcile(stream.clone(), JournalCursor::new(0), limit)
            .run(Arc::clone(&environment)),
    );
    assert!(
        matches!(observed, Exit::Success(report) if report.next.value() == 1 && report.notification.is_some())
    );
    assert!(
        matches!(block_on(subscriber.take().run(Arc::clone(&environment))), Exit::Success(SubscriptionEvent::Item(batch)) if batch.keys.as_ref() == [invalidation.clone()])
    );

    assert!(matches!(
        block_on(
            registry
                .shutdown(PubSubShutdown::Discard)
                .run(Arc::clone(&environment))
        ),
        Exit::Success(true)
    ));
    let notification_failure = block_on(
        observation
            .reconcile(stream.clone(), JournalCursor::new(0), limit)
            .run(Arc::clone(&environment)),
    );
    assert!(
        matches!(notification_failure, Exit::Fail(effect::eventlog::EventObservationError::Notification { replay_after, .. }) if replay_after.value() == 0)
    );

    let dropping_registry = InvalidationRegistry::new(PubSubOptions {
        capacity: NonZeroUsize::MIN,
        overflow: PubSubOverflow::DropNewest,
        replay: ReplayPolicy::None,
    })
    .map_err(|error| format!("dropping invalidation registry: {error:?}"))?;
    let _slow_subscriber = block_on(
        dropping_registry
            .subscribe::<Environment>()
            .run(Arc::clone(&environment)),
    );
    assert!(matches!(
        block_on(
            dropping_registry
                .invalidate::<Environment>(vec![invalidation.clone()])
                .run(Arc::clone(&environment))
        ),
        Exit::Success(Some(_))
    ));
    let dropping_observation = EventObservation::new(dropping_registry, {
        let invalidation = invalidation.clone();
        move |_| vec![invalidation.clone()]
    });
    assert!(matches!(
        block_on(dropping_observation.reconcile(stream.clone(), JournalCursor::new(0), limit)
            .run(Arc::clone(&environment))),
        Exit::Fail(effect::eventlog::EventObservationError::NotificationDropped {
            replay_after, dropped,
        }) if replay_after.value() == 0 && dropped.get() == 1
    ));

    // Reconciliation is replayable; the journal remains the append authority.
    let cursor = block_on(environment.journal.cursor(stream).run(Arc::new(())));
    assert!(matches!(cursor, Exit::Success(value) if value.value() == 1));
    let saved = block_on(
        environment
            .projections
            .load(projection.key)
            .run(Arc::new(())),
    );
    assert!(matches!(saved, Exit::Success(Some(snapshot)) if *snapshot.state == 1));
    Ok(())
}
