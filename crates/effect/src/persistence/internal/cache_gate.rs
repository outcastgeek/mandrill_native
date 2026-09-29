//! Per-key coordination of cache lookup and invalidation.

use crate::{Effect, Semaphore, SemaphoreError, SemaphorePermit};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, Weak};

/// Type alias for gates in persistence internal cache gate.
type Gates = BTreeMap<Arc<[u8]>, Weak<Semaphore>>;

/// Serializes one persisted key's lookup and invalidation without coupling
/// unrelated keys or retaining identities after the last operation settles.
#[derive(Clone)]
pub(crate) struct CacheGate {
    gates: Arc<Mutex<Gates>>,
}

/// Typed gate handle used by persistence internal cache gate.
struct GateHandle {
    gates: Arc<Mutex<Gates>>,
    key: Arc<[u8]>,
    semaphore: Arc<Semaphore>,
}

impl Clone for GateHandle {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            gates: Arc::clone(&self.gates),
            key: Arc::clone(&self.key),
            semaphore: Arc::clone(&self.semaphore),
        }
    }
}

impl Drop for GateHandle {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        let mut gates = self
            .gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if Arc::strong_count(&self.semaphore) == NonZeroUsize::MIN.get()
            && gates
                .get(&self.key)
                .is_some_and(|entry| Weak::ptr_eq(entry, &Arc::downgrade(&self.semaphore)))
        {
            gates.remove(&self.key);
        }
    }
}

/// Field order releases the permit before the final handle can remove the
/// keyed semaphore. A new caller therefore cannot bypass an issued permit.
pub(crate) struct CacheGatePermit {
    _permit: SemaphorePermit,
    _handle: GateHandle,
}

impl CacheGate {
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new() -> Self {
        Self {
            gates: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    ///
    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `GateHandle`.
    /// Returns `GateHandle`.
    fn handle(&self, key: Arc<[u8]>) -> GateHandle {
        let semaphore = {
            let mut gates = self
                .gates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(existing) = gates.get(&key).and_then(Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(Semaphore::full(NonZeroUsize::MIN));
                gates.insert(Arc::clone(&key), Arc::downgrade(&created));
                created
            }
        };
        GateHandle {
            gates: Arc::clone(&self.gates),
            key,
            semaphore,
        }
    }

    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    ///
    /// # Parameters
    /// - `key` (`Arc<[u8]>`): Stable key used to locate or identify the selected value.
    ///
    /// # Returns
    /// Type: `Effect<CacheGatePermit, SemaphoreError, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub(crate) fn acquire<R: Send + Sync + 'static>(
        &self,
        key: Arc<[u8]>,
    ) -> Effect<CacheGatePermit, SemaphoreError, R> {
        let gate = self.clone();
        Effect::defer(move || {
            let handle = gate.handle(Arc::clone(&key));
            handle
                .semaphore
                .acquire(NonZeroUsize::MIN)
                .map(move |permit| CacheGatePermit {
                    _permit: permit,
                    _handle: handle.clone(),
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cancellation, Exit};
    use futures_executor::block_on;
    use std::future::Future;
    use std::task::{Context, Wake, Waker};

    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }

    #[test]
    fn same_key_shares_one_gate_and_last_handle_releases_identity() {
        let registry = CacheGate::new();
        let key: Arc<[u8]> = Arc::from([]);
        let first = registry.handle(Arc::clone(&key));
        let second = registry.handle(key);
        assert!(Arc::ptr_eq(&first.semaphore, &second.semaphore));
        assert!(
            !registry
                .gates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
        drop(first);
        assert!(
            !registry
                .gates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
        drop(second);
        assert!(
            registry
                .gates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
    }

    #[test]
    fn cancelled_waiter_does_not_retain_the_key_or_permit() {
        let registry = CacheGate::new();
        let key: Arc<[u8]> = Arc::from([]);
        let held = match block_on(registry.acquire::<()>(Arc::clone(&key)).run(Arc::new(()))) {
            Exit::Success(permit) => permit,
            _ => panic!(),
        };
        let cancellation = Arc::new(Cancellation::new());
        let mut waiter = Box::pin(
            registry
                .acquire::<Cancellation>(Arc::clone(&key))
                .run_cancellable(Arc::clone(&cancellation)),
        );
        let waker = Waker::from(Arc::new(Noop));
        let mut context = Context::from_waker(&waker);
        assert!(waiter.as_mut().poll(&mut context).is_pending());
        assert!(cancellation.request());
        assert!(matches!(block_on(waiter), Exit::Interrupted));
        drop(held);
        assert!(
            registry
                .gates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
        assert!(matches!(
            block_on(registry.acquire::<()>(key).run(Arc::new(()))),
            Exit::Success(_)
        ));
    }
}
