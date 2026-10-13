use effect::{reactive_source, transaction};
use serde_json::json;
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
use effect::reactivity::{AtomId, AtomRegistry};
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

fn key(namespace: &KeyNamespace<Domain>, value: u64) -> Result<StableKey<Domain>, String> {
    StableKey::new(namespace, [KeySegment::Unsigned(value)])
        .map_err(|error| format!("stable key: {error:?}"))
}

fn run<A: Send + 'static, E: std::fmt::Debug + Send + 'static>(
    operation: Effect<A, E>,
) -> Result<A, String> {
    match block_on(operation.run(Arc::new(()))) {
        Exit::Success(value) => Ok(value),
        Exit::Fail(error) => Err(format!("contended failure: {error:?}")),
        Exit::Die(defect) => Err(format!("contended defect: {defect:?}")),
        Exit::Interrupted => Err("contended operation interrupted".into()),
        Exit::FinalizationFailed(error) => Err(format!("contended finalization: {error:?}")),
    }
}

fn number(config: &JsonValue, path: &[&str]) -> Result<u64, String> {
    path.iter()
        .try_fold(config, |value, field| {
            value
                .get(*field)
                .ok_or_else(|| format!("campaign field: {field}"))
        })?
        .as_u64()
        .ok_or_else(|| format!("campaign integer: {path:?}"))
}

fn contend(
    workers: usize,
    operations: usize,
    operation: impl Fn(u64) -> Result<(), String> + Send + Sync,
) -> Result<usize, String> {
    assert!(workers > 0 && operations >= workers);
    let first = Barrier::new(workers);
    let active = AtomicUsize::new(0);
    let overlap = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let operation = &operation;
        let first = &first;
        let active = &active;
        let overlap = &overlap;
        let mut handles = Vec::with_capacity(workers);
        for worker in 0..workers {
            handles.push(scope.spawn(move || -> Result<(), String> {
                for index in (worker..operations).step_by(workers) {
                    let prior = active.fetch_add(1, Ordering::AcqRel);
                    if prior > 0 {
                        overlap.fetch_add(1, Ordering::AcqRel);
                    }
                    if index == worker {
                        first.wait();
                    }
                    let result = u64::try_from(index)
                        .map_err(|error| format!("operation index: {error}"))
                        .and_then(operation);
                    active.fetch_sub(1, Ordering::AcqRel);
                    result?;
                }
                Ok(())
            }));
        }
        let results: Vec<_> = handles
            .into_iter()
            .map(std::thread::ScopedJoinHandle::join)
            .collect();
        for result in results {
            result.map_err(|panic| {
                format!("contention worker: {:?}", effect::Defect::from_panic(panic))
            })??;
        }
        Ok::<_, String>(())
    })?;
    assert_eq!(active.load(Ordering::Acquire), 0);
    let observed = overlap.load(Ordering::Acquire);
    if workers > 1 {
        assert!(observed > 0, "native operations did not overlap");
    }
    Ok(observed)
}

#[test]
fn configured_native_contention_preserves_eight_feature_invariants() -> Result<(), String> {
    let configured = std::env::var_os("EFFECT_CAMPAIGN_CONFIG")
        .map(std::fs::read_to_string)
        .transpose()
        .map_err(|error| format!("campaign configuration: {error}"))?
        .unwrap_or_else(|| include_str!("feature_campaigns.json").to_owned());
    let config: JsonValue =
        serde_json::from_str(&configured).map_err(|error| format!("campaign JSON: {error}"))?;
    let operations = usize::try_from(number(
        &config,
        &["campaigns", "native_contention", "operations_per_feature"],
    )?)
    .map_err(|error| format!("operation count: {error}"))?;
    let max_conflicts = NonZeroUsize::new(
        usize::try_from(number(
            &config,
            &["campaigns", "native_contention", "max_conflicts"],
        )?)
        .map_err(|error| format!("retry bound: {error}"))?,
    )
    .ok_or("zero retry bound")?;
    let levels = config
        .get("campaigns")
        .and_then(|campaigns| campaigns.get("native_contention"))
        .and_then(|native| native.get("levels"))
        .ok_or("missing concurrency levels")?
        .as_array()
        .ok_or("invalid concurrency levels")?;
    let namespace =
        KeyNamespace::<Domain>::new("native").map_err(|error| format!("namespace: {error:?}"))?;
    let mut overlap_by_level = Vec::new();

    for level in levels {
        let workers = usize::try_from(level.as_u64().ok_or("invalid worker count")?)
            .map_err(|error| format!("worker count: {error}"))?;
        let mut overlaps = [0_usize; 8];

        let schema = Arc::new(json_i64());
        overlaps[0] = contend(workers, operations, |index| {
            let value = i64::try_from(index).map_err(|error| format!("schema index: {error}"))?;
            let encoded = schema
                .encode_ref(&value)
                .map_err(|error| format!("schema encode: {error:?}"))?;
            assert_eq!(
                schema
                    .decode_ref(&encoded)
                    .map_err(|error| format!("schema decode: {error:?}"))?,
                value
            );
            Ok(())
        })?;

        let store = Arc::new(MemoryKeyValueStore::new(StoreCapacity::Unbounded));
        let record_key: Arc<[u8]> = Arc::from(&b"hot-record"[..]);
        overlaps[1] = contend(workers, operations, |index| {
            run(store.put(
                Arc::clone(&record_key),
                StoredValue {
                    bytes: Arc::from(index.to_be_bytes()),
                    expires_at: None,
                },
            ))?;
            Ok(())
        })?;
        let StoreRead::Found(entry) = run(store.get(record_key, UtcMillis::new(0)))? else {
            return Err("hot record missing".into());
        };
        assert_eq!(entry.revision.value(), operations as u64);

        let mut registry = RpcRegistry::new();
        let rpc_spec = RpcOperationSpec {
            id: OperationId::new("hot-operation")
                .map_err(|error| format!("operation identity: {error:?}"))?,
            version: RpcVersion::new(NonZeroU32::MIN),
            kind: RpcOperationKind::Unary,
        };
        registry
            .register(&rpc_spec)
            .map_err(|error| format!("register operation: {error:?}"))?;
        let registry = Arc::new(registry);
        overlaps[2] = contend(workers, operations, |_| {
            assert_eq!(
                registry.kind(&rpc_spec.id, rpc_spec.version),
                Some(rpc_spec.kind)
            );
            Ok(())
        })?;

        let graph = AtomRegistry::new(PubSubOptions {
            capacity: NonZeroUsize::MIN,
            overflow: PubSubOverflow::Backpressure,
            replay: ReplayPolicy::None,
        })
        .map_err(|error| format!("reactive registry: {error:?}"))?;
        let atom = reactive_source! { registry: graph, id: AtomId::from_stable(key(&namespace, 0)?), initial: 0_u64, capacity: NonZeroUsize::MIN, overflow: PubSubOverflow::Backpressure,  }
        .map_err(|error| format!("hot atom: {error:?}"))?;
        overlaps[3] = contend(workers, operations, |index| {
            run(atom.set::<()>(index))?;
            Ok(())
        })?;
        assert_eq!(run(atom.revision::<()>())?.get(), operations as u64 + 1);

        let cluster = Arc::new(MemoryClusterMessageStore::new(
            MessageStoreCapacity::Unbounded,
        ));
        let address = ClusterAddress {
            entity_type: ClusterEntityType::new("hot-entity")
                .map_err(|error| format!("entity type: {error:?}"))?,
            entity: ClusterEntityId::from_stable(key(&namespace, 1)?),
            shard: ClusterShardId::from_stable(key(&namespace, 2)?),
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
                    id: ClusterRequestId::from_stable(key(&namespace, index)?),
                    payload: Arc::from(index.to_be_bytes()),
                    idempotency_key: None,
                    accepted_at: UtcMillis::new(0),
                    policy,
                }))?,
                MessageAcceptance::Accepted { .. }
            ));
            Ok(())
        })?;
        assert_eq!(
            run(cluster.scan(MessageScan {
                address,
                limit: NonZeroUsize::new(operations).ok_or("zero scan limit")?,
            }))?
            .messages
            .len(),
            operations
        );

        let engine = Arc::new(MemoryWorkflowEngine::new());
        let definition = WorkflowDefinitionId::new("hot-workflow")
            .map_err(|error| format!("definition: {error:?}"))?;
        let version = WorkflowVersion::new(NonZeroU32::MIN);
        run(engine.register(WorkflowRegistration {
            id: definition.clone(),
            version,
            compatibility: Arc::from(&b"v1"[..]),
        }))?;
        overlaps[5] = contend(workers, operations, |index| {
            let run_id = WorkflowRunId::from_stable(key(&namespace, index)?);
            let snapshot = run(engine.start(
                WorkflowHandle {
                    run: run_id.clone(),
                    definition: definition.clone(),
                    version,
                },
                Arc::from(&b"input"[..]),
            ))?;
            assert_eq!(snapshot.handle.run, run_id);
            Ok(())
        })?;

        let journal = Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
            streams: NonZeroUsize::MIN,
            entries_per_stream: NonZeroUsize::new(operations).ok_or("zero journal capacity")?,
            tombstones_per_stream: NonZeroUsize::MIN,
        }));
        let stream = JournalStream {
            journal: EventJournalId::from_stable(key(&namespace, 3)?),
            aggregate: AggregateId::from_stable(key(&namespace, 4)?),
        };
        let group = EventGroupId::new("native").map_err(|error| format!("group: {error:?}"))?;
        let tag = EventTag::new("written").map_err(|error| format!("tag: {error:?}"))?;
        overlaps[6] = contend(workers, operations, |index| {
            let event = EventEnvelope {
                journal: stream.journal.clone(),
                aggregate: stream.aggregate.clone(),
                id: EventId::from_stable(key(&namespace, index)?),
                group: group.clone(),
                tag: tag.clone(),
                version: EventVersion::new(NonZeroU32::MIN),
                payload: Arc::from(index.to_be_bytes()),
                outcome: None,
                occurred_at: UtcMillis::new(0),
            };
            loop {
                let cursor = run(journal.cursor(stream.clone()))?;
                match run(journal.append(event.clone(), cursor))? {
                    AppendReceipt::Decided(AppendDecision::Committed(_)) => break,
                    AppendReceipt::Decided(AppendDecision::StaleCursor { .. }) => continue,
                    outcome @ (AppendReceipt::Accepted { .. } | AppendReceipt::Decided(_)) => {
                        return Err(format!("unexpected journal append outcome: {outcome:?}"));
                    }
                }
            }
            Ok(())
        })?;
        assert_eq!(run(journal.cursor(stream))?.value(), operations as u64);

        let domain = Arc::new(TransactionDomain::new(NonZeroUsize::MIN));
        let environment = Arc::new(TransactionEnvironment(Arc::clone(&domain)));
        let reference = domain
            .reference(0_u64)
            .map_err(|error| format!("hot transaction reference: {error:?}"))?;
        overlaps[7] = contend(workers, operations, |_| {
            let result = block_on(
                transaction! {
                modify; reference: reference, modify: |value| Ok::<_, ()>((*value + 1, ())),
                }
                .atomically(TransactionPolicy { max_conflicts })
                .run(Arc::clone(&environment)),
            );
            assert!(matches!(result, Exit::Success(())));
            Ok(())
        })?;
        let result = block_on(
            reference
                .read::<()>()
                .atomically(TransactionPolicy { max_conflicts })
                .run(Arc::clone(&environment)),
        );
        assert!(matches!(result, Exit::Success(value) if *value == operations as u64));

        overlap_by_level.push(json!({
            "workers": workers,
            "per_feature_operations": operations,
            "overlapped_operations": overlaps,
        }));
    }

    campaign_report::completed(
        "native_contention",
        json!({
            "operations_per_feature": operations,
            "levels": overlap_by_level,
            "feature_families": 8,
        }),
    )?;
    Ok(())
}
