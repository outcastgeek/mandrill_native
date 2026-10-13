use effect::encoding::{BinaryIdentity, BinaryLimits, FramePolicy};
use effect::eventlog::{
    AggregateId, EventCommitPolicy, EventGroupId, EventId, EventInvocation, EventJournal,
    EventJournalId, EventLog, EventRunOutcome, EventTag, EventVersion, HasEventJournal,
    JournalCursor, JournalStream, MemoryEventJournal, MemoryJournalCapacity,
};
use effect::reactivity::{
    InvalidationKey, InvalidationRegistry, RpcMiddlewareMutationAtom, RpcMiddlewareMutationError,
    RpcMutationAtom, RpcMutationError,
};
use effect::rpc::{
    InProcessRpcChannel, OperationId, RpcCallId, RpcClient, RpcClientMiddleware, RpcEnvelopePolicy,
    RpcServerError, RpcUnaryServer, RpcVersion,
};
use effect::{
    Effect, Exit, KeyDomain, KeyNamespace, KeySegment, ParseLimits, PubSubOptions, PubSubOverflow,
    ReplayPolicy, StableKey, SubscriptionEvent, UtcMillis, json_i64,
};
use effect::{effect, effect_map_error, effect_sync, effect_try_sync, event, rpc_operation};
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
fn rpc_mutation_invalidates_only_after_event_append_is_acknowledged() -> Result<(), String> {
    let limit = NonZeroUsize::MIN.saturating_add(511);
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
    let namespace = KeyNamespace::<Domain>::new("events")
        .map_err(|error| format!("event namespace: {error:?}"))?;
    let key = |name: &'static str| {
        StableKey::new(&namespace, [KeySegment::Text(Arc::from(name))])
            .map_err(|error| format!("{name} identity: {error:?}"))
    };
    let journal_id = EventJournalId::from_stable(key("journal")?);
    let aggregate = AggregateId::from_stable(key("aggregate")?);
    let event_id = EventId::from_stable(key("event")?);
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
    let version = NonZeroU64::MIN;
    let identity = |name| {
        BinaryIdentity::new(name, version)
            .map_err(|error| format!("{name} binary identity: {error:?}"))
    };
    let event_definition = event! { group: EventGroupId::new("orders")
    .map_err(|error| format!("event group: {error:?}"))?, tag: EventTag::new("changed").map_err(|error| format!("event tag: {error:?}"))?, version: EventVersion::new(NonZeroU32::MIN), payload: json_i64(), success: json_i64(), error: json_i64(), payload_identity: identity("request")?, success_identity: identity("result")?, error_identity: identity("error")?, limits: binary_limits, };
    let event_log = EventLog::new(event_definition, EventCommitPolicy::SuccessOnly);
    let operation = rpc_operation! { id: OperationId::new("change-order").map_err(|error| format!("RPC operation: {error:?}"))?, version: RpcVersion::new(NonZeroU32::MIN), request: json_i64(), success: json_i64(), error: json_i64(), limits: limits,  };
    let client_operation = operation.clone();
    let handler = rpc_operation! { operation: operation, handler: move |request| {
           if request < 0 {
               return effect_try_sync! { body: { let __dsl_error = || -1; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
           }
           effect! {
    bind;
    program: effect_map_error! { program: event_log
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
                       { let __dsl_program: Effect < Result < i64 , i64 > , () , ServerEnvironment > = effect_sync! { once; body: move || {
                           Ok(request + 1)
                       }, }; __dsl_program }
                   },
               ), map: |_| -2, },
    body: |outcome| match outcome {
                   EventRunOutcome::Committed {
                       result: Ok(value), ..
                   }
                   | EventRunOutcome::Duplicate {
                       result: Ok(value), ..
                   } => effect_sync! { once; body: move || value, },
                   EventRunOutcome::Committed { result: Err(_), .. }
                   | EventRunOutcome::Duplicate { result: Err(_), .. }
                   | EventRunOutcome::Accepted
                   | EventRunOutcome::Rejected(_)
                   | EventRunOutcome::ConflictingEventId
                   | EventRunOutcome::CompactedEventId
                   | EventRunOutcome::StaleCursor { .. } => effect_try_sync! { body: { let __dsl_error = || -3; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, },
               },
    }
       }, };
    let server = RpcUnaryServer::new(handler, policy);
    let channel = Arc::new(InProcessRpcChannel::new(server, server_environment));
    let client = RpcClient::<i64, i64, i64, RpcServerError<i64>>::new(
        client_operation,
        policy,
        NonZeroUsize::MIN,
    );
    let middleware_client = client.clone();
    let registry = InvalidationRegistry::new(PubSubOptions {
        capacity: limit,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    })
    .map_err(|error| format!("invalidation registry: {error:?}"))?;
    let invalidation = InvalidationKey::from_stable(key("query")?);
    let mutation = RpcMutationAtom::new(client, registry.clone(), [invalidation.clone()]);
    let client_environment = Arc::new(Arc::clone(&channel));
    let Exit::Success(subscriber) =
        block_on(registry.subscribe().run(Arc::clone(&client_environment)))
    else {
        return Err(String::from("invalidation subscription must succeed"));
    };

    let failed = mutation.execute(
        -1,
        RpcCallId::new("rejected").map_err(|error| format!("rejected call: {error:?}"))?,
        BTreeMap::new(),
    );
    assert!(matches!(
        block_on(failed.run(Arc::clone(&client_environment))),
        Exit::Fail(RpcMutationError::Client(_))
    ));
    assert!(
        matches!(block_on(journal.cursor(stream.clone()).run(Arc::new(()))), Exit::Success(cursor) if cursor.value() == 0)
    );

    let committed = mutation.execute(
        7,
        RpcCallId::new("committed").map_err(|error| format!("committed call: {error:?}"))?,
        BTreeMap::new(),
    );
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
        |request| {
            let __dsl_program: Effect<i64, &'static str, ClientEnvironment> =
                effect_sync! { once; body: move || request, };
            __dsl_program
        },
        |_| {
            let __dsl_program: Effect<i64, &'static str, ClientEnvironment> = effect_try_sync! { body: { let __dsl_error = || "after failed"; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
            __dsl_program
        },
    );
    let middleware_mutation =
        RpcMiddlewareMutationAtom::new(middleware, registry.clone(), [invalidation.clone()]);
    let after_failed = middleware_mutation.execute(
        7,
        RpcCallId::new("after-failed").map_err(|error| format!("middleware call: {error:?}"))?,
        BTreeMap::new(),
    );
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
    let acknowledged_without_notification = mutation.execute(
        7,
        RpcCallId::new("retry").map_err(|error| format!("retry call: {error:?}"))?,
        BTreeMap::new(),
    );
    assert!(matches!(
        block_on(acknowledged_without_notification.run(client_environment)),
        Exit::Fail(RpcMutationError::CommittedWithoutInvalidation(_))
    ));
    assert!(
        matches!(block_on(journal.cursor(stream).run(Arc::new(()))), Exit::Success(cursor) if cursor.value() == 1)
    );
    Ok(())
}
