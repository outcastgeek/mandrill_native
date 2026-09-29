#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::cluster::{
    AcceptancePolicy, AcceptedRequest, ClusterAddress, ClusterEntityId, ClusterEntityType,
    ClusterMessageStore, ClusterRequestId, ClusterShardId, MemoryClusterMessageStore,
    MessageAcceptance, MessageScan, MessageStoreCapacity, PayloadConflictPolicy,
};
use effect::eventlog::{
    AggregateId, AppendDecision, AppendReceipt, EventEnvelope, EventGroupId, EventId, EventJournal,
    EventJournalId, EventTag, EventVersion, JournalStream, MemoryEventJournal,
    MemoryJournalCapacity,
};
use effect::persistence::{
    KeyValueStore, MemoryKeyValueStore, StoreCapacity, StoreRead, StoredValue,
};
use effect::reactivity::{AtomId, AtomRef, AtomRegistry};
use effect::rpc::{OperationId, RpcOperationKind, RpcOperationSpec, RpcRegistry, RpcVersion};
use effect::transactional::{HasTransactionDomain, TransactionDomain, TransactionPolicy};
use effect::workflow::{
    MemoryWorkflowEngine, WorkflowDefinitionId, WorkflowEngine, WorkflowHandle,
    WorkflowRegistration, WorkflowRunId, WorkflowVersion,
};
use effect::{
    Effect, Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, PubSubOptions, PubSubOverflow,
    ReplayPolicy, StableKey, UtcMillis, json_i64,
};
use futures_executor::block_on;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "native-contention-campaign";
}

struct TransactionEnvironment(Arc<TransactionDomain>);
impl HasTransactionDomain for TransactionEnvironment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.0)
    }
}

fn key(namespace: &KeyNamespace<Domain>, value: u64) -> StableKey<Domain> {
    StableKey::new(namespace, [KeySegment::Unsigned(value)]).expect("stable key")
}

fn run<A: Send + 'static, E: Send + 'static>(operation: Effect<A, E>) -> A {
    match block_on(operation.run(Arc::new(()))) {
        Exit::Success(value) => value,
        _ => panic!("contended operation failed"),
    }
}

fn number(config: &JsonValue, path: &[&str]) -> u64 {
    path.iter()
        .fold(config, |value, field| {
            value.get(*field).expect("campaign field")
        })
        .as_u64()
        .expect("campaign integer")
}

fn contend(workers: usize, operations: usize, operation: impl Fn(u64) + Send + Sync) -> usize {
    assert!(workers > 0 && operations >= workers);
    let first = Barrier::new(workers);
    let active = AtomicUsize::new(0);
    let overlap = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let operation = &operation;
        let first = &first;
        let active = &active;
        let overlap = &overlap;
        for worker in 0..workers {
            scope.spawn(move || {
                for index in (worker..operations).step_by(workers) {
                    let prior = active.fetch_add(1, Ordering::AcqRel);
                    if prior > 0 {
                        overlap.fetch_add(1, Ordering::AcqRel);
                    }
                    if index == worker {
                        first.wait();
                    }
                    operation(u64::try_from(index).expect("operation index fits u64"));
                    active.fetch_sub(1, Ordering::AcqRel);
                }
            });
        }
    });
    assert_eq!(active.load(Ordering::Acquire), 0);
    let observed = overlap.load(Ordering::Acquire);
    if workers > 1 {
        assert!(observed > 0, "native operations did not overlap");
    }
    observed
}

#[test]
fn configured_native_contention_preserves_eight_feature_invariants() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(|path| std::fs::read_to_string(path).expect("campaign configuration"))
        .unwrap_or_else(|| include_str!("feature_campaigns.json").to_owned());
    let config: JsonValue = serde_json::from_str(&configured).expect("campaign JSON");
    let operations = usize::try_from(number(
        &config,
        &["campaigns", "native_contention", "operations_per_feature"],
    ))
    .expect("operation count fits usize");
    let max_conflicts = NonZeroUsize::new(
        usize::try_from(number(
            &config,
            &["campaigns", "native_contention", "max_conflicts"],
        ))
        .expect("retry bound fits usize"),
    )
    .expect("positive retry bound");
    let levels = config["campaigns"]["native_contention"]["levels"]
        .as_array()
        .expect("concurrency levels");
    let namespace = KeyNamespace::<Domain>::new("native").expect("namespace");
    let mut overlap_by_level = Vec::new();

    for level in levels {
        let workers = usize::try_from(level.as_u64().expect("worker count"))
            .expect("worker count fits usize");
        let mut overlaps = [0_usize; 8];

        let schema = Arc::new(json_i64());
        overlaps[0] = contend(workers, operations, |index| {
            let value = i64::try_from(index).expect("index fits i64");
            let encoded = schema.encode_ref(&value).expect("schema encode");
            assert_eq!(schema.decode_ref(&encoded).expect("schema decode"), value);
        });

        let store = Arc::new(MemoryKeyValueStore::new(StoreCapacity::Unbounded));
        let record_key: Arc<[u8]> = Arc::from(&b"hot-record"[..]);
        overlaps[1] = contend(workers, operations, |index| {
            run(store.put(
                Arc::clone(&record_key),
                StoredValue {
                    bytes: Arc::from(index.to_be_bytes()),
                    expires_at: None,
                },
            ));
        });
        let StoreRead::Found(entry) = run(store.get(record_key, UtcMillis::new(0))) else {
            panic!("hot record missing");
        };
        assert_eq!(entry.revision.value(), operations as u64);

        let mut registry = RpcRegistry::new();
        let rpc_spec = RpcOperationSpec {
            id: OperationId::new("hot-operation").expect("operation identity"),
            version: RpcVersion::new(NonZeroU32::MIN),
            kind: RpcOperationKind::Unary,
        };
        registry.register(&rpc_spec).expect("register operation");
        let registry = Arc::new(registry);
        overlaps[2] = contend(workers, operations, |_| {
            assert_eq!(
                registry.kind(&rpc_spec.id, rpc_spec.version),
                Some(rpc_spec.kind)
            );
        });

        let graph = AtomRegistry::new(PubSubOptions {
            capacity: NonZeroUsize::MIN,
            overflow: PubSubOverflow::Backpressure,
            replay: ReplayPolicy::None,
        })
        .expect("reactive registry");
        let atom = AtomRef::new(
            graph,
            AtomId::from_stable(key(&namespace, 0)),
            0_u64,
            NonZeroUsize::MIN,
            PubSubOverflow::Backpressure,
        )
        .expect("hot atom");
        overlaps[3] = contend(workers, operations, |index| {
            run(atom.set::<()>(index));
        });
        assert_eq!(run(atom.revision::<()>()).get(), operations as u64 + 1);

        let cluster = Arc::new(MemoryClusterMessageStore::new(
            MessageStoreCapacity::Unbounded,
        ));
        let address = ClusterAddress {
            entity_type: ClusterEntityType::new("hot-entity").expect("entity type"),
            entity: ClusterEntityId::from_stable(key(&namespace, 1)),
            shard: ClusterShardId::from_stable(key(&namespace, 2)),
        };
        let policy = AcceptancePolicy {
            dedup_retention: Duration::ZERO,
            reply_retention: Duration::ZERO,
            payload_conflict: PayloadConflictPolicy::Reject,
            maximum_attempts: NonZeroU32::MIN,
        };
        overlaps[4] = contend(workers, operations, |index| {
            assert!(matches!(
                run(cluster.accept(AcceptedRequest {
                    address: address.clone(),
                    id: ClusterRequestId::from_stable(key(&namespace, index)),
                    payload: Arc::from(index.to_be_bytes()),
                    idempotency_key: None,
                    accepted_at: UtcMillis::new(0),
                    policy,
                })),
                MessageAcceptance::Accepted { .. }
            ));
        });
        assert_eq!(
            run(cluster.scan(MessageScan {
                address,
                limit: NonZeroUsize::new(operations).expect("positive scan limit"),
            }))
            .messages
            .len(),
            operations
        );

        let engine = Arc::new(MemoryWorkflowEngine::new());
        let definition = WorkflowDefinitionId::new("hot-workflow").expect("definition");
        let version = WorkflowVersion::new(NonZeroU32::MIN);
        run(engine.register(WorkflowRegistration {
            id: definition.clone(),
            version,
            compatibility: Arc::from(&b"v1"[..]),
        }));
        overlaps[5] = contend(workers, operations, |index| {
            let run_id = WorkflowRunId::from_stable(key(&namespace, index));
            let snapshot = run(engine.start(
                WorkflowHandle {
                    run: run_id.clone(),
                    definition: definition.clone(),
                    version,
                },
                Arc::from(&b"input"[..]),
            ));
            assert_eq!(snapshot.handle.run, run_id);
        });

        let journal = Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: NonZeroUsize::MIN,
            entries_per_stream: NonZeroUsize::new(operations).expect("journal capacity"),
            tombstones_per_stream: NonZeroUsize::MIN,
        }));
        let stream = JournalStream {
            journal: EventJournalId::from_stable(key(&namespace, 3)),
            aggregate: AggregateId::from_stable(key(&namespace, 4)),
        };
        let group = EventGroupId::new("native").expect("group");
        let tag = EventTag::new("written").expect("tag");
        overlaps[6] = contend(workers, operations, |index| {
            let event = EventEnvelope {
                journal: stream.journal.clone(),
                aggregate: stream.aggregate.clone(),
                id: EventId::from_stable(key(&namespace, index)),
                group: group.clone(),
                tag: tag.clone(),
                version: EventVersion::new(NonZeroU32::MIN),
                payload: Arc::from(index.to_be_bytes()),
                outcome: None,
                occurred_at: UtcMillis::new(0),
            };
            loop {
                let cursor = run(journal.cursor(stream.clone()));
                match run(journal.append(event.clone(), cursor)) {
                    AppendReceipt::Decided(AppendDecision::Committed(_)) => break,
                    AppendReceipt::Decided(AppendDecision::StaleCursor { .. }) => continue,
                    _ => panic!("unexpected journal append outcome"),
                }
            }
        });
        assert_eq!(run(journal.cursor(stream)).value(), operations as u64);

        let domain = Arc::new(TransactionDomain::new(NonZeroUsize::MIN));
        let environment = Arc::new(TransactionEnvironment(Arc::clone(&domain)));
        let reference = domain.reference(0_u64).expect("hot transaction reference");
        overlaps[7] = contend(workers, operations, |_| {
            let result = block_on(
                reference
                    .update(|value| Ok::<_, ()>((*value + 1, ())))
                    .atomically(TransactionPolicy { max_conflicts })
                    .run(Arc::clone(&environment)),
            );
            assert!(matches!(result, Exit::Success(())));
        });
        let result = block_on(
            reference
                .read::<()>()
                .atomically(TransactionPolicy { max_conflicts })
                .run(Arc::clone(&environment)),
        );
        assert!(matches!(result, Exit::Success(value) if *value == operations as u64));

        overlap_by_level.push(serde_json::json!({
            "workers": workers,
            "per_feature_operations": operations,
            "overlapped_operations": overlaps,
        }));
    }

    campaign_report::completed(
        "native_contention",
        serde_json::json!({
            "operations_per_feature": operations,
            "levels": overlap_by_level,
            "feature_families": 8,
        }),
    );
}
