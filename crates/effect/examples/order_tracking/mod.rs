//! Application models, codecs and one order-desk lookup shared by RPC examples.

use super::cafe_support::{Cafe, parse_policy};
use effect::effect_die;
use effect::{Effect, EffectJsonSchema, effect_fail, effect_from_option, json_i64, json_string, service};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
/// The selected desk is supplied beside local Effect providers.
pub type CoffeeShop = Cafe<OrderDesk>;
/// Drinks sold by the order desk.
#[derive(Clone, Debug, EffectJsonSchema)]
#[effect_json_schema(
    factory = drink_schema,
    policy = parse_policy(),
    error_context = "drink schema",
    tag_field = "drink",
    content_field = "options",
)]
pub enum Drink {
    /// Dairy-free espresso drink.
    #[effect_json_schema(tag = "oat-latte")]
    OatLatte,
    /// Espresso and water.
    #[effect_json_schema(tag = "americano")]
    Americano,
}
/// Owned request sent by a customer; no Clone bound is required.
#[derive(Debug, EffectJsonSchema)]
#[effect_json_schema(factory = order_schema, policy = parse_policy(), error_context = "order schema")]
pub struct CoffeeOrder {
    /// Name for the pickup counter.
    #[effect_json_schema(codec = json_string())]
    pub customer: String,
    /// Requested drink.
    #[effect_json_schema(codec = drink_schema()?)]
    pub drink: Drink,
    /// Requested cups.
    #[effect_json_schema(codec = json_i64())]
    pub quantity: i64,
}
/// Ticket returned by the café and looked up by its order number.
#[derive(Clone, Debug, EffectJsonSchema)]
#[effect_json_schema(factory = ticket_schema, policy = parse_policy(), error_context = "ticket schema")]
pub struct OrderTicket {
    /// Number printed on the pickup ticket.
    #[effect_json_schema(codec = json_string())]
    pub number: String,
    /// Name to call when ready.
    #[effect_json_schema(codec = json_string())]
    pub customer: String,
    /// Accepted drink.
    #[effect_json_schema(codec = drink_schema()?)]
    pub drink: Drink,
    /// Accepted cup count.
    #[effect_json_schema(codec = json_i64())]
    pub quantity: i64,
}
/// Lookup/watch requests share the customer's ticket identity.
#[derive(Debug, EffectJsonSchema)]
#[effect_json_schema(factory = find_schema, policy = parse_policy(), error_context = "lookup schema")]
pub struct FindOrder {
    /// Pickup ticket number, e.g. 104.
    #[effect_json_schema(codec = json_string())]
    pub number: String,
}
/// Human-readable lifecycle shown on Maya's phone.
#[derive(Debug, EffectJsonSchema)]
#[effect_json_schema(
    factory = status_schema,
    policy = parse_policy(),
    error_context = "status schema",
    tag_field = "status",
    content_field = "details",
)]
pub enum OrderStatus {
    /// The counter accepted the order.
    #[effect_json_schema(tag = "accepted")]
    Accepted,
    /// A barista is making the drinks.
    #[effect_json_schema(tag = "preparing")]
    Preparing,
    /// Drinks are waiting at the pickup counter.
    #[effect_json_schema(tag = "ready")]
    Ready {
        /// Customer to call.
        #[effect_json_schema(codec = json_string())]
        customer: String,
        /// Recorded completed cups.
        #[effect_json_schema(codec = json_i64())]
        cups: i64,
    },
}
/// The café's application service; this is not an Effect runtime or store engine.
pub struct OrderDesk {
    /// Accepted ticket snapshots indexed by printed number.
    pub tickets: Mutex<BTreeMap<String, TrackedOrder>>,
}
/// Business problems understood by the customer or staff.
#[derive(Debug, EffectJsonSchema)]
#[effect_json_schema(
    factory = problem_schema,
    policy = parse_policy(),
    error_context = "problem schema",
    tag_field = "problem",
    content_field = "details",
)]
pub enum OrderProblem {
    /// The requested ticket is unknown.
    #[effect_json_schema(tag = "unknown-ticket")]
    UnknownTicket,
    /// The counter only accepts one through four cups per request.
    #[effect_json_schema(tag = "invalid-quantity")]
    InvalidQuantity,
    /// Staff have closed the counter.
    #[effect_json_schema(tag = "counter-closed")]
    CounterClosed,
    /// The desk has no recorded stages for this ticket.
    #[effect_json_schema(tag = "no-history")]
    NoHistory,
}
/// Select the café desk with service! and look up the ticket when the recipe executes.
pub fn lookup_order(request: FindOrder) -> Effect<OrderTicket, OrderProblem, Arc<OrderDesk>> {
    service! {
        select: |desk: &Arc<OrderDesk>| desk.clone(),
        run |desk: &Arc<OrderDesk>| {
            let number = request.number.clone();
            let found = desk.tickets.lock()
                .map(|records| records.get(&number).map(|record| record.ticket.clone()))
                .map_err(|error| effect::Defect::new(error.to_string()));
            let found = match found { Ok(found) => found, Err(error) => effect_die! { defect: error, }?, };
            match found {
                Some(ticket) => ticket,
                None => effect_fail! { error: OrderProblem::UnknownTicket, }?,
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
        select: |desk: &Arc<OrderDesk>| desk.clone(),
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
