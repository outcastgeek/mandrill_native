#[path = "support/campaign_report.rs"]
mod campaign_report;

use effect::cluster::{
    AcceptancePolicy, AcceptedRequest, ClusterAddress, ClusterEntityId, ClusterEntityType,
    ClusterMessageStore, ClusterRequestId, ClusterShardId, MemoryClusterMessageStore,
    MessageAcceptance, MessageStoreCapacity, PayloadConflictPolicy,
};
use effect::eventlog::{
    AggregateId, AppendDecision, AppendReceipt, EventEnvelope, EventGroupId, EventId, EventJournal,
    EventJournalId, EventTag, EventVersion, JournalCursor, JournalStream, MemoryEventJournal,
    MemoryJournalCapacity,
};
use effect::persistence::{
    KeyValueStore, MemoryKeyValueStore, StoreCapacity, StoreRead, StoredValue,
};
use effect::reactivity::{AtomId, AtomRef, AtomRegistry};
use effect::rpc::{OperationId, RpcOperationKind, RpcOperationSpec, RpcRegistry, RpcVersion};
use effect::transactional::{HasTransactionDomain, TransactionDomain, TransactionPolicy};
use effect::workflow::{
    EncodedWorkflowOutcome, MemoryWorkflowEngine, WorkflowActivityId, WorkflowDefinitionId,
    WorkflowEngine, WorkflowHandle, WorkflowOutcomeKind, WorkflowRegistration, WorkflowRunId,
    WorkflowStatus, WorkflowVersion,
};
use effect::{
    Effect, Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, PubSubOptions, PubSubOverflow,
    PubSubShutdown, ReplayPolicy, StableKey, UtcMillis, json_i64,
};
use futures_executor::block_on;
use std::num::NonZeroU32;
use std::num::NonZeroUsize;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "lifecycle-retention-campaign";
}

struct TransactionEnvironment(Arc<TransactionDomain>);
impl HasTransactionDomain for TransactionEnvironment {
    fn transaction_domain(&self) -> Arc<TransactionDomain> {
        Arc::clone(&self.0)
    }
}

struct DropProbe(Arc<AtomicUsize>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn key(namespace: &KeyNamespace<Domain>, number: u64) -> StableKey<Domain> {
    StableKey::new(namespace, [KeySegment::Unsigned(number)]).expect("stable key")
}

fn run<A: Send + 'static, E: Send + 'static>(operation: Effect<A, E>) -> A {
    match block_on(operation.run(Arc::new(()))) {
        Exit::Success(value) => value,
        _ => panic!("lifecycle operation failed"),
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

#[test]
fn configured_lifecycle_cycles_release_all_eight_feature_families() {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(|path| std::fs::read_to_string(path).expect("campaign configuration"))
        .unwrap_or_else(|| include_str!("feature_campaigns.json").to_owned());
    let config: JsonValue = serde_json::from_str(&configured).expect("campaign JSON");
    let cycles = number(&config, &["campaigns", "lifecycle_retention", "cycles"]);
    let warmup = number(
        &config,
        &["campaigns", "lifecycle_retention", "warmup_cycles"],
    );
    assert!(warmup < cycles);
    let namespace = KeyNamespace::<Domain>::new("lifecycle").expect("namespace");
    let drops = Arc::new(AtomicUsize::new(0));
    let mut retained_after_disposal = 0_u64;
    let mut maximum_retained_after_warmup = 0_u64;

    for cycle in 0..cycles {
        let identity = key(&namespace, cycle);
        let codec = Arc::new(json_i64());
        let codec_weak = Arc::downgrade(&codec);
        let encoded = codec
            .encode_ref(&i64::try_from(cycle).expect("cycle fits i64"))
            .expect("encode schema value");
        assert_eq!(
            codec.decode_ref(&encoded).expect("decode schema value"),
            cycle as i64
        );
        drop(codec);
        retained_after_disposal += u64::from(codec_weak.upgrade().is_some());

        let store = Arc::new(MemoryKeyValueStore::new(StoreCapacity::Unbounded));
        let store_weak = Arc::downgrade(&store);
        let record_key: Arc<[u8]> = Arc::from(identity.as_bytes());
        run(store.put(
            Arc::clone(&record_key),
            StoredValue {
                bytes: Arc::from(&b"record"[..]),
                expires_at: None,
            },
        ));
        assert!(matches!(
            run(store.get(record_key, UtcMillis::new(0))),
            StoreRead::Found(_)
        ));
        drop(store);
        retained_after_disposal += u64::from(store_weak.upgrade().is_some());

        let mut registry = RpcRegistry::new();
        let rpc_spec = RpcOperationSpec {
            id: OperationId::new("lifecycle").expect("RPC identity"),
            version: RpcVersion::new(NonZeroU32::MIN),
            kind: RpcOperationKind::Unary,
        };
        registry
            .register(&rpc_spec)
            .expect("register RPC operation");
        assert_eq!(
            registry.kind(&rpc_spec.id, rpc_spec.version),
            Some(rpc_spec.kind)
        );
        let registry = Arc::new(registry);
        let registry_weak = Arc::downgrade(&registry);
        drop(registry);
        retained_after_disposal += u64::from(registry_weak.upgrade().is_some());

        let graph = AtomRegistry::new(PubSubOptions {
            capacity: NonZeroUsize::MIN,
            overflow: PubSubOverflow::Backpressure,
            replay: ReplayPolicy::None,
        })
        .expect("reactive graph");
        let atom_id = AtomId::from_stable(identity.clone());
        let atom = AtomRef::new(
            graph.clone(),
            atom_id.clone(),
            DropProbe(Arc::clone(&drops)),
            NonZeroUsize::MIN,
            PubSubOverflow::Backpressure,
        )
        .expect("register atom");
        let subscription = run(graph.invalidations().subscribe::<()>());
        let value = run(atom.read::<()>());
        drop(value);
        assert!(run(graph.remove::<()>(atom_id)));
        assert!(run(subscription.unsubscribe::<()>()));
        drop(subscription);
        assert!(run(graph.dispose::<()>(PubSubShutdown::Discard)));
        drop(atom);
        drop(graph);

        let cluster = Arc::new(MemoryClusterMessageStore::new(
            MessageStoreCapacity::Unbounded,
        ));
        let cluster_weak = Arc::downgrade(&cluster);
        let address = ClusterAddress {
            entity_type: ClusterEntityType::new("lifecycle").expect("entity type"),
            entity: ClusterEntityId::from_stable(identity.clone()),
            shard: ClusterShardId::from_stable(identity.clone()),
        };
        let request = ClusterRequestId::from_stable(identity.clone());
        assert!(matches!(
            run(cluster.accept(AcceptedRequest {
                address: address.clone(),
                id: request.clone(),
                payload: Arc::from(&b"request"[..]),
                idempotency_key: None,
                accepted_at: UtcMillis::new(0),
                policy: AcceptancePolicy {
                    dedup_retention: Duration::ZERO,
                    reply_retention: Duration::ZERO,
                    payload_conflict: PayloadConflictPolicy::Reject,
                    maximum_attempts: NonZeroU32::MIN,
                },
            })),
            MessageAcceptance::Accepted { .. }
        ));
        assert!(run(cluster.inspect(address, request)).is_some());
        drop(cluster);
        retained_after_disposal += u64::from(cluster_weak.upgrade().is_some());

        let engine = Arc::new(MemoryWorkflowEngine::new());
        let engine_weak = Arc::downgrade(&engine);
        let definition = WorkflowDefinitionId::new("lifecycle").expect("workflow identity");
        let version = WorkflowVersion::new(NonZeroU32::MIN);
        run(engine.register(WorkflowRegistration {
            id: definition.clone(),
            version,
            compatibility: Arc::from(&b"v1"[..]),
        }));
        let handle = WorkflowHandle {
            run: WorkflowRunId::from_stable(identity.clone()),
            definition,
            version,
        };
        assert_eq!(
            run(engine.start(handle.clone(), Arc::from(&b"input"[..]))).status,
            WorkflowStatus::Ready
        );
        assert_eq!(
            run(engine.transition(
                handle.run.clone(),
                WorkflowStatus::Ready,
                WorkflowStatus::Running
            ))
            .status,
            WorkflowStatus::Running
        );
        let activity = WorkflowActivityId::from_stable(identity.clone());
        let failed = EncodedWorkflowOutcome {
            kind: WorkflowOutcomeKind::Failed,
            payload: Arc::from(&b"retry"[..]),
        };
        let succeeded = EncodedWorkflowOutcome {
            kind: WorkflowOutcomeKind::Succeeded,
            payload: Arc::from(&b"done"[..]),
        };
        run(engine.record_activity(
            handle.run.clone(),
            activity.clone(),
            NonZeroU32::MIN,
            Arc::from(&b"step"[..]),
            failed,
        ));
        run(engine.record_activity(
            handle.run.clone(),
            activity,
            NonZeroU32::new(2).expect("second attempt"),
            Arc::from(&b"step"[..]),
            succeeded,
        ));
        assert_eq!(
            run(engine.transition(
                handle.run.clone(),
                WorkflowStatus::Running,
                WorkflowStatus::Suspended
            ))
            .status,
            WorkflowStatus::Suspended
        );
        assert_eq!(
            run(engine.transition(handle.run, WorkflowStatus::Suspended, WorkflowStatus::Ready))
                .status,
            WorkflowStatus::Ready
        );
        drop(engine);
        retained_after_disposal += u64::from(engine_weak.upgrade().is_some());

        let journal = Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: NonZeroUsize::MIN,
            entries_per_stream: NonZeroUsize::MIN,
            tombstones_per_stream: NonZeroUsize::MIN,
        }));
        let journal_weak = Arc::downgrade(&journal);
        let stream = JournalStream {
            journal: EventJournalId::from_stable(identity.clone()),
            aggregate: AggregateId::from_stable(identity.clone()),
        };
        let event = EventEnvelope {
            journal: stream.journal.clone(),
            aggregate: stream.aggregate.clone(),
            id: EventId::from_stable(identity.clone()),
            group: EventGroupId::new("lifecycle").expect("event group"),
            tag: EventTag::new("recorded").expect("event tag"),
            version: EventVersion::new(NonZeroU32::MIN),
            payload: Arc::from(&b"event"[..]),
            outcome: None,
            occurred_at: UtcMillis::new(0),
        };
        assert!(matches!(
            run(journal.append(event, JournalCursor::new(0))),
            AppendReceipt::Decided(AppendDecision::Committed(_))
        ));
        assert_eq!(run(journal.cursor(stream)).value(), 1);
        drop(journal);
        retained_after_disposal += u64::from(journal_weak.upgrade().is_some());

        let transaction = Arc::new(TransactionDomain::new(NonZeroUsize::MIN));
        let transaction_weak = Arc::downgrade(&transaction);
        let reference = transaction
            .reference(i64::try_from(cycle).expect("cycle fits i64"))
            .expect("transaction reference");
        let environment = Arc::new(TransactionEnvironment(Arc::clone(&transaction)));
        let observed = block_on(
            reference
                .read::<()>()
                .atomically(TransactionPolicy {
                    max_conflicts: NonZeroUsize::MIN,
                })
                .run(Arc::clone(&environment)),
        );
        assert!(matches!(observed, Exit::Success(_)));
        drop(observed);
        drop(reference);
        drop(environment);
        drop(transaction);
        retained_after_disposal += u64::from(transaction_weak.upgrade().is_some());

        assert_eq!(
            drops.load(Ordering::SeqCst),
            usize::try_from(cycle + 1).expect("probe count fits usize")
        );
        if cycle >= warmup {
            maximum_retained_after_warmup =
                maximum_retained_after_warmup.max(retained_after_disposal);
        }
        assert_eq!(
            retained_after_disposal, 0,
            "released resources remain retained"
        );
    }

    campaign_report::completed(
        "lifecycle_retention",
        serde_json::json!({
            "cycles": cycles,
            "warmup_cycles": warmup,
            "feature_families": 8,
            "released_payloads": drops.load(Ordering::SeqCst),
            "maximum_retained_after_warmup": maximum_retained_after_warmup,
        }),
    );
}
