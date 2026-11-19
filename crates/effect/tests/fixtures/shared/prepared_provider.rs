//! Small typed working-set and explicit diagnostic sink for provider fixtures.

use effect::memory::{
    Bytes, FramePool, MemoryError, MemoryPlan, Partition, PreparedMemory, RegionId, SlotLease,
    SlotPool,
};
use effect::prepared_provider::{
    PreparedDiagnosticLease, ProviderBudget, ProviderDiagnostic, ProviderDiagnosticSinkLease,
    ProviderNativeEnvelope, ProviderRefusal, ProviderWorkingSet,
};
use std::alloc::Layout;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub const REGION: RegionId = RegionId::named("provider-fixture");
pub const NATIVE: ProviderNativeEnvelope = ProviderNativeEnvelope {
    workers: 1,
    stack_bytes: 1024 * 1024,
    contract: "one native worker, one prepared entry, existing borrowed runtime reactor",
};

pub fn memory() -> Result<PreparedMemory<'static>, MemoryError> {
    let bytes = Bytes::new(2 * 1024 * 1024);
    let partitions = [Partition {
        id: REGION,
        capacity: bytes,
    }];
    MemoryPlan::new(bytes, &partitions)?
        .metadata(REGION)?
        .prepare()
}

pub struct ScalarWork;

// SAFETY: fixture callbacks capture fixed inline scalars or share their already
// created runtime. Output is an inline scalar. Worker wake controls are
// prepared by the backend, and the caller owns the selected native reactor.
unsafe impl ProviderWorkingSet for ScalarWork {
    const CONTRACT: &'static str = "inline test values and preconfigured native runtime";
}

struct DiagnosticSink {
    record: SlotLease<'static, Option<ProviderDiagnostic>>,
    delivered: Arc<AtomicUsize>,
}

// SAFETY: This sink checks out a concrete regional record for each event
// before testing work. It consumes the event immediately and counts each
// delivery. Test payloads are ordinary provider scalars/static panic text.
unsafe impl ProviderDiagnosticSinkLease for DiagnosticSink {
    const CONTRACT: &'static str = "preallocated fixture record with immediate counted receipt";

    fn deliver(mut self, event: ProviderDiagnostic) {
        *self.record = Some(event);
        self.delivered.fetch_add(1, Ordering::Relaxed);
        drop(self.record.take());
    }
}

pub struct Diagnostics {
    records: SlotPool<'static, Option<ProviderDiagnostic>>,
    frames: FramePool<'static>,
    delivered: Arc<AtomicUsize>,
}

impl Diagnostics {
    pub fn prepare(memory: &PreparedMemory<'static>) -> Result<Self, MemoryError> {
        Ok(Self {
            records: memory.slots(REGION, 3, |_| Ok(None))?,
            frames: memory.frames(REGION, 3, Layout::new::<DiagnosticSink>())?,
            delivered: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn reserve(&self) -> Result<PreparedDiagnosticLease, ProviderRefusal> {
        let frame = self.frames.reserve()?;
        let record = self.records.try_checkout()?;
        PreparedDiagnosticLease::new(
            frame,
            DiagnosticSink {
                record,
                delivered: Arc::clone(&self.delivered),
            },
        )
    }

    pub fn budget<D: ProviderWorkingSet>(
        &self,
        working_set: D,
    ) -> Result<ProviderBudget<D>, ProviderRefusal> {
        Ok(ProviderBudget {
            working_set,
            notification: self.reserve()?,
            worker_retirement: self.reserve()?,
            execution_retirement: self.reserve()?,
        })
    }

    pub fn deliveries(&self) -> usize {
        self.delivered.load(Ordering::Relaxed)
    }
}
