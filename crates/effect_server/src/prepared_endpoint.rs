//! Opt-in endpoint descriptions before Effect/ScopedBody erasure. Demand,
//! construction, late selected-R publication and execution use the canonical
//! prepared graph, including its existing Scope/Step/drive and output owners.
use crate::Reply;
use effect::__private::prepared_graph::{
    self as graph, ConstructedGraph, ConstructedGraphPlan, Description, InputConstructor,
    PendingGraphInvocation, PendingOwnedSharedGraph, PendingSharedGraphInvocation, PreparedGraph,
    PreparedGraphPlan,
};
use effect::memory::{Buffer, BufferLease, FramePool, MemoryError, PreparedMemory, RegionId, SlotPool};
use effect::{ConstructionShape, PreparedInvocation};
use std::marker::PhantomData;
use std::sync::Arc;

/// Caller-prepared byte pools for each admitted endpoint request.
pub struct PreparedEndpointBytePools {
    body: Option<SlotPool<'static, Buffer<'static>>>,
    decoded: Option<SlotPool<'static, Buffer<'static>>>,
    output: Option<SlotPool<'static, Buffer<'static>>>,
    response_headers: Option<SlotPool<'static, Buffer<'static>>>,
}
impl PreparedEndpointBytePools {
    /// Select independent pools for encoded input, decoded byte storage and output.
    #[must_use]
    pub fn new(
        body: SlotPool<'static, Buffer<'static>>,
        decoded: SlotPool<'static, Buffer<'static>>,
        output: SlotPool<'static, Buffer<'static>>,
    ) -> Self {
        Self { body: Some(body), decoded: Some(decoded), output: Some(output), response_headers: None }
    }

    /// Select just the shared encoded body buffer for readers such as form policy.
    #[must_use]
    pub fn body_only(body: SlotPool<'static, Buffer<'static>>) -> Self {
        Self { body: Some(body), decoded: None, output: None, response_headers: None }
    }

    /// Select generated-header backing without claiming a body buffer.
    #[must_use]
    pub fn response_headers_only(headers: SlotPool<'static, Buffer<'static>>) -> Self {
        Self { body: None, decoded: None, output: None, response_headers: Some(headers) }
    }

    /// Select independent byte backing for a generated response header.
    #[must_use]
    pub fn with_response_header_buffer(mut self, headers: SlotPool<'static, Buffer<'static>>) -> Self {
        self.response_headers = Some(headers);
        self
    }

    pub(crate) fn reserve(&self, shared: Option<&Self>) -> Result<EndpointByteLeases, MemoryError> {
        Ok(EndpointByteLeases {
            body: reserve_endpoint_bytes(self.body.as_ref(), shared.and_then(|pools| pools.body.as_ref()))?,
            decoded: reserve_endpoint_bytes(self.decoded.as_ref(), shared.and_then(|pools| pools.decoded.as_ref()))?,
            output: reserve_endpoint_bytes(self.output.as_ref(), shared.and_then(|pools| pools.output.as_ref()))?,
            response_headers: reserve_endpoint_bytes(self.response_headers.as_ref(), shared.and_then(|pools| pools.response_headers.as_ref()))?,
        })
    }
}

fn reserve_endpoint_bytes(
    selected: Option<&SlotPool<'static, Buffer<'static>>>,
    shared: Option<&SlotPool<'static, Buffer<'static>>>,
) -> Result<Option<AdmittedByteLease>, MemoryError> {
    let Some(selected) = selected else { return Ok(None) };
    if shared.is_some_and(|shared| selected.same_pool(shared)) {
        return Ok(None);
    }
    let mut lease = selected.try_checkout()?;
    lease.clear();
    Ok(Some(AdmittedByteLease(Some(lease))))
}

pub(crate) struct AdmittedByteLease(pub(crate) Option<BufferLease<'static>>);

pub(crate) struct EndpointByteLeases {
    pub(crate) body: Option<AdmittedByteLease>,
    pub(crate) decoded: Option<AdmittedByteLease>,
    pub(crate) output: Option<AdmittedByteLease>,
    pub(crate) response_headers: Option<AdmittedByteLease>,
}

/// A reusable typed endpoint recipe, not an ordinary ScopedBody in a native root.
pub struct PreparedEndpointPlan<F, D: Description> {
    plan: PreparedGraphPlan<F, D>,
    on_capacity: Option<crate::StaticOverloadReply>,
    byte_pools: Option<Arc<PreparedEndpointBytePools>>,
}
/// A physically prepared concrete endpoint. No business A/E erasure occurs.
pub struct PreparedEndpoint<F, D: Description> {
    graph: PreparedGraph<F, D>,
    pub(crate) on_capacity: Option<crate::StaticOverloadReply>,
    pub(crate) byte_pools: Option<Arc<PreparedEndpointBytePools>>,
}
/// Prepared endpoint recipe that consumes one owned, late-published route input.
pub struct PreparedInputEndpointPlan<F, I, D: Description> {
    plan: PreparedGraphPlan<graph::InputRecipe<F, I>, D>,
    on_capacity: Option<crate::StaticOverloadReply>,
    byte_pools: Option<Arc<PreparedEndpointBytePools>>,
}
/// Finite prepared endpoint for a typed route input.
pub struct PreparedInputEndpoint<F, I, D: Description> {
    graph: PreparedGraph<graph::InputRecipe<F, I>, D>,
    frames: FramePool<'static>,
    pub(crate) on_capacity: Option<crate::StaticOverloadReply>,
    pub(crate) byte_pools: Option<Arc<PreparedEndpointBytePools>>,
}
type CaptureDecoder<I> = dyn Fn(&crate::Captures) -> Result<I, crate::ServerError> + Send + Sync;
struct CaptureRecord<
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
> {
    pending: graph::PendingOwnedInputSharedGraph<I, A, E, R>,
    decoder: Arc<CaptureDecoder<I>>,
}
type CaptureResult<A, E, R> = (
    crate::Captures,
    Result<PendingOwnedSharedGraph<A, E, R>, crate::ServerError>,
);
fn finish_capture<
    I: Send + 'static,
    A: Send + 'static,
    E: Send + 'static,
    R: Send + Sync + 'static,
>(
    record: CaptureRecord<I, A, E, R>,
    captures: crate::Captures,
) -> CaptureResult<A, E, R> {
    let result = (record.decoder)(&captures).map(|input| record.pending.publish_input(input));
    (captures, result)
}
pub(crate) type CaptureOnce<E, R> = effect::__private::ReservedOnce<
    crate::Captures,
    (
        crate::Captures,
        Result<PendingOwnedSharedGraph<Reply<E, R>, E, R>, crate::ServerError>,
    ),
>;
impl<F, I, D> PreparedInputEndpointPlan<F, I, D>
where
    D: Description,
    I: Send + 'static,
    F: Fn(I) -> D + Send + Sync + 'static,
{
    #[doc(hidden)]
    pub fn from_graph<E, R>(plan: PreparedGraphPlan<graph::InputRecipe<F, I>, D>) -> Self
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        D: Description<Value = Reply<E, R>, Error = E, Env = R>,
    {
        Self {
            plan: plan.with_shared_environment(),
            on_capacity: None,
            byte_pools: None,
        }
    }
    /// Select the early capacity-refusal response when this endpoint is mounted.
    #[must_use]
    pub fn on_capacity(mut self, reply: crate::StaticOverloadReply) -> Self {
        self.on_capacity = Some(reply);
        self
    }
    /// Select physical byte buffers reserved before typed input or endpoint work.
    #[must_use]
    pub fn with_byte_pools(mut self, pools: Arc<PreparedEndpointBytePools>) -> Self {
        self.byte_pools = Some(pools);
        self
    }
    #[must_use]
    pub fn with_cleanup(mut self, cleanup: effect::PreparedCleanupConfig<D::Error>) -> Self {
        self.plan = self.plan.with_cleanup(cleanup);
        self
    }
    /// Supply the explicit startup-prepared panic diagnostic pool.
    #[must_use]
    pub fn with_panic_diagnostics(mut self, pool: effect::PreparedPanicPool) -> Self {
        self.plan = self.plan.with_panic_diagnostics(pool);
        self
    }
    pub fn demand(&self, capacity: usize) -> Result<graph::Demand, MemoryError> {
        let layout = effect::__private::ReservedOnce::<
            crate::Captures,
            CaptureResult<D::Value, D::Error, D::Env>,
        >::layout::<CaptureRecord<I, D::Value, D::Error, D::Env>>();
        self.plan
            .demand(capacity)?
            .with_reserved_callback_layout(layout)
    }
    pub fn prepare(
        self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: FramePool<'static>,
    ) -> Result<PreparedInputEndpoint<F, I, D>, MemoryError> {
        let demand = self.demand(capacity)?;
        let required = demand
            .frames_per_run
            .checked_mul(capacity)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        if frames.layout().align() < demand.frame_layout.align() {
            return Err(MemoryError::UnsupportedAlignment {
                requested: demand.frame_layout.align(),
                supported: frames.layout().align(),
            });
        }
        if frames.layout().size() < demand.frame_layout.size() {
            return Err(MemoryError::ValueTooLarge {
                requested: effect::memory::Bytes::new(demand.frame_layout.size()),
                capacity: effect::memory::Bytes::new(frames.layout().size()),
            });
        }
        if frames.capacity() < required {
            return Err(MemoryError::NoSlot);
        }
        let handoff_frames = frames.try_clone()?;
        let graph = effect::effect_prepare! {prepare;plan:self.plan,memory:memory,region:region,capacity:capacity,frames:frames,}?;
        Ok(PreparedInputEndpoint {
            graph,
            frames: handoff_frames,
            on_capacity: self.on_capacity,
            byte_pools: self.byte_pools,
        })
    }
}
impl<F, I, D> PreparedInputEndpoint<F, I, D>
where
    D: Description,
    I: Send + 'static,
    F: Fn(I) -> D + Send + Sync + 'static,
{
    #[allow(clippy::type_complexity)]
    pub(crate) fn admit_input(
        &self,
        decoder: Arc<CaptureDecoder<I>>,
    ) -> Result<
        effect::__private::ReservedOnce<
            crate::Captures,
            (
                crate::Captures,
                Result<PendingOwnedSharedGraph<D::Value, D::Error, D::Env>, crate::ServerError>,
            ),
        >,
        MemoryError,
    > {
        let frame =
            effect::__private::ReservedOnce::<
                crate::Captures,
                CaptureResult<D::Value, D::Error, D::Env>,
            >::reserve::<CaptureRecord<I, D::Value, D::Error, D::Env>>(&self.frames)?;
        let pending = self.graph.admit_input_shared()?;
        let origins = pending.reserve_output_origins()?;
        let pending = pending
            .into_owned(origins)
            .map_err(|_| MemoryError::UnsupportedRunContract)?;
        Ok(frame.initialize(
            CaptureRecord { pending, decoder },
            finish_capture::<I, D::Value, D::Error, D::Env>,
        ))
    }
}
/// A whole endpoint admitted before selecting/constructing request R. No runnable
/// invocation is exposed until the actual value has been published.
pub struct PreparedEndpointAdmission<E: Send + 'static, R: Send + Sync + 'static> {
    pending: PendingGraphInvocation<Reply<E, R>, E, R>,
}
/// Admitted endpoint awaiting an externally owned shared environment.
pub struct PreparedSharedEndpointAdmission<E: Send + 'static, R: Send + Sync + 'static> {
    pending: PendingSharedGraphInvocation<Reply<E, R>, E, R>,
}
pub(crate) struct PreparedOwnedEndpointAdmission<E: Send + 'static, R: Send + Sync + 'static> {
    pub(crate) pending: PendingOwnedSharedGraph<Reply<E, R>, E, R>,
}
impl<E: Send + 'static, R: Send + Sync + 'static> PreparedOwnedEndpointAdmission<E, R> {
    pub(crate) fn publish_shared(
        self,
        environment: Arc<R>,
    ) -> effect::PreparedOwnedDriver<Reply<E, R>, E> {
        self.pending.publish_shared(environment)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> PreparedSharedEndpointAdmission<E, R> {
    pub(crate) fn reserve_output_origins(
        &self,
    ) -> Result<effect::ReservedOutputOrigins<Reply<E, R>, E>, MemoryError> {
        self.pending.reserve_output_origins()
    }
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub(crate) fn into_owned(
        self,
        origins: effect::ReservedOutputOrigins<Reply<E, R>, E>,
    ) -> Result<
        PreparedOwnedEndpointAdmission<E, R>,
        (Self, effect::ReservedOutputOrigins<Reply<E, R>, E>),
    > {
        match self.pending.into_owned(origins) {
            Ok(pending) => Ok(PreparedOwnedEndpointAdmission { pending }),
            Err((pending, origins)) => Err((Self { pending }, origins)),
        }
    }
    pub fn publish_shared(self, environment: Arc<R>) -> PreparedInvocation<Reply<E, R>, E> {
        self.pending.publish_shared(environment)
    }
}
impl<E: Send + 'static, R: Send + Sync + 'static> PreparedEndpointAdmission<E, R> {
    pub fn publish(self, environment: R) -> PreparedInvocation<Reply<E, R>, E> {
        self.pending.publish(environment)
    }
    /// Transfer the actual admitted body to a finite construction site for the
    /// canonical late-environment import. This is not an ordinary Effect value.
    #[must_use]
    pub fn into_pending(self) -> PendingGraphInvocation<Reply<E, R>, E, R> {
        self.pending
    }
}
impl<F, D> PreparedEndpointPlan<F, D>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
{
    /// Expansion support: the supplied graph is compiled from the SAME body
    /// grammar before erasure. Its actual owned R cell contributes to demand.
    #[doc(hidden)]
    pub fn from_graph<E, R>(plan: PreparedGraphPlan<F, D>) -> Self
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        D: Description<Value = Reply<E, R>, Error = E, Env = R>,
    {
        Self {
            plan: plan.with_owned_environment(),
            on_capacity: None,
            byte_pools: None,
        }
    }
    /// Select the early capacity-refusal response when this endpoint is mounted.
    #[must_use]
    pub fn on_capacity(mut self, reply: crate::StaticOverloadReply) -> Self {
        self.on_capacity = Some(reply);
        self
    }
    /// Select physical byte buffers reserved before endpoint execution.
    #[must_use]
    pub fn with_byte_pools(mut self, pools: Arc<PreparedEndpointBytePools>) -> Self {
        self.byte_pools = Some(pools);
        self
    }
    /// Supply explicit, independently prepared cleanup backing for this graph.
    #[must_use]
    pub fn with_cleanup(mut self, cleanup: effect::PreparedCleanupConfig<D::Error>) -> Self {
        self.plan = self.plan.with_cleanup(cleanup);
        self
    }
    /// Supply the explicit startup-prepared panic diagnostic pool.
    #[must_use]
    pub fn with_panic_diagnostics(mut self, pool: effect::PreparedPanicPool) -> Self {
        self.plan = self.plan.with_panic_diagnostics(pool);
        self
    }
    /// Select late shared-environment publication before demand and preparation.
    #[must_use]
    pub fn with_shared_environment(mut self) -> Self {
        self.plan = self.plan.with_shared_environment();
        self
    }
    pub fn demand(&self, capacity: usize) -> Result<graph::Demand, MemoryError> {
        self.plan.demand(capacity)
    }
    pub fn prepare(
        self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: FramePool<'static>,
    ) -> Result<PreparedEndpoint<F, D>, MemoryError> {
        let graph = effect::effect_prepare! {prepare;plan:self.plan,memory:memory,region:region,capacity:capacity,frames:frames,}?;
        Ok(PreparedEndpoint {
            graph,
            on_capacity: self.on_capacity,
            byte_pools: self.byte_pools,
        })
    }
    /// Actual available input, not unpublished/projected R, supplies finite
    /// construction-site dependencies before auth/routing/body work.
    pub fn with_construction<P, I, G>(
        self,
        shape: P,
        constructor: G,
    ) -> ConstructedEndpointPlan<F, D, P, I, G>
    where
        P: ConstructionShape,
        I: 'static,
        G: Fn(&I, P::Vacancies<D::Value, D::Error>) -> Result<(), MemoryError>
            + Send
            + Sync
            + 'static,
    {
        ConstructedEndpointPlan {
            plan: self.plan.with_inputs_construction(shape, constructor),
            marker: PhantomData,
        }
    }
}
impl<F, D> PreparedEndpoint<F, D>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
{
    pub fn admit<E, R>(&self) -> Result<PreparedEndpointAdmission<E, R>, MemoryError>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        D: Description<Value = Reply<E, R>, Error = E, Env = R>,
    {
        Ok(PreparedEndpointAdmission {
            pending: self.graph.admit_unpublished()?,
        })
    }
    pub fn admit_unpublished_shared<E, R>(
        &self,
    ) -> Result<PreparedSharedEndpointAdmission<E, R>, MemoryError>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        D: Description<Value = Reply<E, R>, Error = E, Env = R>,
    {
        Ok(PreparedSharedEndpointAdmission {
            pending: self.graph.admit_unpublished_shared()?,
        })
    }
    pub fn close(&self) {
        self.graph.close();
    }
    pub fn active(&self) -> usize {
        self.graph.active()
    }
}
/// Typed native constructor recipe for a compiled endpoint's exact target sites.
pub struct ConstructedEndpointPlan<F, D: Description, P, I, G> {
    plan: ConstructedGraphPlan<F, D, P, InputConstructor<I, G>>,
    marker: PhantomData<fn() -> I>,
}
/// Prepared endpoint with real pre-business construction authority.
pub struct ConstructedEndpoint<F, D: Description, P, I: 'static, G: Send + Sync + 'static> {
    graph: ConstructedGraph<F, D, P, InputConstructor<I, G>>,
    marker: PhantomData<fn() -> I>,
}
impl<F, D, P, I, G> ConstructedEndpointPlan<F, D, P, I, G>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
    P: ConstructionShape,
    I: 'static,
    G: Fn(&I, P::Vacancies<D::Value, D::Error>) -> Result<(), MemoryError> + Send + Sync + 'static,
{
    pub fn demand(&self, capacity: usize) -> Result<graph::Demand, MemoryError> {
        self.plan.demand(capacity)
    }
    pub fn prepare(
        self,
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
        frames: FramePool<'static>,
    ) -> Result<ConstructedEndpoint<F, D, P, I, G>, MemoryError> {
        let graph = effect::effect_prepare! {prepare;plan:self.plan,memory:memory,region:region,capacity:capacity,frames:frames,}?;
        Ok(ConstructedEndpoint {
            graph,
            marker: PhantomData,
        })
    }
}
impl<F, D, P, I, G> ConstructedEndpoint<F, D, P, I, G>
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
    P: ConstructionShape,
    I: 'static,
    G: Fn(&I, P::Vacancies<D::Value, D::Error>) -> Result<(), MemoryError> + Send + Sync + 'static,
{
    pub fn admit<E, R>(&self, inputs: &I) -> Result<PreparedEndpointAdmission<E, R>, MemoryError>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        D: Description<Value = Reply<E, R>, Error = E, Env = R>,
    {
        Ok(PreparedEndpointAdmission {
            pending: self.graph.admit_unpublished(inputs)?,
        })
    }
    pub fn admit_unpublished_shared<E, R>(
        &self,
        inputs: &I,
    ) -> Result<PreparedSharedEndpointAdmission<E, R>, MemoryError>
    where
        E: Send + 'static,
        R: Send + Sync + 'static,
        D: Description<Value = Reply<E, R>, Error = E, Env = R>,
    {
        Ok(PreparedSharedEndpointAdmission {
            pending: self.graph.admit_unpublished_shared(inputs)?,
        })
    }
    pub fn close(&self) {
        self.graph.close();
    }
    pub fn active(&self) -> usize {
        self.graph.active()
    }
}

/// Type-erased startup endpoint admission; no endpoint factory runs per request.
pub(crate) trait SharedEndpointAdmitter<E: Send + 'static, R: Send + Sync + 'static>:
    Send + Sync
{
    fn admit_shared(&self) -> Result<PreparedSharedEndpointAdmission<E, R>, MemoryError>;
}
impl<F, D, E, R> SharedEndpointAdmitter<E, R> for PreparedEndpoint<F, D>
where
    E: Send + 'static,
    R: Send + Sync + 'static,
    D: Description<Value = Reply<E, R>, Error = E, Env = R>,
    F: Fn() -> D + Send + Sync + 'static,
    PreparedEndpoint<F, D>: Send + Sync,
{
    fn admit_shared(&self) -> Result<PreparedSharedEndpointAdmission<E, R>, MemoryError> {
        self.admit_unpublished_shared()
    }
}
