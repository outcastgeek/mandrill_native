//! Owned concurrent Effect execution and result observation.

use crate::execution::TaskEvent;
use crate::internal::notification::notify_all;
use crate::internal::outcome;
use crate::internal::runtime::{self, Context, Finish, Step};
use crate::scope::Registered;
use crate::{
    Cancellation, Defect, Effect, Exit, HasTaskSpawner, LibraryDefect, ScheduledTask, Scope,
    TaskSpawner,
};
use event_listener::Event;
use futures_util::future::select;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Variants of admission used by fiber.
enum Admission {
    Submitting,
    Accepted,
    Rejected,
    Panicked,
}

/// Variants of completion used by fiber.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
enum Completion<A: Send + 'static, E: Send + 'static> {
    Pending,
    Ready(Exit<A, E>),
    Shared(SharedExit<A, E>),
    Lost(Defect),
    Consumed,
}

/// Typed state used by fiber.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct State<A: Send + 'static, E: Send + 'static> {
    admission: Admission,
    started: bool,
    completion: Completion<A, E>,
    status: Option<FiberStatus>,
    observed: bool,
    protected: bool,
    origin: Option<JoinedOrigin>,
}

#[derive(Clone, Copy)]
/// Variants of joined origin used by fiber.
pub(crate) enum JoinedOrigin {
    Program,
    DriverLost,
    ObservationFailed,
}

/// Typed joined used by fiber.
///
/// # Type parameters
/// - `A`: Application value type.
/// - `E`: Domain-error type.
pub(crate) struct Joined<A, E> {
    pub(crate) outcome: Exit<A, E>,
    pub(crate) protected: bool,
    pub(crate) origin: JoinedOrigin,
}

/// Typed inner used by fiber.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
struct Inner<A: Send + 'static, E: Send + 'static> {
    cancellation: Cancellation,
    state: Mutex<State<A, E>>,
    changed: Event,
}

/// Terminal classification. Observing it does not consume or recover errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeStatus {
    /// The operation completed successfully.
    Succeeded,
    /// The operation completed with a failure.
    Failed,
    /// The operation ended in a defect outside its expected-error channel.
    Defected,
    /// Execution was interrupted.
    Interrupted,
    /// Cleanup failed after the primary operation settled.
    FinalizationFailed,
}

/// Terminal classification used by fiber status observations.
pub type FiberStatus = OutcomeStatus;

impl OutcomeStatus {
    ///
    /// # Type parameters
    /// - `A`: Successful application-value type.
    /// - `E`: Expected application-error type.
    ///
    /// # Parameters
    /// - `outcome` (`&Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Self`.
    const fn of<A, E>(outcome: &Exit<A, E>) -> Self {
        match outcome {
            Exit::Success(_) => Self::Succeeded,
            Exit::Fail(_) => Self::Failed,
            Exit::Die(_) => Self::Defected,
            Exit::Interrupted => Self::Interrupted,
            Exit::FinalizationFailed(_) => Self::FinalizationFailed,
        }
    }
}

/// Shared ownership of one complete outcome, without cloning A or E.
///
/// Inspection borrows under a data gate and cannot return escaping references.
/// Callbacks are synchronous and must not re-enter inspection of the same data.
/// This data gate is separate from its producer's completion/status gate. Handles
/// keep the outcome alive after scope closure or Deferred destruction. This wrapper holds only the
/// outcome/status, with no additional environment or provider reference.
#[derive(Debug)]
#[must_use = "inspect the shared outcome to handle its complete exit"]
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct SharedExit<A: Send + 'static, E: Send + 'static> {
    outcome: Arc<Mutex<Exit<A, E>>>,
    status: OutcomeStatus,
}

impl<A: Send + 'static, E: Send + 'static> Clone for SharedExit<A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            outcome: Arc::clone(&self.outcome),
            status: self.status,
        }
    }
}

impl<A: Send + 'static, E: Send + 'static> SharedExit<A, E> {
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `outcome` (`Exit<A, E>`): Complete terminal outcome, including expected errors and cleanup failures.
    ///
    /// # Returns
    /// Type: `Self`.
    pub(crate) fn new(outcome: Exit<A, E>) -> Self {
        Self {
            status: OutcomeStatus::of(&outcome),
            outcome: Arc::new(Mutex::new(outcome)),
        }
    }

    #[must_use]
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `OutcomeStatus`.
    pub const fn status(&self) -> OutcomeStatus {
        self.status
    }

    /// Borrow the outcome without cloning or moving its payloads. Panicking
    /// inspection becomes a defect and leaves the canonical outcome intact.
    ///
    /// # Type parameters
    /// - `B`: Transformed value type.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `inspect` (`impl FnOnce(&Exit<A, E>) -> B`): The inspect supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<B, Defect>`.
    pub fn with<B>(&self, inspect: impl FnOnce(&Exit<A, E>) -> B) -> Result<B, Defect> {
        let outcome = self
            .outcome
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        catch_unwind(AssertUnwindSafe(|| inspect(&outcome))).map_err(Defect::from_panic)
    }

    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Result<Exit<A, E>, Self>`.
    pub(crate) fn into_owned(self) -> Result<Exit<A, E>, Self> {
        match Arc::try_unwrap(self.outcome) {
            Ok(outcome) => Ok(outcome
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner)),
            Err(outcome) => Err(Self {
                outcome,
                status: self.status,
            }),
        }
    }
}

/// A child owned by its scope and driven by the task provider selected in R.
///
/// Owned observations transfer the outcome once, without Clone/Sync bounds on
/// A or E. Shared observations are repeatable and retain the same payloads.
/// Owned transfer after sharing succeeds only when all shared handles are gone.
/// Cloning a Fiber or SharedExit clones Arc ownership, never result/error data.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
pub struct Fiber<A: Send + 'static, E: Send + 'static> {
    inner: Arc<Inner<A, E>>,
}

impl<A: Send + 'static, E: Send + 'static> Clone for Fiber<A, E> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[derive(Clone, Copy)]
/// Variants of observation mode used by fiber.
enum ObservationMode {
    Await,
    Interrupt,
    Owner,
}

#[derive(Clone, Copy)]
/// Variants of ownership used by fiber.
enum Ownership {
    Owned,
    Shared,
}

/// Variants of observed used by fiber.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
enum Observed<A: Send + 'static, E: Send + 'static> {
    Owned(Exit<A, E>),
    Shared(SharedExit<A, E>),
}

/// Variants of cleanup used by fiber.
///
/// # Type parameters
/// - `A`: Application value type. Bound or type: `Send + 'static`.
/// - `E`: Domain-error type. Bound or type: `Send + 'static`.
enum Cleanup<A: Send + 'static, E: Send + 'static> {
    Observed,
    Outcome(Exit<A, E>),
    Loss { defect: Defect, report: bool },
}

impl<A: Send + 'static, E: Send + 'static> Fiber<A, E> {
    ///
    /// # Parameters
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    fn new(cancellation: Cancellation) -> Self {
        Self {
            inner: Arc::new(Inner {
                cancellation,
                state: Mutex::new(State {
                    admission: Admission::Submitting,
                    started: false,
                    completion: Completion::Pending,
                    status: None,
                    observed: false,
                    protected: false,
                    origin: None,
                }),
                changed: Event::new(),
            }),
        }
    }

    ///
    /// # Returns
    /// Type: `()`.
    fn started(&self) {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .started = true;
    }

    ///
    /// # Parameters
    /// - `admission` (`Admission`): The admission supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    fn admitted(&self, admission: Admission) {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .admission = admission;
        notify_all(&self.inner.changed);
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `completion` (`Completion<A, E>`): The completion supplied to this operation.
    /// - `protected` (`bool`): The protected supplied to this operation.
    ///
    /// # Returns
    /// Type: `()`.
    fn publish(&self, completion: Completion<A, E>, protected: bool) {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !matches!(state.completion, Completion::Pending) {
                return;
            }
            state.status = match &completion {
                Completion::Ready(outcome) => Some(FiberStatus::of(outcome)),
                Completion::Lost(_) => Some(FiberStatus::Defected),
                Completion::Shared(shared) => Some(shared.status()),
                Completion::Pending | Completion::Consumed => None,
            };
            state.protected = protected;
            state.origin = match &completion {
                Completion::Ready(_) | Completion::Shared(_) => Some(JoinedOrigin::Program),
                Completion::Lost(_) => Some(JoinedOrigin::DriverLost),
                Completion::Pending | Completion::Consumed => None,
            };
            state.completion = completion;
        }
        notify_all(&self.inner.changed);
    }

    /// Await classification without marking an error observed.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `FiberStatus`.
    pub async fn settled(&self) -> FiberStatus {
        loop {
            let changed = self.inner.changed.listen();
            let status = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .status;
            if let Some(status) = status {
                return status;
            }
            changed.await;
        }
    }

    /// # Type parameters
    /// - `A`: Application value type. Bound: `Send + 'static`.
    /// - `E`: Domain-error type. Bound: `Send + 'static`.
    ///
    /// # Returns
    /// Type: `()`.
    pub(crate) fn cancel(&self) {
        let _ = self.inner.cancellation.request();
    }

    /// A parallel operation owns every branch and joins even after cancellation.
    /// Payloads move once; no observer or extra shared-data allocation is needed.
    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Joined<A, E>`.
    pub(crate) async fn take_owned(&self) -> Joined<A, E> {
        let outcome = match self
            .observe(
                self.inner.cancellation.clone(),
                ObservationMode::Owner,
                Ownership::Owned,
            )
            .await
        {
            Ok(Some(Observed::Owned(outcome))) => outcome,
            Ok(Some(Observed::Shared(_))) | Ok(None) => {
                Exit::Die(Defect::from(LibraryDefect::FiberObservationOwnership))
            }
            Err(defect) => {
                return Joined {
                    outcome: Exit::Die(defect),
                    protected: false,
                    origin: JoinedOrigin::ObservationFailed,
                };
            }
        };
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Joined {
            outcome,
            protected: state.protected,
            origin: state.origin.unwrap_or(JoinedOrigin::ObservationFailed),
        }
    }

    /// Transfer the complete child outcome once. Cancellation leaves the child
    /// unobserved; its scope continues to own it and retains its failures.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<Exit<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn await_exit<R: Send + Sync + 'static>(&self) -> Effect<Exit<A, E>, E, R> {
        self.owned_observation(ObservationMode::Await)
    }

    /// Transfer the child outcome directly into the caller's E channel once.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<A, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn join<R: Send + Sync + 'static>(&self) -> Effect<A, E, R> {
        let fiber = self.clone();
        Effect::new(move |_, context, finish| {
            let fiber = fiber.clone();
            Step::Suspended(Box::pin(async move {
                let outcome = match fiber
                    .observe(
                        context.cancellation,
                        ObservationMode::Await,
                        Ownership::Owned,
                    )
                    .await
                {
                    Ok(Some(Observed::Owned(outcome))) => outcome,
                    Ok(Some(Observed::Shared(_))) => {
                        Exit::Die(Defect::from(LibraryDefect::FiberObservationOwnership))
                    }
                    Ok(None) => Exit::Interrupted,
                    Err(defect) => Exit::Die(defect),
                };
                runtime::finish(finish, outcome)
            }))
        })
    }

    /// Request interruption, then join even if the caller is cancelled while
    /// waiting. A cancelled caller leaves the joined outcome unobserved.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<Exit<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn interrupt<R: Send + Sync + 'static>(&self) -> Effect<Exit<A, E>, E, R> {
        self.owned_observation(ObservationMode::Interrupt)
    }

    /// Repeatable shared observation. A and E are retained without cloning.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<SharedExit<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn await_exit_shared<R: Send + Sync + 'static>(&self) -> Effect<SharedExit<A, E>, E, R> {
        self.shared_observation(ObservationMode::Await)
    }

    /// Repeatable cancel-and-join with shared ownership of the same outcome.
    ///
    /// # Type parameters
    /// - `R`: Environment type. Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Effect<SharedExit<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn interrupt_shared<R: Send + Sync + 'static>(&self) -> Effect<SharedExit<A, E>, E, R> {
        self.shared_observation(ObservationMode::Interrupt)
    }

    ///
    /// # Type parameters
    /// - `R`: Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `mode` (`ObservationMode`): Selected behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<Exit<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn owned_observation<R: Send + Sync + 'static>(
        &self,
        mode: ObservationMode,
    ) -> Effect<Exit<A, E>, E, R> {
        let fiber = self.clone();
        Effect::new(move |_, context, finish| {
            let fiber = fiber.clone();
            Step::Suspended(Box::pin(async move {
                if matches!(mode, ObservationMode::Interrupt) {
                    let _ = fiber.inner.cancellation.request();
                }
                let outcome = match fiber
                    .observe(context.cancellation, mode, Ownership::Owned)
                    .await
                {
                    Ok(Some(Observed::Owned(outcome))) => Exit::Success(outcome),
                    Ok(Some(Observed::Shared(_))) => {
                        Exit::Die(Defect::from(LibraryDefect::FiberObservationOwnership))
                    }
                    Ok(None) => Exit::Interrupted,
                    Err(defect) => Exit::Die(defect),
                };
                runtime::finish(finish, outcome)
            }))
        })
    }

    ///
    /// # Type parameters
    /// - `R`: Bounds: `Send + Sync + 'static`.
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `mode` (`ObservationMode`): Selected behavior for this operation.
    ///
    /// # Returns
    /// Type: `Effect<SharedExit<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    fn shared_observation<R: Send + Sync + 'static>(
        &self,
        mode: ObservationMode,
    ) -> Effect<SharedExit<A, E>, E, R> {
        let fiber = self.clone();
        Effect::new(move |_, context, finish| {
            let fiber = fiber.clone();
            Step::Suspended(Box::pin(async move {
                if matches!(mode, ObservationMode::Interrupt) {
                    let _ = fiber.inner.cancellation.request();
                }
                let outcome = match fiber
                    .observe(context.cancellation, mode, Ownership::Shared)
                    .await
                {
                    Ok(Some(Observed::Shared(outcome))) => Exit::Success(outcome),
                    Ok(Some(Observed::Owned(_))) => {
                        Exit::Die(Defect::from(LibraryDefect::FiberObservationOwnership))
                    }
                    Ok(None) => Exit::Interrupted,
                    Err(defect) => Exit::Die(defect),
                };
                runtime::finish(finish, outcome)
            }))
        })
    }

    ///
    /// # Type parameters
    /// - `A`: Success or application-value type.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `cancellation` (`Cancellation`): The cancellation token for this operation.
    /// - `mode` (`ObservationMode`): Selected behavior for this operation.
    /// - `ownership` (`Ownership`): The ownership supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Option<Observed<A, E>>, Defect>`.
    async fn observe(
        &self,
        cancellation: Cancellation,
        mode: ObservationMode,
        ownership: Ownership,
    ) -> Result<Option<Observed<A, E>>, Defect> {
        loop {
            let changed = self.inner.changed.listen();
            let available = {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // Interrupt waits through settlement, then checks the caller's
                // token before marking observation, like the source's observe.
                if cancellation.is_requested()
                    && (matches!(mode, ObservationMode::Await)
                        || (matches!(mode, ObservationMode::Interrupt) && state.status.is_some()))
                {
                    return Ok(None);
                }
                if state.admission == Admission::Submitting
                    || matches!(state.completion, Completion::Pending)
                {
                    None
                } else {
                    let completion = std::mem::replace(&mut state.completion, Completion::Consumed);
                    let result = match (completion, ownership) {
                        (Completion::Ready(outcome), Ownership::Owned) => {
                            Ok(Observed::Owned(outcome))
                        }
                        (Completion::Lost(defect), Ownership::Owned) => {
                            Ok(Observed::Owned(Exit::Die(defect)))
                        }
                        (Completion::Ready(outcome), Ownership::Shared) => {
                            let shared = SharedExit::new(outcome);
                            state.completion = Completion::Shared(shared.clone());
                            Ok(Observed::Shared(shared))
                        }
                        (Completion::Lost(defect), Ownership::Shared) => {
                            let shared = SharedExit::new(Exit::Die(defect));
                            state.completion = Completion::Shared(shared.clone());
                            Ok(Observed::Shared(shared))
                        }
                        (Completion::Shared(shared), Ownership::Shared) => {
                            state.completion = Completion::Shared(shared.clone());
                            Ok(Observed::Shared(shared))
                        }
                        (Completion::Shared(shared), Ownership::Owned) => match shared.into_owned()
                        {
                            Ok(outcome) => Ok(Observed::Owned(outcome)),
                            Err(shared) => {
                                state.completion = Completion::Shared(shared);
                                Err(Defect::from(LibraryDefect::FiberOutcomeShared))
                            }
                        },
                        (Completion::Consumed, Ownership::Owned | Ownership::Shared) => {
                            Err(Defect::from(LibraryDefect::FiberOutcomeConsumed))
                        }
                        (Completion::Pending, Ownership::Owned | Ownership::Shared) => {
                            state.completion = Completion::Pending;
                            Err(Defect::from(LibraryDefect::FiberObservationOwnership))
                        }
                    };
                    state.observed |= result.is_ok();
                    Some(result)
                }
            };
            if let Some(result) = available {
                return result.map(Some);
            }
            match mode {
                ObservationMode::Await => {
                    let cancelled = std::pin::pin!(cancellation.cancelled());
                    let _ = select(changed, cancelled).await;
                }
                ObservationMode::Interrupt | ObservationMode::Owner => changed.await,
            }
        }
    }

    ///
    /// # Type parameters
    /// - `E`: Domain-error type.
    ///
    /// # Returns
    /// Type: `Exit<(), E>`.
    /// Returns `Exit<(), E>`.
    async fn cleanup(&self) -> Exit<(), E> {
        let _ = self.inner.cancellation.request();
        loop {
            let changed = self.inner.changed.listen();
            let completed = {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.admission == Admission::Submitting || state.status.is_none() {
                    None
                } else if state.observed {
                    Some(Cleanup::Observed)
                } else {
                    match std::mem::replace(&mut state.completion, Completion::Consumed) {
                        Completion::Ready(outcome) => Some(Cleanup::Outcome(outcome)),
                        Completion::Lost(defect) => Some(Cleanup::Loss {
                            defect,
                            report: state.admission == Admission::Accepted || state.started,
                        }),
                        Completion::Consumed | Completion::Shared(_) => Some(Cleanup::Observed),
                        Completion::Pending => {
                            state.completion = Completion::Pending;
                            None
                        }
                    }
                }
            };
            if let Some(completed) = completed {
                self.inner.cancellation.unlink();
                return match completed {
                    Cleanup::Observed => Exit::Success(()),
                    Cleanup::Outcome(Exit::Success(value)) => {
                        match catch_unwind(AssertUnwindSafe(|| drop(value))) {
                            Ok(()) => Exit::Success(()),
                            Err(panic) => Exit::Die(Defect::from_panic(panic)),
                        }
                    }
                    Cleanup::Outcome(Exit::Interrupted) => Exit::Success(()),
                    Cleanup::Outcome(failure) => match failure.into_failure() {
                        Ok(_) => Exit::Success(()),
                        Err(failure) => failure,
                    },
                    Cleanup::Loss { defect, report } => {
                        if report {
                            Exit::Die(defect)
                        } else {
                            Exit::Success(())
                        }
                    }
                };
            }
            changed.await;
        }
    }
}

impl<E: Send + 'static> Scope<E> {
    /// Register ownership before submitting through the current R. Closing
    /// rejects without accessing the provider. Cleanup cancels and joins every
    /// submitted task, including provider rejection/panic and cancellation races.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasTaskSpawner + Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
    ///
    /// # Returns
    /// Type: `Effect<Fiber<A, E>, E, R>`.
    /// Returns a lazy `Effect` with the declared success, error, and environment types.
    pub fn fork<A, R>(&self, program: Effect<A, E, R>) -> Effect<Fiber<A, E>, E, R>
    where
        A: Send + 'static,
        R: HasTaskSpawner + Send + Sync + 'static,
    {
        let scope = self.clone();
        Effect::new(move |env, context, finish| {
            scope.submit(program.clone(), &env, context.child(), finish)
        })
    }

    /// Already-entered parallel operations finish admission and own joining
    /// regardless of a later parent request. R still supplies the only provider.
    ///
    /// # Type parameters
    /// - `A`: Application value type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type. Additional bound: `HasTaskSpawner + Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
    /// - `env` (`&Arc<R>`): The environment supplying this operation's capabilities.
    /// - `context` (`Context`): Execution context carrying cancellation, scope, and trace state.
    /// - `finish` (`Finish<Fiber<A, E>, E>`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `Step`.
    pub(crate) fn start_parallel<A, R>(
        &self,
        program: Effect<A, E, R>,
        env: &Arc<R>,
        context: Context,
        finish: Finish<Fiber<A, E>, E>,
    ) -> Step
    where
        A: Send + 'static,
        R: HasTaskSpawner + Send + Sync + 'static,
    {
        self.submit(program, env, context, finish)
    }

    ///
    /// # Type parameters
    /// - `A`: Successful application-value type. Additional bound: `Send + 'static`.
    /// - `R`: Environment type supplying required capabilities. Additional bound: `HasTaskSpawner + Send + Sync + 'static`.
    /// - `E`: Domain-error type.
    ///
    /// # Parameters
    /// - `program` (`Effect<A, E, R>`): Lazy Effect description executed or transformed here.
    /// - `env` (`&Arc<R>`): The environment supplying this operation's capabilities.
    /// - `child_context` (`Context`): The child context supplied to this operation.
    /// - `finish` (`Finish<Fiber<A, E>, E>`): Continuation receiving the complete terminal outcome.
    ///
    /// # Returns
    /// Type: `Step`.
    /// Returns `Step`.
    fn submit<A, R>(
        &self,
        program: Effect<A, E, R>,
        env: &Arc<R>,
        child_context: Context,
        finish: Finish<Fiber<A, E>, E>,
    ) -> Step
    where
        A: Send + 'static,
        R: HasTaskSpawner + Send + Sync + 'static,
    {
        let fiber = Fiber::new(child_context.cancellation.clone());
        let cleanup = fiber.clone();
        let abandon = fiber.clone();
        let finalizer = Registered::with_abandon(
            move |finish| {
                Step::Suspended(Box::pin(async move {
                    runtime::finish(finish, cleanup.cleanup().await)
                }))
            },
            move || {
                let _ = abandon.inner.cancellation.request();
            },
        );
        if let Err(finalizer) = self.try_register(finalizer) {
            fiber.admitted(Admission::Rejected);
            fiber.publish(Completion::Ready(Exit::Interrupted), false);
            return finalizer.execute(runtime::continuation(move |cleanup| {
                runtime::finish(
                    finish,
                    outcome::append_cleanup(
                        Exit::Die(Defect::from(LibraryDefect::ScopeRegistrationClosed)),
                        cleanup,
                    ),
                )
            }));
        }
        let completed = fiber.clone();
        // One notification capture handles startup and loss, rather than
        // separately boxing each callback and retaining two Fiber handles.
        let notification = fiber.clone();
        let child_env = Arc::clone(env);
        let task = ScheduledTask::new(
            move |settlement| {
                Box::pin(async move {
                    let outcome =
                        runtime::evaluate_context(program, child_env, &child_context).await;
                    settlement.release();
                    completed.publish(Completion::Ready(outcome), child_context.was_protected());
                })
            },
            move |event| match event {
                TaskEvent::Started => notification.started(),
                TaskEvent::Lost(defect) => notification.publish(Completion::Lost(defect), false),
            },
        );
        match catch_unwind(AssertUnwindSafe(|| env.task_spawner().spawn(task))) {
            Ok(Ok(())) => {
                fiber.admitted(Admission::Accepted);
                runtime::finish(finish, Exit::Success(fiber))
            }
            Ok(Err(defect)) => {
                fiber.admitted(Admission::Rejected);
                runtime::finish(finish, Exit::Die(defect))
            }
            Err(panic) => {
                fiber.admitted(Admission::Panicked);
                runtime::finish(finish, Exit::Die(Defect::from_panic(panic)))
            }
        }
    }
}
