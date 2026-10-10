//! Sam checks Maya's pickup pad and closes the café counter at its scheduled time.
//!
//! Run: `cargo run -p effect --example close_coffee_shop`.
//! An OrderPad is café state. Effect owns activation-scoped resources, addressed
//! dispatch, leased closing responsibility, and calendar planning. The local store
//! demonstrates ownership within this process; it is not a distributed deployment.
pub mod cafe_support;
use cafe_support::transport::request;
use cafe_support::{Cafe, key, limits, nz, parse_policy, rpc_policy};
use effect::cluster::{
    ActivationId, ActivationOutcome, ClusterActivationStore, ClusterAddress, ClusterDefinition,
    ClusterEntityId, ClusterEntityType, ClusterExecutor, ClusterOwner, ClusterScheduleError,
    ClusterScheduleId, ClusterShardId, ClusterSingleton, ClusterStoreError,
    ConcurrentOccurrencePolicy, EntityActivationError, EntityDispatchError, EntityResourcePlan,
    SingletonError, SingletonOutcome,
};
use effect::rpc::{OperationId, RpcVersion};
use effect::{
    CalendarTimeError, CalendarTransitionPolicy, Cron, CronSearchBudget, DayMatch, Effect, Exit,
    FixedOffsetZone, GapPolicy, JsonValue, MissedOccurrencePolicy, OverlapPolicy, PureCodec,
    Release, ResourceBinding, ResourceValue, Scope, TestWallClock, UtcMillis, cluster,
    cluster_schedule, effect, entity, entity_resource, json_i64, json_string, rpc_operation,
    schema, schema_enum, service, singleton,
};
use std::convert::Infallible;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

/// A barista's pickup pad is acquired for one ticket's active ownership.
pub struct OrderPad {
    /// Printed order number.
    number: String,
    /// Pickup name.
    customer: String,
    /// Completed cups waiting for pickup.
    cups: i64,
}
/// Customer status returned by the addressed pickup counter.
#[derive(Debug)]
pub struct OrderStatus {
    /// Customer whose drinks are ready.
    customer: String,
    /// Ready cup count.
    cups: i64,
}
/// Daily staff result after the final counter inspection.
#[derive(Debug)]
pub struct ClosingSummary {
    /// Staff member responsible for locking the counter.
    barista: String,
    /// Cups left for collection before locking up.
    waiting_cups: i64,
}
/// Domain feedback stays distinct from resource and ownership errors.
#[derive(Debug)]
pub enum CounterProblem {
    /// A request was sent to a different ticket's pickup pad.
    UnknownTicket,
}
/// Acquisition failures can include a nested activation error without losing its cause.
type PadActivationError = EntityActivationError<ClosingProblem, ClusterStoreError, Infallible>;
/// Staff recipe retains store, acquisition, dispatch, calendar, and singleton details.
#[derive(Debug)]
pub enum ClosingProblem {
    /// The ownership store failed.
    Store(ClusterStoreError),
    /// Another barista currently owns the ticket.
    Busy,
    /// Acquiring the ticket's resources failed.
    Pad(Box<PadActivationError>),
    /// Checked addressed dispatch failed.
    Pickup(EntityActivationError<Infallible, ClusterStoreError, Infallible>),
    /// Legacy counter dispatch failed.
    Counter(EntityDispatchError),
    /// Closing ownership or the body failed.
    Closing(SingletonError<CounterProblem, ClusterStoreError, Infallible>),
    /// The bounded closing calendar could not be planned.
    Calendar(ClusterScheduleError<CalendarTimeError>),
}
/// This shift uses local café state acquired per ticket rather than a global business service.
type CoffeeShop = Cafe;
/// Typed pickup response, separate from runtime ownership metadata.
fn status_schema() -> PureCodec<OrderStatus, JsonValue> {
    schema! { struct OrderStatus { customer: json_string(), cups: json_i64() } policy: parse_policy() }.expect("pickup status")
}
/// Customer error codec preserves the ticket mismatch as business feedback.
fn problem_schema() -> PureCodec<CounterProblem, JsonValue> {
    schema_enum! {
        enum CounterProblem { UnknownTicket => "unknown-ticket" }
        tag_field: "problem", content_field: "details", policy: parse_policy()
    }
    .expect("counter problem")
}
/// Reusable activation resource helper opens and releases the barista's ticket pad.
fn pickup_pad() -> EntityResourcePlan<ResourceValue<OrderPad>, ClosingProblem, CoffeeShop> {
    entity_resource! {
        ResourceBinding::new(Effect::sync(|| {
            println!("Sam opens the pickup pad for Maya's ticket 104.");
            OrderPad { number: "104".into(), customer: "Maya".into(), cups: 2 }
        }), |_, _| Release::sync(|| println!("Sam puts away Maya's pickup pad when its activation scope closes.")))?
    }
}
/// The addressed handler reads its acquired pad and accepts only that ticket number.
fn pickup_status(
    pad: Arc<ResourceValue<OrderPad>>,
    number: String,
) -> Effect<OrderStatus, CounterProblem, CoffeeShop> {
    effect! {
        let pad = pad.clone(); let number = number.clone();
        let status = pad.access(move |pad| {
            if pad.number == number { Ok(OrderStatus { customer: pad.customer.clone(), cups: pad.cups }) }
            else { Err(CounterProblem::UnknownTicket) }
        })?;
        let status = Effect::from_result_once(status)?;
        println!("Pickup pad: {} has {} cups ready.", status.customer, status.cups);
        status
    }
}
/// One order activation owns the pad until every addressed request in this scope finishes.
fn check_pickup_pad() -> Effect<(), ClosingProblem, CoffeeShop> {
    Scope::scoped(|scope| {
        effect! {
            let scope = scope.clone();
            let ticket_type = ClusterEntityType::new("coffee-pickup-ticket").expect("ticket type");
            let status = rpc_operation! {
                id: OperationId::new("pickup-status").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
                request: json_string(), success: status_schema(), error: problem_schema(), limits: limits() };
            let wire_request = request(&status, "maya-pickup", &String::from("104"));
            let pad = entity! { entity_type: ticket_type.clone(), resources: pickup_pad(), handlers: [status => pickup_status] };
            let counter_type = ClusterEntityType::new("coffee-counter").expect("counter type");
            let cups = rpc_operation! {
                id: OperationId::new("waiting-cups").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
                request: json_string(), success: json_i64(), error: problem_schema(), limits: limits(), handler |_barista: String| {
                    println!("Counter inspection: two oat-latte cups await Maya."); 2i64
                }
            };
            let counter_request = request(cups.operation(), "sam-inspection", &String::from("Sam"));
            let mut counter = entity! { policy: rpc_policy(), entity_type: counter_type.clone(), handler: cups }.expect("counter registration");
            let notice = rpc_operation! {
                id: OperationId::new("counter-notice").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
                request: json_string(), success: json_string(), error: problem_schema(), limits: limits(), handler |barista: String| {
                    let notice = format!("{barista}: pickup counter closes at 17:00 UTC");
                    println!("Counter notice: {notice}."); notice
                }
            };
            let notice_request = request(notice.operation(), "sam-closing-notice", &String::from("Sam"));
            entity! { dispatcher: &mut counter, entity_type: counter_type.clone(), handler: notice }.expect("counter notice registration");
            let counter_address = ClusterAddress { entity_type: counter_type.clone(), entity: ClusterEntityId::from_stable(key("front-counter")), shard: ClusterShardId::from_stable(key("cafe")) };
            counter.execute(counter_address.clone(), Arc::from(counter_request)).map_error(ClosingProblem::Counter)?;
            counter.execute(counter_address, Arc::from(notice_request)).map_error(ClosingProblem::Counter)?;
            let catalog: ClusterDefinition<ClosingProblem, CoffeeShop> = cluster! { policy: rpc_policy(), entities: [pad, (counter_type, counter)] }.expect("café catalog");
            let address = ClusterAddress { entity_type: ticket_type, entity: ClusterEntityId::from_stable(key("order-104")), shard: ClusterShardId::from_stable(key("cafe")) };
            let store = service! {
                select: |cafe: &CoffeeShop| cafe.activation.clone(),
                on_error: |problem: ClosingProblem| problem,
                run |store| { Arc::clone(store) }
            }?;
            let ownership = store.activate(address.clone(), ActivationId::from_stable(key("sam-order-104")), UtcMillis::new(0), UtcMillis::new(100))
                .provide_some(|_: &CoffeeShop| ()).map_error(ClosingProblem::Store)?;
            let owner = match ownership {
                ActivationOutcome::Activated { lease, .. } => lease.owner,
                _ => Effect::fail_with(|| ClosingProblem::Busy)?,
            };
            let live = catalog.activate(scope, owner).map_error(|problem| ClosingProblem::Pad(Box::new(problem)))?;
            live.execute(address, Arc::from(wire_request)).map_error(ClosingProblem::Pickup)?;
        }
    })
}
/// Plan the daily 17:00 UTC closing occurrence, then assign its work to one leased barista.
fn closing_shift() -> Effect<(), ClosingProblem, CoffeeShop> {
    effect! {
        check_pickup_pad()?;
        let closing_time = UtcMillis::new(17 * 60 * 60 * 1000);
        service! {
            select: |cafe: &CoffeeShop| cafe.wall.clone(),
            on_error: |problem: ClosingProblem| problem,
            run |clock: &TestWallClock| { clock.set(closing_time); }
        }?;
        let schedule = cluster_schedule! {
            id: ClusterScheduleId::from_stable(key("daily-counter-close")),
            cron: Cron::parse("0 0 17 * * *", DayMatch::Both).expect("daily closing time"),
            transitions: CalendarTransitionPolicy { gap: GapPolicy::Skip, overlap: OverlapPolicy::Earlier },
            catch_up: MissedOccurrencePolicy::Latest, concurrent: ConcurrentOccurrencePolicy::Skip, deadline_after: Duration::from_secs(60) };
        let due = Effect::from_result_once(schedule.plan_due(UtcMillis::new(17 * 60 * 60 * 1000 - 1), closing_time,
            &FixedOffsetZone::east_seconds(0).expect("UTC café clock"), CronSearchBudget { days: nz(3), candidates: nz(4) }, false)).map_error(ClosingProblem::Calendar)?;
        for occurrence in due {
            println!("The daily 17:00 UTC closing task is due ({:?}).", occurrence.disposition);
            let closed = singleton! {
                singleton: ClusterSingleton::new(ClusterAddress {
                    entity_type: ClusterEntityType::new("cafe-closing-shift").expect("closing type"),
                    entity: ClusterEntityId::from_stable(key("front-counter-close")), shard: ClusterShardId::from_stable(key("cafe")) }),
                activation: ActivationId::from_stable(key("sam-closing-shift")), lease_until: UtcMillis::new(17 * 60 * 60 * 1000 + 100), run |_owner: ClusterOwner| {
                    Effect::<ClosingSummary, CounterProblem, CoffeeShop>::cpu_once(|_| ClosingSummary { barista: "Sam".into(), waiting_cups: 2 })?
                }
            }.map_error(ClosingProblem::Closing)?;
            match closed {
                SingletonOutcome::Ran(summary) => println!("{} completes the closing sheet: {} cups awaiting pickup; call Maya before locking up.", summary.barista, summary.waiting_cups),
                SingletonOutcome::Busy(_) => println!("Another barista is completing the closing sheet."),
            }
        }
    }
}
/// Select local providers, then run one finite closing shift.
fn main() {
    let cafe: Arc<CoffeeShop> = Cafe::open((), nz(16), nz(4));
    match futures_executor::block_on(closing_shift().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit => panic!("Café closing failed: {exit:?}"),
    }
    cafe.base.finish();
}
