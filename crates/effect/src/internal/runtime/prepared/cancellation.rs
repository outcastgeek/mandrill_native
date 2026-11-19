//! Standalone prepared cancellation using the existing context domain.
use super::{Domain, Id, Kind, ListenerId, PreparedCancellationObservation};
use crate::memory::{Bytes, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A fallibly shared cancellation node backed by a finite prepared context domain.
pub struct PreparedCancellation {
    domain: SharedSlot<'static, Domain>,
    id: Id,
}
/// A physically admitted cancellation listener retaining its actual node.
pub struct PreparedCancellationSignalWait {
    token: PreparedCancellation,
    listener: Option<ListenerId>,
    ready: bool,
}
impl PreparedCancellation {
    /// Domain and vector payload layouts excluding allocator headers and padding.
    /// Zero clock ticks are provisioned: no clock operations are exposed here.
    pub fn storage_bytes(nodes: usize, listeners: usize) -> Result<Bytes, MemoryError> {
        if nodes == 0 {
            return Err(MemoryError::NoSlot);
        }
        SlotPool::<Domain>::storage_bytes(1)?.checked_add(Domain::payload(nodes, listeners, 0)?)
    }
    /// Prepare explicitly bounded cancellation nodes and simultaneous listeners.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        nodes: usize,
        listeners: usize,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(nodes, listeners)?;
        let mut domain = Some(Domain::prepare(memory, region, nodes, listeners, 0)?);
        let pool = memory.slots(region, 1, |_| domain.take().ok_or(MemoryError::NoSlot))?;
        let domain = pool.checkout()?.share();
        let id = domain.create(Kind::Root, None, None, None)?;
        Ok(Self { domain, id })
    }
    /// Retain the exact node and its prepared domain without allocating.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        let domain = self.domain.try_clone()?;
        domain.retain(self.id)?;
        Ok(Self {
            domain,
            id: self.id,
        })
    }
    /// Reserve a child whose cancellation inherits this node's actual parent link.
    pub fn child(&self) -> Result<Self, MemoryError> {
        let domain = self.domain.try_clone()?;
        let id = domain.create(Kind::Child, Some(self.id), Some(self.id), None)?;
        Ok(Self { domain, id })
    }
    /// Request this node once, notifying its registered descendants.
    #[must_use]
    pub fn request(&self) -> bool {
        self.domain.request(self.id)
    }
    /// Check this node and its retained inherited parent chain.
    #[must_use]
    pub fn is_requested(&self) -> bool {
        self.domain.cancelled(self.id)
    }
    /// Reserve one physical cancellation listener before starting a wait.
    pub fn cancelled(&self) -> Result<PreparedCancellationSignalWait, MemoryError> {
        let token = self.try_clone()?;
        let listener = token.domain.listener(token.id)?;
        Ok(PreparedCancellationSignalWait {
            token,
            listener: Some(listener),
            ready: false,
        })
    }
}
impl Drop for PreparedCancellation {
    fn drop(&mut self) {
        self.domain.release(self.id);
    }
}
impl Future for PreparedCancellationSignalWait {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        if this.ready {
            return Poll::Ready(());
        }
        let Some(listener) = this.listener else {
            return Poll::Pending;
        };
        match this
            .token
            .domain
            .observe(this.token.id, listener, cx.waker())
        {
            Some(PreparedCancellationObservation::Requested) => {
                this.token.domain.unlink(listener);
                this.listener = None;
                this.ready = true;
                Poll::Ready(())
            }
            Some(PreparedCancellationObservation::Closed) | None => Poll::Pending,
        }
    }
}
impl Drop for PreparedCancellationSignalWait {
    fn drop(&mut self) {
        if let Some(listener) = self.listener.take() {
            self.token.domain.unlink(listener);
        }
    }
}
