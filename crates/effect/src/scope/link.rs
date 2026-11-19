//! Private owner abstraction over the SAME Scope phase/state/close-drain engine.
//! Ordinary Scope keeps its existing Arc/Clone contract; prepared handles use
//! counted storage and explicitly fallible sharing before body execution.
use super::{DrainGuard, Inner, Phase, Scope, ScopeCloseStatus, drain, drain_prepared};
use crate::Exit;
use crate::internal::runtime::{self, Finish, Step};
use crate::memory::SharedSlot;
use std::sync::Arc;

pub(super) enum ScopeLink<E: Send + 'static> {
    System(Arc<Inner<E>>),
    Prepared(SharedSlot<'static, Inner<E>>),
}
impl<E: Send + 'static> ScopeLink<E> {
    pub(super) fn inner(&self) -> &Inner<E> {
        match self {
            Self::System(inner) => inner,
            Self::Prepared(inner) => inner,
        }
    }
    pub(super) fn settle(&self, outcome: Exit<(), E>) {
        Scope::<E>::settle_inner(self.inner(), outcome);
    }
    pub(super) fn status(&self) -> ScopeCloseStatus {
        Scope::<E>::status_inner(self.inner())
    }
    /// Claim canonical close work once. Storage adapters consume this SAME
    /// phase transition/DrainGuard; a failed claim retains the link for joining.
    pub(super) fn begin_close(self) -> Result<DrainGuard<E>, Self> {
        let work = {
            let mut state = self
                .inner()
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
            Some(work) => Ok(DrainGuard {
                scope: self,
                work,
                settled: false,
            }),
            None => Err(self),
        }
    }
    pub(super) fn join_close_prepared<A: Send + 'static>(
        self,
        primary: Exit<A, E>,
        finish: Finish<A, E>,
        buffer: crate::CleanupFailures<E>,
    ) -> Step {
        match self.begin_close() {
            Ok(guard) => drain_prepared(
                guard,
                primary,
                crate::PreparedCleanupAccumulator::from_reserved(Exit::Success(()), buffer),
                finish,
            ),
            Err(scope) => Step::suspended(Box::pin(async move {
                let _ = Scope::<E>::closed_inner(scope.inner()).await;
                runtime::finish(finish, primary)
            })),
        }
    }
    pub(super) fn join_close<A: Send + 'static>(
        self,
        primary: Exit<A, E>,
        finish: Finish<A, E>,
    ) -> Step {
        match self.begin_close() {
            Ok(guard) => drain(guard, primary, Exit::Success(()), finish),
            Err(scope) => Step::suspended(Box::pin(async move {
                let _ = Scope::<E>::closed_inner(scope.inner()).await;
                runtime::finish(finish, primary)
            })),
        }
    }
}
