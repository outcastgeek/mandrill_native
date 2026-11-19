//! A finite café batch moves through a prepared queue, counters and a broadcast hub.
//! The capacities below are illustrative application choices. Native runtime,
//! allocator headers and application-owned data remain separately accounted.
use effect::memory::{Bytes, MemoryError, RegionId};
use effect::{
    DynamicRunContract, Exit, PanicMessageOverflow, PreparedAtomicRef, PreparedCleanupConfig,
    PreparedCleanupPool, PreparedPanicConfig, PreparedPanicPool, PreparedPubSub,
    PreparedPubSubCapacity, PreparedQueue, PreparedQueueCapacity, PreparedSubscription,
    PreparedSubscriptionRef, PreparedSynchronizedRef, PubSubOptions, PubSubOverflow, QueueCapacity,
    QueueError, ReplayPolicy, effect_prepare, memory,
};
use std::{
    io::Write,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Cafe {
    queue: PreparedQueue<u32>,
    hub: PreparedPubSub<u32>,
    subscription: PreparedSubscription<u32>,
    served: PreparedAtomicRef<usize>,
    sales: PreparedSynchronizedRef<usize>,
    status: PreparedSubscriptionRef<u32>,
    cleanup: PreparedCleanupConfig<CafeError>,
}
#[derive(Debug)]
enum CafeError {
    Capacity,
    Queue,
    Publish,
    Subscription,
    Sync,
}

fn record_served_order()
-> impl effect::__private::prepared_graph::Description<Value = (), Error = CafeError, Env = Cafe> {
    effect::effect_ref! {
        prepared_update;
        reference |env: &Cafe| { &env.served }, update |served: &usize| { served + 1 },
    }
}
fn enqueue_order(
    order_id: u32,
) -> impl effect::__private::prepared_graph::Description<Value = (), Error = CafeError, Env = Cafe>
{
    effect::effect_future! {
        prepared_queue_offer;
        queue |env: &Cafe| { &env.queue }, cleanup |env: &Cafe| { &env.cleanup },
        on_capacity |_error: MemoryError| { CafeError::Capacity }, on_error |_error: QueueError| { CafeError::Queue },
        body |_env: &Cafe| { order_id },
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let batch_position = Arc::new(AtomicUsize::new(0));
    let plan = effect_prepare! {
        graph;
        program: effect! {
            let _batch = effect! {
                while_loop;
                guard: {
                    let position = Arc::clone(&batch_position);
                    move || position.load(Ordering::Relaxed) < 3
                },
                body: {
                    let position = Arc::clone(&batch_position);
                    move || {
                        let position = Arc::clone(&position);
                        let read_position = Arc::clone(&position);
                        effect! {
                        let order_id = effect_sync! {
                            body {
                                    match read_position.load(Ordering::Relaxed) { 0 => 104_u32, 1 => 105_u32, _ => 106_u32 }
                                }, }?;
                                let _offered = enqueue_order(order_id)?;
                        let order = effect_future! {
                            prepared_queue_take;
                            queue |env: &Cafe| { &env.queue }, cleanup |env: &Cafe| { &env.cleanup },
                            on_capacity |_error: MemoryError| { CafeError::Capacity }, on_error |_error: QueueError| { CafeError::Queue },
                        }?;
                        let order_id = effect_sync! { body { order.into_inner() }, }?;
                        let _report = effect_future! {
                            prepared_pubsub_publish;
                            channel |env: &Cafe| { &env.hub }, cleanup |env: &Cafe| { &env.cleanup },
                            on_capacity |_error: MemoryError| { CafeError::Capacity }, on_error |_error: effect::PreparedPubSubError| { CafeError::Publish },
                            body |_env: &Cafe| { order_id },
                        }?;
                        let _updated = record_served_order()?;
                        let _base = effect_ref! {
                            prepared_synchronized_pure;
                            reference |env: &Cafe| { &env.sales },
                            modify |current: effect::PreparedPayloadView<usize>| { (*current, *current + 475) },
                            on_error |_error: effect::PreparedSemaphoreError| { CafeError::Sync },
                        }?;
                        let _fee = effect_ref! {
                            prepared_synchronized;
                            reference |env: &Cafe| { &env.sales },
                            modify |current: effect::PreparedPayloadView<usize>| {
                                effect_sync! { body { (*current, *current + 25) }, }?
                            },
                            on_error |_error: effect::PreparedSemaphoreError| { CafeError::Sync },
                        }?;
                        let _status_set = effect_future! {
                            prepared_subscription_set;
                            reference |env: &Cafe| { &env.status }, cleanup |env: &Cafe| { &env.cleanup },
                            on_capacity |_error: MemoryError| { CafeError::Capacity }, on_error |_error: effect::PreparedPubSubError| { CafeError::Subscription },
                            body |_env: &Cafe| { order_id },
                        }?;
                            let _event = effect_future! {
                                prepared_subscription_take;
                                subscription |env: &Cafe| { &env.subscription }, cleanup |env: &Cafe| { &env.cleanup },
                                on_capacity |_error: MemoryError| { CafeError::Capacity }, on_error |_error: effect::PubSubError| { CafeError::Subscription },
                            }?;
                            let _advanced = effect_sync! {
                                body {
                                drop(_event);
                                let _ = position.fetch_add(1, Ordering::Relaxed);
                            }, }?;
                            ()
                        }
                    }
                },
                on_capacity: |_error: MemoryError| CafeError::Capacity,
            }?;
            let snapshot = effect_ref! {
                prepared_subscription_snapshot;
                reference |env: &Cafe| { &env.status }, on_error |_error: effect::PreparedPubSubError| { CafeError::Subscription },
            }?;
            let current = effect_ref! {
                prepared_subscription_read;
                reference |env: &Cafe| { &env.status }, on_error |_error: effect::PreparedPubSubError| { CafeError::Subscription },
            }?;
            (snapshot, current)
        },
        // Five queue/hub/status native nodes plus three native nodes for each of the
        // two synchronized resources. The root and two protected resources
        // need three context nodes; each native node reserves one listener.
        contract: DynamicRunContract::GeneratedGraph { clock_ticks: 0, contexts: 3, listeners: 11, bridges: 0 },
    };
    let demand = plan.demand(1)?;
    let queue_capacity = PreparedQueueCapacity {
        payloads: 2,
        producers: 1,
        takers: 1,
    };
    let queue_logical = QueueCapacity::Bounded(NonZeroUsize::MIN);
    let queue_demand = PreparedQueue::<u32>::demand(queue_logical, queue_capacity)?;
    // Graph sites remain admitted while queue shutdown authority and at most
    // one sequential data operation are held; publication needs one more.
    let panic_slots = demand
        .panic_sites
        .checked_add(queue_demand.shutdown_panic_slots)
        .and_then(|n| {
            n.checked_add(
                queue_demand
                    .offer_panic_slots
                    .max(queue_demand.take_panic_slots),
            )
        })
        .and_then(|n| n.checked_add(1))
        .ok_or(MemoryError::ArithmeticOverflow)?;
    // One target-mapping carrier is used at a time in this sequential graph.
    let app_cleanup_count = demand
        .cleanup_sites
        .checked_add(1)
        .ok_or(MemoryError::ArithmeticOverflow)?;
    let queue_cleanup_count = 2_usize
        .checked_add(queue_capacity.producers)
        .and_then(|n| n.checked_add(queue_capacity.takers))
        .ok_or(MemoryError::ArithmeticOverflow)?;
    // Graph records include the loop's private lane/control storage. The extra
    // metadata and unpartitioned space are illustrative allocator headroom.
    let metadata_bytes = demand.record_payload.checked_add(Bytes::new(256 * 1024))?;
    let orders_bytes = Bytes::new(1536 * 1024);
    let total_bytes = metadata_bytes
        .checked_add(orders_bytes)?
        .checked_add(Bytes::new(256 * 1024))?;
    let memory = memory! { total: total_bytes, mode: prepared, metadata: metadata, partition metadata: metadata_bytes, partition orders: orders_bytes, }?.prepare()?;
    let metadata = RegionId::named("metadata");
    let region = RegionId::named("orders");
    let panics = PreparedPanicPool::prepare(
        &memory,
        region,
        PreparedPanicConfig {
            slots: panic_slots,
            bytes_per_message: 128.into(),
            overflow: PanicMessageOverflow::Summary,
        },
    )?;
    let app_pool =
        PreparedCleanupPool::<CafeError>::prepare(&memory, region, app_cleanup_count, 4)?;
    let app_frames = memory.frames(
        region,
        app_cleanup_count,
        PreparedCleanupPool::<CafeError>::frame_layout(),
    )?;
    let app_cleanup = PreparedCleanupConfig::new(app_pool, app_frames);
    let queue_pool =
        PreparedCleanupPool::<QueueError>::prepare(&memory, region, queue_cleanup_count, 4)?;
    let queue_frames = memory.frames(
        region,
        queue_cleanup_count,
        PreparedCleanupPool::<QueueError>::frame_layout(),
    )?;
    let queue_cleanup = PreparedCleanupConfig::new(queue_pool, queue_frames);
    let queue = PreparedQueue::prepare(
        &memory,
        region,
        queue_logical,
        queue_capacity,
        &queue_cleanup,
        &panics,
    )?;
    let hub = PreparedPubSub::prepare(
        &memory,
        region,
        PubSubOptions {
            capacity: NonZeroUsize::MIN,
            overflow: PubSubOverflow::Backpressure,
            replay: ReplayPolicy::None,
        },
        PreparedPubSubCapacity {
            payloads: 2,
            publishers: 1,
            subscribers: 1,
            takers_per_subscriber: 1,
        },
        false,
        &panics,
    )
    .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
    let subscription = hub
        .subscribe()
        .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
    let served =
        PreparedAtomicRef::prepare(&memory, region, 0_usize).map_err(|(error, _)| error)?;
    let sales = PreparedSynchronizedRef::prepare(&memory, region, 3, 1, &panics, 0_usize)
        .map_err(|(error, _)| std::io::Error::other(format!("{error:?}")))?;
    let status = PreparedSubscriptionRef::prepare(
        &memory,
        region,
        NonZeroUsize::MIN,
        PubSubOverflow::Backpressure,
        PreparedPubSubCapacity {
            payloads: 2,
            publishers: 1,
            subscribers: 1,
            takers_per_subscriber: 1,
        },
        &panics,
        0_u32,
    )
    .map_err(|(error, _)| std::io::Error::other(format!("{error:?}")))?;
    let frames = memory.frames(region, demand.frames_per_run, demand.frame_layout)?;
    let graph = plan
        .with_cleanup(app_cleanup.try_clone()?)
        .with_panic_diagnostics(panics.try_clone()?);
    let run = effect_prepare! { prepare; plan: graph, memory: memory, region: metadata, capacity: 1, frames: frames, }?;
    let environment = Arc::new(Cafe {
        queue,
        hub,
        subscription,
        served,
        sales,
        status,
        cleanup: app_cleanup,
    });
    let invocation = effect_prepare! { admit; run: &run, environment: Arc::clone(&environment), }?;
    let output = futures_executor::block_on(invocation.run());
    let rendered = output
        .map(|outcome| match outcome {
            Exit::Success((snapshot, current)) => {
                let served = environment.served.get_now();
                let sales = environment.sales.read_now().map_err(std::io::Error::other)?;
                writeln!(
                    std::io::stdout().lock(),
                    "Café served {served} orders, earned {} cents, and finished at status {} (snapshot {})",
                    *sales, **current, *snapshot.current
                )
            }
            Exit::Fail(_)
            | Exit::Die(_)
            | Exit::Interrupted
            | Exit::FinalizationFailed(_) => {
                writeln!(std::io::stdout().lock(), "Café order was not delivered")
            }
        })
        .map_err(|_| std::io::Error::other("prepared output still has another consumer"))?;
    run.close();
    match &*rendered {
        Ok(()) => Ok(()),
        Err(error) => Err(std::io::Error::other(error.to_string()).into()),
    }
}
