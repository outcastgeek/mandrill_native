//! Stable typed result with region-framed heterogeneous pending payloads. Used
//! by generated graph descriptors; scheduling remains PreparedProvider's SAME
//! physical queue/worker engine. No Box future or ordinary spawn fallback.
use super::*;
use crate::{Exit, memory::RuntimeFrame};
use std::marker::PhantomData;

pub struct PreparedTaskFuture<A: Send + 'static, E: Send + 'static> {
    frame: RuntimeFrame,
    poll: unsafe fn(&mut RuntimeFrame, &mut Context<'_>) -> Poll<Exit<A, E>>,
    marker: PhantomData<fn() -> (A, E)>,
}
impl<A: Send + 'static, E: Send + 'static> PreparedTaskFuture<A, E> {
    pub(crate) fn place<P: Future<Output = Exit<A, E>> + Send + 'static>(
        reservation: FrameReservation<'static, P>,
        future: P,
    ) -> Self {
        unsafe fn poll<P: Future>(
            frame: &mut RuntimeFrame,
            cx: &mut Context<'_>,
        ) -> Poll<P::Output> {
            // SAFETY: exact constructor pairing; frame is stable before any
            // poll. No unpinned projection or extraction is exposed.
            unsafe { Pin::new_unchecked(frame.pointer().cast::<P>().as_mut()).poll(cx) }
        }
        Self {
            frame: reservation.erase(future),
            poll: poll::<P>,
            marker: PhantomData,
        }
    }
}
impl<A: Send + 'static, E: Send + 'static> Future for PreparedTaskFuture<A, E> {
    type Output = Exit<A, E>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Only the separately stored payload is pinned. Moving this handle
        // never moves it; private shim exclusively projects that stable payload.
        let poll = self.poll;
        unsafe { poll(&mut self.frame, cx) }
    }
}
pub enum TaskProvider {}
pub enum CpuProvider {}
pub enum BlockingProvider {}
/// Explicit R-selected prepared provider and actual transitive/diagnostic
/// preflight. K distinguishes task/CPU/blocking domains without installing a
/// runtime. Implement for each concrete A/E used by the application's graph.
/// Native allocation, worker entry and diagnostic sink contracts remain those
/// of PreparedProvider/ProviderBudget, not pointer-sized envelope inference.
pub trait HasPreparedProvider<A: Send + 'static, E: Send + 'static, K>:
    Send + Sync + 'static
{
    type WorkingSet: ProviderWorkingSet;
    const CONSTRUCTION_KEY: Option<u64> = None;
    fn prepared_provider(
        &self,
    ) -> &PreparedSubmitter<PreparedTaskFuture<A, E>, Exit<A, E>, Self::WorkingSet>;
    fn reserve_provider_budget(&self) -> Result<ProviderBudget<Self::WorkingSet>, MemoryError>;
}

/// A real unpublished submission plus its actual working-set/event ownership.
/// Construct only in an explicit native pre-business hook; no projected R value
/// or global provider lookup is required to acquire this selected target.
pub struct PreparedProviderConstruction<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet>
{
    pub(crate) submission: PreparedSubmission<PreparedTaskFuture<A, E>, Exit<A, E>, D>,
    pub(crate) diagnostics: ProviderDiagnostics,
    pub(crate) retention: PreparedProviderOutputLease<PreparedTaskFuture<A, E>, Exit<A, E>, D>,
}
impl<A: Send + 'static, E: Send + 'static, D: ProviderWorkingSet>
    PreparedProviderConstruction<A, E, D>
{
    pub fn reserve<X>(
        provider: &PreparedSubmitter<PreparedTaskFuture<A, E>, Exit<A, E>, D>,
        budget: impl FnOnce() -> Result<ProviderBudget<D>, X>,
    ) -> Result<Self, PreparedSubmitError<X>> {
        let submission = provider.reserve().map_err(PreparedSubmitError::Refused)?;
        let retention = submission
            .retain_graph_origin_preflight()
            .map_err(|error| PreparedSubmitError::Refused(error.into()))?;
        let budget = budget().map_err(PreparedSubmitError::WorkingSet)?;
        let diagnostics = submission.install_graph_budget(budget);
        Ok(Self {
            submission,
            diagnostics,
            retention,
        })
    }
}
/// Explicit type-level construction site selector. This contains no provider
/// pointer/global: the invocation registry must supply the actual typed owner.
pub struct ConstructionProvider<K, const KEY: u64>(PhantomData<fn() -> K>);
impl<A, E, R, K, const KEY: u64> HasPreparedProvider<A, E, ConstructionProvider<K, KEY>> for R
where
    A: Send + 'static,
    E: Send + 'static,
    R: HasPreparedProvider<A, E, K>,
    K: 'static,
{
    type WorkingSet = <R as HasPreparedProvider<A, E, K>>::WorkingSet;
    fn prepared_provider(
        &self,
    ) -> &PreparedSubmitter<PreparedTaskFuture<A, E>, Exit<A, E>, Self::WorkingSet> {
        <R as HasPreparedProvider<A, E, K>>::prepared_provider(self)
    }
    fn reserve_provider_budget(&self) -> Result<ProviderBudget<Self::WorkingSet>, MemoryError> {
        <R as HasPreparedProvider<A, E, K>>::reserve_provider_budget(self)
    }
    const CONSTRUCTION_KEY: Option<u64> = Some(KEY);
}
