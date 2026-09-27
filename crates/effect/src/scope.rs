use crate::LibraryDefect;
use crate::internal::destruction::FlatDrop;
use crate::internal::notification::notify_all;
use crate::internal::outcome;
use crate::internal::runtime::{self, Finish, Step};
use crate::{Defect, Effect, Exit, Release, ResourceBinding, ResourceValue};
use event_listener::Event;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

type Finalizer<E> = Box<dyn FnOnce(Finish<(), E>) -> Step + Send>;
pub(crate) struct Registered<E: Send + 'static> {
    operation: Option<FlatDrop<Finalizer<E>>>,
    abandon: Option<Box<dyn FnOnce() + Send>>,
}

impl<E: Send + 'static> Registered<E> {
    pub(crate) fn new(callback: impl FnOnce(Finish<(), E>) -> Step + Send + 'static) -> Self {
        Self {
            operation: Some(FlatDrop::new(Box::new(callback))),
            abandon: None,
        }
    }

    pub(crate) fn with_abandon(
        callback: impl FnOnce(Finish<(), E>) -> Step + Send + 'static,
        abandon: impl FnOnce() + Send + 'static,
    ) -> Self {
        let mut registered = Self::new(callback);
        registered.abandon = Some(Box::new(abandon));
        registered
    }

    pub(crate) fn execute(mut self, finish: Finish<(), E>) -> Step {
        self.abandon = None;
        match self.operation.take().and_then(FlatDrop::take) {
            Some(callback) => callback(finish),
            None => runtime::finish(
                finish,
                Exit::Die(Defect::from(LibraryDefect::ScopeFinalizerConsumed)),
            ),
        }
    }
}

impl<E: Send + 'static> Drop for Registered<E> {
    fn drop(&mut self) {
        if let Some(abandon) = self.abandon.take() {
            abandon();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Open,
    Closing,
    Closed,
}

struct State<E: Send + 'static> {
    phase: Phase,
    finalizers: Vec<Registered<E>>,
    outcome: Option<Exit<(), E>>,
    status: ScopeCloseStatus,
}

struct Inner<E: Send + 'static> {
    state: Mutex<State<E>>,
    changed: Event,
}

/// Acknowledgement shared by concurrent closers. The enclosing scoped effect
/// retains and publishes the owned cleanup errors, including non-Clone errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeCloseStatus {
    Clean,
    Failed,
}

/// Per-run dynamic finalizers. Construct with `scoped`; the enclosing effect
/// owns the canonical cleanup outcome. Cloning a handle does not extend scope.
pub struct Scope<E: Send + 'static> {
    inner: Arc<Inner<E>>,
}

impl<E: Send + 'static> Clone for Scope<E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<E: Send + 'static> Scope<E> {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    phase: Phase::Open,
                    finalizers: Vec::new(),
                    outcome: None,
                    status: ScopeCloseStatus::Clean,
                }),
                changed: Event::new(),
            }),
        }
    }

    /// Build a fresh scope lazily on each run and await all finalizers before
    /// publishing the body outcome. Cleanup errors remain in the E channel.
    pub fn scoped<A, R>(
        body: impl Fn(Self) -> Effect<A, E, R> + Send + Sync + 'static,
    ) -> Effect<A, E, R>
    where
        A: Send + 'static,
        R: Send + Sync + 'static,
    {
        Effect::new(move |env, context, finish| {
            let scope = Self::new();
            let mut owner = OwnerGuard {
                scope: scope.clone(),
                settled: false,
            };
            let body_context = context.clone();
            let after_body = runtime::continuation(move |result| {
                let result = runtime::publish(result, &context);
                let join_context = context.clone();
                owner.scope.clone().join_close(
                    result,
                    runtime::continuation(move |result| {
                        let cleanup = owner.scope.take_outcome();
                        owner.settled = true;
                        drop(owner);
                        let result = runtime::publish(result, &join_context);
                        runtime::finish(finish, outcome::append_cleanup(result, cleanup))
                    }),
                )
            });
            match catch_unwind(AssertUnwindSafe(|| body(scope))) {
                Ok(program) => program.execute(env, body_context, after_body),
                Err(panic) => runtime::finish(after_body, Exit::Die(Defect::from_panic(panic))),
            }
        })
    }

    pub(crate) fn checkpoint(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finalizers
            .len()
    }
    pub(crate) fn take_since<A>(
        &self,
        checkpoint: usize,
    ) -> Result<Vec<Registered<E>>, Exit<A, E>> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.phase != Phase::Open || checkpoint > state.finalizers.len() {
            return Err(Exit::Die(Defect::from(
                LibraryDefect::ScopeRegistrationClosed,
            )));
        }
        Ok(state.finalizers.split_off(checkpoint))
    }
    pub(crate) fn execute_workflow<A: Send + 'static, R: Send + Sync + 'static>(
        self,
        body: crate::internal::workflow::Workflow<A, E, R>,
        env: Arc<R>,
        context: crate::internal::runtime::Context,
        finish: Finish<A, E>,
    ) -> Step {
        let mut owner = OwnerGuard {
            scope: self,
            settled: false,
        };
        let body_context = context.clone();
        body.execute(
            env,
            body_context,
            runtime::continuation(move |result| {
                let result = runtime::publish(result, &context);
                owner.scope.clone().join_close(
                    result,
                    runtime::continuation(move |result| {
                        let cleanup = owner.scope.take_outcome();
                        owner.settled = true;
                        drop(owner);
                        runtime::finish(
                            finish,
                            outcome::append_cleanup(runtime::publish(result, &context), cleanup),
                        )
                    }),
                )
            }),
        )
    }

    /// Register a reusable finalizer, retaining the environment at registration.
    /// Registration after closing starts awaits this finalizer immediately and
    /// reports a defect, preserving any additional cleanup failures.
    pub fn add_finalizer<R: Send + Sync + 'static>(
        &self,
        finalizer: Effect<(), E, R>,
    ) -> Effect<(), E, R> {
        self.add_release(move || Release::from_effect(finalizer.clone()))
    }

    /// Register a fresh single-use release on each execution.
    pub fn add_release<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> Release<E, R> + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        let scope = self.clone();
        let factory = Arc::new(factory);
        Effect::new(move |env, context, finish| {
            let factory = Arc::clone(&factory);
            let finalizer =
                Registered::new(
                    move |finish| match catch_unwind(AssertUnwindSafe(|| factory())) {
                        Ok(release) => release.execute(env, context.protected(), finish),
                        Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
                    },
                );
            scope.register_or_release(finalizer, (), finish)
        })
    }

    /// Acquire into this scope with release declared before acquisition starts.
    /// Late success is owned before cancellation or a closing-scope rejection.
    pub fn acquire<T, R>(
        &self,
        acquisition: Effect<T, E, R>,
        release: impl Fn(T) -> Release<E, R> + Send + Sync + 'static,
    ) -> Effect<ResourceValue<T>, E, R>
    where
        T: Send + Sync + 'static,
        R: Send + Sync + 'static,
    {
        ResourceBinding::new(acquisition, move |value, _| release(value))
            .acquire_scoped(self.clone())
    }

    /// Close once and join the same drain from every concurrent caller.
    /// This returns an acknowledgement; cleanup errors are published by the
    /// enclosing `scoped` effect, rather than cloned into every caller.
    /// Once started, close is protected from cooperative cancellation.
    pub fn close<R: Send + Sync + 'static>(&self) -> Effect<ScopeCloseStatus, E, R> {
        let scope = self.clone();
        Effect::new(move |_, _, finish| {
            let completed = scope.clone();
            scope.clone().join_close(
                Exit::Success(()),
                runtime::continuation(move |_| {
                    runtime::finish(finish, Exit::Success(completed.status()))
                }),
            )
        })
    }

    /// Observe settlement without taking execution ownership or closing it.
    pub async fn closed(&self) -> ScopeCloseStatus {
        loop {
            let changed = self.inner.changed.listen();
            {
                let state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.phase == Phase::Closed {
                    return state.status;
                }
            }
            changed.await;
        }
    }

    fn status(&self) -> ScopeCloseStatus {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .status
    }

    pub(crate) fn register_or_release<A: Send + 'static>(
        &self,
        finalizer: Registered<E>,
        value: A,
        finish: Finish<A, E>,
    ) -> Step {
        match self.try_register(finalizer) {
            Ok(()) => runtime::finish(finish, Exit::Success(value)),
            Err(finalizer) => finalizer.execute(runtime::continuation(move |cleanup| {
                drop(value);
                runtime::finish(
                    finish,
                    outcome::append_cleanup(
                        Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
                        cleanup,
                    ),
                )
            })),
        }
    }

    pub(crate) fn try_register(&self, finalizer: Registered<E>) -> Result<(), Registered<E>> {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.phase == Phase::Open {
                state.finalizers.push(finalizer);
                Ok(())
            } else {
                Err(finalizer)
            }
        }
    }

    fn join_close<A: Send + 'static>(self, primary: Exit<A, E>, finish: Finish<A, E>) -> Step {
        let work = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match state.phase {
                Phase::Open => {
                    state.phase = Phase::Closing;
                    Some(std::mem::take(&mut state.finalizers))
                }
                Phase::Closing | Phase::Closed => None,
            }
        };
        match work {
            Some(work) => {
                let guard = DrainGuard {
                    scope: self,
                    work,
                    settled: false,
                };
                drain(guard, primary, Exit::Success(()), finish)
            }
            None => Step::Suspended(Box::pin(async move {
                let _ = self.closed().await;
                runtime::finish(finish, primary)
            })),
        }
    }

    fn settle(&self, outcome: Exit<(), E>) {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.status = if matches!(outcome, Exit::Success(())) {
                ScopeCloseStatus::Clean
            } else {
                ScopeCloseStatus::Failed
            };
            state.outcome = Some(outcome);
            state.phase = Phase::Closed;
        }
        notify_all(&self.inner.changed);
    }

    fn take_outcome(&self) -> Exit<(), E> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .outcome
            .take()
            .unwrap_or_else(|| Exit::Die(Defect::from(LibraryDefect::ScopeOutcomeConsumed)))
    }

    fn abandon_open(&self) {
        let discarded = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.phase != Phase::Open {
                return;
            }
            state.phase = Phase::Closing;
            std::mem::take(&mut state.finalizers)
        };
        // User captures can have destructors. Never run them under the gate.
        drop(discarded);
        self.settle(Exit::Die(Defect::from(LibraryDefect::ScopeOwnerDropped)));
    }
}

struct OwnerGuard<E: Send + 'static> {
    scope: Scope<E>,
    settled: bool,
}

impl<E: Send + 'static> Drop for OwnerGuard<E> {
    fn drop(&mut self) {
        if !self.settled {
            self.scope.abandon_open();
        }
    }
}

struct DrainGuard<E: Send + 'static> {
    scope: Scope<E>,
    work: Vec<Registered<E>>,
    settled: bool,
}

impl<E: Send + 'static> Drop for DrainGuard<E> {
    fn drop(&mut self) {
        if !self.settled {
            // Invalidate resources synchronously before broadcasting loss;
            // deferred destruction of deep capture graphs can finish later.
            drop(std::mem::take(&mut self.work));
            self.scope.settle(Exit::Die(Defect::from(
                LibraryDefect::ScopeCloseDriverDropped,
            )));
        }
    }
}

fn drain<A: Send + 'static, E: Send + 'static>(
    mut guard: DrainGuard<E>,
    primary: Exit<A, E>,
    cleanup: Exit<(), E>,
    finish: Finish<A, E>,
) -> Step {
    runtime::more(move || match guard.work.pop() {
        Some(finalizer) => {
            let after_release = runtime::continuation(move |result| {
                drain(
                    guard,
                    primary,
                    outcome::append_cleanup(cleanup, result),
                    finish,
                )
            });
            finalizer.execute(after_release)
        }
        None => {
            guard.scope.settle(cleanup);
            guard.settled = true;
            drop(guard);
            runtime::finish(finish, primary)
        }
    })
}
