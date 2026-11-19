//! Prepared ownership of an admitted unary RPC exchange.

use super::RpcCallId;
use crate::memory::{
    Buffer, Bytes, MemoryError, PreparedMemory, RegionId, SlotId, SlotLease, SlotPool,
};
use std::alloc::Layout;
use std::num::NonZeroUsize;

/// Physical pending-call and contiguous frame limits for the selected transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreparedRpcCapacity {
    /// Simultaneous admitted exchanges, including encoding, transport and decoding.
    pub exchanges: NonZeroUsize,
    /// Complete encoded request frame bytes per exchange, including its header.
    pub request: Bytes,
    /// Complete encoded response frame bytes per exchange, including its header.
    pub response: Bytes,
}

struct ExchangeSlot {
    call: Option<RpcCallId>,
    request: Buffer<'static>,
    response: Buffer<'static>,
}

/// Startup storage for pending calls and their request and response frames.
/// Wire codec and transport reservations form separate parts of call admission.
pub struct PreparedRpcMemory {
    slots: SlotPool<'static, ExchangeSlot>,
}

impl PreparedRpcMemory {
    /// Slot records and actual frame payload bytes. Allocator metadata and
    /// alignment are additional. The caller supplies the call ID's text owner.
    pub fn storage_bytes(capacity: PreparedRpcCapacity) -> Result<Bytes, MemoryError> {
        let exchanges = capacity.exchanges.get();
        let request =
            Layout::array::<u8>(capacity.request.get()).map_err(|_| MemoryError::InvalidLayout)?;
        let response =
            Layout::array::<u8>(capacity.response.get()).map_err(|_| MemoryError::InvalidLayout)?;
        let frames = Bytes::new(request.size())
            .checked_add(Bytes::new(response.size()))?
            .checked_mul(exchanges)?;
        SlotPool::<ExchangeSlot>::storage_bytes(exchanges)?.checked_add(frames)
    }

    /// Provision every pending record and its byte backing before execution.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedRpcCapacity,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(capacity)?;
        Ok(Self {
            slots: memory.slots(region, capacity.exchanges.get(), |_| {
                Ok(ExchangeSlot {
                    call: None,
                    request: memory.buffer(region, capacity.request)?,
                    response: memory.buffer(region, capacity.response)?,
                })
            })?,
        })
    }

    /// Share the same physical exchange budget.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            slots: self.slots.try_clone()?,
        })
    }

    /// Claim the pending record and both complete frame buffers together.
    /// Refusal creates no waiter and invokes no producer or transport.
    pub fn try_admit(&self, call: RpcCallId) -> Result<PreparedRpcExchange, MemoryError> {
        let mut slot = self.slots.try_checkout()?;
        slot.call = Some(call);
        Ok(PreparedRpcExchange { slot })
    }

    pub(crate) fn matches_exchange(&self, id: SlotId, exchange: &PreparedRpcExchange) -> bool {
        exchange.slot.belongs_to(&self.slots) && exchange.id() == id
    }
}

/// The affine pending call and its two actual frame buffers. A channel can
/// move this owner into its admitted work and return it after settlement.
pub struct PreparedRpcExchange {
    slot: SlotLease<'static, ExchangeSlot>,
}

impl PreparedRpcExchange {
    pub(crate) fn id(&self) -> SlotId {
        self.slot.id()
    }

    /// Borrow the correlation identity retained with this exchange.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "an admitted exchange always retains its call ID"
    )]
    pub fn call(&self) -> &RpcCallId {
        self.slot
            .call
            .as_ref()
            .expect("live RPC exchange has a call ID")
    }

    #[must_use]
    pub fn request(&self) -> &[u8] {
        self.slot.request.as_slice()
    }

    /// The physical request frame bound, including its header.
    #[must_use]
    pub fn request_capacity(&self) -> Bytes {
        self.slot.request.capacity()
    }

    pub(super) fn request_mut(&mut self) -> &mut Buffer<'static> {
        &mut self.slot.request
    }

    #[must_use]
    pub fn response(&self) -> &[u8] {
        self.slot.response.as_slice()
    }

    pub(super) fn response_mut(&mut self) -> &mut Buffer<'static> {
        &mut self.slot.response
    }

    /// Append received bytes to the admitted response without moving its backing.
    pub fn append_response(&mut self, bytes: &[u8]) -> Result<(), MemoryError> {
        self.slot.response.extend_from_slice(bytes)
    }

    /// The physical response frame bound, including its header.
    #[must_use]
    pub fn response_capacity(&self) -> Bytes {
        self.slot.response.capacity()
    }
}

impl Drop for PreparedRpcExchange {
    fn drop(&mut self) {
        // Retire the call owner and visible bytes before returning the record.
        self.slot.request.clear();
        self.slot.response.clear();
        drop(self.slot.call.take());
    }
}
