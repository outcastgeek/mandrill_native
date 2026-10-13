//! Application models, codecs and one order-desk lookup shared by RPC examples.

use super::cafe_support::{Cafe, parse_policy};
use effect::effect_die;
use effect::{
    Effect, JsonValue, PureCodec, effect_from_option, json_i64, json_string, schema, schema_enum,
    service,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
/// The selected desk is supplied beside local Effect providers.
pub type CoffeeShop = Cafe<OrderDesk>;
/// Drinks sold by the order desk.
#[derive(Clone, Debug)]
pub enum Drink {
    /// Dairy-free espresso drink.
    OatLatte,
    /// Espresso and water.
    Americano,
}
/// Owned request sent by a customer; no Clone bound is required.
#[derive(Debug)]
pub struct CoffeeOrder {
    /// Name for the pickup counter.
    pub customer: String,
    /// Requested drink.
    pub drink: Drink,
    /// Requested cups.
    pub quantity: i64,
}
/// Ticket returned by the café and looked up by its order number.
#[derive(Clone, Debug)]
pub struct OrderTicket {
    /// Number printed on the pickup ticket.
    pub number: String,
    /// Name to call when ready.
    pub customer: String,
    /// Accepted drink.
    pub drink: Drink,
    /// Accepted cup count.
    pub quantity: i64,
}
/// Lookup/watch requests share the customer's ticket identity.
#[derive(Debug)]
pub struct FindOrder {
    /// Pickup ticket number, e.g. 104.
    pub number: String,
}
/// Human-readable lifecycle shown on Maya's phone.
#[derive(Debug)]
pub enum OrderStatus {
    /// The counter accepted the order.
    Accepted,
    /// A barista is making the drinks.
    Preparing,
    /// Drinks are waiting at the pickup counter.
    Ready {
        /// Customer to call.
        customer: String,
        /// Recorded completed cups.
        cups: i64,
    },
}
/// The café's application service; this is not an Effect runtime or store engine.
pub struct OrderDesk {
    /// Accepted ticket snapshots indexed by printed number.
    pub tickets: Mutex<BTreeMap<String, TrackedOrder>>,
}
/// Business problems understood by the customer or staff.
#[derive(Debug)]
pub enum OrderProblem {
    /// The requested ticket is unknown.
    UnknownTicket,
    /// The counter only accepts one through four cups per request.
    InvalidQuantity,
    /// Staff have closed the counter.
    CounterClosed,
    /// The desk has no recorded stages for this ticket.
    NoHistory,
}
/// Explicit drink names define the wire vocabulary.
pub fn drink_schema() -> Result<PureCodec<Drink, JsonValue>, effect::Defect> {
    schema_enum! {
        enum Drink { OatLatte => "oat-latte", Americano => "americano" }
        tag_field: "drink", content_field: "options", policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("drink schema: {error:?}")))
}
/// Customer request shape, separate from its business validation.
pub fn order_schema() -> Result<PureCodec<CoffeeOrder, JsonValue>, effect::Defect> {
    let drink = drink_schema()?;
    schema! {
        struct CoffeeOrder { customer: json_string(), drink: drink, quantity: json_i64() }
        policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("order schema: {error:?}")))
}
/// Accepted ticket shape shared by placement and lookup handlers.
pub fn ticket_schema() -> Result<PureCodec<OrderTicket, JsonValue>, effect::Defect> {
    let drink = drink_schema()?;
    schema! {
        struct OrderTicket {
            number: json_string(), customer: json_string(), drink: drink, quantity: json_i64()
        } policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("ticket schema: {error:?}")))
}
/// Typed identity for both lookup and watch operations.
pub fn find_schema() -> Result<PureCodec<FindOrder, JsonValue>, effect::Defect> {
    schema! { struct FindOrder { number: json_string() } policy: parse_policy() }
        .map_err(|error| effect::Defect::new(format!("lookup schema: {error:?}")))
}
/// Lifecycle chunks and the terminal ready message use the same model.
pub fn status_schema() -> Result<PureCodec<OrderStatus, JsonValue>, effect::Defect> {
    schema_enum! {
        enum OrderStatus {
            Accepted => "accepted", Preparing => "preparing", Ready { customer: json_string(), cups: json_i64() } => "ready"
        } tag_field: "status", content_field: "details", policy: parse_policy()
    }.map_err(|error| effect::Defect::new(format!("status schema: {error:?}")))
}
/// Preserve typed customer feedback in protocol error replies.
pub fn problem_schema() -> Result<PureCodec<OrderProblem, JsonValue>, effect::Defect> {
    schema_enum! {
        enum OrderProblem {
            UnknownTicket => "unknown-ticket", InvalidQuantity => "invalid-quantity", CounterClosed => "counter-closed", NoHistory => "no-history"
        } tag_field: "problem", content_field: "details", policy: parse_policy()
    }.map_err(|error| effect::Defect::new(format!("problem schema: {error:?}")))
}
/// Select the café desk with service! and look up the ticket when the recipe executes.
pub fn lookup_order(request: FindOrder) -> Effect<OrderTicket, OrderProblem, Arc<OrderDesk>> {
    service! {
        select: |desk: &Arc<OrderDesk>| desk.clone(),
        on_error: |problem: OrderProblem| problem,
        run |desk: &Arc<OrderDesk>| {
            let number = request.number.clone();
            let found = desk.tickets.lock()
                .map(|records| records.get(&number).map(|record| record.ticket.clone()))
                .map_err(|error| effect::Defect::new(error.to_string()));
            let found = match found { Ok(found) => found, Err(error) => effect_die! { defect: error, }?, };
            match found {
                Some(ticket) => ticket,
                None => effect_from_option! {
                    once; value: None::<OrderTicket>,
                    missing { OrderProblem::UnknownTicket },
                }?,
            }
        }
    }
}

/// Saved ticket plus its actual recorded preparation stages.
pub struct TrackedOrder {
    /// Accepted order data supplied by the desk.
    pub ticket: OrderTicket,
    /// Recorded stages for this specific ticket, in chronological order.
    pub history: Vec<Stage>,
}
/// A recorded stage; readiness is supplied by the input, never inferred from existence.
#[derive(Clone, Copy)]
pub enum Stage {
    /// Order accepted at the counter.
    Accepted,
    /// Preparation started.
    Preparing,
    /// Completed cups arrived at pickup.
    Ready,
}
/// Read this ticket's stored history and attach its actual customer/cup data.
pub fn status_history(
    request: FindOrder,
) -> Effect<Vec<OrderStatus>, OrderProblem, Arc<OrderDesk>> {
    service! {
        select: |desk: &Arc<OrderDesk>| desk.clone(), on_error: |error: OrderProblem| error,
        run |desk: &Arc<OrderDesk>| {
            let history = desk.tickets.lock().map(|records| records.get(&request.number).map(|record| {
                record.history.iter().map(|stage| match stage {
                    Stage::Accepted => OrderStatus::Accepted,
                    Stage::Preparing => OrderStatus::Preparing,
                    Stage::Ready => OrderStatus::Ready { customer: record.ticket.customer.clone(), cups: record.ticket.quantity },
                }).collect::<Vec<_>>()
            })).map_err(|error| effect::Defect::new(error.to_string()));
            let history = match history { Ok(history) => history, Err(error) => effect_die! { defect: error, }?, };
            effect_from_option! { once; value: history, missing { OrderProblem::UnknownTicket }, }?

        },
    }
}
