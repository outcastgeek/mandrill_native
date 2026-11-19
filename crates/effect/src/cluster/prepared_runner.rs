//! Selected native delivery through the existing Cluster fencing contract.

use super::{
    ClaimOptions, ClaimToken, ClusterAddress, ClusterOwner, ClusterRunOutcome, ClusterRunner,
    ClusterStoreError, MemoryClusterMessageStore, PreparedClusterClaim,
    PreparedClusterPayloadReservation, PreparedClusterStoreError, ReplyKind,
};
use crate::internal::runtime::prepared_graph::FutureContext;
use crate::memory::{
    Bytes, FixedVector, FramePool, MemoryError, PreparedMemory, RegionId, RetainedValue,
    RetainedValueReservation, SharedSlot, SlotPool,
};
use crate::rpc::{PreparedRpcEncoder, PreparedRpcHeader, PreparedRpcWireError};
use crate::{Exit, JsonWriter, SchemaIssue, UtcMillis};
use futures_util::future::{Either, select};
use std::alloc::Layout;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::pin;
use std::sync::Mutex;

type ActiveRuns = Mutex<FixedVector<'static, Option<ClusterAddress>>>;

/// Actual regional pending-delivery records and a per-entity concurrency bound.
pub struct PreparedClusterRunnerMemory {
    active: SharedSlot<'static, ActiveRuns>,
    diagnostics: FramePool<'static>,
    per_entity: NonZeroUsize,
}

impl PreparedClusterRunnerMemory {
    /// Controls and retained address records, excluding caller-owned identity
    /// bytes and the partition allocator's headers and alignment.
    pub fn storage_bytes(maximum_pending: NonZeroUsize) -> Result<Bytes, MemoryError> {
        let records = Layout::array::<Option<ClusterAddress>>(maximum_pending.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<ActiveRuns>::storage_bytes(1)?
            .checked_add(Bytes::new(records.size()))?
            .checked_add(FramePool::storage_bytes(
                maximum_pending.get(),
                RetainedValueReservation::<ClaimToken, ()>::layout(),
            )?)
    }

    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        maximum_pending: NonZeroUsize,
        per_entity: NonZeroUsize,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(maximum_pending)?;
        let storage = memory.slots(region, 1, |_| {
            let mut active =
                FixedVector::prepare(memory.allocator(region)?, maximum_pending.get())?;
            for _ in 0..maximum_pending.get() {
                active.push_reserved(None);
            }
            Ok(Mutex::new(active))
        })?;
        Ok(Self {
            active: storage.try_checkout()?.share(),
            diagnostics: memory.frames(
                region,
                maximum_pending.get(),
                RetainedValueReservation::<ClaimToken, ()>::layout(),
            )?,
            per_entity,
        })
    }

    fn reserve(&self, address: &ClusterAddress) -> Result<ActiveRun, MemoryError> {
        let active = self.active.try_clone()?;
        let index = {
            let mut records = active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if records
                .as_slice()
                .iter()
                .flatten()
                .filter(|current| *current == address)
                .count()
                >= self.per_entity.get()
            {
                return Err(MemoryError::NoSlot);
            }
            let (index, slot) = records
                .as_mut_slice()
                .iter_mut()
                .enumerate()
                .find(|(_, slot)| slot.is_none())
                .ok_or(MemoryError::NoSlot)?;
            *slot = Some(address.clone());
            index
        };
        Ok(ActiveRun { active, index })
    }
}

struct ActiveRun {
    active: SharedSlot<'static, ActiveRuns>,
    index: usize,
}

impl Drop for ActiveRun {
    fn drop(&mut self) {
        let address = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut_slice()
            .get_mut(self.index)
            .and_then(Option::take);
        drop(address);
    }
}

/// The delivery claim and its preadmitted reply writer. A native executor
/// keeps this owner until its work settles, including any external cancellation
/// handoff, and returns the same delivery when a reply is ready to record.
pub struct PreparedClusterDelivery {
    claim: PreparedClusterClaim,
    reply: PreparedClusterPayloadReservation,
    _active: ActiveRun,
}

impl PreparedClusterDelivery {
    #[must_use]
    pub fn token(&self) -> &ClaimToken {
        &self.claim.token
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        self.claim.payload.as_ref()
    }

    #[must_use]
    pub fn reply_capacity(&self) -> Bytes {
        self.reply.capacity()
    }

    /// Append within the preadmitted reply size.
    pub fn append_reply(&mut self, bytes: &[u8]) -> Result<(), MemoryError> {
        self.reply.extend_from_slice(bytes)
    }

    /// Write a prepared RPC envelope directly to this delivery's reply slot.
    pub fn encode_reply<'a>(
        &mut self,
        encoder: PreparedRpcEncoder,
        header: PreparedRpcHeader<'_>,
        metadata: impl IntoIterator<Item = (&'a str, &'a str)>,
        payload: impl FnOnce(&mut JsonWriter) -> Result<(), SchemaIssue>,
    ) -> Result<(), PreparedRpcWireError> {
        encoder.encode(header, metadata, self.reply.buffer_mut(), payload)
    }
}

/// Selects a native executor and the actual demand needed before claiming work.
pub trait PreparedClusterExecutor: Send + Sync {
    type Reservation: Send;
    type Error: Send + 'static;

    fn try_reserve(
        &self,
        owner: &ClusterOwner,
        reply_capacity: Bytes,
    ) -> Result<Self::Reservation, MemoryError>;

    fn execute(
        &self,
        reservation: Self::Reservation,
        delivery: PreparedClusterDelivery,
    ) -> impl Future<Output = Result<(PreparedClusterDelivery, ReplyKind), Self::Error>> + Send;
}

/// One fenced claim request. Multiple callers can submit independently using
/// the same pending records and per-entity bound.
pub struct PreparedClusterRunRequest {
    pub owner: ClusterOwner,
    pub now: UtcMillis,
    pub lease_until: UtcMillis,
}

#[derive(Debug)]
pub enum PreparedClusterRunnerError<E, C> {
    /// The prepared operation refused before claiming work.
    Memory(MemoryError),
    UncertainClaim(ClusterStoreError),
    Execution {
        token: PreparedClusterClaimToken,
        source: E,
    },
    UncertainCompletionClock {
        token: PreparedClusterClaimToken,
        source: C,
    },
    UncertainCompletion {
        token: PreparedClusterClaimToken,
        source: PreparedClusterStoreError,
    },
}

/// An error's original fencing token, retained in a preadmitted region frame.
pub struct PreparedClusterClaimToken(RetainedValue<ClaimToken, ()>);

impl PreparedClusterClaimToken {
    #[must_use]
    pub fn token(&self) -> &ClaimToken {
        &self.0
    }
}

impl std::fmt::Debug for PreparedClusterClaimToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.token(), formatter)
    }
}

type RunError<T, C> = PreparedClusterRunnerError<<T as PreparedClusterExecutor>::Error, C>;

struct ReadyDelivery<R> {
    provider: R,
    delivery: PreparedClusterDelivery,
    token: PreparedClusterClaimToken,
}

struct DeliveryReservation<R> {
    provider: R,
    reply: PreparedClusterPayloadReservation,
    active: ActiveRun,
    diagnostic: RetainedValueReservation<ClaimToken, ()>,
}

impl From<PreparedClusterRunRequest> for ClaimOptions {
    fn from(request: PreparedClusterRunRequest) -> Self {
        Self {
            owner: request.owner,
            now: request.now,
            lease_until: request.lease_until,
            limit: NonZeroUsize::MIN,
        }
    }
}

impl ClusterRunner {
    /// Reserve every local delivery owner, claim one message and run its native
    /// executor. The completion clock and store transition execute together
    /// after the reply returns, with no suspension between them.
    pub async fn run_next_prepared<T: PreparedClusterExecutor, C: Send + 'static>(
        store: &MemoryClusterMessageStore,
        memory: &PreparedClusterRunnerMemory,
        executor: &T,
        request: PreparedClusterRunRequest,
        completed_at: impl FnOnce() -> Result<UtcMillis, C> + Send,
    ) -> Result<Option<ClusterRunOutcome>, RunError<T, C>> {
        let options = request.into();
        let reservation = reserve_delivery(store, memory, executor, &options)?;
        let Some(ready) = claim_delivery(store, options, reservation)? else {
            return Ok(None);
        };
        dispatch(store, executor, ready, completed_at)
            .await
            .map(Some)
    }

    /// Run with the enclosing native context's already reserved cancellation
    /// registration. A started claim remains fenced and recoverable after
    /// interruption. Once a reply returns, completion runs without yielding.
    pub async fn run_next_prepared_in<T, C, X>(
        context: &mut FutureContext<X>,
        store: &MemoryClusterMessageStore,
        memory: &PreparedClusterRunnerMemory,
        executor: &T,
        request: PreparedClusterRunRequest,
        completed_at: impl FnOnce() -> Result<UtcMillis, C> + Send,
    ) -> Exit<Option<ClusterRunOutcome>, RunError<T, C>>
    where
        T: PreparedClusterExecutor,
        C: Send + 'static,
        X: Send + 'static,
    {
        if context.is_requested() {
            return Exit::Interrupted;
        }
        let options = request.into();
        let reservation = match reserve_delivery(store, memory, executor, &options) {
            Ok(reservation) => reservation,
            Err(error) => return Exit::Fail(error),
        };
        if context.is_requested() {
            return Exit::Interrupted;
        }
        let ready = match claim_delivery(store, options, reservation) {
            Ok(Some(ready)) => ready,
            Ok(None) => return Exit::Success(None),
            Err(error) => return Exit::Fail(error),
        };
        if context.is_requested() {
            return Exit::Interrupted;
        }
        let interrupted = pin!(context.cancelled());
        let work = pin!(dispatch(store, executor, ready, completed_at));
        match select(interrupted, work).await {
            Either::Left(_) => Exit::Interrupted,
            Either::Right((Ok(outcome), _)) => Exit::Success(Some(outcome)),
            Either::Right((Err(error), _)) => Exit::Fail(error),
        }
    }
}

fn reserve_delivery<T: PreparedClusterExecutor, C>(
    store: &MemoryClusterMessageStore,
    memory: &PreparedClusterRunnerMemory,
    executor: &T,
    options: &ClaimOptions,
) -> Result<DeliveryReservation<T::Reservation>, RunError<T, C>> {
    store
        .check_prepared_claim_options(options)
        .map_err(claim_error)?;
    let active = memory
        .reserve(&options.owner.address)
        .map_err(PreparedClusterRunnerError::Memory)?;
    let diagnostic = RetainedValueReservation::reserve(&memory.diagnostics)
        .map_err(PreparedClusterRunnerError::Memory)?;
    let reply = store
        .reserve_prepared_payload()
        .map_err(PreparedClusterRunnerError::Memory)?;
    let provider = executor
        .try_reserve(&options.owner, reply.capacity())
        .map_err(PreparedClusterRunnerError::Memory)?;
    Ok(DeliveryReservation {
        provider,
        reply,
        active,
        diagnostic,
    })
}

fn claim_delivery<R, E, C>(
    store: &MemoryClusterMessageStore,
    options: ClaimOptions,
    reservation: DeliveryReservation<R>,
) -> Result<Option<ReadyDelivery<R>>, PreparedClusterRunnerError<E, C>> {
    let mut claims = store.claim_prepared_now(options).map_err(claim_error)?;
    Ok(claims.pop_front().map(|claim| ReadyDelivery {
        token: PreparedClusterClaimToken(reservation.diagnostic.publish(claim.token.clone(), ())),
        provider: reservation.provider,
        delivery: PreparedClusterDelivery {
            claim,
            reply: reservation.reply,
            _active: reservation.active,
        },
    }))
}

fn claim_error<E, C>(error: PreparedClusterStoreError) -> PreparedClusterRunnerError<E, C> {
    match error {
        PreparedClusterStoreError::Memory(error) => PreparedClusterRunnerError::Memory(error),
        PreparedClusterStoreError::Store(error) => {
            PreparedClusterRunnerError::UncertainClaim(error)
        }
    }
}

async fn dispatch<T: PreparedClusterExecutor, C>(
    store: &MemoryClusterMessageStore,
    executor: &T,
    ready: ReadyDelivery<T::Reservation>,
    completed_at: impl FnOnce() -> Result<UtcMillis, C>,
) -> Result<ClusterRunOutcome, RunError<T, C>> {
    let ReadyDelivery {
        provider,
        delivery,
        token,
    } = ready;
    let (delivery, kind) = match executor.execute(provider, delivery).await {
        Ok(output) => output,
        Err(source) => return Err(PreparedClusterRunnerError::Execution { token, source }),
    };
    if delivery.token() != token.token() {
        return Err(PreparedClusterRunnerError::UncertainCompletion {
            token,
            source: PreparedClusterStoreError::Store(ClusterStoreError::StaleClaim),
        });
    }
    let completed_at = match completed_at() {
        Ok(completed_at) => completed_at,
        Err(source) => {
            return Err(PreparedClusterRunnerError::UncertainCompletionClock { token, source });
        }
    };
    let request = token.token().request.clone();
    let PreparedClusterDelivery {
        claim,
        reply,
        _active: active,
    } = delivery;
    let completion = store.complete_prepared_now(claim.token.clone(), kind, reply, completed_at);
    drop(claim);
    drop(active);
    let completion = completion
        .map_err(|source| PreparedClusterRunnerError::UncertainCompletion { token, source })?;
    Ok(ClusterRunOutcome {
        request,
        completion,
    })
}
