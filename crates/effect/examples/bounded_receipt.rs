//! Render a cafe receipt with prepared payload/future/finalizer storage. The
//! reusable scope owns its receipt lease and its slot quota until the prepared
//! flush completes; cleanup frames remain separately admitted physical owners.
//! Ordinary interpreter/context/listener/stdout domains are still separate: this
//! is not the full prepared web-service/512 MiB application from the plan.
use effect::memory::{
    Admission, BackedAdmission, CapacityUnit, Memory, MemoryError, RegionId, ResourceCapacity,
    RunFrames, SlotPool, Text, TextLease,
};
use effect::{
    Effect, Exit, PreparedScopeLimits, effect, effect_future, effect_resource, effect_scope,
    effect_scope_acquire, effect_try_sync, memory, memory_vec, memory_write, service,
};
use std::io::Write;
use std::sync::Arc;

struct Cafe {
    memory: Memory<'static>,
    receipts: SlotPool<'static, Text<'static>>,
    admission: Admission<'static, 1>,
}
memory! { capability Cafe => self.memory; }

#[derive(Debug)]
enum ReceiptError {
    Memory(MemoryError),
    OutputCapacity,
    OutputIo(std::io::Error),
    Resource(effect::Defect),
}
impl std::fmt::Display for ReceiptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(error) => error.fmt(formatter),
            Self::OutputCapacity => formatter.write_str("receipt exceeds output capacity"),
            Self::OutputIo(error) => error.fmt(formatter),
            Self::Resource(error) => write!(formatter, "{error:?}"),
        }
    }
}
impl std::error::Error for ReceiptError {}

fn receipt(total_cents: u32) -> Effect<BackedAdmission<'static, 1, TextLease<'static>>, ReceiptError, Cafe> {
    effect! {
        let total_cents = effect_future! {
            prepared; body: move |_cancel| async move { total_cents },
            on_capacity: ReceiptError::Memory,
        }?;
        let reservation = service! {
            env |cafe: &Cafe| {
            cafe.admission.reserve_backed([1], [0], || cafe.receipts.checkout())
        }, }?;
        effect_try_sync! {
            once; body: move || {
            let mut output = reservation.map_err(ReceiptError::Memory)?;
            let text = output.slot_value_mut();
            text.clear();
            memory_write! {
                into: *text,
                format: format_args!("Cafe receipt\n2 oat lattes\nTotal: {}.{:02}\n", total_cents / 100, total_cents % 100),
            }.map_err(|_| ReceiptError::OutputCapacity)?;
            Ok::<_, ReceiptError>(output)
        }, }?
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scope_limits = PreparedScopeLimits {
        finalizers: 4,
        segments: 1,
    };
    let scope_count = 1_usize;
    let scope_demand = scope_limits.pool_demand::<ReceiptError>(scope_count)?;
    let configuration = memory! {
        total: 192 KiB, mode: prepared, metadata: metadata,
        partition metadata: 32 KiB, partition receipts: 128 KiB,
    }?;
    let prepared = configuration.prepare()?;
    let metadata = RegionId::named("metadata");
    let payload = RegionId::named("receipts");
    let receipts = prepared.texts(payload, 8, 1024.into())?;
    let frame_layout = std::alloc::Layout::from_size_align(4096, 8)?;
    let prices = memory_vec! {
        allocator: prepared.allocator(metadata)?, capacity: 2,
        init |_index| { Ok::<_, std::convert::Infallible>(475_u32) },
    }?;
    let total_cents = prices.iter().copied().sum();
    let resource_plan = effect_resource! {
        plan_future; acquire: receipt(total_cents),
        release: move |receipt, _reason, _environment, _context| async move {
            let flushed = std::io::stdout().lock().flush().map_err(ReceiptError::OutputIo);
            // This actual lease and both cleanup frames were admitted before acquire.
            drop(receipt);
            match flushed { Ok(()) => Exit::Success(()), Err(error) => Exit::Fail(error) }
        }, on_capacity: ReceiptError::Memory,
    };
    let resource_demand = resource_plan.demand(receipts.capacity())?;
    let cleanup_demand = resource_demand.cleanup.ok_or(MemoryError::InvalidLayout)?;
    let run_frames = RunFrames::new(
        prepared.frames(payload, 4, frame_layout)?,
        prepared.frames(
            payload,
            cleanup_demand.frames,
            cleanup_demand.frame_layout()?,
        )?,
    )?;
    let callback_count = resource_demand.callback_frames.min(
        scope_demand
            .maximum_retained_finalizers
            .checked_mul(2)
            .ok_or(MemoryError::ArithmeticOverflow)?,
    );
    // Two scope-close buffers and one resource-refusal buffer per scope.
    // This example selects four retained secondary failures per buffer.
    let cleanup_count = scope_count
        .checked_mul(3)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    let cleanup_pool =
        effect::PreparedCleanupPool::<ReceiptError>::prepare(&prepared, payload, cleanup_count, 4)?;
    let cleanup_frames = prepared.frames(
        payload,
        cleanup_count,
        effect::PreparedCleanupPool::<ReceiptError>::frame_layout(),
    )?;
    let scope_pool = effect_scope! {
        prepare_pool; memory: prepared, metadata: metadata, scopes: scope_count,
        limits: scope_limits,
        callbacks: prepared.frames(payload, callback_count, resource_demand.callback_layout)?,
        cleanup: effect::PreparedCleanupConfig::new(cleanup_pool, cleanup_frames),
    }?;
    let binding = effect_resource! { prepare_future; plan: resource_plan, memory: prepared, metadata: metadata, resources: receipts.capacity(), }?;
    let admission = Admission::prepare(
        &prepared,
        metadata,
        [ResourceCapacity {
            unit: CapacityUnit::Slots,
            work: 8,
            control: 0,
        }],
    )?;
    let environment = Arc::new(Cafe {
        memory: prepared.seal(),
        receipts,
        admission,
    });
    let application = effect_scope! {
        prepared_pool; pool: scope_pool, on_capacity: ReceiptError::Memory,
        try_body: move |scope| {
            let acquire = effect_scope_acquire! { prepared_future; scope: scope, binding: binding.try_clone()?, };
            Ok::<_, MemoryError>(effect! {
                let receipt = acquire.clone()?;
                effect_try_sync! {
                    once; body: move || {
                    receipt.with(|receipt| std::io::stdout().lock().write_all(receipt.owner().as_str().as_bytes()).map_err(ReceiptError::OutputIo))
                        .map_err(ReceiptError::Resource)?
                }, }?
            })
        },
    };
    match futures_executor::block_on(effect! {
        run_with_frames;
        program: application,
        environment: environment,
        frames: run_frames,
    }) {
        Exit::Success(()) => Ok(()),
        Exit::Fail(error) => Err(error.into()),
        Exit::Die(error) => Err(std::io::Error::other(format!("{error:?}")).into()),
        Exit::Interrupted => Err(std::io::Error::from(std::io::ErrorKind::Interrupted).into()),
        Exit::FinalizationFailed(_) => Err(std::io::Error::other("receipt cleanup failed").into()),
    }
}
