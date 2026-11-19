//! Explicit, physically reserved, separately budgeted diagnostic delivery.
use super::ProviderRefusal;
use crate::memory::{FrameReservation, RuntimeFrame};
use std::any::Any;

/// Side-channel failures never replace a published application outcome.
#[derive(Clone, Copy, Debug)]
pub enum ProviderDiagnosticSite {
    Notification,
    WorkerRetirement,
    ExecutionRetirement,
}
pub struct ProviderDiagnostic {
    pub site: ProviderDiagnosticSite,
    pub payload: Box<dyn Any + Send>,
}

/// One already-admitted event owner in a caller-selected diagnostic domain.
/// Dropping unused ownership returns its reserved event/control capacity.
/// Delivery transfers that capacity with the raw payload until the selected
/// sink acknowledges/releases its LAST retained event owner. The lease must
/// own actual backing, not merely a quota around an allocating callback.
///
/// # Safety
/// `deliver` must run without panicking and without allocation beyond the
/// lease's explicit named storage contract. It must consume the event exactly
/// once using the caller-selected retention/overflow policy, never an implicit
/// default loss policy. Any native storage reachable from the payload/sink,
/// including payload destruction, must have a separately budgeted contract.
/// Refusal/saturation must be resolved when obtaining the lease, before work;
/// `deliver` cannot fail after admission. Unused-lease Drop must also be
/// nonpanicking, return its actual event/control ownership without late
/// allocation, and preserve the selected native domain owner until that return.
/// The sink must retain payload backing AND its budget until final acknowledgment
/// even if the provider, receiver, or an intermediate sink handle disappears;
/// owner-loss behavior is part of the caller's explicit contract, not a default
/// policy chosen here. Send transfers this unique lease; it grants no shared
/// mutation or Sync assumption about the payload/sink. No string copying/Defect
/// formatting or pointer-sized pointee proof is authorized by this trait.
pub unsafe trait ProviderDiagnosticSinkLease: Send + 'static {
    const CONTRACT: &'static str;
    fn deliver(self, event: ProviderDiagnostic);
}

/// Erased control is placed in an exact-layout prepared frame, not a Box/Arc.
/// The selected sink owns the actual separately budgeted event backing.
pub struct PreparedDiagnosticLease {
    frame: RuntimeFrame,
    deliver: unsafe fn(&mut RuntimeFrame, ProviderDiagnostic),
}
impl PreparedDiagnosticLease {
    pub fn new<S: ProviderDiagnosticSinkLease>(
        reservation: FrameReservation<'static, S>,
        lease: S,
    ) -> Result<Self, ProviderRefusal> {
        if S::CONTRACT.is_empty() {
            return Err(ProviderRefusal::InvalidEnvelope);
        }
        unsafe fn deliver<S: ProviderDiagnosticSinkLease>(
            frame: &mut RuntimeFrame,
            event: ProviderDiagnostic,
        ) {
            // SAFETY: constructor selected this shim for exactly S. The lease
            // was never pinned or exposed, and this consumed owner delivers once.
            let lease = unsafe { frame.take::<S>() };
            lease.deliver(event);
        }
        Ok(Self {
            frame: reservation.erase(lease),
            deliver: deliver::<S>,
        })
    }
    pub(super) fn report(mut self, site: ProviderDiagnosticSite, payload: Box<dyn Any + Send>) {
        // SAFETY: matching typed constructor owns initialized S until this call.
        unsafe { (self.deliver)(&mut self.frame, ProviderDiagnostic { site, payload }) };
    }
}

/// Each operation pre-admits exactly three provider-owned side-channel events:
/// one completion waiter wake, one final worker-side entry/result retirement,
/// and one execution-owner retirement. The worker executes/catches each site
/// once, after the future has returned/dropped, never once per poll. Future/poll
/// panic is instead the retained ProviderSettlement::Panicked result. Context
/// startup/guard failures are retained by startup errors/owner join. Caller-side
/// Waker cloning, user output Drop, sink delivery and double-panic behavior keep
/// their explicit external/Rust contracts; they are not certified by this bound.
///
/// The selected sink domain is prepared/budgeted by the caller; this bundle
/// checks out real event/backing leases before capture construction. It does not
/// select saturation/overflow behavior: failure to acquire any selected lease
/// rejects preparation, and accepted events cannot be implicitly discarded.
pub struct ProviderBudget<D> {
    pub working_set: D,
    pub notification: PreparedDiagnosticLease,
    pub worker_retirement: PreparedDiagnosticLease,
    pub execution_retirement: PreparedDiagnosticLease,
}
