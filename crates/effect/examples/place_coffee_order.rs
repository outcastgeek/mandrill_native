//! Submit a two-cup order and retrieve its accepted pickup ticket through local RPC.
//!
//! Run: `cargo +1.99.0 run -p effect --example place_coffee_order`.
//! Input: ` Maya ` requests two oat lattes. Output: accepted ticket 104 for Maya.
//! Request middleware trims the pickup name; the handler validates cup count
//! and saves the actual ticket in the selected local order desk.

use effect::{effect_die, effect_try_sync};
pub mod cafe_support;
use std::num::NonZeroUsize;
pub mod order_tracking;
use cafe_support::transport::{request, response};
use cafe_support::{Cafe, limits, parse_policy, rpc_policy};
use effect::rpc::{OperationId, RpcProtocolError, RpcServerMiddlewareError, RpcVersion};
use effect::{
    Effect, Exit, effect, rpc_middleware, rpc_operation, rpc_protocol, schema_enum, service,
};
use order_tracking::{
    CoffeeOrder, CoffeeShop, Drink, OrderDesk, OrderProblem, OrderTicket, Stage, TrackedOrder,
    order_schema, problem_schema, ticket_schema,
};
use std::{
    collections::BTreeMap,
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

/// Normalize the customer's input, store a validated order, and acknowledge its ticket.
fn place_order() -> Effect<(), RpcProtocolError<OrderError>, CoffeeShop> {
    effect! {
        let operation_id = match OperationId::new("place-order") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("operation: {error:?}")), }?, };
        let order_codec = match order_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let accepted_ticket_codec = match ticket_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let domain_error_codec = match problem_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let accept = rpc_operation! {
            id: operation_id,
            version: RpcVersion::new(NonZeroU32::MIN), request: order_codec,
            success: accepted_ticket_codec, error: domain_error_codec, limits: limits(),
            handler |order: CoffeeOrder| {
                if !(1..=4).contains(&order.quantity) {
                    { let __dsl_program: Effect < () , OrderProblem , CoffeeShop > = effect_try_sync! { body: { let __dsl_error = || OrderProblem::InvalidQuantity; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; __dsl_program }?;
                }
                let desk = service! { env |env: &CoffeeShop| { env.services.clone() }, }?;
                let ticket = OrderTicket {
                    number: "104".into(), customer: order.customer,
                    drink: order.drink, quantity: order.quantity };
                let saved = desk.tickets.lock().map(|mut records| records.insert(ticket.number.clone(), TrackedOrder { ticket: ticket.clone(), history: vec![Stage::Accepted] })).map_err(|error| effect::Defect::new(error.to_string()));
                match saved { Ok(_) => (), Err(error) => effect_die! { defect: error, }?, };
                ticket
            },
        };
        let problem = match problem_schema() { Ok(problem) => problem, Err(error) => effect_die! { defect: error, }?, };
        let errors = match schema_enum! {
            enum OrderError {
                Before(problem.clone()) => "input",
                Domain(problem.clone()) => "order",
                After(problem.clone()) => "acknowledgement",
            } tag_field: "phase", content_field: "problem", policy: parse_policy()
        } { Ok(errors) => errors, Err(error) => effect_die! { defect: effect::Defect::new(format!("middleware codec: {error:?}")), }?, };
        let accept = rpc_middleware! {
            handler: accept, error_codec: errors,
            before |order: CoffeeOrder| { CoffeeOrder { customer: order.customer.trim().to_owned(), ..order } },
            after |ticket: OrderTicket| {
                use std::io::Write;
                let (ticket, written) = {
                    let written = writeln!(std::io::stdout().lock(), "Accepted ticket {}: {} requests {} {:?} cups.", ticket.number, ticket.customer, ticket.quantity, ticket.drink);
                    (ticket, written)
                };
                match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
                ticket
            },
        };
        let operation = accept.operation().clone();
        let submitted = request(&operation, "maya-order", &CoffeeOrder {
            customer: " Maya ".into(), drink: Drink::OatLatte, quantity: 2,
        });
        let submitted = match submitted { Ok(submitted) => submitted, Err(error) => effect_die! { defect: error, }?, };
        let protocol = match rpc_protocol! { policy: rpc_policy(), handlers: [accept] } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("order protocol: {error:?}")), }?, };
        let received = protocol.handle(submitted)?;
        let ticket = match response(&operation, &received) { Ok(ticket) => ticket, Err(error) => effect_die! { defect: error, }?, };
        assert_eq!(ticket.customer, "Maya");
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Maya receives accepted ticket {} through the typed response.", ticket.number); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
    }
}
/// Request and acknowledgement failures remain distinct from order rejection.
type OrderError = RpcServerMiddlewareError<OrderProblem, OrderProblem>;
/// Execute against a fresh local desk and join admitted provider work.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env: Arc<CoffeeShop> = Cafe::open(
        OrderDesk {
            tickets: Mutex::new(BTreeMap::new()),
        },
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(place_order().run(env.clone()));
    env.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => (),
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("order acceptance failed: {other:?}").into());
        }
    }
    Ok(())
}
