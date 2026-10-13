//! Replay one ticket's accepted/preparing/ready history to its customer.
//!
//! Run: `cargo +1.99.0 run -p effect --example track_coffee_order`.
//! Input: a request for ticket ` 104 ` and Maya's saved two-cup order.
//! Output: accepted, preparing, then Maya's pickup announcement. The inline
//! middleware normalizes the number and displays each pulled status exactly once.
//! This local history demonstration makes no claim to control a coffee machine.

use effect::effect_die;
pub mod cafe_support;
use std::num::NonZeroUsize;
pub mod order_tracking;
use cafe_support::transport::{Session, sequence, stream_request};
use cafe_support::{Cafe, limits, parse_policy, rpc_policy};
use effect::rpc::{
    OperationId, RpcStreamMiddlewareError, RpcStreamProtocol, RpcStreamProtocolError, RpcVersion,
};
use effect::{
    Effect, Exit, effect, effect_from_option, rpc_stream_middleware, rpc_stream_operation,
    rpc_stream_protocol, schema_enum,
};
use order_tracking::{
    CoffeeShop, Drink, FindOrder, OrderDesk, OrderProblem, OrderStatus, OrderTicket, Stage,
    TrackedOrder, find_schema, problem_schema, status_history, status_schema,
};
use std::{
    collections::BTreeMap,
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

/// A single subscription owns normalization, pull-driven display and terminal pickup.
fn follow_order() -> Effect<(), RpcStreamProtocolError<StatusError, ()>, CoffeeShop> {
    effect! {
        let watch_operation_id = match OperationId::new("follow-pickup") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("operation: {error:?}")), }?, };
        let lookup_codec = match find_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let chunk_codec = match status_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let terminal_codec = match status_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let lookup_errors = match problem_schema() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let watch = rpc_stream_operation! {
            id: watch_operation_id,
            version: RpcVersion::new(NonZeroU32::MIN), request: lookup_codec,
            chunk: chunk_codec, terminal: terminal_codec, error: lookup_errors, limits: limits(),
            handler |request: FindOrder, _context| {
                let mut history = status_history(request)?;
                let terminal = effect_from_option! {
                    once; value: history.pop(), missing { OrderProblem::NoHistory },
                }?;
                sequence(history, terminal)
            },
            project |env: &CoffeeShop| { env.services.clone() },
        };
        let problem = match problem_schema() { Ok(problem) => problem, Err(error) => effect_die! { defect: error, }?, };
        let errors = match schema_enum! {
            enum StatusError {
                Before(problem.clone()) => "request",
                Domain(problem.clone()) => "lookup",
                Chunk(problem.clone()) => "display",
                Terminal(problem.clone()) => "pickup",
            } tag_field: "phase", content_field: "problem", policy: parse_policy()
        } { Ok(errors) => errors, Err(error) => effect_die! { defect: effect::Defect::new(format!("middleware codec: {error:?}")), }?, };
        let watch = rpc_stream_middleware! {
            handler: watch, error_codec: errors,
            before |request: FindOrder| { FindOrder { number: request.number.trim().to_owned() } },
            chunk |status: OrderStatus| {
                use std::io::Write;
                let (status, written) = {
                    let written = writeln!(std::io::stdout().lock(), "Ticket 104: {status:?}.");
                    (status, written)
                };
                match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
                status
            },
            terminal |status: OrderStatus| {
                let (status, text) = {
                    let text = match &status {
                        OrderStatus::Ready { customer, cups } => Some(format!("{customer}, your {cups} cups are ready at pickup.")),
                        OrderStatus::Accepted | OrderStatus::Preparing => None,
                    };
                    (status, text)
                };
                if let Some(text) = text {
                    use std::io::Write;
                    let written = writeln!(std::io::stdout().lock(), "{text}");
                    match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
                }
                status
            },
        };
        let encoded = stream_request(watch.operation(), "maya-phone", &FindOrder { number: " 104 ".into() });
        let encoded = match encoded { Ok(encoded) => encoded, Err(error) => effect_die! { defect: error, }?, };
        let phone = Session::new(encoded);
        let protocol: RpcStreamProtocol<StatusError, (), CoffeeShop, Session> = match rpc_stream_protocol! {
            policy: rpc_policy(), handlers: [watch],
        } { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("pickup status protocol: {error:?}")), }?, };
        protocol.serve(phone.clone())?;
        assert!(phone.is_closed(), "the phone session closes after its terminal status");
    }
}
/// The display keeps the request, lookup, chunk and terminal error phases distinct.
type StatusError = RpcStreamMiddlewareError<OrderProblem, OrderProblem>;
/// Supply an existing ticket and run the subscription with one explicit R.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env: Arc<CoffeeShop> = Cafe::open(
        OrderDesk {
            tickets: Mutex::new(BTreeMap::from([(
                "104".into(),
                TrackedOrder {
                    ticket: OrderTicket {
                        number: "104".into(),
                        customer: "Maya".into(),
                        drink: Drink::OatLatte,
                        quantity: 2,
                    },
                    history: vec![Stage::Accepted, Stage::Preparing, Stage::Ready],
                },
            )])),
        },
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(follow_order().run(env.clone()));
    env.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => (),
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("order tracking failed: {other:?}").into());
        }
    }
    Ok(())
}
