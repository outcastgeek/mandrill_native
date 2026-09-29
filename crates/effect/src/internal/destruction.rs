//! Recipe and continuation links are destroyed iteratively, including abandoned runs.
use std::collections::HashMap;
use std::sync::Mutex;
use std::thread::ThreadId;

#[derive(Default)]
/// Typed queue used by internal destruction.
struct Queue {
    draining: bool,
    pending: Vec<Box<dyn Send>>,
}

// Per-thread queues live outside TLS, so deep graphs remain safe during TLS teardown.
// The mutex only protects queue bookkeeping, never user destruction or execution.
static QUEUES: Mutex<Option<HashMap<ThreadId, Queue>>> = Mutex::new(None);

/// Private ownership wrapper. No payload erasure/downcasts occur during interpretation.
///
/// # Type parameters
/// - `T`: Value or item type. Bound or type: `Send + 'static`.
pub(crate) struct FlatDrop<T: Send + 'static> {
    value: Option<T>,
}

impl<T: Clone + Send + 'static> Clone for FlatDrop<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

impl<T: Send + 'static> FlatDrop<T> {
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`T`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new(value: T) -> Self {
        Self { value: Some(value) }
    }
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Option<&T>`.
    /// Returns the value when present, or `None`.
    pub(crate) fn as_ref(&self) -> Option<&T> {
        self.value.as_ref()
    }
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Option<T>`.
    /// Returns the value when present, or `None`.
    pub(crate) fn take(mut self) -> Option<T> {
        self.value.take()
    }
}

/// Typed drain guard used by internal destruction.
struct DrainGuard(ThreadId);

impl Drop for DrainGuard {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        let pending = {
            let mut queues = QUEUES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            queues.as_mut().and_then(|queues| queues.remove(&self.0))
        };
        // Also releases queued captures if a user destructor unwinds.
        drop(pending);
    }
}

impl<T: Send + 'static> Drop for FlatDrop<T> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        let Some(value) = self.value.take() else {
            return;
        };
        let id = std::thread::current().id();
        {
            let mut queues = QUEUES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let queue = queues
                .get_or_insert_with(HashMap::new)
                .entry(id)
                .or_default();
            queue.pending.push(Box::new(value));
            if queue.draining {
                return;
            }
            queue.draining = true;
        }
        let _guard = DrainGuard(id);
        loop {
            let next = {
                let mut queues = QUEUES
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                queues
                    .as_mut()
                    .and_then(|queues| queues.get_mut(&id))
                    .and_then(|queue| queue.pending.pop())
            };
            match next {
                Some(value) => drop(value),
                None => break,
            }
        }
    }
}
