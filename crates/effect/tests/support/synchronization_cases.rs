use fx::{AtomicRef, Deferred, Effect, Exit, Latch, Scope};
use std::cell::Cell;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Initial,
    Changed,
    Error,
}

#[derive(Debug, PartialEq, Eq)]
struct Payload(Cell<Marker>); // Neither Clone nor Sync.

pub async fn synchronization_cases() {
    let references: Effect<(Marker, Marker), Payload> = fx::effect! {
        let reference = AtomicRef::make_with(|| Payload(Cell::new(Marker::Initial)))?;
        let previous = reference.modify(|current| {
            (current.0.get(), Payload(Cell::new(Marker::Changed)))
        })?;
        let current = reference.with(|current| current.0.get())?;
        (previous, current)
    };
    for _ in 0..2 {
        // Repeated executions create fresh state; no stored payload is copied.
        assert_eq!(
            references.clone().run(Arc::new(())).await,
            Exit::Success((Marker::Initial, Marker::Changed))
        );
    }

    let deferred: Deferred<Payload, Payload> = Deferred::new();
    assert_eq!(
        deferred
            .fail(Payload(Cell::new(Marker::Error)))
            .run(Arc::new(()))
            .await,
        Exit::Success(true)
    );
    let first = deferred.await_shared().run(Arc::new(())).await;
    let second = deferred.poll_shared().run(Arc::new(())).await;
    assert!(matches!(first, Exit::Success(_)));
    assert!(matches!(second, Exit::Success(Some(_))));
    if let (Exit::Success(first), Exit::Success(Some(second))) = (first, second) {
        let address = |exit: &Exit<Payload, Payload>| match exit {
            Exit::Fail(error) => Some(std::ptr::from_ref(error)),
            Exit::Success(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
                None
            }
        };
        assert!(matches!(first.with(address), Ok(Some(_))));
        assert_eq!(first.with(address), second.with(address));
    }
    assert_eq!(
        deferred.await_value().run(Arc::new(())).await,
        Exit::Fail(Payload(Cell::new(Marker::Error)))
    );

    let env = Arc::new(super::tasks::Environment::default());
    let workflow: Effect<(), Payload, super::tasks::Environment> = Scope::scoped(|scope| {
        fx::effect! {
            let scope = scope.clone(); // Per-run ownership handle for the continuation.
            let latch = Latch::make()?;
            let waiter = scope.fork(latch.await_open())?;
            let first = latch.open()?;
            assert!(first);
            waiter.join()?;
            let again = latch.open()?;
            assert!(!again);
            latch.await_open()?;
        }
    });
    assert_eq!(workflow.run(Arc::clone(&env)).await, Exit::Success(()));
    assert_eq!(env.tasks.submitted(), 1); // Exactly the one latch waiter owned above.
    env.tasks.finish();
}
