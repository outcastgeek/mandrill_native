use effect::Cancellation;
use futures_executor::block_on;
use std::future::Future;
use std::sync::{Arc, Barrier};
use std::task::{Context, Waker};

#[test]
fn child_requests_are_local_and_parent_requests_reach_all_descendants() {
    let parent = Cancellation::new();
    let left = parent.child();
    let right = parent.child();
    let leaf = left.child();
    assert!(left.request());
    assert!(leaf.is_requested());
    assert!(!parent.is_requested());
    assert!(!right.is_requested());
    assert!(parent.request());
    assert!(right.is_requested());
    assert!(parent.child().child().is_requested());
    assert!(!leaf.request());
}

#[test]
fn descendants_preserve_forwarding_after_intermediate_handle_is_dropped() {
    let parent = Cancellation::new();
    let middle = parent.child();
    let leaf = middle.child();
    drop(middle);
    assert!(!leaf.is_requested());
    assert!(parent.request());
    assert!(leaf.is_requested());
    block_on(leaf.cancelled());
}

#[test]
fn cancellation_broadcast_reaches_waiters_at_each_link() {
    let parent = Cancellation::new();
    let child = parent.child();
    let leaf = child.child();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut root_wait = Box::pin(parent.cancelled());
    let mut child_wait = Box::pin(child.cancelled());
    let mut leaf_wait = Box::pin(leaf.cancelled());
    assert!(root_wait.as_mut().poll(&mut context).is_pending());
    assert!(child_wait.as_mut().poll(&mut context).is_pending());
    assert!(leaf_wait.as_mut().poll(&mut context).is_pending());
    assert!(parent.request());
    assert!(root_wait.as_mut().poll(&mut context).is_ready());
    assert!(child_wait.as_mut().poll(&mut context).is_ready());
    assert!(leaf_wait.as_mut().poll(&mut context).is_ready());
}

#[test]
fn registration_racing_parent_request_cannot_leave_an_unrequested_descendant() {
    // Repeated coverage exercises both lock orders; the barrier's two actual
    // participants are this thread and the registration worker, not a pool size.
    const RACE_ATTEMPTS: usize = 256;
    const PARTICIPANTS: usize = 2;
    for _ in 0..RACE_ATTEMPTS {
        let parent = Cancellation::new();
        let registering = parent.clone();
        let barrier = Arc::new(Barrier::new(PARTICIPANTS));
        let gate = Arc::clone(&barrier);
        let worker = std::thread::spawn(move || {
            let _ = gate.wait();
            registering.child().child()
        });
        let _ = barrier.wait();
        assert!(parent.request());
        let child = worker.join();
        assert!(child.is_ok());
        if let Ok(child) = child {
            assert!(child.is_requested());
        }
    }
}
