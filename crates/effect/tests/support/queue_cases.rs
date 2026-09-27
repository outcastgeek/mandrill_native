use fx::{AsyncQueue, Effect, Exit, QueueCapacity, QueueError, QueueShutdown, Scope};
use std::cell::Cell;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    First,
    Second,
}
#[derive(Debug, PartialEq, Eq)]
struct Payload(Cell<Marker>); // Neither Clone nor Sync.

pub async fn queue_cases() {
    // Actual const context for the source's zero-capacity rendezvous contract.
    const CAPACITY: QueueCapacity = QueueCapacity::from_limit(Some(0));
    let env = Arc::new(super::tasks::Environment::default());
    let workflow: Effect<Payload, QueueError, super::tasks::Environment> = Scope::scoped(|scope| {
        fx::effect! {
            let scope = scope.clone();
            let queue = AsyncQueue::make(CAPACITY)?;
            let sender = scope.fork(queue.offer(Payload(Cell::new(Marker::First))))?;
            let value = queue.take()?;
            sender.join()?;
            queue.shutdown(QueueShutdown::Drain)?;
            value
        }
    });
    for _ in 0..2 {
        // A fresh queue and moved payload per run; no stored payload copying.
        assert_eq!(
            workflow.clone().run(Arc::clone(&env)).await,
            Exit::Success(Payload(Cell::new(Marker::First)))
        );
    }
    assert_eq!(env.tasks.submitted(), 2); // One sender for each of the two executions.
    env.tasks.finish();

    // The reference's two committed buffer values, with explicit fixture policy.
    let queue = AsyncQueue::new(QueueCapacity::from_limit(Some(2)));
    for marker in [Marker::First, Marker::Second] {
        assert_eq!(
            queue
                .offer(Payload(Cell::new(marker)))
                .run(Arc::new(()))
                .await,
            Exit::Success(())
        );
    }
    assert_eq!(
        queue.shutdown(QueueShutdown::Drain).run(Arc::new(())).await,
        Exit::Success(())
    );
    for marker in [Marker::First, Marker::Second] {
        assert_eq!(
            queue.take().run(Arc::new(())).await,
            Exit::Success(Payload(Cell::new(marker)))
        );
    }
    assert_eq!(
        queue.take().run(Arc::new(())).await,
        Exit::Fail(QueueError::Shutdown)
    );
}
