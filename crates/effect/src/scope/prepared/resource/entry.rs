//! Physical root-entry promises, acquired before native acquisition.
use super::super::{Inner, Phase, PreparedScope, Registered, RegistrationError};
use crate::memory::{MemoryError, SharedSlot};

pub(in crate::scope::prepared) enum PreflightError {
    Capacity(MemoryError),
    Closed,
}
impl From<MemoryError> for PreflightError {
    fn from(error: MemoryError) -> Self {
        Self::Capacity(error)
    }
}
pub(crate) struct Entry<E: Send + 'static> {
    inner: Option<SharedSlot<'static, Inner<E>>>,
}
impl<E: Send + 'static> Entry<E> {
    pub(in crate::scope::prepared) fn reserve(
        scope: &PreparedScope<E>,
    ) -> Result<Self, PreflightError> {
        let inner = scope.inner.try_clone()?;
        let mut state = scope
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.phase != Phase::Open {
            return Err(PreflightError::Closed);
        }
        if state.finalizers.available().unwrap_or(0) <= state.reserved_entries {
            return Err(MemoryError::NoSlot.into());
        }
        state.reserved_entries += 1;
        Ok(Self { inner: Some(inner) })
    }
    pub(crate) fn matches(&self, scope: &PreparedScope<E>) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|inner| inner.same_slot(&scope.inner))
    }
    pub(crate) fn commit_prepared(self, finalizer: Registered<E>) -> Result<(), Registered<E>> {
        self.commit(finalizer).map_err(|error| match error {
            RegistrationError::Full(finalizer) | RegistrationError::Closed(finalizer) => finalizer,
        })
    }
    pub(super) fn commit(mut self, finalizer: Registered<E>) -> Result<(), RegistrationError<E>> {
        let Some(inner) = self.inner.take() else {
            return Err(RegistrationError::Closed(finalizer));
        };
        let mut state = inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.reserved_entries -= 1;
        if state.phase != Phase::Open {
            return Err(RegistrationError::Closed(finalizer));
        }
        // Same gate as normal registration: it cannot steal a promised vacancy.
        // Detaching a suffix only adds vacancies; phase changes reject ownership
        // for awaited immediate cleanup. No new post-acquisition capacity claim.
        state
            .finalizers
            .push(finalizer)
            .map_err(RegistrationError::Full)
    }
}
impl<E: Send + 'static> Drop for Entry<E> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .reserved_entries -= 1;
        }
    }
}
