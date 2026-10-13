//! Sam inspects recorded pickup cups and files the scheduled closing checklist.
//!
//! Run: `cargo +1.99.0 run -p effect --example close_coffee_shop`.
//! An OrderPad is café state. Effect owns activation-scoped resources, addressed
//! dispatch, leased closing responsibility, and calendar planning. The local store
//! demonstrates ownership within this process; it is not a distributed deployment.
//! Input: ticket 104 has two cups waiting. Expected: inspection and a 17:00 UTC
//! checklist recording those same two cups. The supplied wall clock is advanced
//! to that recorded closing instant; this does not lock a physical café.

use effect::{
    effect_die, effect_map_error, effect_provide_some, effect_resource, effect_sync,
    effect_try_sync,
};
pub mod cafe_support;
use cafe_support::transport::request;
use cafe_support::{Cafe, key, limits, parse_policy, rpc_policy};
use effect::cluster::{
    ActivationId, ActivationOutcome, ClusterActivationStore, ClusterAddress, ClusterDefinition,
    ClusterEntityId, ClusterEntityType, ClusterOwner, ClusterScheduleError, ClusterScheduleId,
    ClusterShardId, ClusterStoreError, ConcurrentOccurrencePolicy, EntityActivationError,
    EntityResourcePlan, SingletonError, SingletonOutcome,
};
use effect::rpc::{OperationId, RpcVersion};
use effect::{
    CalendarTimeError, CalendarTransitionPolicy, Cron, CronSearchBudget, DayMatch, Effect, Exit,
    FixedOffsetZone, GapPolicy, JsonValue, MissedOccurrencePolicy, OverlapPolicy, PureCodec,
    ResourceValue, Scope, TestWallClock, UtcMillis, cluster, cluster_schedule, effect,
    effect_scope, entity, entity_resource, json_i64, json_string, rpc_operation, schema,
    schema_enum, service, singleton,
};
use std::convert::Infallible;
use std::num::NonZeroU32;
use std::num::NonZeroUsize;
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
    /// Closing ownership or the body failed.
    Closing(SingletonError<CounterProblem, ClusterStoreError, Infallible>),
    /// The bounded closing calendar could not be planned.
    Calendar(ClusterScheduleError<CalendarTimeError>),
}
/// This shift uses local café state acquired per ticket rather than a global business service.
type CoffeeShop = Cafe<OrderPad>;
/// Typed pickup response, separate from runtime ownership metadata.
fn status_schema() -> Result<PureCodec<OrderStatus, JsonValue>, effect::Defect> {
    schema! { struct OrderStatus { customer: json_string(), cups: json_i64() } policy: parse_policy() }.map_err(|error| effect::Defect::new(format!("pickup status: {error:?}")))
}
/// Customer error codec preserves the ticket mismatch as business feedback.
fn problem_schema() -> Result<PureCodec<CounterProblem, JsonValue>, effect::Defect> {
    schema_enum! {
        enum CounterProblem { UnknownTicket => "unknown-ticket" }
        tag_field: "problem", content_field: "details", policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("counter problem: {error:?}")))
}
/// Reusable activation resource helper opens and releases the barista's ticket pad.
fn pickup_pad() -> EntityResourcePlan<ResourceValue<Arc<OrderPad>>, ClosingProblem, CoffeeShop> {
    entity_resource! {
        acquire |_activation| {
        let pad = service! { env: |cafe: &CoffeeShop| cafe.services.clone(), }?;
        use std::io::Write;
        let (pad, written) = {
            let written = writeln!(std::io::stdout().lock(), "Sam opens the recorded pickup pad for ticket {}.", pad.number);
            (pad, written)
        };
        match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
        effect_resource! {
            binding; acquire: effect_sync! { once; body: move || pad, }, release: |_, _| effect_resource! {
            release_effect; program: effect! {
            use std::io::Write;
            let written = writeln!(std::io::stdout().lock(), "Sam puts away the recorded pickup pad.");
            match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
        }, }, }?
        },
    }
}
/// The addressed handler reads its acquired pad and accepts only that ticket number.
fn pickup_status(
    pad: Arc<ResourceValue<Arc<OrderPad>>>,
    number: String,
) -> Effect<OrderStatus, CounterProblem, CoffeeShop> {
    effect! {
        let pad = pad.clone(); let number = number.clone();
        let status = pad.access(move |pad| {
            if pad.number == number { Ok(OrderStatus { customer: pad.customer.clone(), cups: pad.cups }) }
            else { Err(CounterProblem::UnknownTicket) }
        })?;
        let status = effect_try_sync! { once; body: { let __dsl_result = status; move || __dsl_result }, on_error: ::core::convert::identity, }?;
        use std::io::Write;
        let (status, written) = {
            let written = writeln!(std::io::stdout().lock(), "Pickup pad: {} has {} cups ready.", status.customer, status.cups);
            (status, written)
        };
        match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
        status
    }
}
/// One order activation owns the pad until every addressed request in this scope finishes.
fn check_pickup_pad() -> Effect<(), ClosingProblem, CoffeeShop> {
    effect_scope! {
        body |scope: Scope<ClosingProblem>| {
            let scope = scope.clone();
            let ticket_type = match ClusterEntityType::new("coffee-pickup-ticket") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("ticket type: {error:?}")), }?, };
            let pickup_operation_id = match OperationId::new("pickup-status") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("operation: {error:?}")), }?, };
            let pickup_status_codec = match status_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
            let pickup_error_codec = match problem_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
            let status = rpc_operation! {
                id: pickup_operation_id, version: RpcVersion::new(NonZeroU32::MIN),
                request: json_string(), success: pickup_status_codec, error: pickup_error_codec, limits: limits() };
            let wire_request = request(&status, "maya-pickup", &String::from("104"));
            let wire_request = match wire_request { Ok(request) => request, Err(error) => effect_die! { defect: error, }?, };
            let pad = entity! { entity_type: ticket_type.clone(), resources: pickup_pad(), handlers: [status => pickup_status] };
            let counter_type = match ClusterEntityType::new("coffee-counter") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("counter type: {error:?}")), }?, };
            let cups_operation_id = match OperationId::new("waiting-cups") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("operation: {error:?}")), }?, };
            let cups_error_codec = match problem_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
            let cups = rpc_operation! {
                id: cups_operation_id, version: RpcVersion::new(NonZeroU32::MIN),
                request: json_string(), success: json_i64(), error: cups_error_codec, limits: limits(),
            };
            let counter_request = request(&cups, "sam-inspection", &String::from("Sam"));
            let counter_request = match counter_request { Ok(request) => request, Err(error) => effect_die! { defect: error, }?, };
            let notice_operation_id = match OperationId::new("counter-notice") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("operation: {error:?}")), }?, };
            let notice_error_codec = match problem_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
            let notice = rpc_operation! {
                id: notice_operation_id, version: RpcVersion::new(NonZeroU32::MIN),
                request: json_string(), success: json_string(), error: notice_error_codec, limits: limits(),
            };
            let notice_request = request(&notice, "sam-closing-notice", &String::from("Sam"));
            let notice_request = match notice_request { Ok(request) => request, Err(error) => effect_die! { defect: error, }?, };
            let counter = entity! {
                entity_type: counter_type.clone(), resources: entity_resource! { service! { env: |cafe: &CoffeeShop| cafe.services.clone(), }? },
                handlers: [
                    cups => |pad: Arc<Arc<OrderPad>>, _barista: String| {
                        use std::io::Write;
                        let (pad, written) = {
                            let written = writeln!(std::io::stdout().lock(), "Counter inspection: {} cups await {}.", pad.cups, pad.customer);
                            (pad, written)
                        };
                        match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
                        pad.cups
                    },
                    notice => |pad: Arc<Arc<OrderPad>>, barista: String| {
                        let notice = format!("{barista}: ticket {} is awaiting pickup before the 17:00 UTC closing check", pad.number);
                        use std::io::Write;
                        let (notice, written) = {
                            let written = writeln!(std::io::stdout().lock(), "Counter notice: {notice}.");
                            (notice, written)
                        };
                        match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
                        notice
                    },
                ],
            };
            let counter_address = ClusterAddress { entity_type: counter_type, entity: ClusterEntityId::from_stable(match key("front-counter") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, }), shard: ClusterShardId::from_stable(match key("cafe") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, }) };
            let catalog: ClusterDefinition<ClosingProblem, CoffeeShop> = match cluster! { policy: rpc_policy(), entities: [pad, counter] } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("café catalog: {error:?}")), }?, };
            let address = ClusterAddress { entity_type: ticket_type, entity: ClusterEntityId::from_stable(match key("order-104") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, }), shard: ClusterShardId::from_stable(match key("cafe") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, }) };
            let store = service! {
                select: |cafe: &CoffeeShop| cafe.activation.clone(),
                on_error: |problem: ClosingProblem| problem,
                run |store| { Arc::clone(store) }
            }?;
            let inspection_activation = ActivationId::from_stable(match key("sam-counter-inspection") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, });
            let counter_ownership = effect_map_error! { program: effect_provide_some! { program: store.activate(counter_address.clone(), inspection_activation, UtcMillis::new(0), UtcMillis::new(100)), project: |_: &CoffeeShop| (), }, map: ClosingProblem::Store, }?;
            let counter_owner = match counter_ownership {
                ActivationOutcome::Activated { lease, .. } => lease.owner,
                ActivationOutcome::AlreadyActive(_) | ActivationOutcome::Busy(_) => effect_try_sync! { body: { let __dsl_error = || ClosingProblem::Busy; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }?,
            };
            let live_counter = effect_map_error! { program: catalog.activate(scope.clone(), counter_owner), map: |problem| ClosingProblem::Pad(Box::new(problem)), }?;
            effect_map_error! { program: live_counter.execute(counter_address.clone(), Arc::from(counter_request)), map: ClosingProblem::Pickup, }?;
            effect_map_error! { program: live_counter.execute(counter_address, Arc::from(notice_request)), map: ClosingProblem::Pickup, }?;
            let order_activation = ActivationId::from_stable(match key("sam-order-104") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, });
            let ownership = effect_map_error! { program: effect_provide_some! { program: store.activate(address.clone(), order_activation, UtcMillis::new(0), UtcMillis::new(100)), project: |_: &CoffeeShop| (), }, map: ClosingProblem::Store, }?;
            let owner = match ownership {
                ActivationOutcome::Activated { lease, .. } => lease.owner,
                ActivationOutcome::AlreadyActive(_) | ActivationOutcome::Busy(_) => effect_try_sync! { body: { let __dsl_error = || ClosingProblem::Busy; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }?,
            };
            let live = effect_map_error! { program: catalog.activate(scope, owner), map: |problem| ClosingProblem::Pad(Box::new(problem)), }?;
            effect_map_error! { program: live.execute(address, Arc::from(wire_request)), map: ClosingProblem::Pickup, }?;
        }
    }
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
        let closing_schedule_key = match key("daily-counter-close") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let closing_calendar = match Cron::parse("0 0 17 * * *", DayMatch::Both) { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("daily closing time: {error:?}")), }?, };
        let schedule = cluster_schedule! {
            id: ClusterScheduleId::from_stable(closing_schedule_key),
            cron: closing_calendar,
            transitions: CalendarTransitionPolicy { gap: GapPolicy::Skip, overlap: OverlapPolicy::Earlier },
            catch_up: MissedOccurrencePolicy::Latest, concurrent: ConcurrentOccurrencePolicy::Skip, deadline_after: Duration::from_secs(60) };
        let zone = match FixedOffsetZone::east_seconds(0) { Ok(zone) => zone, Err(error) => effect_die! { defect: effect::Defect::new(format!("UTC café clock: {error:?}")), }?, };
        let due = effect_map_error! {
            program: effect_try_sync! {
            once; body: {
            let __dsl_result = schedule.plan_due(UtcMillis::new(17 * 60 * 60 * 1000 - 1), closing_time,
            &zone, CronSearchBudget { days: NonZeroUsize::MIN.saturating_add(2), candidates: NonZeroUsize::MIN.saturating_add(3) }, false); move || __dsl_result }, on_error: ::core::convert::identity, }, map: ClosingProblem::Calendar, }?;
        for occurrence in due {
            { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "The daily 17:00 UTC closing task is due ({:?}).", occurrence.disposition); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
            let closing_entity_type = match ClusterEntityType::new("cafe-closing-shift") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("closing type: {error:?}")), }?, };
            let closing_entity_key = match key("front-counter-close") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
            let closing_shard_key = match key("cafe") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
            let closing_owner_key = match key("sam-closing-shift") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
            let closed = effect_map_error! {
                program: singleton! {
                singleton: singleton! { 
                    address: ClusterAddress {
                    entity_type: closing_entity_type,
                    entity: ClusterEntityId::from_stable(closing_entity_key), shard: ClusterShardId::from_stable(closing_shard_key) }, },
                activation: ActivationId::from_stable(closing_owner_key), lease_until: UtcMillis::new(17 * 60 * 60 * 1000 + 100), run |_owner: ClusterOwner| {
                    let pad = service! { env: |cafe: &CoffeeShop| cafe.services.clone(), }?;
                    ClosingSummary { barista: "Sam".into(), waiting_cups: pad.cups }
                }
            }, map: ClosingProblem::Closing, }?;
            match closed {
                SingletonOutcome::Ran(summary) => { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "{} files the closing checklist: {} cups awaiting pickup; contact the waiting customer before locking up.", summary.barista, summary.waiting_cups); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } },
                SingletonOutcome::Busy(_) => { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Another barista is completing the closing sheet."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } },
            }
        }
    }
}
/// Select local providers, then run one finite closing shift.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cafe: Arc<CoffeeShop> = Cafe::open(
        OrderPad {
            number: "104".into(),
            customer: "Maya".into(),
            cups: 2,
        },
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    match futures_executor::block_on(closing_shift().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("Café closing failed: {exit:?}").into());
        }
    }
    cafe.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    Ok(())
}
