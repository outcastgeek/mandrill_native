//! Private native-invocation outcomes preserved across Identity's narrow errors.
//!
//! A selected component factory receives this invocation's store clone. It must
//! retain the application's validators/crypto/policies while replacing only that
//! store handle. No global context, nested interpreter or synchronous I/O is used.

use super::SqlxIdentityStore;
use crate::{effect_future, effect_uninterruptible};
use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool};
use effect::{Effect, Exit};
use effect_sql::SqlError;
use std::{
    future::Future,
    sync::{Arc, Mutex},
};

/// One private carrier owned by exactly one native workflow invocation.
pub(crate) struct InvocationDiagnostics {
    /// The first complete failure already includes its ordered SQL cleanup tree.
    failure: Mutex<Option<SqlError>>,
}
impl InvocationDiagnostics {
    /// Retain a failure before the native trait returns only StoreError.
    pub(crate) fn retain(&self, error: SqlError) {
        let mut retained = self
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if retained.is_none() {
            *retained = Some(error);
        }
    }
    /// Move the complete original outcome into its outer native owner.
    pub(crate) fn take(&self) -> Option<SqlError> {
        self.failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}
/// Startup-provisioned diagnostic carriers for selected native Identity workflows.
pub struct PreparedIdentityInvocations {
    cells: SlotPool<'static, Option<InvocationDiagnostics>>,
}
impl PreparedIdentityInvocations {
    /// Region space for complete carriers, with allocator headers budgeted by the owner.
    pub fn storage_bytes(capacity: usize) -> Result<Bytes, MemoryError> {
        SlotPool::<Option<InvocationDiagnostics>>::storage_bytes(capacity)
    }

    /// Prepare one reusable carrier per concurrently live invocation.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            cells: memory.slots(region, capacity, |_| Ok(None))?,
        })
    }

    fn reserve(&self) -> Result<InvocationOwner, MemoryError> {
        let mut cell = self.cells.try_checkout()?;
        *cell = Some(InvocationDiagnostics {
            failure: Mutex::new(None),
        });
        Ok(InvocationOwner::Prepared(Some(cell.share())))
    }
}

pub(crate) enum InvocationOwner {
    Ordinary(Arc<InvocationDiagnostics>),
    Prepared(Option<SharedSlot<'static, Option<InvocationDiagnostics>>>),
}
impl Clone for InvocationOwner {
    #[expect(
        clippy::expect_used,
        reason = "an existing carrier only fails to clone if its reference count is unrepresentable"
    )]
    fn clone(&self) -> Self {
        match self {
            Self::Ordinary(cell) => Self::Ordinary(cell.clone()),
            Self::Prepared(cell) => Self::Prepared(cell.as_ref().map(|cell| {
                cell.try_clone()
                    .expect("identity invocation reference count")
            })),
        }
    }
}
impl InvocationOwner {
    fn cell(&self) -> Option<&InvocationDiagnostics> {
        match self {
            Self::Ordinary(cell) => Some(cell),
            Self::Prepared(cell) => cell.as_ref().and_then(|cell| (**cell).as_ref()),
        }
    }

    pub(crate) fn retain(&self, error: SqlError) {
        if let Some(cell) = self.cell() {
            cell.retain(error);
        }
    }

    pub(crate) fn take(&self) -> Option<SqlError> {
        self.cell().and_then(InvocationDiagnostics::take)
    }
}
impl Drop for InvocationOwner {
    fn drop(&mut self) {
        if let Self::Prepared(cell) = self
            && let Some(cell) = cell.take()
            && let Some(mut cell) = cell.into_last()
        {
            drop(cell.take());
        }
    }
}

/// Native application rejection or restored original SQL failure.
/// Defects/interruption/finalization remain Exit channels, not expected errors.
#[derive(Debug)]
pub enum InvocationError<E> {
    /// Native manager/application failure without a narrowed SQL outcome.
    Native(E),
    /// Original provider/commit/configuration failure with retained diagnostic source.
    Storage(SqlError),
}
impl<E: std::fmt::Display> std::fmt::Display for InvocationError<E> {
    /// Preserve safe source displays without adding credential-bearing context.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(error) => std::fmt::Display::fmt(error, f),
            Self::Storage(error) => std::fmt::Display::fmt(error, f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for InvocationError<E> {
    /// Expose diagnostic sources only through the explicit error interface.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Native(error) => Some(error),
            Self::Storage(error) => Some(error),
        }
    }
}
/// Owned one-invocation store replacement and full-outcome restoration handle.
/// This is not Clone: copying a store handle does not create another invocation.
pub struct StoreInvocation {
    /// Scoped replacement handle sharing the installed pool and policy.
    store: SqlxIdentityStore,
    /// Private carrier survives through native manager settlement.
    diagnostics: InvocationOwner,
}
impl StoreInvocation {
    /// Establish a fresh private carrier for this original store configuration.
    pub(crate) fn new(backend: &SqlxIdentityStore) -> Self {
        Self::with_diagnostics(
            backend,
            InvocationOwner::Ordinary(Arc::new(InvocationDiagnostics {
                failure: Mutex::new(None),
            })),
        )
    }

    pub(crate) fn new_prepared(
        backend: &SqlxIdentityStore,
        pool: &PreparedIdentityInvocations,
    ) -> Result<Self, MemoryError> {
        Ok(Self::with_diagnostics(backend, pool.reserve()?))
    }

    fn with_diagnostics(backend: &SqlxIdentityStore, diagnostics: InvocationOwner) -> Self {
        let mut store = backend.shared_backend();
        store.invocation = Some(diagnostics.clone());
        Self { store, diagnostics }
    }

    pub(super) fn with_security(mut self, security: super::schema::SecurityScope) -> Self {
        self.store.security_scope = security;
        self
    }
    /// Supply the scoped store to an explicitly selected native component factory.
    /// Sharing its service handle copies no account, password or request payload.
    #[must_use]
    pub fn store(&self) -> SqlxIdentityStore {
        self.store.shared_backend()
    }
    /// Consume the invocation after native work has settled and restore its first
    /// complete failure even if a narrower native component suppressed its category.
    pub fn finish<A, E>(self, result: Result<A, E>) -> Exit<A, InvocationError<E>> {
        if let Some(error) = self.diagnostics.take() {
            error.into_exit().map_error(InvocationError::Storage)
        } else {
            match result {
                Ok(value) => Exit::Success(value),
                Err(error) => Exit::Fail(InvocationError::Native(error)),
            }
        }
    }
}
/// Lift a whole native workflow lazily using its per-invocation replacement store.
/// The callback creates fresh request inputs and selected components for each run.
/// SQL settles every operation-owned transaction before native results are restored.
pub fn invoke<A, E, R, F>(
    select: impl Fn(&R) -> SqlxIdentityStore + Send + Sync + 'static,
    operation: impl Fn(Arc<R>, SqlxIdentityStore) -> F + Send + Sync + 'static,
) -> Effect<A, InvocationError<E>, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
    F: Future<Output = Result<A, E>> + Send + 'static,
{
    let operation = Arc::new(operation);
    effect_uninterruptible! {
        program: effect_future! {
        exit_with_context; body: move |env: Arc<R>, _| {
        let invocation = select(&env).invocation();
        let operation = operation.clone();
        async move {
            let result = operation(env, invocation.store()).await;
            invocation.finish(result)
        }
    }, }, }
}

#[cfg(test)]
mod contracts {
    use super::InvocationDiagnostics;
    use effect_sql::SqlError;
    use std::sync::Mutex;

    /// A supplied uncertain native acknowledgment remains the first complete
    /// outcome across narrower categories and later cleanup/provider failures.
    #[test]
    fn uncertainty_is_retained_once_without_replay_or_error_replacement() {
        let diagnostics = InvocationDiagnostics {
            failure: Mutex::new(None),
        };
        diagnostics.retain(SqlError::CommitOutcomeUnknown(sqlx::Error::PoolClosed));
        diagnostics.retain(SqlError::Configuration("later native narrowing".into()));
        assert!(matches!(
            diagnostics.take(),
            Some(SqlError::CommitOutcomeUnknown(sqlx::Error::PoolClosed))
        ));
        assert!(diagnostics.take().is_none());
    }
}
