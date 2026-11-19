//! Lifetime ownership and ordered asynchronous finalization.
#![doc = include_str!("scope/README.md")]

use crate::LibraryDefect;
use crate::internal::runtime_storage::{Consume, Once, Settlement};
use crate::memory::{FrameReservation, RuntimeFrame};

mod link;
mod prepared;
mod storage;
use crate::internal::notification::notify_all;
use crate::internal::outcome;
use crate::internal::runtime::{self, Finish, Step};
use crate::{Defect, Effect, Exit, Release, ResourceBinding, ResourceValue};
use event_listener::Event;
use link::ScopeLink;
pub(crate) use prepared::PreparedScopeEntry;
pub use prepared::{
    PreparedCheckpoint, PreparedCleanupDemand, PreparedCloseWait, PreparedFutureResourceBinding,
    PreparedFutureResourcePlan, PreparedRelease, PreparedReleasePlan, PreparedResourceBinding,
    PreparedResourceDemand, PreparedResourcePlan, PreparedResourceValue, PreparedScope,
    PreparedScopeAdmissionError, PreparedScopeClose, PreparedScopeCloseDemand, PreparedScopeDemand,
    PreparedScopeLimits, PreparedScopeOwner, PreparedScopePool,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

pub(crate) struct UnitRetirement<E: Send + 'static> {
    // A weak link prevents the Fiber retained by its finalizer from retaining
    // the scope. The cookie is compared only; the finalizer owns the Fiber
    // while it is present, so its address cannot be reused for another entry.
    scope: std::sync::Weak<Inner<E>>,
    tag: usize,
}
impl<E: Send + 'static> UnitRetirement<E> {
    pub(crate) fn retire(self) {
        let Some(scope) = self.scope.upgrade() else {
            return;
        };
        let removed = {
            let mut state = scope
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.phase != Phase::Open {
                None
            } else {
                state.finalizers.remove_tag(self.tag)
            }
        };
        // Scope close may already have taken the entry. In that case its normal
        // cleanup owns it. Never run abandon or destroy captures under the gate.
        if let Some(mut registered) = removed {
            registered.abandon = None;
            drop(registered);
        }
    }
}
pub(crate) use storage::FinalizerBatch;

/// Typed registered used by scope.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub(crate) struct Registered<E: Send + 'static> {
    operation: Option<Once<Finish<(), E>, Step>>,
    abandon: Option<Once<(), ()>>,
    retirement_tag: Option<usize>,
}

impl<E: Send + 'static> Registered<E> {
    pub(crate) fn set_retirement_tag(&mut self, tag: usize) {
        self.retirement_tag = Some(tag);
    }
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce(Finish<(), E>) -> Step + Send + 'static`): The callback invoked by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new(callback: impl FnOnce(Finish<(), E>) -> Step + Send + 'static) -> Self {
        Self {
            operation: Some(Once::system(callback)),
            abandon: None,
            retirement_tag: None,
        }
    }

    fn from_prepared<T: Consume<Finish<(), E>, Step> + Send + 'static>(
        reservation: FrameReservation<'static, T>,
        callback: T,
    ) -> Self {
        Self {
            operation: Some(Once::prepared(reservation, callback)),
            abandon: None,
            retirement_tag: None,
        }
    }

    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `callback` (`impl FnOnce(Finish<(), E>) -> Step + Send + 'static`): The callback invoked by this operation.
    /// - `abandon` (`impl FnOnce() + Send + 'static`): The abandon supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn with_abandon(
        callback: impl FnOnce(Finish<(), E>) -> Step + Send + 'static,
        abandon: impl FnOnce() + Send + 'static,
    ) -> Self {
        let mut registered = Self::new(callback);
        registered.abandon = Some(Once::system(move |()| abandon()));
        registered
    }

    pub(crate) fn with_prepared_abandon<T, A>(
        operation: FrameReservation<'static, T>,
        release: T,
        abandon: FrameReservation<'static, A>,
        abandoned: A,
    ) -> Self
    where
        T: Consume<Finish<(), E>, Step> + Send + 'static,
        A: Consume<(), ()> + Send + 'static,
    {
        let mut registered = Self::from_prepared(operation, release);
        registered.abandon = Some(Once::prepared(abandon, abandoned));
        registered
    }

    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `finish` (`Finish<(), E>`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `Step`.
    pub(crate) fn from_prepared_settlement<T, G, A>(
        reservation: FrameReservation<'static, Settlement<T, G>>,
        value: Settlement<T, G>,
        abandon: FrameReservation<'static, A>,
        abandoned: A,
    ) -> Self
    where
        T: Consume<Finish<(), E>, Step> + Send + 'static,
        G: Send + 'static,
        A: Consume<(), ()> + Send + 'static,
    {
        Self {
            operation: Some(Once::prepared_settlement(reservation, value)),
            abandon: Some(Once::prepared(abandon, abandoned)),
            retirement_tag: None,
        }
    }
    pub(crate) fn execute(self, finish: Finish<(), E>) -> Step {
        self.execute_with(move |frame| match frame {
            None => finish,
            Some(frame) => runtime::continuation_retained(
                move |outcome| finish.call(outcome).unwrap_or(Step::Done),
                Some(frame),
            ),
        })
    }
    pub(crate) fn execute_with(
        self,
        build: impl FnOnce(Option<RuntimeFrame>) -> Finish<(), E>,
    ) -> Step {
        self.execute_with_missing(build, runtime::finish)
    }
    pub(crate) fn execute_prepared_with(
        self,
        build: impl FnOnce(Option<RuntimeFrame>) -> Finish<(), E>,
    ) -> Step {
        self.execute_with_missing(build, |finish, outcome| {
            finish.call(outcome).unwrap_or(Step::Done)
        })
    }
    fn execute_with_missing(
        mut self,
        build: impl FnOnce(Option<RuntimeFrame>) -> Finish<(), E>,
        missing: impl FnOnce(Finish<(), E>, Exit<(), E>) -> Step,
    ) -> Step {
        self.abandon = None;
        match self.operation.take() {
            Some(callback) => callback.call_with(build).unwrap_or(Step::Done),
            None => missing(
                build(None),
                Exit::Die(Defect::from(LibraryDefect::ScopeFinalizerConsumed)),
            ),
        }
    }
}

impl<E: Send + 'static> Drop for Registered<E> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if let Some(abandon) = self.abandon.take() {
            let _ = abandon.call(());
        }
    }
}

enum RegistrationError<E: Send + 'static> {
    Closed(Registered<E>),
    Full(Registered<E>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Variants of phase used by scope.
enum Phase {
    Open,
    Closing,
    Closed,
}

/// Typed state used by scope.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct State<E: Send + 'static> {
    phase: Phase,
    prepared_waiters: prepared::closed::Waiters,
    finalizers: FinalizerBatch<E>,
    reserved_entries: usize,
    segments_in_use: usize,
    segment_limit: Option<usize>,
    outcome: Option<Exit<(), E>>,
    /// Explicit native early-close transfer awaiting its enclosing scoped owner.
    scoped_outcome_transferred: bool,
    status: ScopeCloseStatus,
}

/// Typed inner used by scope.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Inner<E: Send + 'static> {
    state: Mutex<State<E>>,
    changed: Event,
}

/// Acknowledgement shared by concurrent closers. The enclosing scoped effect
/// retains and publishes the owned cleanup errors, including non-Clone errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeCloseStatus {
    /// Cleanup settled without a retained failure.
    Clean,
    /// The operation completed with a failure.
    Failed,
}

/// Per-run dynamic finalizers. Construct with `scoped`; the enclosing effect
/// owns the canonical cleanup outcome. Cloning a handle does not extend scope.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct Scope<E: Send + 'static> {
    inner: Arc<Inner<E>>,
}

impl<E: Send + 'static> Clone for Scope<E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<E: Send + 'static> Scope<E> {
    pub(crate) fn unit_retirement(&self, tag: usize) -> UnitRetirement<E> {
        UnitRetirement {
            scope: Arc::downgrade(&self.inner),
            tag,
        }
    }
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Self::new_inner(FinalizerBatch::system())),
        }
    }
    fn new_state(finalizers: FinalizerBatch<E>) -> State<E> {
        State {
            phase: Phase::Open,
            prepared_waiters: prepared::closed::Waiters::new(),
            scoped_outcome_transferred: false,
            finalizers,
            reserved_entries: 0,
            segments_in_use: 0,
            segment_limit: None,
            outcome: None,
            status: ScopeCloseStatus::Clean,
        }
    }
    fn new_inner(finalizers: FinalizerBatch<E>) -> Inner<E> {
        Inner {
            state: Mutex::new(Self::new_state(finalizers)),
            changed: Event::new(),
        }
    }

    /// Build a fresh scope lazily on each run and await all finalizers before
    /// publishing the body outcome. Cleanup errors remain in the E channel.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `body` (`impl Fn(Self) -> Effect<A, E, R> + Send + Sync + 'static`): Application callback or effect executed inside the owning scope.
    ///
    /// # Returns
    /// Type: `Effect<A, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
                        let cleanup = owner.scope.take_scoped_outcome();
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

    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `usize`.
    pub(crate) fn checkpoint(&self) -> usize {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .finalizers
            .system_checkpoint()
            .unwrap_or_else(|| state.finalizers.len())
    }
    /// # Type parameters
    /// - `A`: Application value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `checkpoint` (`usize`): The checkpoint supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Vec<Registered<E>>, Exit<A, E>>`.
    pub(crate) fn take_since<A>(&self, checkpoint: usize) -> Result<FinalizerBatch<E>, Exit<A, E>> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.phase != Phase::Open {
            return Err(Exit::Die(Defect::from(
                LibraryDefect::ScopeRegistrationClosed,
            )));
        }
        state
            .finalizers
            .system_suffix(checkpoint)
            .ok_or_else(|| Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)))
    }
    /// # Type parameters
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `body` (`crate::internal::workflow::Workflow<A, E, R>`): Application callback or effect executed inside the owning scope.
    /// - `env` (`Arc<R>`): The environment supplying this operation's capabilities.
    /// - `context` (`crate::internal::runtime::Context`): Execution context carrying cancellation, scope, and trace state.
    /// - `finish` (`Finish<A, E>`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `Step`.
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
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `finalizer` (`Effect<(), E, R>`): Cleanup description awaited after the primary outcome.
    ///
    /// # Returns
    /// Type: `Effect<(), E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn add_finalizer<R: Send + Sync + 'static>(
        &self,
        finalizer: Effect<(), E, R>,
    ) -> Effect<(), E, R> {
        self.add_release(move || Release::from_effect(finalizer.clone()))
    }

    /// Lazily register a factory for a fresh single-use release when scope cleanup runs.
    ///
    /// Construction and execution run in protected cleanup context; a panic while
    /// constructing the release becomes a retained cleanup defect. If registration
    /// finds the scope closed, the release runs immediately. Re-executing registration
    /// creates another finalizer; the release payload itself is not cloned.
    pub fn add_release<R: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> Release<E, R> + Send + Sync + 'static,
    ) -> Effect<(), E, R> {
        let scope = self.clone();
        let factory = Arc::new(factory);
        Effect::new(move |env, context, finish| {
            let factory = Arc::clone(&factory);
            let context = context.protected();
            let finalizer =
                Registered::new(
                    move |finish| match catch_unwind(AssertUnwindSafe(|| factory())) {
                        Ok(release) => release.execute(env, context, finish),
                        Err(panic) => runtime::finish(finish, Exit::Die(Defect::from_panic(panic))),
                    },
                );
            scope.register_or_release(finalizer, (), finish)
        })
    }

    /// Acquire into this scope with release declared before acquisition starts.
    /// Late success is owned before cancellation or a closing-scope rejection.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Additional bound: `Send + Sync + 'static`.
    /// - `R`: Environment type. Additional bound: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `acquisition` (`Effect<T, E, R>`): The acquisition supplied to this operation.
    /// - `release` (`impl Fn(T) -> Release<E, R> + Send + Sync + 'static`): The release supplied to this operation.
    ///
    /// # Returns
    /// Type: `Effect<ResourceValue<T>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<ScopeCloseStatus, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
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

    /// Own an existing scope while a differently typed domain body executes, mapping only
    /// its eventual cleanup channel. The guard is installed before body execution, and
    /// abandonment follows the same owner path as `scoped`; cleanup is drained exactly once.
    pub(crate) fn own_mapped<A, F, R>(
        &self,
        body: Effect<A, F, R>,
        map: impl Fn(E) -> F + Send + Sync + 'static,
    ) -> Effect<A, F, R>
    where
        A: Send + 'static,
        F: Send + 'static,
        R: Send + Sync + 'static,
    {
        let scope = self.clone();
        let map = Arc::new(map);
        Effect::new(move |env, context, finish| {
            let mut owner = OwnerGuard {
                scope: scope.clone(),
                settled: false,
            };
            let map = Arc::clone(&map);
            let body_context = context.clone();
            body.clone().execute(
                env,
                body_context,
                runtime::continuation(move |result| {
                    let result = runtime::publish(result, &context);
                    owner.scope.clone().join_close(
                        Exit::Success(()),
                        runtime::continuation(move |_| {
                            let cleanup = owner.scope.take_outcome().map_error(|error| map(error));
                            owner.settled = true;
                            drop(owner);
                            runtime::finish(
                                finish,
                                outcome::append_cleanup(
                                    runtime::publish(result, &context),
                                    cleanup,
                                ),
                            )
                        }),
                    )
                }),
            )
        })
    }

    /// Transfer ownership to one registered finalizer, retaining an abandonment guard
    /// from construction through protected settlement. Unlike `close`, this drains the
    /// cleanup outcome rather than returning only an acknowledgement. Cloned settlement
    /// descriptions coordinate one drain. Later/concurrent calls join actual settlement before
    /// acknowledging; only the caller holding the owner consumes the non-Clone cleanup outcome.
    pub(crate) fn close_owned<R: Send + Sync + 'static>(&self) -> Effect<(), E, R> {
        self.close_owned_with_transfer(false)
    }

    /// Move cleanup to a native scoped body before its observer runs. That body
    /// must retain the full exit; its enclosing scoped owner acknowledges this
    /// explicit transfer instead of attempting to consume the same errors again.
    pub(crate) fn close_scoped_owned<R: Send + Sync + 'static>(&self) -> Effect<(), E, R> {
        self.close_owned_with_transfer(true)
    }

    /// Reuse the canonical drain/abandonment guard for both ownership contracts.
    fn close_owned_with_transfer<R: Send + Sync + 'static>(
        &self,
        transfer: bool,
    ) -> Effect<(), E, R> {
        let scope = self.clone();
        let owner = Mutex::new(Some(OwnerGuard {
            scope: self.clone(),
            settled: false,
        }));
        Effect::new(move |_, _, finish| {
            let guard = owner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            match guard {
                Some(mut owner) => owner.scope.clone().join_close(
                    Exit::Success(()),
                    runtime::continuation(move |_| {
                        let cleanup = owner.scope.take_outcome_transferring(transfer);
                        owner.settled = true;
                        drop(owner);
                        runtime::finish(finish, cleanup)
                    }),
                ),
                None => scope.clone().join_close(Exit::Success(()), finish),
            }
        })
    }

    /// Observe settlement without taking execution ownership or closing it.
    ///
    /// # Type parameters
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `ScopeCloseStatus`.
    pub async fn closed(&self) -> ScopeCloseStatus {
        Self::closed_inner(&self.inner).await
    }
    async fn closed_inner(inner: &Inner<E>) -> ScopeCloseStatus {
        loop {
            let changed = inner.changed.listen();
            {
                let state = inner
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

    ///
    /// # Returns
    /// Type: `ScopeCloseStatus`.
    /// Returns `ScopeCloseStatus`.
    fn status(&self) -> ScopeCloseStatus {
        Self::status_inner(&self.inner)
    }
    fn status_inner(inner: &Inner<E>) -> ScopeCloseStatus {
        inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .status
    }

    /// # Type parameters
    /// - `A`: Application value type. Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `finalizer` (`Registered<E>`): Cleanup description awaited after the primary outcome.
    /// - `value` (`A`): Value read, transformed, encoded, or stored by this operation.
    /// - `finish` (`Finish<A, E>`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `Step`.
    pub(crate) fn register_or_release<A: Send + 'static>(
        &self,
        finalizer: Registered<E>,
        value: A,
        finish: Finish<A, E>,
    ) -> Step {
        match self.try_register(finalizer) {
            Ok(()) => runtime::finish(finish, Exit::Success(value)),
            Err(finalizer) => finalizer.execute_with(move |frame| {
                runtime::continuation_retained(
                    move |cleanup| {
                        drop(value);
                        runtime::finish(
                            finish,
                            outcome::append_cleanup(
                                Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
                                cleanup,
                            ),
                        )
                    },
                    frame,
                )
            }),
        }
    }

    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `finalizer` (`Registered<E>`): Cleanup description awaited after the primary outcome.
    ///
    /// # Returns
    /// Type: `Result<(), Registered<E>>`.
    pub(crate) fn try_register(&self, finalizer: Registered<E>) -> Result<(), Registered<E>> {
        Self::register_inner(&self.inner, finalizer).map_err(|error| match error {
            RegistrationError::Closed(finalizer) | RegistrationError::Full(finalizer) => finalizer,
        })
    }
    fn register_inner(
        inner: &Inner<E>,
        finalizer: Registered<E>,
    ) -> Result<(), RegistrationError<E>> {
        let mut state = inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.phase == Phase::Open {
            if state
                .finalizers
                .available()
                .is_some_and(|available| available <= state.reserved_entries)
            {
                return Err(RegistrationError::Full(finalizer));
            }
            state
                .finalizers
                .push(finalizer)
                .map_err(RegistrationError::Full)
        } else {
            Err(RegistrationError::Closed(finalizer))
        }
    }

    ///
    /// # Type parameters
    /// - `A`: Bounds: `Send + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `primary` (`Exit<A, E>`): The primary supplied to this operation.
    /// - `finish` (`Finish<A, E>`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `Step`.
    /// Returns `Step`.
    fn join_close<A: Send + 'static>(self, primary: Exit<A, E>, finish: Finish<A, E>) -> Step {
        ScopeLink::System(self.inner).join_close(primary, finish)
    }

    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`Exit<(), E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `()`.
    fn settle_inner(inner: &Inner<E>, outcome: Exit<(), E>) {
        let replaced = {
            let mut state = inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.status = if matches!(outcome, Exit::Success(())) {
                ScopeCloseStatus::Clean
            } else {
                ScopeCloseStatus::Failed
            };
            let replaced = state.outcome.replace(outcome);
            state.phase = Phase::Closed;
            replaced
        };
        let _prepared = prepared::closed::NotifyOnDrop { inner };
        notify_all(&inner.changed);
        // Recovery settlement can replace a prior user E after a wake unwinds.
        // Never destroy it under the phase gate; prepared notification remains
        // armed through its destructor as well as through existing Event wakes.
        drop(replaced);
    }

    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Exit<(), E>`.
    /// Returns `Exit<(), E>`.
    fn take_outcome(&self) -> Exit<(), E> {
        self.take_outcome_transferring(false)
    }

    /// Consume an explicit early-transfer acknowledgement once; an absent ordinary
    /// outcome still reports the original consumed-outcome defect.
    fn take_scoped_outcome(&self) -> Exit<(), E> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if std::mem::take(&mut state.scoped_outcome_transferred) {
            Exit::Success(())
        } else {
            state
                .outcome
                .take()
                .unwrap_or_else(|| Exit::Die(Defect::from(LibraryDefect::ScopeOutcomeConsumed)))
        }
    }

    /// Mark transfer only when this owner actually moves a settled cleanup exit.
    /// A failed second take never creates a scoped acknowledgement.
    fn take_outcome_transferring(&self, transfer: bool) -> Exit<(), E> {
        Self::take_outcome_inner(&self.inner, transfer)
    }
    fn take_outcome_inner(inner: &Inner<E>, transfer: bool) -> Exit<(), E> {
        let mut state = inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match state.outcome.take() {
            Some(outcome) => {
                state.scoped_outcome_transferred |= transfer;
                outcome
            }
            None => Exit::Die(Defect::from(LibraryDefect::ScopeOutcomeConsumed)),
        }
    }

    ///
    /// # Returns
    /// Type: `()`.
    fn abandon_open(&self) {
        Self::abandon_inner(&self.inner);
    }
    fn discard_inner(inner: &Inner<E>, work: FinalizerBatch<E>, reason: LibraryDefect) {
        // Captures invalidate outside the gate. Settling is an unwind guard,
        // not a statement skipped if a user resource destructor panics.
        let _settlement = LossSettlement { inner, reason };
        drop(work);
    }
    fn abandon_inner(inner: &Inner<E>) {
        let discarded = {
            let mut state = inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.phase != Phase::Open {
                return;
            }
            state.phase = Phase::Closing;
            std::mem::take(&mut state.finalizers)
        };
        Self::discard_inner(inner, discarded, LibraryDefect::ScopeOwnerDropped);
    }
}

struct LossSettlement<'a, E: Send + 'static> {
    inner: &'a Inner<E>,
    reason: LibraryDefect,
}
impl<E: Send + 'static> Drop for LossSettlement<'_, E> {
    fn drop(&mut self) {
        Scope::<E>::settle_inner(self.inner, Exit::Die(Defect::from(self.reason)));
    }
}

/// Typed owner guard used by scope.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct OwnerGuard<E: Send + 'static> {
    scope: Scope<E>,
    settled: bool,
}

impl<E: Send + 'static> Drop for OwnerGuard<E> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if !self.settled {
            self.scope.abandon_open();
        }
    }
}

/// Typed drain guard used by scope.
///
/// # Type parameters
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct DrainGuard<E: Send + 'static> {
    scope: ScopeLink<E>,
    work: FinalizerBatch<E>,
    settled: bool,
}

impl<E: Send + 'static> Drop for DrainGuard<E> {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if !self.settled {
            // Invalidate resources synchronously before broadcasting loss;
            // deferred destruction of deep capture graphs can finish later.
            Scope::<E>::discard_inner(
                self.scope.inner(),
                std::mem::take(&mut self.work),
                LibraryDefect::ScopeCloseDriverDropped,
            );
        }
    }
}

///
/// # Type parameters
/// - `A`: Bounds: `Send + 'static`.
/// - `E`: Bounds: `Send + 'static`.
///
/// # Parameters
/// - `guard` (`DrainGuard<E>`): The guard supplied to this operation.
/// - `primary` (`Exit<A, E>`): The primary supplied to this operation.
/// - `cleanup` (`Exit<(), E>`): The cleanup supplied to this operation.
/// - `finish` (`Finish<A, E>`): Continuation receiving the complete terminal outcome.
///
/// # Returns
/// Type: `Step`.
/// Returns `Step`.
/// Prepared close uses independently reserved cleanup storage while retaining
/// the same phase transition and finalizer order as ordinary close.
fn drain_prepared<A: Send + 'static, E: Send + 'static>(
    mut guard: DrainGuard<E>,
    primary: Exit<A, E>,
    mut cleanup: crate::PreparedCleanupAccumulator<(), E>,
    finish: Finish<A, E>,
) -> Step {
    runtime::more(move || match guard.work.pop() {
        Some(finalizer) => finalizer.execute_with(move |frame| {
            runtime::continuation_retained(
                move |result| {
                    cleanup.append_cleanup(result);
                    drain_prepared(guard, primary, cleanup, finish)
                },
                frame,
            )
        }),
        None => {
            guard.scope.settle(cleanup.finish());
            guard.settled = true;
            drop(guard);
            runtime::finish(finish, primary)
        }
    })
}

fn drain<A: Send + 'static, E: Send + 'static>(
    mut guard: DrainGuard<E>,
    primary: Exit<A, E>,
    cleanup: Exit<(), E>,
    finish: Finish<A, E>,
) -> Step {
    runtime::more(move || match guard.work.pop() {
        Some(finalizer) => finalizer.execute_with(move |frame| {
            runtime::continuation_retained(
                move |result| {
                    drain(
                        guard,
                        primary,
                        outcome::append_cleanup(cleanup, result),
                        finish,
                    )
                },
                frame,
            )
        }),
        None => {
            guard.scope.settle(cleanup);
            guard.settled = true;
            drop(guard);
            runtime::finish(finish, primary)
        }
    })
}
