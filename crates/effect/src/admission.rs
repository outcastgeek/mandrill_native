use crate::internal::notification::notify_all;
use event_listener::Event;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionCapacity {
    Unbounded,
    Bounded(NonZeroUsize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Closed,
    Full,
    Overflow,
}
struct State {
    closed: bool,
    active: usize,
    drivers: usize,
}
struct Inner {
    state: Mutex<State>,
    capacity: AdmissionCapacity,
    changed: Event,
}
#[derive(Clone)]
pub struct ProviderAdmission {
    inner: Arc<Inner>,
}
pub struct AdmissionPermit {
    inner: Option<Arc<Inner>>,
}
pub struct ProviderDrain {
    inner: Arc<Inner>,
}
impl ProviderAdmission {
    #[must_use]
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
    pub fn active(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active
    }
    #[must_use]
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

type SettlementAction = Box<dyn FnOnce() + Send>;
#[derive(Clone, Default)]
pub(crate) struct Settlement(Arc<Mutex<Vec<SettlementAction>>>);
impl Settlement {
    pub(crate) fn add(&self, action: impl FnOnce() + Send + 'static) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Box::new(action));
    }
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
    fn from(error: AdmissionError) -> Self {
        Self::from(match error {
            AdmissionError::Closed => crate::LibraryDefect::ProviderClosed,
            AdmissionError::Full => crate::LibraryDefect::ProviderCapacityFull,
            AdmissionError::Overflow => crate::LibraryDefect::ProviderCountOverflow,
        })
    }
}

struct CapacityPermit(Option<Arc<Inner>>);
struct DriverPermit(Option<Arc<Inner>>);
impl Drop for CapacityPermit {
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
pub struct AdmittedTask {
    task: Option<crate::ScheduledTask>,
    driver: DriverPermit,
}
pub struct AdmittedJob {
    job: Option<crate::ScheduledJob>,
    driver: DriverPermit,
    diagnostics: crate::Diagnostics,
}
impl AdmissionPermit {
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
    pub fn task(self, task: crate::ScheduledTask) -> AdmittedTask {
        let (capacity, driver) = self.split();
        AdmittedTask {
            task: Some(task.on_settled(move || drop(capacity))),
            driver,
        }
    }
    #[must_use]
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
    type Output = ();
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
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(self.task.take())));
        self.driver = DriverPermit(None);
    }
}
impl AdmittedJob {
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
    fn drop(&mut self) {
        drop(self.job.take());
        self.driver = DriverPermit(None);
    }
}
