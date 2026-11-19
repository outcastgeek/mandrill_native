//! Joined, independently reserved SDK cleanup for the selected owned providers.

use super::prepared_shutdown::{PreparedLifecycleError, PreparedShutdown};
use super::{
    BorrowedProviders, LifecycleError, ProviderFactories, ShutdownPolicy, build_providers,
};
use crate::{effect_future, effect_scope_acquire, effect_try_blocking, effect_uninterruptible};
use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use effect::prepared_provider::ProviderWorkingSet;
use effect::{
    AdmissionCapacity, Defect, Effect, Exit, HasBlockingExecutor, LibraryDefect, PreparedDeferred,
    PreparedDeferredError, PreparedDeferredWait, PreparedProviderAdmission, PreparedProviderDrain,
    ResourceValue, Scope,
};
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError, atomic::Ordering};

/// Selected SDK acquisition, cleanup, or observation failure.
#[derive(Debug)]
pub enum PreparedProviderLifecycleError {
    /// The original acquisition and partial rollback error.
    Acquire(LifecycleError),
    /// The canonical receipt of the dedicated shutdown worker.
    Close(PreparedLifecycleError),
    /// Refusal to add another observer of the already-admitted cleanup.
    Observe(MemoryError),
}

impl fmt::Display for PreparedProviderLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Acquire(error) => error.fmt(formatter),
            Self::Close(error) => error.fmt(formatter),
            Self::Observe(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PreparedProviderLifecycleError {}

type Completion = PreparedDeferred<(), PreparedLifecycleError>;
type CloseWait = PreparedDeferredWait<(), PreparedLifecycleError>;

struct CloseStart<D: ProviderWorkingSet> {
    shutdown: PreparedShutdown<D>,
    publish: Completion,
    wait: CloseWait,
    drain: PreparedProviderDrain,
}

/// One selected provider installation. All close controls are held before
/// SDK acquisition, including a dedicated first-closer observer.
pub struct PreparedProviderLifecycle<D: ProviderWorkingSet> {
    factories: ProviderFactories,
    admission: Arc<PreparedProviderAdmission>,
    completion: Completion,
    start: CloseStart<D>,
}

impl<D: ProviderWorkingSet> PreparedProviderLifecycle<D> {
    /// Regional access/drain and canonical completion controls. The independent
    /// worker, its diagnostics, and allocator overhead are provisioned separately.
    pub fn storage_bytes(additional_observers: usize) -> Result<Bytes, MemoryError> {
        PreparedProviderAdmission::storage_bytes(1)?.checked_add(Completion::storage_bytes(
            additional_observers
                .checked_add(1)
                .ok_or(MemoryError::ArithmeticOverflow)?,
        )?)
    }

    /// Prepare checked access, retain the exact selected factories, and prehold
    /// all resources needed by the first close. If these factories already
    /// specify a stricter native access capacity, retain that limit.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        factories: ProviderFactories,
        native_access: NonZeroUsize,
        additional_observers: usize,
        shutdown: PreparedShutdown<D>,
    ) -> Result<Self, MemoryError> {
        let additional = additional_observers
            .checked_add(1)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let native_access = match factories.admission {
            AdmissionCapacity::Unbounded => native_access,
            AdmissionCapacity::Bounded(selected) => native_access.min(selected),
        };
        let admission = Arc::new(PreparedProviderAdmission::prepare(
            memory,
            region,
            native_access,
            1,
        )?);
        let drain = admission.admit_drain()?;
        let completion = Completion::prepare(memory, region, additional)?;
        let publish = completion.try_clone()?;
        let wait = completion.wait()?;
        Ok(Self {
            factories,
            admission,
            completion,
            start: CloseStart {
                shutdown,
                publish,
                wait,
                drain,
            },
        })
    }
}

struct Lifecycle<D: ProviderWorkingSet> {
    providers: BorrowedProviders,
    admission: Arc<PreparedProviderAdmission>,
    policy: ShutdownPolicy,
    completion: Completion,
    start: Mutex<Option<CloseStart<D>>>,
}

/// A selected SDK owner with a canonical prepared close. Live admitted operations
/// hold the same access owner that the dedicated shutdown command drains.
pub struct PreparedOwnedProviders<D: ProviderWorkingSet>(Arc<Lifecycle<D>>);

impl<D: ProviderWorkingSet> Clone for PreparedOwnedProviders<D> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<D: ProviderWorkingSet> PreparedOwnedProviders<D> {
    /// Acquire native SDK providers after the independent close is already
    /// admitted. The existing blocking capability runs startup and partial
    /// rollback within the same accepted startup job.
    pub fn start<R>(
        prepared: PreparedProviderLifecycle<D>,
        policy: ShutdownPolicy,
    ) -> Effect<Self, PreparedProviderLifecycleError, R>
    where
        R: HasBlockingExecutor + Send + Sync + 'static,
    {
        effect_uninterruptible! {
            program: effect_try_blocking! {
                once;
                body: move |_| {
                    let mut providers = build_providers(&prepared.factories, policy)
                        .map_err(PreparedProviderLifecycleError::Acquire)?;
                    providers.prepared_admission = Some(Arc::clone(&prepared.admission));
                    Ok(Self(Arc::new(Lifecycle {
                        providers,
                        admission: prepared.admission,
                        policy,
                        completion: prepared.completion,
                        start: Mutex::new(Some(prepared.start)),
                    })))
                },
            },
        }
    }

    /// Install the same owned providers with a protected prepared close.
    pub fn acquire<R>(
        scope: Scope<PreparedProviderLifecycleError>,
        prepared: PreparedProviderLifecycle<D>,
        policy: ShutdownPolicy,
    ) -> Effect<ResourceValue<Self>, PreparedProviderLifecycleError, R>
    where
        R: HasBlockingExecutor + Send + Sync + 'static,
    {
        effect_scope_acquire! {
            scope: scope,
            acquire: crate::otel! { start_prepared; prepared: prepared, shutdown: policy, },
            release: |providers| crate::otel! { close; source: providers, },
        }
    }

    /// Share handles without transferring provider shutdown authority.
    #[must_use]
    pub fn borrowed(&self) -> BorrowedProviders {
        self.0.providers.clone()
    }

    /// Close once, finish already admitted work, and join the same outcome.
    /// SDK flush/shutdown work uses the independent preadmitted worker.
    /// End any span this caller still owns before asking its provider to drain.
    pub fn close<R>(&self) -> Effect<(), PreparedProviderLifecycleError, R>
    where
        R: Send + Sync + 'static,
    {
        let lifecycle = Arc::clone(&self.0);
        effect_uninterruptible! {
            program: effect_future! {
                exit_with_context;
                body: move |_: Arc<R>, _| {
                    let lifecycle = Arc::clone(&lifecycle);
                    async move { lifecycle.close().await }
                },
            },
        }
    }
}

impl<D: ProviderWorkingSet> Lifecycle<D> {
    async fn close(&self) -> Exit<(), PreparedProviderLifecycleError> {
        let start = self
            .start
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(start) = start {
            if let Some(open) = &self.providers.open {
                open.store(false, Ordering::Release);
            }
            self.admission.stop_now();
            let _ = start.shutdown.submit(
                self.providers.clone(),
                self.policy,
                start.drain,
                start.publish,
            );
            start.wait.await;
        } else {
            if let Some(outcome) = self.outcome() {
                return outcome;
            }
            match self.completion.wait() {
                Ok(wait) => wait.await,
                Err(error) => {
                    return self
                        .outcome()
                        .unwrap_or(Exit::Fail(PreparedProviderLifecycleError::Observe(error)));
                }
            }
        }
        self.outcome().unwrap_or_else(missing_outcome)
    }

    fn outcome(&self) -> Option<Exit<(), PreparedProviderLifecycleError>> {
        let shared = match self.completion.try_share() {
            Ok(Some(shared)) => shared,
            Ok(None) => return None,
            Err(PreparedDeferredError::Memory(error)) => {
                return Some(Exit::Fail(PreparedProviderLifecycleError::Observe(error)));
            }
            Err(PreparedDeferredError::Library(defect)) => {
                return Some(Exit::Die(Defect::from(defect)));
            }
        };
        let Some(borrow) = shared.borrow() else {
            return Some(missing_outcome());
        };
        Some(match borrow.get() {
            Some(Exit::Success(())) => Exit::Success(()),
            Some(Exit::Fail(error)) => Exit::Fail(match error.try_clone() {
                Ok(error) => PreparedProviderLifecycleError::Close(error),
                Err(error) => PreparedProviderLifecycleError::Observe(error),
            }),
            Some(Exit::Die(defect)) => Exit::Die(defect.clone()),
            Some(Exit::Interrupted) => Exit::Interrupted,
            // The dedicated worker publishes lifecycle failures as one ordered report.
            Some(Exit::FinalizationFailed(_)) | None => missing_outcome(),
        })
    }
}

fn missing_outcome() -> Exit<(), PreparedProviderLifecycleError> {
    Exit::Die(Defect::from(LibraryDefect::DeferredOutcomeConsumed))
}
