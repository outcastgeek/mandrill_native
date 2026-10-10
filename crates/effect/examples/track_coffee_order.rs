//! Maya places two oat lattes, looks up her ticket, and watches them become ready.
//!
//! Run: `cargo run -p effect --example track_coffee_order`.
//! The application owns the order desk and customer models. Effect supplies typed
//! RPC operations/protocols and middleware; local wire/producer mechanics are in
//! cafe_support::transport. No network, payment, or notification service is used.
pub mod cafe_support;
use cafe_support::transport::{Session, request, sequence, stream_request};
use cafe_support::{Cafe, limits, nz, parse_policy, rpc_policy};
use effect::rpc::{
    OperationId, RpcCallId, RpcProduced, RpcProtocol, RpcProtocolError, RpcProtocolGroup,
    RpcServerMiddlewareError, RpcServiceMiddleware, RpcServiceMiddlewareError, RpcStreamContext,
    RpcStreamMiddlewareError, RpcStreamProducer, RpcStreamProtocol, RpcStreamProtocolError,
    RpcStreamServerError, RpcVersion,
};
use effect::{
    Effect, Exit, JsonValue, PureCodec, Release, ResourceBinding, ResourceValue, effect, json_i64,
    json_string, rpc_middleware, rpc_operation, rpc_protocol, rpc_stream_middleware,
    rpc_stream_operation, rpc_stream_protocol, schema, schema_enum, service,
};
use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

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
    customer: String,
    /// Requested drink.
    drink: Drink,
    /// Requested cups.
    quantity: i64,
}
/// Ticket returned by the café and looked up by its order number.
#[derive(Clone, Debug)]
pub struct OrderTicket {
    /// Number printed on the pickup ticket.
    number: String,
    /// Name to call when ready.
    customer: String,
    /// Accepted drink.
    drink: Drink,
    /// Accepted cup count.
    quantity: i64,
}
/// Lookup/watch requests share the customer's ticket identity.
#[derive(Debug)]
pub struct FindOrder {
    /// Pickup ticket number, e.g. 104.
    number: String,
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
    },
}
/// The café's application service; this is not an Effect runtime or store engine.
pub struct OrderDesk {
    /// Accepted ticket snapshots indexed by printed number.
    tickets: Mutex<BTreeMap<String, OrderTicket>>,
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
}
/// Scoped staff service opened before an authorized handler/session executes.
pub struct CounterSession {
    /// Staff member serving the request.
    barista: String,
}
/// Typed callback middleware keeps the phase of a failure.
type CustomerChecks = RpcServerMiddlewareError<OrderProblem, OrderProblem>;
/// Streaming middleware distinguishes status and terminal transformation failures.
type StatusChecks = RpcStreamMiddlewareError<OrderProblem, OrderProblem>;
/// The order application retains business, protocol, and scoped-service failures.
#[derive(Debug)]
pub enum TrackingProblem {
    /// Customer handler failed.
    Order(OrderProblem),
    /// Customer request/result middleware failed.
    Customer(CustomerChecks),
    /// Status middleware failed.
    Status(StatusChecks),
    /// Kiosk unary protocol failed.
    Kiosk(RpcProtocolError<OrderProblem>),
    /// Separate/grouped tracking protocol failed.
    Tracking(RpcStreamProtocolError<OrderProblem, ()>),
    /// Acquiring or using a counter session failed.
    Counter(RpcServiceMiddlewareError<OrderProblem, OrderProblem>),
    /// Full status serving failed while holding the counter session.
    Serving(RpcServiceMiddlewareError<OrderProblem, RpcStreamServerError<OrderProblem, ()>>),
}
/// Runtime capabilities and this café's order desk coexist in the same R.
type CoffeeShop = Cafe<OrderDesk>;
/// Explicit drink names define the wire vocabulary.
fn drink_schema() -> PureCodec<Drink, JsonValue> {
    schema_enum! {
        enum Drink { OatLatte => "oat-latte", Americano => "americano" }
        tag_field: "drink", content_field: "options", policy: parse_policy()
    }
    .expect("drink schema")
}
/// Customer request shape, separate from its business validation.
fn order_schema() -> PureCodec<CoffeeOrder, JsonValue> {
    schema! {
        struct CoffeeOrder { customer: json_string(), drink: drink_schema(), quantity: json_i64() }
        policy: parse_policy()
    }
    .expect("order schema")
}
/// Accepted ticket shape shared by placement and lookup handlers.
fn ticket_schema() -> PureCodec<OrderTicket, JsonValue> {
    schema! {
        struct OrderTicket {
            number: json_string(), customer: json_string(), drink: drink_schema(), quantity: json_i64()
        } policy: parse_policy()
    }
    .expect("ticket schema")
}
/// Typed identity for both lookup and watch operations.
fn find_schema() -> PureCodec<FindOrder, JsonValue> {
    schema! { struct FindOrder { number: json_string() } policy: parse_policy() }
        .expect("lookup schema")
}
/// Lifecycle chunks and the terminal ready message use the same model.
fn status_schema() -> PureCodec<OrderStatus, JsonValue> {
    schema_enum! {
        enum OrderStatus {
            Accepted => "accepted", Preparing => "preparing", Ready { customer: json_string() } => "ready"
        } tag_field: "status", content_field: "details", policy: parse_policy()
    }.expect("status schema")
}
/// Preserve typed customer feedback in protocol error replies.
fn problem_schema() -> PureCodec<OrderProblem, JsonValue> {
    schema_enum! {
        enum OrderProblem {
            UnknownTicket => "unknown-ticket", InvalidQuantity => "invalid-quantity", CounterClosed => "counter-closed"
        } tag_field: "problem", content_field: "details", policy: parse_policy()
    }.expect("problem schema")
}
/// Select the café desk with service! and look up the ticket when the recipe executes.
fn lookup_order(request: FindOrder) -> Effect<OrderTicket, OrderProblem, CoffeeShop> {
    service! {
        select: |cafe: &CoffeeShop| cafe.services.clone(),
        on_error: |problem: OrderProblem| problem,
        run |desk: &Arc<OrderDesk>| {
            let number = request.number.clone();
            let found = desk.tickets.lock().expect("order desk").get(&number).cloned();
            match found {
                Some(ticket) => ticket,
                None => Effect::fail_with(|| OrderProblem::UnknownTicket)?,
            }
        }
    }
}
/// A reusable final-status helper also serves the counter's small ready-only endpoint.
fn ready_order(
    request: FindOrder,
    _: RpcStreamContext,
) -> Effect<
    RpcStreamProducer<OrderStatus, OrderStatus, OrderProblem, CoffeeShop>,
    OrderProblem,
    CoffeeShop,
> {
    effect! {
        let request = FindOrder { number: request.number.clone() };
        let ticket = lookup_order(request)?;
        sequence(Vec::<OrderStatus>::new(), OrderStatus::Ready { customer: ticket.customer })
    }
}
/// Customers and staff can follow this recipe without reading transport internals.
fn follow_mayas_order() -> Effect<(), TrackingProblem, CoffeeShop> {
    effect! {
        let placed = rpc_operation! {
            id: OperationId::new("place-coffee-order").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
            request: order_schema(), success: ticket_schema(), error: problem_schema(), limits: limits(), handler |order: CoffeeOrder| {
                if !(1..=4).contains(&order.quantity) { Effect::<(), OrderProblem, CoffeeShop>::fail_with(|| OrderProblem::InvalidQuantity)?; }
                let desk = service! {
                    select: |cafe: &CoffeeShop| cafe.services.clone(),
                    on_error: |problem: OrderProblem| problem,
                    run |desk| { Arc::clone(desk) }
                }?;
                let ticket = OrderTicket { number: "104".into(), customer: order.customer, drink: order.drink, quantity: order.quantity };
                desk.tickets.lock().expect("order desk").insert(ticket.number.clone(), ticket.clone());
                ticket
            }
        };
        let checked = rpc_middleware! {
            handler: placed, error_codec: schema_enum! {
                enum CustomerChecks {
                    Before(problem_schema()) => "before", Domain(problem_schema()) => "order", After(problem_schema()) => "after"
                } tag_field: "phase", content_field: "problem", policy: parse_policy() }.expect("customer checks"),
            before |order: CoffeeOrder| { CoffeeOrder { customer: order.customer.trim().to_owned(), ..order } },
            after |ticket: OrderTicket| { println!("Counter accepted ticket {} for {}.", ticket.number, ticket.customer); ticket }
        };
        let ticket = checked.call(CoffeeOrder { customer: " Maya ".into(), drink: Drink::OatLatte, quantity: 2 }).map_error(TrackingProblem::Customer)?;
        println!("Maya receives ticket {} for {} {:?} cups.", ticket.number, ticket.quantity, ticket.drink);
        let lookup = rpc_operation! {
            id: OperationId::new("find-coffee-order").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
            request: find_schema(), success: ticket_schema(), error: problem_schema(), limits: limits(), handler: lookup_order };
        let found = lookup.call(FindOrder { number: ticket.number.clone() }).map_error(TrackingProblem::Order)?;
        println!("Lookup finds {}'s order {}.", found.customer, found.number);
        let ready = rpc_stream_operation! {
            id: OperationId::new("ready-coffee-order").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
            request: find_schema(), chunk: status_schema(), terminal: status_schema(), error: problem_schema(), limits: limits(), handler: ready_order };
        let watch = rpc_stream_operation! {
            id: OperationId::new("follow-coffee-order").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
            request: find_schema(), chunk: status_schema(), terminal: status_schema(), error: problem_schema(), limits: limits(), handler |request: FindOrder, _context| {
                let ticket = lookup_order(request)?;
                sequence(vec![OrderStatus::Accepted, OrderStatus::Preparing], OrderStatus::Ready { customer: ticket.customer })
            }
        };
        let watch = rpc_stream_middleware! {
            handler: watch, error_codec: schema_enum! {
                enum StatusChecks {
                    Before(problem_schema()) => "before", Domain(problem_schema()) => "order", Chunk(problem_schema()) => "status", Terminal(problem_schema()) => "ready"
                } tag_field: "phase", content_field: "problem", policy: parse_policy() }.expect("status checks"),
            before |request: FindOrder| { request },
            chunk |status: OrderStatus| { status },
            terminal |status: OrderStatus| { println!("Pickup counter: Maya, your drinks are ready!"); status }
        };
        let statuses = watch.call(FindOrder { number: ticket.number.clone() }, RpcStreamContext { call: RpcCallId::new("maya-phone").expect("call"), metadata: Default::default() }).map_error(TrackingProblem::Status)?;
        loop {
            match statuses.pull().map_error(TrackingProblem::Status)? {
                RpcProduced::Chunk(status) => println!("Maya's phone: {status:?}"),
                RpcProduced::Terminal(status) => { println!("Maya's phone: {status:?}"); break; }
            }
        }
        let kiosk_description = rpc_operation! {
            id: OperationId::new("find-coffee-order").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
            request: find_schema(), success: ticket_schema(), error: problem_schema(), limits: limits() };
        let local_request = request(&kiosk_description, "maya-kiosk", &FindOrder { number: ticket.number.clone() });
        let watch_request = stream_request(ready.operation(), "maya-counter", &FindOrder { number: ticket.number.clone() });
        let unary = rpc_protocol! { policy: rpc_policy(), handlers: [lookup.clone()] }.expect("kiosk protocol");
        let _reply = unary.handle(local_request.clone()).map_error(TrackingProblem::Kiosk)?;
        let separate: RpcStreamProtocol<OrderProblem, (), CoffeeShop, Session> = rpc_stream_protocol! { policy: rpc_policy(), handlers: [ready.clone()] }.expect("tracking protocol");
        separate.serve(Session::new(watch_request.clone())).map_error(TrackingProblem::Tracking)?;
        let endpoints: RpcProtocolGroup<RpcProtocol<OrderProblem, CoffeeShop>, RpcStreamProtocol<OrderProblem, (), CoffeeShop, Session>> = rpc_protocol! {
            policy: rpc_policy(), unary: [lookup], streaming: [ready]
        }.expect("coffee counter endpoints");
        let _reply = endpoints.unary.handle(local_request).map_error(TrackingProblem::Kiosk)?;
        endpoints.streaming.serve(Session::new(watch_request)).map_error(TrackingProblem::Tracking)?;
        println!("The kiosk and staffed counter both retrieved Maya’s accepted ticket.");
        staff_counter_session(ticket.customer)?;
    }
}
/// A real acquired staff service stays alive through the handler and session close.
fn staff_counter_session(customer: String) -> Effect<(), TrackingProblem, CoffeeShop> {
    effect! {
        let customer = customer.clone();
        let staff: RpcServiceMiddleware<ResourceValue<CounterSession>, OrderProblem, CoffeeShop> = rpc_middleware! {
            acquire {
                ResourceBinding::new(Effect::sync(|| CounterSession { barista: "Sam".into() }), |_, _| Release::sync(|| println!("Sam closes the counter session")))?
            } };
        let greeting = staff.apply(|session, _: &CoffeeShop| session, |customer: String| effect! {
            let customer = customer.clone();
            let barista = service! {
                select: |session: &Arc<ResourceValue<CounterSession>>| session.clone(),
                on_error: |problem: OrderProblem| problem,
                run |session: &Arc<ResourceValue<CounterSession>>| { session.with(|session| session.barista.clone()).expect("open counter") }
            }?;
            format!("{barista} hands {customer} the pickup ticket")
        }, customer.clone()).map_error(TrackingProblem::Counter)?;
        println!("{greeting}.");
        let accepted = Session::new(stream_request(&rpc_stream_operation! {
            id: OperationId::new("staff-ready-status").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
            request: find_schema(), chunk: status_schema(), terminal: status_schema(), error: problem_schema(), limits: limits() }, "staff-counter", &FindOrder { number: "104".into() }));
        let closing = accepted.clone();
        let staff: RpcServiceMiddleware<ResourceValue<CounterSession>, OrderProblem, CoffeeShop> = rpc_middleware! {
            acquire {
                let closing = closing.clone();
                ResourceBinding::new(Effect::sync(|| CounterSession { barista: "Sam".into() }), move |_, _| {
                    let closing = closing.clone(); Release::sync(move || { if closing.is_closed() { println!("Sam's staff access is released after the customer connection closes."); } })
                })?
            } };
        rpc_stream_middleware! {
            middleware: staff, connect: |session, _: &CoffeeShop| session, policy: rpc_policy(), accepted: accepted,
            handler: rpc_stream_operation! {
                id: OperationId::new("staff-ready-status").expect("operation"), version: RpcVersion::new(NonZeroU32::MIN),
                request: find_schema(), chunk: status_schema(), terminal: status_schema(), error: problem_schema(), limits: limits(), handler |_request, _context| {
                    let barista = service! {
                        select: |session: &Arc<ResourceValue<CounterSession>>| session.clone(),
                        on_error: |problem: OrderProblem| problem,
                        run |session: &Arc<ResourceValue<CounterSession>>| { session.with(|session| session.barista.clone()).expect("open staff service") }
                    }?;
                    println!("{barista} confirms pickup readiness.");
                    sequence(Vec::<OrderStatus>::new(), OrderStatus::Ready { customer: "Maya".into() })
                }
            }
        }.map_error(TrackingProblem::Serving)?;
    }
}
/// Supply the order desk and local providers, then execute one customer journey.
fn main() {
    let cafe: Arc<CoffeeShop> = Cafe::open(
        OrderDesk {
            tickets: Mutex::new(BTreeMap::new()),
        },
        nz(16),
        nz(4),
    );
    match futures_executor::block_on(follow_mayas_order().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit => panic!("Order tracking failed: {exit:?}"),
    }
    cafe.base.finish();
}
