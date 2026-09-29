use effect::encoding::{BinaryIdentity, BinaryLimits, FramePolicy};
use effect::eventlog::{
    AggregateId, EventBinaryFormats, EventCodecs, EventCommitPolicy, EventDefinition, EventGroupId,
    EventId, EventInvocation, EventJournal, EventJournalId, EventLog, EventRunOutcome, EventShape,
    EventTag, EventVersion, HasEventJournal, JournalCursor, JournalStream, MemoryEventJournal,
    MemoryJournalCapacity,
};
use effect::reactivity::{
    InvalidationKey, InvalidationRegistry, RpcMiddlewareMutationAtom, RpcMiddlewareMutationError,
    RpcMutationAtom, RpcMutationError,
};
use effect::rpc::{
    InProcessRpcChannel, OperationId, RpcCallId, RpcClient, RpcClientMiddleware, RpcEnvelopePolicy,
    RpcHandler, RpcOperation, RpcServerError, RpcUnaryServer, RpcVersion,
};
use effect::{
    Effect, Exit, KeyDomain, KeyNamespace, KeySegment, ParseLimits, PubSubOptions, PubSubOverflow,
    ReplayPolicy, StableKey, SubscriptionEvent, UtcMillis, json_i64,
};
use futures_executor::block_on;
use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::Arc;

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "rpc-event-test";
}

struct ServerEnvironment(Arc<MemoryEventJournal>);
impl HasEventJournal for ServerEnvironment {
    type Journal = MemoryEventJournal;
    fn event_journal(&self) -> Arc<Self::Journal> {
        Arc::clone(&self.0)
    }
}

#[test]
fn rpc_mutation_invalidates_only_after_event_append_is_acknowledged() {
    let limit = NonZeroUsize::new(512).unwrap();
    let limits = ParseLimits {
        bytes: limit,
        depth: limit,
        nodes: limit,
        collection: limit,
        issues: limit,
    };
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: limit,
        },
        json: limits,
    };
    let binary_limits = BinaryLimits {
        bytes: limit,
        depth: limit,
        nodes: limit,
        collection: limit,
        steps: limit,
    };
    let namespace = KeyNamespace::<Domain>::new("events").unwrap();
    let key = |name: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(name))]).unwrap()
    };
    let journal_id = EventJournalId::from_stable(key("journal"));
    let aggregate = AggregateId::from_stable(key("aggregate"));
    let event_id = EventId::from_stable(key("event"));
    let stream = JournalStream {
        journal: journal_id.clone(),
        aggregate: aggregate.clone(),
    };
    let journal = Arc::new(MemoryEventJournal::new(MemoryJournalCapacity {
        streams: limit,
        entries_per_stream: limit,
        tombstones_per_stream: limit,
    }));
    let server_environment = Arc::new(ServerEnvironment(Arc::clone(&journal)));
    let version = NonZeroU64::new(1).unwrap();
    let identity = |name| BinaryIdentity::new(name, version).unwrap();
    let event_definition = EventDefinition::new(
        EventShape {
            group: EventGroupId::new("orders").unwrap(),
            tag: EventTag::new("changed").unwrap(),
            version: EventVersion::new(NonZeroU32::new(1).unwrap()),
        },
        EventCodecs {
            payload: json_i64(),
            success: json_i64(),
            error: json_i64(),
        },
        EventBinaryFormats {
            payload: identity("request"),
            success: identity("result"),
            error: identity("error"),
            limits: binary_limits,
        },
    );
    let event_log = EventLog::new(event_definition, EventCommitPolicy::SuccessOnly);
    let operation = RpcOperation::unary(
        OperationId::new("change-order").unwrap(),
        RpcVersion::new(NonZeroU32::new(1).unwrap()),
        json_i64(),
        json_i64(),
        json_i64(),
        limits,
    );
    let client_operation = operation.clone();
    let handler = RpcHandler::new(operation, move |request| {
        if request < 0 {
            return Effect::fail_with(|| -1);
        }
        event_log
            .execute(
                EventInvocation {
                    journal: journal_id.clone(),
                    aggregate: aggregate.clone(),
                    event: event_id.clone(),
                    expected: JournalCursor::new(0),
                    occurred_at: UtcMillis::new(1),
                    payload: Arc::new(request),
                },
                move |_| {
                    Effect::<Result<i64, i64>, (), ServerEnvironment>::sync_once(move || {
                        Ok(request + 1)
                    })
                },
            )
            .map_error(|_| -2)
            .flat_map(|outcome| match outcome {
                EventRunOutcome::Committed {
                    result: Ok(value), ..
                }
                | EventRunOutcome::Duplicate {
                    result: Ok(value), ..
                } => Effect::sync_once(move || value),
                _ => Effect::fail_with(|| -3),
            })
    });
    let server = RpcUnaryServer::new(handler, policy);
    let channel = Arc::new(InProcessRpcChannel::new(server, server_environment));
    let client = RpcClient::<i64, i64, i64, RpcServerError<i64>>::new(
        client_operation,
        policy,
        NonZeroUsize::new(1).unwrap(),
    );
    let middleware_client = client.clone();
    let registry = InvalidationRegistry::new(PubSubOptions {
        capacity: limit,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    })
    .unwrap();
    let invalidation = InvalidationKey::from_stable(key("query"));
    let mutation = RpcMutationAtom::new(client, registry.clone(), [invalidation.clone()]);
    let client_environment = Arc::new(Arc::clone(&channel));
    let subscriber = match block_on(registry.subscribe().run(Arc::clone(&client_environment))) {
        Exit::Success(value) => value,
        _ => panic!("subscription failed"),
    };

    let failed = mutation.execute(-1, RpcCallId::new("rejected").unwrap(), BTreeMap::new());
    assert!(matches!(
        block_on(failed.run(Arc::clone(&client_environment))),
        Exit::Fail(RpcMutationError::Client(_))
    ));
    assert!(
        matches!(block_on(journal.cursor(stream.clone()).run(Arc::new(()))), Exit::Success(cursor) if cursor.value() == 0)
    );

    let committed = mutation.execute(7, RpcCallId::new("committed").unwrap(), BTreeMap::new());
    assert!(matches!(
        block_on(committed.run(Arc::clone(&client_environment))),
        Exit::Success(8)
    ));
    assert!(
        matches!(block_on(journal.cursor(stream.clone()).run(Arc::new(()))), Exit::Success(cursor) if cursor.value() == 1)
    );
    assert!(
        matches!(block_on(subscriber.take().run(Arc::clone(&client_environment))), Exit::Success(SubscriptionEvent::Item(batch)) if batch.keys.as_ref() == [invalidation.clone()])
    );

    type ClientEnvironment = Arc<InProcessRpcChannel<i64, i64, i64, ServerEnvironment>>;
    let middleware = RpcClientMiddleware::new(
        middleware_client,
        |request| Effect::<i64, &'static str, ClientEnvironment>::sync_once(move || request),
        |_| Effect::<i64, &'static str, ClientEnvironment>::fail_with(|| "after failed"),
    );
    let middleware_mutation =
        RpcMiddlewareMutationAtom::new(middleware, registry.clone(), [invalidation.clone()]);
    let after_failed =
        middleware_mutation.execute(7, RpcCallId::new("after-failed").unwrap(), BTreeMap::new());
    assert!(matches!(
        block_on(after_failed.run(Arc::clone(&client_environment))),
        Exit::Fail(RpcMiddlewareMutationError::CommittedWithMiddlewareFailure(
            "after failed"
        ))
    ));
    assert!(
        matches!(block_on(subscriber.take().run(Arc::clone(&client_environment))), Exit::Success(SubscriptionEvent::Item(batch)) if batch.keys.as_ref() == [invalidation.clone()])
    );

    assert!(matches!(
        block_on(
            registry
                .shutdown(effect::PubSubShutdown::Discard)
                .run(Arc::clone(&client_environment))
        ),
        Exit::Success(true)
    ));
    let acknowledged_without_notification =
        mutation.execute(7, RpcCallId::new("retry").unwrap(), BTreeMap::new());
    assert!(matches!(
        block_on(acknowledged_without_notification.run(client_environment)),
        Exit::Fail(RpcMutationError::CommittedWithoutInvalidation(_))
    ));
    assert!(
        matches!(block_on(journal.cursor(stream).run(Arc::new(()))), Exit::Success(cursor) if cursor.value() == 1)
    );
}
