use effect::transactional::{
    HasTransactionDomain, Transaction, TransactionPolicy, TxChunk, TxDeferred, TxExecutionId,
    TxHashMap, TxHashSet, TxPriorityQueue, TxPubSub, TxPubSubOverflow, TxQueue, TxReentrantLock,
    TxSemaphore,
};
use effect::{Exit, KeyDomain, KeyNamespace, KeySegment, StableKey};
use std::num::NonZeroUsize;
use std::sync::Arc;

struct Domain;

impl KeyDomain for Domain {
    const TAG: &'static str = "provider-transactional-collections";
}

/// One journal commits heterogeneous collection updates. The remaining
/// transactional primitives execute through the same domain and R projection.
pub async fn run<R>(environment: Arc<R>) -> bool
where
    R: HasTransactionDomain + Send + Sync + 'static,
{
    let domain = environment.transaction_domain();
    let Ok(chunk) = TxChunk::<i64>::new(&domain) else {
        return false;
    };
    let Ok(map) = TxHashMap::<i64, i64>::new(&domain) else {
        return false;
    };
    let Ok(set) = TxHashSet::<i64>::new(&domain) else {
        return false;
    };
    let Ok(queue) = TxQueue::<i64>::new(&domain) else {
        return false;
    };
    let Ok(priority) = TxPriorityQueue::<i64, i64>::new(&domain) else {
        return false;
    };
    let Ok(deferred) = TxDeferred::<i64>::new(&domain) else {
        return false;
    };
    let Ok(semaphore) = TxSemaphore::new(&domain, NonZeroUsize::MIN.get()) else {
        return false;
    };
    let Ok(lock) = TxReentrantLock::new(&domain) else {
        return false;
    };
    let Ok(pubsub) = TxPubSub::<i64>::new(&domain, NonZeroUsize::MIN, TxPubSubOverflow::Reject)
    else {
        return false;
    };
    let policy = TransactionPolicy {
        max_conflicts: NonZeroUsize::MIN,
    };

    let writers = Transaction::<(), ()>::for_each(
        {
            let chunk = chunk.clone();
            let map = map.clone();
            let set = set.clone();
            let queue = queue.clone();
            let priority = priority.clone();
            move || {
                vec![
                    chunk.append::<()>(Arc::new(41)),
                    map.insert::<()>(Arc::new(41), Arc::new(42)).map(|_| ()),
                    set.insert::<()>(Arc::new(42)).map(|_| ()),
                    queue.offer::<()>(Arc::new(42)),
                    priority.offer::<()>(Arc::new(1), Arc::new(42)),
                ]
            }
        },
        |step| step,
    );
    if !matches!(
        writers
            .atomically(policy)
            .run(Arc::clone(&environment))
            .await,
        Exit::Success(())
    ) {
        return false;
    }
    let snapshot = chunk
        .snapshot::<()>()
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let mapped = map
        .get::<()>(Arc::new(41))
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let contained = set
        .contains::<()>(Arc::new(42))
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let queued = queue
        .take::<()>()
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let prioritized = priority
        .take::<()>()
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    if !matches!(snapshot, Exit::Success(values)
        if values.len() == 1 && values.first().is_some_and(|value| **value == 41))
        || !matches!(mapped, Exit::Success(Some(value)) if *value == 42)
        || !matches!(contained, Exit::Success(true))
        || !matches!(queued, Exit::Success(value) if *value == 42)
        || !matches!(prioritized, Exit::Success((key, value))
            if *key == 1 && *value == 42)
    {
        return false;
    }
    let popped = chunk
        .pop::<()>()
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let removed_map = map
        .remove::<()>(Arc::new(41))
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let removed_set = set
        .remove::<()>(Arc::new(42))
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    if !matches!(popped, Exit::Success(Some(value)) if *value == 41)
        || !matches!(removed_map, Exit::Success(Some(value)) if *value == 42)
        || !matches!(removed_set, Exit::Success(true))
    {
        return false;
    }

    let completed = deferred
        .complete::<()>(Arc::new(42))
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let awaited = deferred
        .await_value::<()>()
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    if !matches!(completed, Exit::Success(true))
        || !matches!(awaited, Exit::Success(value) if *value == 42)
    {
        return false;
    }
    let acquired = semaphore
        .acquire::<()>(NonZeroUsize::MIN)
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let empty = semaphore
        .available::<()>()
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let released = semaphore
        .release(NonZeroUsize::MIN)
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    if !matches!(acquired, Exit::Success(()))
        || !matches!(empty, Exit::Success(0))
        || !matches!(released, Exit::Success(()))
    {
        return false;
    }

    let namespace = KeyNamespace::<Domain>::new("locks").expect("valid lock namespace");
    let owner = TxExecutionId::from_stable(
        StableKey::new(&namespace, [KeySegment::Text("execution".into())])
            .expect("valid logical execution identity"),
    );
    let acquired = lock
        .acquire(owner.clone())
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let held = lock
        .held_by::<()>(owner.clone())
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let released = lock
        .release(owner)
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    if !matches!(acquired, Exit::Success(()))
        || !matches!(held, Exit::Success(true))
        || !matches!(released, Exit::Success(()))
    {
        return false;
    }

    let subscription = pubsub.subscribe::<R>().run(Arc::clone(&environment)).await;
    let Exit::Success(subscription) = subscription else {
        return false;
    };
    let published = pubsub
        .publish(Arc::new(42))
        .atomically(policy)
        .run(Arc::clone(&environment))
        .await;
    let observed = subscription.next::<R>().run(Arc::clone(&environment)).await;
    let disposed = subscription.dispose();
    matches!(published, Exit::Success(receipt) if receipt.sequence.get() == 1)
        && matches!(observed, Exit::Success((sequence, value))
            if sequence.get() == 1 && *value == 42)
        && disposed
}
