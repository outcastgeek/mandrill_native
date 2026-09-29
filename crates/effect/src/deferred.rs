//! Single-assignment completion and cancellation-aware observation.

use crate::internal::notification::notify_all;
use crate::internal::runtime::{self, Step};
use crate::{Cancellation, Defect, Effect, Exit, LibraryDefect, SharedExit};
use event_listener::Event;
use futures_util::future::select;
use std::sync::{Arc, Mutex};

/// Variants of completion used by deferred.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
enum Completion<A: Send + 'static, E: Send + 'static> {
    Pending,
    Ready(Exit<A, E>),
    Shared(SharedExit<A, E>),
    Consumed,
}

/// Typed inner used by deferred.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Inner<A: Send + 'static, E: Send + 'static> {
    completion: Mutex<Completion<A, E>>,
    changed: Event,
}

/// A persistent, first-write completion carrying a complete typed outcome.
///
/// Owned waits move A/E once. Shared waits/polls retain one canonical outcome
/// and inspect it by borrowing, with no Clone or Sync bound on its payloads.
/// Cancelling or dropping a pending waiter does not complete/cancel the Deferred.
/// No runtime/task provider is needed; the host polls each returned Effect.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct Deferred<A: Send + 'static, E: Send + 'static> {
    inner: Arc<Inner<A, E>>,
}

impl<A: Send + 'static, E: Send + 'static> Clone for Deferred<A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> Default for Deferred<A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Send + 'static, E: Send + 'static> Deferred<A, E> {
    /// Create an uncompleted handle now. `make` creates one lazily per run.
    #[must_use]
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                completion: Mutex::new(Completion::Pending),
                changed: Event::new(),
            }),
        }
    }

    /// Create a `Deferred` through the declared Effect and environment capabilities.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<Self, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn make<R: Send + Sync + 'static>() -> Effect<Self, E, R> {
        Effect::sync(Self::new)
    }

    /// Move a supplied outcome into this Deferred on the first execution.
    /// Re-executing this completion recipe returns false; it never clones A/E.
    /// Pre-cancelled execution leaves the supplied outcome available for a run.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Effect<bool, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn complete<R: Send + Sync + 'static>(&self, outcome: Exit<A, E>) -> Effect<bool, E, R> {
        let deferred = self.clone();
        let outcome = Mutex::new(Some(outcome));
        Effect::sync(move || {
            let outcome = outcome
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            outcome.is_some_and(|outcome| deferred.publish(outcome))
        })
    }

    /// Produce a fresh owned candidate per execution. Only the first completion
    /// is accepted; later candidates are destroyed outside the completion gate.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `factory` (`impl Fn() -> Exit<A, E> + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
    ///
    /// # Returns
    /// Type: `Effect<bool, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn complete_with<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> Exit<A, E> + Send + Sync + 'static,
    ) -> Effect<bool, E, R> {
        let deferred = self.clone();
        Effect::sync(move || deferred.publish(factory()))
    }

    /// Complete the deferred value successfully if it is still pending.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Effect<bool, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn succeed<R: Send + Sync + 'static>(&self, value: A) -> Effect<bool, E, R> {
        self.complete(Exit::Success(value))
    }

    /// Complete the deferred value with an expected error if it is still pending.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `error` (`E`): The error to propagate or record.
    ///
    /// # Returns
    /// Type: `Effect<bool, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn fail<R: Send + Sync + 'static>(&self, error: E) -> Effect<bool, E, R> {
        self.complete(Exit::Fail(error))
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    fn publish(&self, outcome: Exit<A, E>) -> bool {
        let rejected = {
            let mut completion = self
                .inner
                .completion
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(*completion, Completion::Pending) {
                *completion = Completion::Ready(outcome);
                None
            } else {
                Some(outcome)
            }
        };
        match rejected {
            Some(outcome) => {
                drop(outcome);
                false
            }
            None => {
                notify_all(&self.inner.changed);
                true
            }
        }
    }

    /// Transfer the full outcome into this Effect's success/error channels once.
    /// Active shared views prevent transfer without altering their data.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<A, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn await_value<R: Send + Sync + 'static>(&self) -> Effect<A, E, R> {
        let deferred = self.clone();
        Effect::new(move |_, context, finish| {
            let deferred = deferred.clone();
            Step::Suspended(Box::pin(async move {
                let outcome = match deferred.wait(context.cancellation, Self::take_owned).await {
                    Ok(Some(outcome)) => outcome,
                    Ok(None) => Exit::Interrupted,
                    Err(defect) => Exit::Die(defect),
                };
                runtime::finish(finish, outcome)
            }))
        })
    }

    /// Repeatable shared observation. A failed stored outcome is data inside
    /// SharedExit; inspect it explicitly instead of silently handling its E.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<SharedExit<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn await_shared<R: Send + Sync + 'static>(&self) -> Effect<SharedExit<A, E>, E, R> {
        let deferred = self.clone();
        Effect::new(move |_, context, finish| {
            let deferred = deferred.clone();
            Step::Suspended(Box::pin(async move {
                let outcome = match deferred.wait(context.cancellation, Self::share).await {
                    Ok(Some(outcome)) => Exit::Success(outcome),
                    Ok(None) => Exit::Interrupted,
                    Err(defect) => Exit::Die(defect),
                };
                runtime::finish(finish, outcome)
            }))
        })
    }

    /// None means pending. Completed outcomes are shared without cloning A/E.
    /// After an owned wait has consumed the outcome, return a typed defect.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<Option<SharedExit<A, E>>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn poll_shared<R: Send + Sync + 'static>(&self) -> Effect<Option<SharedExit<A, E>>, E, R> {
        let deferred = self.clone();
        Effect::ready(move |_, _| match deferred.share() {
            Ok(outcome) => Exit::Success(outcome),
            Err(defect) => Exit::Die(defect),
        })
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Result<Option<Exit<A, E>>, Defect>`.
    fn take_owned(&self) -> Result<Option<Exit<A, E>>, Defect> {
        let mut completion = self
            .inner
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match std::mem::replace(&mut *completion, Completion::Consumed) {
            Completion::Pending => {
                *completion = Completion::Pending;
                Ok(None)
            }
            Completion::Ready(outcome) => Ok(Some(outcome)),
            Completion::Shared(shared) => match shared.into_owned() {
                Ok(outcome) => Ok(Some(outcome)),
                Err(shared) => {
                    *completion = Completion::Shared(shared);
                    Err(Defect::from(LibraryDefect::DeferredOutcomeShared))
                }
            },
            Completion::Consumed => Err(Defect::from(LibraryDefect::DeferredOutcomeConsumed)),
        }
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Result<Option<SharedExit<A, E>>, Defect>`.
    fn share(&self) -> Result<Option<SharedExit<A, E>>, Defect> {
        let mut completion = self
            .inner
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match std::mem::replace(&mut *completion, Completion::Consumed) {
            Completion::Pending => {
                *completion = Completion::Pending;
                Ok(None)
            }
            Completion::Ready(outcome) => {
                let shared = SharedExit::new(outcome);
                let observed = shared.clone();
                *completion = Completion::Shared(shared);
                Ok(Some(observed))
            }
            Completion::Shared(shared) => {
                let observed = shared.clone();
                *completion = Completion::Shared(shared);
                Ok(Some(observed))
            }
            Completion::Consumed => Err(Defect::from(LibraryDefect::DeferredOutcomeConsumed)),
        }
    }

    ///
    /// # Type parameters
    /// - `B`: Transformed success-value type.
    ///
    /// # Parameters
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    /// - `observe` (`impl Fn(&Self) -> Result<Option<B>, Defect>`): The observe supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Option<B>, Defect>`.
    async fn wait<B>(
        &self,
        cancellation: Cancellation,
        observe: impl Fn(&Self) -> Result<Option<B>, Defect>,
    ) -> Result<Option<B>, Defect> {
        loop {
            // Register before inspecting state so completion cannot be missed.
            let changed = self.inner.changed.listen();
            if cancellation.is_requested() {
                return Ok(None);
            }
            if let Some(outcome) = observe(self)? {
                return Ok(Some(outcome));
            }
            let changed = std::pin::pin!(changed);
            let cancelled = std::pin::pin!(cancellation.cancelled());
            let _ = select(changed, cancelled).await;
        }
    }
}
