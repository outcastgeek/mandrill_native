//! A concrete generated native run with counted reusable root/output records.
//! Consumer ownership survives rendering and native stdout delivery. This is not
//! a claim of full Effect-graph/all13/foreign-allocation or 512MiB qualification.
use effect::memory::{Memory, MemoryError, RegionId, SlotPool, Text, TextLease};
use effect::{
    DynamicRunContract, Exit, LeasedRunResult, PreparedNativeReservation, PreparedRunContext,
    effect_prepare, memory, memory_write,
};
use std::io::Write;
use std::sync::Arc;
struct PriceMenu {
    unit_cents: usize,
}
struct LineItem {
    quantity: usize,
    unit_cents: usize,
}
struct PricingServices {
    quantity: effect::PreparedGraphResourceValue<usize>,
    menu: effect::PreparedGraphResourceValue<PriceMenu>,
}
type PriceValue = Result<usize, effect::Defect>;
type Price = LeasedRunResult<Result<usize, ReceiptError>, PriceValue, !>;
struct Cafe {
    memory: Memory<'static>,
    receipts: SlotPool<'static, Text<'static>>,
    price: Price,
}
memory! { capability Cafe => self.memory; }
#[derive(Debug)]
enum ReceiptError {
    Capacity,
    Io(std::io::Error),
    Outcome,
}
impl std::fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Capacity => f.write_str("receipt capacity exceeded"),
            Self::Io(error) => error.fmt(f),
            Self::Outcome => f.write_str("receipt interrupted or finalization failed"),
        }
    }
}
impl std::error::Error for ReceiptError {}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pricing = effect_prepare! {
        graph;
        program: effect::effect_with_layer! {
            layer: effect::layer! {
                provide;
                left: effect::layer! {
                    merge;
                    left: effect::layer! { build |_scope| { PriceMenu { unit_cents: 475 } }, },
                    right: effect::layer! { build |_scope| { 2_usize }, },
                    combine |menu: PriceMenu, quantity: usize| { LineItem { quantity,unit_cents: menu.unit_cents } },
                },
                right: effect::layer! {
                    construction: (effect::effect_scope_acquire! { construction; },effect::effect_scope_acquire! { construction; }),
                    build |_scope: effect::PreparedScope<!>,(quantity_permit,menu_permit)| {
                        let quantity=effect::effect_scope_acquire! {
                            permit: quantity_permit,
                            acquire: effect::service! { env |line: &LineItem| { line.quantity } },
                            release |quantity| { let _released=quantity; () },
                        }?;
                        let menu=effect::effect_scope_acquire! {
                            permit: menu_permit,
                            acquire: effect::service! { env |line: &LineItem| { PriceMenu { unit_cents: line.unit_cents } } },
                            release |menu| { let _released=menu; () },
                        }?;
                        PricingServices { quantity,menu }
                    },
                },
            },
            body {
                let total = effect::service! { env |services: &PricingServices| { services.quantity.with(|quantity| services.menu.with(|menu| quantity * menu.unit_cents)).and_then(core::convert::identity) } }?;
                total
            },
        },
        contract: DynamicRunContract::GeneratedGraph { clock_ticks: 0, contexts: 2, listeners: 0, bridges: 0 },
    };
    let price_demand = pricing.demand(1)?;
    let plan = effect_prepare! {
        plan;
        body: |environment: Arc<Cafe>, context: PreparedRunContext<TextLease<'static>,ReceiptError>, (mut receipt,reservation): (TextLease<'static>,PreparedNativeReservation<TextLease<'static>,ReceiptError>)| async move {
            if context.is_cancelled() { return Exit::Interrupted; }
            let total = match &*environment.price { Ok(total) => *total,Err(_) => return Exit::Fail(ReceiptError::Outcome) };
            let format = effect_prepare! {
                native_reserved; reservation: reservation, body: move |_child| async move {
                receipt.clear();
                if memory_write! { into: receipt, format: format_args!("Cafe receipt\n2 oat lattes\nTotal: {}.{:02}\n",total / 100,total % 100), }.is_err() {
                    return Exit::Fail(ReceiptError::Capacity);
                }
                Exit::Success(receipt)
            }, };
            format.await
        },
        reserve: |environment: &Arc<Cafe>, context| {
            let receipt = environment.receipts.try_checkout()?;
            let child = context.child()?;
            let reservation = effect_prepare! { reserve_native; context: child, }.map_err(|(error,_child)|error)?;
            Ok((receipt,reservation))
        },
        contract: DynamicRunContract::ConcreteFuture { clock_ticks: 0, contexts: 3, listeners: 2, bridges: 0 },
    };
    let capacity = 2_usize;
    let demand = plan.demand(capacity)?;
    // Engineering headroom for the full lexical close working set, not a
    // measured fit, runtime allocation certification or memory SLO claim.
    let configuration = memory! { total: 512 KiB, mode: prepared, metadata: metadata, partition metadata: 64 KiB, partition receipts: 384 KiB, }?;
    let prepared = configuration.prepare()?;
    let metadata = RegionId::named("metadata");
    let payload = RegionId::named("receipts");
    let receipts = prepared.texts(payload, capacity, 1024.into())?;
    let frames = prepared.frames(
        payload,
        demand
            .frames_per_run
            .checked_mul(capacity)
            .ok_or(MemoryError::ArithmeticOverflow)?,
        demand.frame_layout,
    )?;
    let price_frames = prepared.frames(
        payload,
        price_demand.frames_per_run,
        price_demand.frame_layout,
    )?;
    // This example selects four retained secondary entries per aggregation site;
    // the pool reserves one additional overflow marker in each buffer.
    let cleanup_pool = effect::PreparedCleanupPool::<!>::prepare(
        &prepared,
        payload,
        price_demand.cleanup_sites,
        4,
    )?;
    let cleanup_frames = prepared.frames(
        payload,
        price_demand.cleanup_sites,
        effect::PreparedCleanupPool::<!>::frame_layout(),
    )?;
    let pricing = pricing.with_cleanup(effect::PreparedCleanupConfig::new(
        cleanup_pool,
        cleanup_frames,
    ));
    let pricing = effect_prepare! { prepare; plan: pricing, memory: prepared, region: metadata, capacity: 1, frames: price_frames, }?;
    let run = effect_prepare! { prepare; plan: plan, memory: prepared, region: metadata, capacity: capacity, frames: frames, }?;
    let priced = effect_prepare! { admit; run: &pricing, environment: Arc::new(()), }?;
    let price = futures_executor::block_on(effect_prepare! { run; invocation: priced, })
        .map(|outcome: &Exit<PriceValue, !>| match outcome {
            Exit::Success(Ok(cents)) => Ok(*cents),
            Exit::Success(Err(_)) => Err(ReceiptError::Outcome),
            Exit::Fail(never) => match *never {},
            Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
                Err(ReceiptError::Outcome)
            }
        })
        .map_err(|_| std::io::Error::other("price output is still shared"))?;
    let environment = Arc::new(Cafe {
        memory: prepared.seal(),
        receipts,
        price,
    });
    // Keep the actual pricing-output owner through rendering AND delivery,
    // including consumer Future destruction and Ready-output inspection.
    let _pricing_delivery_owner = Arc::clone(&environment);
    let invocation = effect_prepare! { admit; run: &run, environment: environment, }?;
    let output = futures_executor::block_on(effect_prepare! { run; invocation: invocation, });
    // SAFETY: the original receipt/error moves only into this Future. It is
    // neither sent nor stored elsewhere; write/flush settle synchronously and
    // the captured outcome is destroyed before the consumer's origin lease.
    let consumer = unsafe {
        output.map_owned_unchecked(|outcome| async move {
            match outcome {
                Exit::Success(receipt) => {
                    let delivered = std::io::stdout()
                        .lock()
                        .write_all(receipt.as_str().as_bytes())
                        .and_then(|()| std::io::stdout().lock().flush())
                        .map_err(ReceiptError::Io);
                    drop(receipt);
                    delivered
                }
                Exit::Fail(error) => Err(error),
                Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => {
                    Err(ReceiptError::Outcome)
                }
            }
        })
    }
    .map_err(|_| std::io::Error::other("receipt output is still shared"))?;
    // Ready-output transfer ownership is acquired BEFORE this consumer is polled.
    let consumer = consumer
        .prepare_future()
        .map_err(|(error, _consumer)| error)?;
    let delivered = futures_executor::block_on(consumer);
    run.close();
    match &*delivered {
        Ok(()) => Ok(()),
        Err(error) => Err(std::io::Error::other(error.to_string()).into()),
    }
}
