//! Retained pages, continuation identity, and refusal in prepared stores.

use effect::eventlog::{
    AggregateId, CompactionOutcome, EventGroupId, EventId, EventJournalId, EventTag, EventVersion,
    JournalCursor, MemoryJournalCapacity, PreparedAppendDecision, PreparedEventHeader,
    PreparedJournalError, PreparedJournalReadOutcome, PreparedMemoryEventJournal,
    PreparedMemoryJournalCapacity, ReplayRetention,
};
use effect::memory::{Bytes, MemoryError, RegionId};
use effect::workflow::{
    EncodedWorkflowOutcome, PreparedWorkflowCapacity, PreparedWorkflowHistoryError,
    WorkflowActivityId, WorkflowDefinitionId, WorkflowEngine, WorkflowHandle,
    WorkflowHistoryRecord, WorkflowOutcomeKind, WorkflowRegistration, WorkflowRunId,
    WorkflowVersion,
};
use effect::{
    Effect, Exit, KeyDomain, KeyNamespace, KeySegment, StableKey, UtcMillis, memory, workflow,
};
use futures_executor::block_on;
use std::fmt::Debug;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;

const WORK: RegionId = RegionId::named("work");
const ONE: NonZeroUsize = NonZeroUsize::MIN;
const TWO: NonZeroUsize = ONE.saturating_add(1);
const THREE: NonZeroUsize = TWO.saturating_add(1);

struct TestDomain;

impl KeyDomain for TestDomain {
    const TAG: &'static str = "prepared-history-test";
}

fn checked<T, E: Debug>(result: Result<T, E>) -> Result<T, String> {
    result.map_err(|error| format!("{error:?}"))
}

fn key(segment: u64) -> Result<StableKey<TestDomain>, String> {
    let namespace = checked(KeyNamespace::new("prepared-history"))?;
    checked(StableKey::new(&namespace, [KeySegment::Unsigned(segment)]))
}

fn run<A: Send + 'static, E: Debug + Send + 'static>(effect: Effect<A, E>) -> Result<A, String> {
    match block_on(effect.run(Arc::new(()))) {
        Exit::Success(value) => Ok(value),
        Exit::Fail(error) => Err(format!("{error:?}")),
        Exit::Die(defect) => Err(format!("{defect:?}")),
        Exit::Interrupted => Err("interrupted".into()),
        Exit::FinalizationFailed(_) => Err("finalization failed".into()),
    }
}

#[test]
fn prepared_event_page_preserves_payload_across_compaction_and_refuses_while_retained()
-> Result<(), String> {
    let plan = checked(memory! {
        total: 128 KiB, mode: prepared, metadata: work, partition work: 128 KiB,
    })?;
    let prepared = checked(plan.prepare())?;
    let journal = checked(PreparedMemoryEventJournal::prepare(
        &prepared,
        WORK,
        PreparedMemoryJournalCapacity {
            journal: MemoryJournalCapacity {
                streams: ONE,
                entries_per_stream: TWO,
                tombstones_per_stream: ONE,
            },
            payloads: THREE,
            bytes_per_payload: Bytes::new(32),
            bytes_per_identity: Bytes::new(128),
            pages: ONE,
            entries_per_page: ONE,
        },
    ))?;
    let _sealed = prepared.seal();
    let header = PreparedEventHeader {
        journal: EventJournalId::from_stable(key(0)?),
        aggregate: AggregateId::from_stable(key(1)?),
        id: EventId::from_stable(key(2)?),
        group: checked(EventGroupId::new("prepared"))?,
        tag: checked(EventTag::new("recorded"))?,
        version: EventVersion::new(NonZeroU32::MIN),
        occurred_at: UtcMillis::new(0),
    };
    let stream = header.stream();
    let append = |expected| {
        let mut bytes = checked(journal.reserve_append(false))?;
        checked(bytes.extend_payload(b"original"))?;
        checked(journal.append_prepared(header.clone(), expected, bytes, None))
    };

    let zero = JournalCursor::new(0);
    let one = JournalCursor::new(1);
    assert!(matches!(
        append(zero)?,
        PreparedAppendDecision::Committed(_)
    ));
    assert!(matches!(
        append(zero)?,
        PreparedAppendDecision::Duplicate(_)
    ));
    let PreparedJournalReadOutcome::Complete(retained) =
        checked(journal.read_prepared(&stream, zero, ONE))?
    else {
        return Err("expected journal page".into());
    };
    assert_eq!(retained.next, one);
    assert!(!retained.has_more);
    assert!(matches!(
        journal.read_prepared(&stream, zero, ONE),
        Err(PreparedJournalError::Memory(MemoryError::NoSlot))
    ));

    let authority = journal.retention_authority();
    checked(authority.replay_policy(stream.clone(), ReplayRetention::DiscardHistory))?;
    assert!(matches!(
        checked(journal.compact_prepared(authority.permit(stream.clone(), one)))?,
        CompactionOutcome::Compacted { .. }
    ));
    assert!(matches!(
        append(one)?,
        PreparedAppendDecision::CompactedEventId
    ));
    assert!(matches!(
        checked(journal.read_prepared(&stream, zero, ONE))?,
        PreparedJournalReadOutcome::HistoryTruncated { first_available } if first_available > zero
    ));
    let [entry] = retained.entries() else {
        return Err("retained page lost its entry".into());
    };
    assert_eq!(entry.payload.as_ref(), b"original");
    drop(retained);
    let PreparedJournalReadOutcome::Complete(after) =
        checked(journal.read_prepared(&stream, one, ONE))?
    else {
        return Err("expected page at the head".into());
    };
    assert!(after.entries().is_empty());
    Ok(())
}

#[test]
fn prepared_workflow_history_continuations_fence_changes_but_pages_remain_valid()
-> Result<(), String> {
    let plan = checked(memory! {
        total: 128 KiB, mode: prepared, metadata: work, partition work: 128 KiB,
    })?;
    let prepared = checked(plan.prepare())?;
    let engine = checked(workflow! {
        prepare_engine;
        memory: &prepared, region: WORK,
        capacity: PreparedWorkflowCapacity {
            definitions: ONE, runs: ONE, activities_per_run: THREE, children_per_run: ONE,
            signals_per_run: ONE, compensations_per_run: ONE, races_per_run: ONE,
            pages: ONE, records_per_page: ONE, bytes_per_identity: Bytes::new(128),
            bytes_per_payload: Bytes::new(32), bytes_per_run: Bytes::new(256),
        },
    })?;
    let _sealed = prepared.seal();
    let definition = checked(WorkflowDefinitionId::new("prepared"))?;
    let version = WorkflowVersion::new(NonZeroU32::MIN);
    run(engine.register(WorkflowRegistration {
        id: definition.clone(),
        version,
        compatibility: Arc::from(&b"v1"[..]),
    }))?;
    let run_id = WorkflowRunId::from_stable(key(0)?);
    run(engine.start_head(
        WorkflowHandle {
            run: run_id.clone(),
            definition,
            version,
        },
        Arc::from(&b"input"[..]),
    ))?;
    let record = |identity| {
        run(engine.record_activity(
            run_id.clone(),
            identity,
            NonZeroU32::MIN,
            Arc::from(&b"request"[..]),
            EncodedWorkflowOutcome {
                kind: WorkflowOutcomeKind::Succeeded,
                payload: Arc::from(&b"done"[..]),
            },
        ))
    };
    let first = WorkflowActivityId::from_stable(key(1)?);
    let second = WorkflowActivityId::from_stable(key(2)?);
    record(first.clone())?;
    record(second.clone())?;

    let retained = checked(engine.read_history_page(&run_id, None, ONE))?;
    let continuation = retained.next_cursor().ok_or("missing continuation")?;
    assert!(matches!(
        engine.read_history_page(&run_id, Some(continuation), ONE),
        Err(PreparedWorkflowHistoryError::Memory(MemoryError::NoSlot))
    ));
    let original = record(first.clone())?;
    assert_eq!(original.identity, first);
    let third = WorkflowActivityId::from_stable(key(3)?);
    record(third)?;
    assert!(matches!(
        engine.read_history_page(&run_id, Some(continuation), ONE),
        Err(PreparedWorkflowHistoryError::HistoryChanged)
    ));
    assert!(matches!(
        retained.records(),
        [WorkflowHistoryRecord::Activity(entry)] if entry.identity == first
    ));
    drop(retained);
    let fresh = checked(engine.read_history_page(&run_id, None, ONE))?;
    let next = fresh
        .into_next_cursor()
        .ok_or("missing fresh continuation")?;
    let later = checked(engine.read_history_page(&run_id, Some(&next), ONE))?;
    assert!(matches!(
        later.records(),
        [WorkflowHistoryRecord::Activity(entry)] if entry.identity == second
    ));
    Ok(())
}

#[test]
fn prepared_cluster_claim_refusal_does_not_consume_a_fenced_attempt() -> Result<(), String> {
    use effect::cluster::{
        AcceptancePolicy, ActivationId, ActivationOutcome, ClaimOptions, ClusterActivationStore,
        ClusterAddress, ClusterEntityId, ClusterEntityType, ClusterRequestId, ClusterShardId,
        ClusterStoreError, MessageAcceptance, PayloadConflictPolicy, PreparedAcceptedRequest,
        PreparedClusterActivationCapacity, PreparedClusterStoreCapacity, PreparedClusterStoreError,
        ReplyKind, StoreCompletion,
    };
    use std::time::Duration;

    let plan = checked(memory! {
        total: 128 KiB, mode: prepared, metadata: work, partition work: 128 KiB,
    })?;
    let prepared = checked(plan.prepare())?;
    let store = checked(effect::cluster! {
        memory: &prepared, region: WORK,
        store_capacity: PreparedClusterStoreCapacity {
            activation: PreparedClusterActivationCapacity {
                addresses: ONE, identity_bytes_per_address: Bytes::new(512),
            },
            records: ONE, chunks_per_record: ONE, observers_per_record: ONE,
            identity_bytes_per_record: Bytes::new(512), identity_bytes_per_observer: Bytes::new(128),
            payloads: TWO, bytes_per_payload: Bytes::new(32), snapshots: ONE, records_per_page: ONE,
        },
    })?;
    let _sealed = prepared.seal();
    let address = ClusterAddress {
        entity_type: checked(ClusterEntityType::new("prepared"))?,
        entity: ClusterEntityId::from_stable(key(0)?),
        shard: ClusterShardId::from_stable(key(1)?),
    };
    let request = ClusterRequestId::from_stable(key(2)?);
    let reserve = |bytes: &[u8]| {
        let mut payload = checked(store.reserve_prepared_payload())?;
        checked(payload.extend_from_slice(bytes))?;
        Ok::<_, String>(payload)
    };
    assert!(matches!(
        checked(store.accept_prepared_now(
            PreparedAcceptedRequest {
                address: address.clone(),
                id: request.clone(),
                idempotency_key: None,
                accepted_at: UtcMillis::new(1),
                policy: AcceptancePolicy {
                    dedup_retention: Duration::ZERO,
                    reply_retention: Duration::ZERO,
                    payload_conflict: PayloadConflictPolicy::Reject,
                    maximum_attempts: NonZeroU32::MIN.saturating_add(1),
                },
            },
            reserve(b"request")?,
        ))?,
        MessageAcceptance::Accepted { .. }
    ));
    let activate = |identity, start, expires| -> Result<_, String> {
        let outcome = run(store.activate(
            address.clone(),
            ActivationId::from_stable(key(identity)?),
            UtcMillis::new(start),
            UtcMillis::new(expires),
        ))?;
        match outcome {
            ActivationOutcome::Activated { lease, .. } => Ok(lease.owner),
            ActivationOutcome::AlreadyActive(_) | ActivationOutcome::Busy(_) => {
                Err("expected a newly acquired activation".into())
            }
        }
    };
    let first = activate(3, 1, 5)?;
    let retained = checked(store.claim_prepared_now(ClaimOptions {
        owner: first,
        now: UtcMillis::new(1),
        lease_until: UtcMillis::new(5),
        limit: ONE,
    }))?;
    let [old] = retained.as_slice() else {
        return Err("expected first claim".into());
    };
    assert_eq!(old.payload.as_ref(), b"request");
    assert_eq!(old.token.attempt, NonZeroU32::MIN);
    let old_token = old.token.clone();

    let next = ClaimOptions {
        owner: activate(4, 6, 10)?,
        now: UtcMillis::new(6),
        lease_until: UtcMillis::new(10),
        limit: ONE,
    };
    assert!(matches!(
        store.claim_prepared_now(next.clone()),
        Err(PreparedClusterStoreError::Memory(MemoryError::NoSlot))
    ));
    assert!(matches!(
        store.complete_prepared_now(
            old_token,
            ReplyKind::Success,
            reserve(b"stale")?,
            UtcMillis::new(6)
        ),
        Err(PreparedClusterStoreError::Store(
            ClusterStoreError::WrongOwner
        ))
    ));
    assert_eq!(old.payload.as_ref(), b"request");
    drop(retained);
    let recovered = checked(store.claim_prepared_now(next))?;
    let [claim] = recovered.as_slice() else {
        return Err("expected recovered claim".into());
    };
    assert_eq!(claim.token.attempt, NonZeroU32::MIN.saturating_add(1));
    assert_eq!(
        checked(store.complete_prepared_now(
            claim.token.clone(),
            ReplyKind::Success,
            reserve(b"reply")?,
            UtcMillis::new(7),
        ))?,
        StoreCompletion::Completed
    );
    assert_eq!(claim.payload.as_ref(), b"request");
    Ok(())
}
