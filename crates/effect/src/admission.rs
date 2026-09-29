//! Admission limits and permits shared by runtime-neutral providers.

use crate::internal::notification::notify_all;
use event_listener::Event;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for admission.
pub enum AdmissionCapacity {
    /// No explicit capacity bound is imposed.
    Unbounded,
    /// The supplied positive capacity bound is enforced. Contains `NonZeroUsize`.
    Bounded(NonZeroUsize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A typed reason a provider rejected new work before taking ownership.
pub enum AdmissionError {
    /// The provider has closed admission for new work.
    Closed,
    /// The configured capacity has been reached.
    Full,
    /// A bounded counter or arithmetic operation overflowed.
    Overflow,
}
/// Typed state used by admission.
struct State {
    closed: bool,
    active: usize,
    drivers: usize,
}
/// Typed inner used by admission.
struct Inner {
    state: Mutex<State>,
    capacity: AdmissionCapacity,
    changed: Event,
}
#[derive(Clone)]
/// Shared admission state that bounds accepted tasks or jobs and tracks their settlement.
pub struct ProviderAdmission {
    inner: Arc<Inner>,
}
/// The owned reservation for one accepted provider operation.
pub struct AdmissionPermit {
    inner: Option<Arc<Inner>>,
}
/// A shutdown handle that waits until accepted provider work has settled.
pub struct ProviderDrain {
    inner: Arc<Inner>,
}
impl ProviderAdmission {
    #[must_use]
    /// Construct a `ProviderAdmission` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `capacity` (`AdmissionCapacity`): The caller-supplied capacity bound.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(capacity: AdmissionCapacity) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    closed: false,
                    active: usize::default(),
                    drivers: usize::default(),
                }),
                capacity,
                changed: Event::new(),
            }),
        }
    }
    /// Reserve capacity before transferring ownership of a task or job.
    ///
    /// # Returns
    /// Type: `Result<AdmissionPermit, AdmissionError>`.
    pub fn admit(&self) -> Result<AdmissionPermit, AdmissionError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(AdmissionError::Closed);
        }
        if let AdmissionCapacity::Bounded(limit) = self.inner.capacity
            && state.active == limit.get()
        {
            return Err(AdmissionError::Full);
        }
        let active = state
            .active
            .checked_add(NonZeroUsize::MIN.get())
            .ok_or(AdmissionError::Overflow)?;
        let drivers = state
            .drivers
            .checked_add(NonZeroUsize::MIN.get())
            .ok_or(AdmissionError::Overflow)?;
        state.active = active;
        state.drivers = drivers;
        Ok(AdmissionPermit {
            inner: Some(Arc::clone(&self.inner)),
        })
    }
    #[must_use]
    /// Count provider operations admitted and not yet settled.
    ///
    /// # Returns
    /// Type: `usize`.
    pub fn active(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active
    }
    #[must_use]
    /// Close admission and return a drain for work already accepted.
    ///
    /// # Returns
    /// Type: `ProviderDrain`.
    pub fn stop(&self) -> ProviderDrain {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed = true;
        ProviderDrain {
            inner: Arc::clone(&self.inner),
        }
    }
}
impl Drop for AdmissionPermit {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            let empty = {
                let mut state = inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.active -= NonZeroUsize::MIN.get();
                state.active == usize::default()
            };
            {
                let mut state = inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.drivers -= NonZeroUsize::MIN.get();
            }
            if empty {
                notify_all(&inner.changed);
            }
        }
    }
}
impl ProviderDrain {
    /// Wait for the represented condition or owned work to settle.
    ///
    /// # Returns
    /// Type: `()`.
    pub async fn wait(self) {
        loop {
            let changed = self.inner.changed.listen();
            if self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .drivers
                == usize::default()
            {
                return;
            }
            changed.await;
        }
    }
}

/// Type alias for settlement action in admission.
type SettlementAction = Box<dyn FnOnce() + Send>;
#[derive(Clone, Default)]
/// Typed settlement used by admission.
pub(crate) struct Settlement(Arc<Mutex<Vec<SettlementAction>>>);
impl Settlement {
    /// # Parameters
    /// - `action` (`impl FnOnce() + Send + 'static`): State transition selected for this operation.
    ///
    /// # Returns
    /// Type: `()`.
    pub(crate) fn add(&self, action: impl FnOnce() + Send + 'static) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Box::new(action));
    }
    /// # Returns
    /// Type: `()`.
    pub(crate) fn release(&self) {
        let actions = std::mem::take(
            &mut *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for action in actions {
            action();
        }
    }
}
impl From<AdmissionError> for crate::Defect {
    ///
    /// # Parameters
    /// - `error` (`AdmissionError`): Expected or provider error preserved in the returned outcome.
    ///
    /// # Returns
    /// Type: `Self`.
    fn from(error: AdmissionError) -> Self {
        Self::from(match error {
            AdmissionError::Closed => crate::LibraryDefect::ProviderClosed,
            AdmissionError::Full => crate::LibraryDefect::ProviderCapacityFull,
            AdmissionError::Overflow => crate::LibraryDefect::ProviderCountOverflow,
        })
    }
}

/// Typed capacity permit used by admission.
struct CapacityPermit(Option<Arc<Inner>>);
/// Typed driver permit used by admission.
struct DriverPermit(Option<Arc<Inner>>);
impl Drop for CapacityPermit {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if let Some(inner) = self.0.take() {
            inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .active -= NonZeroUsize::MIN.get();
        }
    }
}
impl Drop for DriverPermit {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        if let Some(inner) = self.0.take() {
            let empty = {
                let mut state = inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.drivers -= NonZeroUsize::MIN.get();
                state.drivers == usize::default()
            };
            if empty {
                notify_all(&inner.changed);
            }
        }
    }
}
/// A scheduled task paired with its admission permit until completion or driver loss.
pub struct AdmittedTask {
    task: Option<crate::ScheduledTask>,
    driver: DriverPermit,
}
/// A synchronous job paired with its admission permit until completion or driver loss.
pub struct AdmittedJob {
    job: Option<crate::ScheduledJob>,
    driver: DriverPermit,
    diagnostics: crate::Diagnostics,
}
impl AdmissionPermit {
    ///
    /// # Returns
    /// Type: `(CapacityPermit, DriverPermit)`.
    /// Returns `(CapacityPermit, DriverPermit)`.
    fn split(mut self) -> (CapacityPermit, DriverPermit) {
        match self.inner.take() {
            Some(inner) => (
                CapacityPermit(Some(Arc::clone(&inner))),
                DriverPermit(Some(inner)),
            ),
            None => (CapacityPermit(None), DriverPermit(None)),
        }
    }
    #[must_use]
    /// Pair a scheduled task with this admission permit until settlement.
    ///
    /// # Parameters
    /// - `task` (`crate::ScheduledTask`): The task supplied to this operation.
    ///
    /// # Returns
    /// Type: `AdmittedTask`.
    pub fn task(self, task: crate::ScheduledTask) -> AdmittedTask {
        let (capacity, driver) = self.split();
        AdmittedTask {
            task: Some(task.on_settled(move || drop(capacity))),
            driver,
        }
    }
    #[must_use]
    /// Pair a synchronous job with this admission permit until settlement.
    ///
    /// # Parameters
    /// - `job` (`crate::ScheduledJob`): Owned synchronous callback transferred to a provider.
    /// - `diagnostics` (`crate::Diagnostics`): Separate sink for instrumentation or provider failures.
    ///
    /// # Returns
    /// Type: `AdmittedJob`.
    pub fn job(self, job: crate::ScheduledJob, diagnostics: crate::Diagnostics) -> AdmittedJob {
        let (capacity, driver) = self.split();
        AdmittedJob {
            job: Some(job.on_settled(move || drop(capacity))),
            driver,
            diagnostics,
        }
    }
}
impl std::future::Future for AdmittedTask {
    /// Type alias for output in admission.
    type Output = ();
    ///
    /// # Parameters
    /// - `cx` (`&mut std::task::Context<'_>`): Rust task context used to register the current waker.
    ///
    /// # Returns
    /// Type: `std::task::Poll<()>`.
    /// Returns `std::task::Poll<()>`.
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let this = self.get_mut();
        let result = match &mut this.task {
            Some(task) => std::pin::Pin::new(task).poll(cx),
            None => std::task::Poll::Ready(()),
        };
        if result.is_ready() {
            drop(this.task.take());
            this.driver = DriverPermit(None);
        }
        result
    }
}
impl Drop for AdmittedTask {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(self.task.take())));
        self.driver = DriverPermit(None);
    }
}
impl AdmittedJob {
    /// Execute or describe one run using the supplied environment and inputs.
    ///
    /// # Returns
    /// Type: `()`.
    pub fn run(mut self) {
        if let Some(job) = self.job.take()
            && let Err(defect) = job.run()
        {
            self.diagnostics.record(defect);
        }
        self.driver = DriverPermit(None);
    }
}
impl Drop for AdmittedJob {
    ///
    /// # Returns
    /// Type: `()`.
    fn drop(&mut self) {
        drop(self.job.take());
        self.driver = DriverPermit(None);
    }
}
