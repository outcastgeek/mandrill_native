#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::eventlog::{
    AggregateId, AppendDecision, AppendReceipt, AtomicProjectionStore, CompactionOutcome,
    EventEnvelope, EventGroupId, EventId, EventInspection, EventJournal, EventJournalId, EventTag,
    EventVersion, HasAtomicProjectionStore, HasEventJournal, JournalCompaction, JournalCursor,
    JournalEntry, JournalRead, JournalReadOutcome, JournalStream, MemoryEventJournal,
    MemoryJournalCapacity, MemoryJournalError, MemoryProjectionStore, MemoryRetentionAuthority,
    Projection, ProjectionAdvance, ProjectionAdvanceError, ProjectionId, ProjectionKey,
    ProjectionVersion, ReplayRetention,
};
use effect::{Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis};
use futures_executor::block_on;
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "eventlog-campaign";
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

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn number(config: &JsonValue, path: &[&str]) -> u64 {
    path.iter()
        .fold(config, |value, field| {
            value
                .get(*field)
                .unwrap_or_else(|| panic!("missing campaign field {field}"))
        })
        .as_u64()
        .expect("campaign field is a nonnegative integer")
}

enum Location {
    Retained(JournalEntry),
    Compacted,
}

struct Model {
    cursor: u64,
    first_available: u64,
    entries: Vec<JournalEntry>,
    by_id: BTreeMap<EventId, Location>,
}

impl Model {
    fn new() -> Self {
        Self {
            cursor: 0,
            first_available: 1,
            entries: Vec::new(),
            by_id: BTreeMap::new(),
        }
    }

    fn append(&mut self, event: EventEnvelope, expected: JournalCursor) -> AppendReceipt {
        let decision = match self.by_id.get(&event.id) {
            Some(Location::Compacted) => AppendDecision::CompactedEventId,
            Some(Location::Retained(entry)) if entry.event == event => {
                AppendDecision::Duplicate(entry.clone())
            }
            Some(Location::Retained(_)) => AppendDecision::ConflictingEventId,
            None if expected.value() != self.cursor => AppendDecision::StaleCursor {
                actual: JournalCursor::new(self.cursor),
            },
            None => {
                self.cursor += 1;
                let entry = JournalEntry {
                    sequence: NonZeroU64::new(self.cursor).expect("model cursor is positive"),
                    event,
                };
                self.by_id
                    .insert(entry.event.id.clone(), Location::Retained(entry.clone()));
                self.entries.push(entry.clone());
                AppendDecision::Committed(entry)
            }
        };
        AppendReceipt::Decided(decision)
    }

    fn inspect(&self, id: &EventId) -> EventInspection {
        match self.by_id.get(id) {
            Some(Location::Retained(entry)) => EventInspection::Present(entry.clone()),
            Some(Location::Compacted) => EventInspection::Compacted,
            None => EventInspection::Missing,
        }
    }

    fn read(&self, after: JournalCursor, limit: NonZeroUsize) -> JournalReadOutcome {
        if after.value() > self.cursor {
            return JournalReadOutcome::CursorAhead {
                head: JournalCursor::new(self.cursor),
            };
        }
        if after.value() + 1 < self.first_available {
            return JournalReadOutcome::HistoryTruncated {
                first_available: JournalCursor::new(self.first_available),
            };
        }
        let remaining: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.sequence.get() > after.value())
            .collect();
        let entries: Vec<_> = remaining
            .iter()
            .take(limit.get())
            .map(|entry| (*entry).clone())
            .collect();
        let next = entries
            .last()
            .map_or(after, |entry| JournalCursor::new(entry.sequence.get()));
        JournalReadOutcome::Complete(JournalRead {
            entries,
            next,
            has_more: remaining.len() > limit.get(),
        })
    }

    fn compact(&mut self, through: JournalCursor) -> CompactionOutcome {
        if self.cursor == 0 || through.value() + 1 <= self.first_available {
            return CompactionOutcome::NoChange;
        }
        let removed = self
            .entries
            .iter()
            .take_while(|entry| entry.sequence.get() <= through.value())
            .count();
        for entry in self.entries.drain(..removed) {
            self.by_id.insert(entry.event.id, Location::Compacted);
        }
        self.first_available = through.value() + 1;
        CompactionOutcome::Compacted {
            removed,
            first_available: JournalCursor::new(self.first_available),
        }
    }
}

#[test]
fn configured_journal_histories_match_independent_model() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/feature_campaigns.json")
        });
    let config: JsonValue = std::fs::read_to_string(configured)
        .expect("campaign configuration is readable")
        .parse()
        .expect("campaign configuration is JSON");
    let seeds = number(&config, &["campaigns", "eventlog", "seeds"]);
    let transitions = number(&config, &["campaigns", "eventlog", "transitions"]);
    let identities = number(&config, &["campaigns", "eventlog", "identities"]);
    let page_limit = NonZeroUsize::new(
        usize::try_from(number(&config, &["campaigns", "eventlog", "page_limit"]))
            .expect("page limit fits usize"),
    )
    .expect("positive page limit");
    let capacity = NonZeroUsize::new(usize::try_from(transitions).expect("workload fits usize"))
        .expect("positive workload");
    let budget = Duration::from_secs(number(&config, &["budgets", "seconds"]));
    assert!(seeds >= 1_000 && transitions >= 100 && identities > 0);
    let namespace = KeyNamespace::<Domain>::new("journal").expect("campaign namespace");
    let key =
        |slot| StableKey::new(&namespace, [KeySegment::Unsigned(slot)]).expect("campaign key");
    let stream = JournalStream {
        journal: EventJournalId::from_stable(key(0)),
        aggregate: AggregateId::from_stable(key(1)),
    };
    let group = EventGroupId::new("campaign").expect("event group");
    let tag = EventTag::new("recorded").expect("event tag");
    let version = EventVersion::new(NonZeroU32::MIN);
    let started = Instant::now();
    let mut operations = [0_u64; 6];
    let mut decisions = [0_u64; 5];
    let mut inspections = [0_u64; 3];
    let mut reads = [0_u64; 3];
    let mut compactions = [0_u64; 2];
    let mut projections = [0_u64; 3];
    for seed in 0..seeds {
        let journal = Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: NonZeroUsize::MIN,
            entries_per_stream: capacity,
            tombstones_per_stream: capacity,
        }));
        let projection_store = Arc::new(MemoryProjectionStore::new(NonZeroUsize::MIN));
        let environment = Arc::new(Environment {
            journal: Arc::clone(&journal),
            projections: Arc::clone(&projection_store),
        });
        let projection = Projection::new(
            ProjectionKey {
                id: ProjectionId::from_stable(key(2)),
                stream: stream.clone(),
            },
            ProjectionVersion::new(NonZeroU32::MIN),
            BTreeMap::from([((group.clone(), tag.clone()), version)]),
            || 0usize,
            |count, _| Ok::<usize, ()>(count + 1),
        );
        let retention = journal.retention_authority();
        if seed == 0 {
            let unrelated = MemoryRetentionAuthority::new();
            assert_eq!(
                block_on(
                    journal
                        .compact(unrelated.permit(stream.clone(), JournalCursor::new(0)))
                        .run(Arc::new(()))
                ),
                Exit::Fail(MemoryJournalError::WrongRetentionAuthority)
            );
        }
        retention
            .replay_policy(stream.clone(), ReplayRetention::DiscardHistory)
            .expect("explicit discard policy");
        let mut model = Model::new();
        let mut projected: Option<(JournalCursor, usize)> = None;
        let mut generator = number(&config, &["seed"]).wrapping_add(seed).max(1);
        for _ in 0..transitions {
            let choice = usize::try_from(next(&mut generator) % operations.len() as u64)
                .expect("operation index fits usize");
            operations[choice] += 1;
            match choice {
                0 => {
                    let slot = next(&mut generator) % identities;
                    let id = EventId::from_stable(key(slot));
                    let event = if let Some(Location::Retained(entry)) = model.by_id.get(&id) {
                        if next(&mut generator) & 1 == 0 {
                            entry.event.clone()
                        } else {
                            let mut changed = entry.event.clone();
                            changed.payload = Arc::from(next(&mut generator).to_be_bytes());
                            changed
                        }
                    } else {
                        EventEnvelope {
                            journal: stream.journal.clone(),
                            aggregate: stream.aggregate.clone(),
                            id,
                            group: group.clone(),
                            tag: tag.clone(),
                            version,
                            payload: Arc::from(next(&mut generator).to_be_bytes()),
                            outcome: None,
                            occurred_at: UtcMillis::new(0),
                        }
                    };
                    let expected = JournalCursor::new(if next(&mut generator) & 1 == 0 {
                        model.cursor
                    } else {
                        model.cursor.saturating_sub(1)
                    });
                    let predicted = model.append(event.clone(), expected);
                    if let AppendReceipt::Decided(decision) = &predicted {
                        let index = match decision {
                            AppendDecision::Committed(_) => 0,
                            AppendDecision::Duplicate(_) => 1,
                            AppendDecision::ConflictingEventId => 2,
                            AppendDecision::CompactedEventId => 3,
                            AppendDecision::StaleCursor { .. } => 4,
                        };
                        decisions[index] += 1;
                    }
                    assert_eq!(
                        block_on(journal.append(event, expected).run(Arc::new(()))),
                        Exit::Success(predicted),
                        "seed {seed}: append disagrees with independent model"
                    );
                }
                1 => {
                    let id = EventId::from_stable(key(next(&mut generator) % identities));
                    let predicted = model.inspect(&id);
                    let index = match &predicted {
                        EventInspection::Present(_) => 0,
                        EventInspection::Compacted => 1,
                        EventInspection::Missing => 2,
                    };
                    inspections[index] += 1;
                    assert_eq!(
                        block_on(
                            journal
                                .inspect(stream.clone(), id.clone())
                                .run(Arc::new(()))
                        ),
                        Exit::Success(predicted),
                        "seed {seed}: inspection disagrees with independent model"
                    );
                }
                2 => {
                    let after = JournalCursor::new(next(&mut generator) % (model.cursor + 2));
                    let predicted = model.read(after, page_limit);
                    let index = match &predicted {
                        JournalReadOutcome::Complete(_) => 0,
                        JournalReadOutcome::HistoryTruncated { .. } => 1,
                        JournalReadOutcome::CursorAhead { .. } => 2,
                    };
                    reads[index] += 1;
                    assert_eq!(
                        block_on(
                            journal
                                .read(stream.clone(), after, page_limit)
                                .run(Arc::new(()))
                        ),
                        Exit::Success(predicted),
                        "seed {seed}: page disagrees with independent model"
                    );
                }
                3 => {
                    let through = JournalCursor::new(next(&mut generator) % (model.cursor + 1));
                    let predicted = model.compact(through);
                    compactions
                        [usize::from(matches!(predicted, CompactionOutcome::Compacted { .. }))] +=
                        1;
                    assert_eq!(
                        block_on(
                            journal
                                .compact(retention.permit(stream.clone(), through))
                                .run(Arc::new(()))
                        ),
                        Exit::Success(predicted),
                        "seed {seed}: compaction disagrees with independent model"
                    );
                }
                4 => {
                    let after = projected.map_or(JournalCursor::new(0), |(cursor, _)| cursor);
                    let current_count = projected.map_or(0, |(_, count)| count);
                    let predicted = model.read(after, page_limit);
                    let actual =
                        block_on(projection.advance(page_limit).run(Arc::clone(&environment)));
                    match predicted {
                        JournalReadOutcome::HistoryTruncated { first_available } => {
                            projections[2] += 1;
                            assert!(
                                matches!(actual, Exit::Fail(
                                ProjectionAdvanceError::HistoryTruncated { first_available: found }
                            ) if found == first_available),
                                "seed {seed}: projection did not reject truncated history"
                            );
                        }
                        JournalReadOutcome::Complete(page) if page.entries.is_empty() => {
                            projections[0] += 1;
                            assert!(
                                matches!(actual, Exit::Success(ProjectionAdvance::Current(snapshot))
                                if snapshot.checkpoint == after && *snapshot.state == current_count),
                                "seed {seed}: current projection disagrees with model"
                            );
                        }
                        JournalReadOutcome::Complete(page) => {
                            projections[1] += 1;
                            let next_count = current_count + page.entries.len();
                            assert!(
                                matches!(actual, Exit::Success(ProjectionAdvance::Advanced { snapshot, has_more })
                                if snapshot.checkpoint == page.next && *snapshot.state == next_count && has_more == page.has_more),
                                "seed {seed}: advanced projection disagrees with model"
                            );
                            projected = Some((page.next, next_count));
                        }
                        JournalReadOutcome::CursorAhead { .. } => {
                            unreachable!("a projection checkpoint cannot exceed this model journal")
                        }
                    }
                    let saved = block_on(
                        projection_store
                            .load(projection.key.clone())
                            .run(Arc::new(())),
                    );
                    match projected {
                        Some((checkpoint, count)) => assert!(matches!(saved,
                            Exit::Success(Some(snapshot))
                            if snapshot.checkpoint == checkpoint && *snapshot.state == count)),
                        None => assert!(matches!(saved, Exit::Success(None))),
                    }
                }
                _ => {}
            }
            assert_eq!(
                block_on(journal.cursor(stream.clone()).run(Arc::new(()))),
                Exit::Success(JournalCursor::new(model.cursor)),
                "seed {seed}: cursor disagrees with independent model"
            );
        }
    }
    assert!(operations.iter().all(|count| *count > 0));
    assert!(decisions.iter().all(|count| *count > 0));
    assert!(inspections.iter().all(|count| *count > 0));
    assert!(reads.iter().all(|count| *count > 0));
    assert!(compactions.iter().all(|count| *count > 0));
    assert!(projections.iter().all(|count| *count > 0));
    assert!(
        started.elapsed() <= budget,
        "event journal campaign exceeded its time budget"
    );
    campaign_report::completed(
        "eventlog",
        serde_json::json!({
            "seeds": seeds, "transitions": transitions,
            "operations": operations, "append_decisions": decisions,
            "inspections": inspections, "reads": reads,
            "compactions": compactions, "projections": projections,
        }),
    );
    eprintln!(
        "event journal/projection model completed {seeds} seeds x {transitions} transitions; operations {operations:?}; append decisions {decisions:?}; inspections {inspections:?}; reads {reads:?}; compactions {compactions:?}; projections {projections:?}"
    );
}
