//! Sam prepares customer drinks once, retrieves the recorded result, and takes a queued ticket.
//!
//! Run: `cargo run -p effect --example prepare_coffee_orders`.
//! CoffeeOrder and PreparedOrder belong to the café. Effect records activities,
//! workflow outcomes, queued delivery and acknowledgement in the supplied local stores.
//! Recording survives repeated execution in this process, not a process restart.
pub mod cafe_support;
use cafe_support::{Cafe, CafeKeys, binary, binary_limits, key, limits, nz, parse_policy};
use effect::persistence::{
    MemoryQueueStoreError, PersistedQueue, PersistedQueueName, QueueClaimOptions,
    QueueDecodeFailure, QueueMessagePolicy, QueueOwner,
};
use effect::workflow::{
    ActivityRunError, Durable, DurableDeferred, DurableQueue, DurableQueueError,
    DurableQueueWorkerError, DurableSignalError, DurableSignalWait, WorkflowCodecError,
    WorkflowDefinition, WorkflowDefinitionId, WorkflowEngine, WorkflowEngineError, WorkflowHandle,
    WorkflowRunError, WorkflowRunId, WorkflowRunResult, WorkflowSignalId, WorkflowVersion,
};
use effect::{
    Effect, Exit, JsonValue, PureCodec, UtcMillis, activity, activity_definition, durable_queue,
    effect, json_i64, json_string, persisted_queue, schema, schema_enum, workflow,
    workflow_definition,
};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Café drinks named on the barista's ticket.
#[derive(Clone, Debug)]
pub enum Drink {
    /// Espresso with steamed oat milk.
    OatLatte,
    /// Espresso topped up with water.
    Americano,
}
/// Customer ticket waiting for preparation.
#[derive(Clone, Debug)]
pub struct CoffeeOrder {
    /// Pickup ticket number.
    number: String,
    /// Name to call at the counter.
    customer: String,
    /// Drink to prepare.
    drink: Drink,
    /// Cups requested by the customer.
    quantity: i64,
}
/// Completed drinks with the customer information needed at pickup.
#[derive(Debug)]
pub struct PreparedOrder {
    /// Original pickup ticket.
    number: String,
    /// Name written on the cups.
    customer: String,
    /// Actual number of cups prepared.
    cups: i64,
}
/// Local kitchen service owns the real preparation log, separate from Effect history.
pub struct Kitchen {
    /// Tickets whose drinks Sam has physically prepared in this demonstration.
    prepared: Mutex<Vec<String>>,
}
/// Business feedback for a ticket the barista cannot prepare.
#[derive(Debug)]
pub enum PreparationProblem {
    /// A ticket must contain one through four cups.
    InvalidQuantity,
}
/// Recorded preparation keeps the kitchen failure separate from engine failures.
type PreparationActivityError = ActivityRunError<WorkflowEngineError, PreparationProblem>;
/// The staff recipe retains every provider/codec/business error channel.
#[derive(Debug)]
pub enum StaffProblem {
    /// Replaying or recording a preparation workflow failed.
    Workflow(WorkflowRunError<WorkflowEngineError, PreparationActivityError>),
    /// Queue admission or ownership failed.
    Queue(DurableQueueError<MemoryQueueStoreError, WorkflowEngineError>),
    /// A claimed ticket was malformed.
    Ticket(QueueDecodeFailure),
    /// Preparing or acknowledging a queued ticket failed.
    Worker(DurableQueueWorkerError<MemoryQueueStoreError, WorkflowEngineError, PreparationProblem>),
    /// Registering/starting the queued workflow failed.
    Engine(WorkflowEngineError),
    /// Encoding the queued workflow input failed.
    Codec(WorkflowCodecError),
    /// Reading the recorded pickup result failed.
    Pickup(DurableSignalError<WorkflowEngineError>),
}
/// Kitchen and independently selected providers are supplied by R.
type CoffeeShop = Cafe<Kitchen>;
/// Stable drink vocabulary used by activity and queue codecs.
fn drink_schema() -> PureCodec<Drink, JsonValue> {
    schema_enum! { enum Drink { OatLatte => "oat-latte", Americano => "americano" }
    tag_field: "drink", content_field: "details", policy: parse_policy() }
    .expect("drink schema")
}
/// Ticket codec carries customer functionality through every recording boundary.
fn order_schema() -> PureCodec<CoffeeOrder, JsonValue> {
    schema! { struct CoffeeOrder { number: json_string(), customer: json_string(), drink: drink_schema(), quantity: json_i64() }
        policy: parse_policy() }.expect("coffee order schema")
}
/// Completed drinks are recorded as a typed application result.
fn prepared_schema() -> PureCodec<PreparedOrder, JsonValue> {
    schema! { struct PreparedOrder { number: json_string(), customer: json_string(), cups: json_i64() }
        policy: parse_policy() }.expect("prepared order schema")
}
/// Persist business feedback alongside successful preparation.
fn problem_schema() -> PureCodec<PreparationProblem, JsonValue> {
    schema_enum! { enum PreparationProblem { InvalidQuantity => "invalid-quantity" }
    tag_field: "problem", content_field: "details", policy: parse_policy() }
    .expect("preparation problem schema")
}
/// The actual kitchen work runs on the blocking provider selected through R.
fn prepare_drinks(
    order: Arc<CoffeeOrder>,
) -> Effect<PreparedOrder, PreparationProblem, CoffeeShop> {
    effect! {
        let order = order.clone();
        if !(1..=4).contains(&order.quantity) {
            Effect::<(), PreparationProblem, CoffeeShop>::fail_with(|| PreparationProblem::InvalidQuantity)?;
        }
        let kitchen = Effect::service(|cafe: &CoffeeShop| cafe.services.clone())?;
        Effect::blocking_once(move |_| {
            println!("Sam prepares {} {:?} cup(s) for {} (ticket {}).", order.quantity, order.drink, order.customer, order.number);
            kitchen.prepared.lock().expect("kitchen log").push(order.number.clone());
            PreparedOrder { number: order.number.clone(), customer: order.customer.clone(), cups: order.quantity }
        })?
    }
}
/// A helper factory owns its immutable order snapshot explicitly for each activity attempt.
fn preparation_body(
    order: Arc<CoffeeOrder>,
) -> Effect<Result<PreparedOrder, PreparationProblem>, PreparationProblem, CoffeeShop> {
    prepare_drinks(order).either()
}
/// Extracted durable coordination uses the description form for an existing kitchen helper.
fn prepare_ticket(
    run: WorkflowRunId,
    order: CoffeeOrder,
) -> Durable<Result<PreparedOrder, PreparationProblem>, PreparationActivityError, CoffeeShop> {
    let order = Arc::new(order);
    workflow! {
        let run = run.clone(); let order = order.clone();
        activity! {
            description: activity_definition! { id: effect::workflow::WorkflowActivityId::from_stable(key("prepare-counter-ticket")),
                success: prepared_schema(), error: problem_schema(), limits: limits() },
            run: run, attempt: NonZeroU32::MIN, body: move || preparation_body(order.clone())
        }?
    }
}
/// Workflow metadata is used to execute counter orders and register queued preparation.
fn counter_workflow() -> WorkflowDefinition<CoffeeOrder, PreparedOrder, PreparationProblem> {
    workflow_definition! { id: WorkflowDefinitionId::new("prepare-counter-order").expect("workflow"),
    version: WorkflowVersion::new(NonZeroU32::MIN), input: order_schema(), success: prepared_schema(), error: problem_schema(),
    compatibility: Arc::from(&b"coffee-preparation-v1"[..]), limits: limits() }
}
/// Print only customer/staff outcomes; workflow statuses remain typed.
fn show_prepared(outcome: WorkflowRunResult<PreparedOrder, PreparationProblem>) {
    match outcome {
        WorkflowRunResult::Completed(Ok(order)) => println!(
            "Pickup: {} cups for {}, ticket {}.",
            order.cups, order.customer, order.number
        ),
        WorkflowRunResult::Completed(Err(problem)) => {
            println!("The barista cannot prepare this ticket: {problem:?}")
        }
        _ => println!("This ticket is still waiting for preparation."),
    }
}
/// Sam's shift makes a mobile order, retrieves it without rebrewing, and handles two more tickets.
fn sams_shift() -> Effect<(), StaffProblem, CoffeeShop> {
    effect! {
        let run = WorkflowRunId::from_stable(key("prepare-order-104"));
        let activity_run = run.clone();
        let brew = activity! { name: effect::workflow::WorkflowActivityId::from_stable(key("prepare-mobile-ticket")),
            input: order_schema(), success: prepared_schema(), error: problem_schema(), limits: limits(), body |order: Arc<CoffeeOrder>| {
                prepare_drinks(order).either()?
            }
        };
        let mobile = workflow! { name: WorkflowDefinitionId::new("prepare-mobile-order").expect("workflow"),
            version: WorkflowVersion::new(NonZeroU32::MIN), payload: order_schema(), success: prepared_schema(), error: problem_schema(),
            compatibility: Arc::from(&b"mobile-coffee-v1"[..]), limits: limits(), run |order: CoffeeOrder| {
                let brew = brew.clone(); let run = activity_run.clone();
                brew.call(run, NonZeroU32::MIN, order)?
            }
        };
        let preparation = mobile.run(run, CoffeeOrder { number: "104".into(), customer: "Maya".into(), drink: Drink::OatLatte, quantity: 2 });
        show_prepared(preparation.clone().map_error(StaffProblem::Workflow)?);
        println!("Maya checks the same pickup result again:");
        show_prepared(preparation.map_error(StaffProblem::Workflow)?);
        let counter = counter_workflow();
        let run = WorkflowRunId::from_stable(key("prepare-order-105")); let helper_run = run.clone();
        show_prepared(counter.execute(run, CoffeeOrder { number: "105".into(), customer: "Lee".into(), drink: Drink::Americano, quantity: 1 },
            move |order| prepare_ticket(helper_run.clone(), order)).map_error(StaffProblem::Workflow)?);
        let queued_order = CoffeeOrder { number: "106".into(), customer: "Nora".into(), drink: Drink::OatLatte, quantity: 1 };
        let queued_run = WorkflowRunId::from_stable(key("prepare-order-106"));
        let engine = Effect::service(|cafe: &CoffeeShop| cafe.workflow.clone())?;
        engine.register(counter.registration()).provide_some(|_: &CoffeeShop| ()).map_error(StaffProblem::Engine)?;
        let input = Effect::from_result_once(counter.encode_input_bytes(&queued_order)).map_error(StaffProblem::Codec)?;
        engine.start(WorkflowHandle { run: queued_run.clone(), definition: counter.id.clone(), version: counter.version }, input)
            .provide_some(|_: &CoffeeShop| ()).map_error(StaffProblem::Engine)?;
        let pickup = DurableDeferred { run: queued_run, signal: WorkflowSignalId::from_stable(key("order-106-ready")),
            version: counter.version, codec: prepared_schema(), limits: limits() };
        let tickets: PersistedQueue<CoffeeOrder, CafeKeys> = persisted_queue! { name: PersistedQueueName::new("barista-tickets").expect("queue"),
            payload: order_schema(), identity: binary("queued-coffee-order"), limits: binary_limits(),
            policy: QueueMessagePolicy { maximum_attempts: NonZeroU32::MIN, acknowledged_retention: Duration::from_secs(60) } };
        let tickets = durable_queue! { queue: tickets };
        tickets.enqueue(key("prepare-order-106"), move || queued_order.clone(), &pickup, UtcMillis::new(0), UtcMillis::new(0)).map_error(StaffProblem::Queue)?;
        println!("Nora's ticket waits in the barista queue.");
        let mut claims = tickets.claim(QueueClaimOptions::new(QueueOwner::new("Sam").expect("barista"), UtcMillis::new(0), UtcMillis::new(100), nz(1)).expect("ticket claim")).map_error(StaffProblem::Queue)?;
        let delivery = Effect::from_result_once(tickets.decode_claim(claims.pop().expect("Nora's queued ticket"))).map_error(StaffProblem::Ticket)?;
        durable_queue! { queue: DurableQueue::new(tickets.queue().clone()), delivery: delivery, deferred: pickup.clone(), acknowledged_at: UtcMillis::new(1),
            handler |order: CoffeeOrder| { prepare_drinks(Arc::new(order))? }
        }.map_error(StaffProblem::Worker)?;
        match pickup.wait().map_error(StaffProblem::Pickup)? {
            DurableSignalWait::Ready(order) => println!("Recorded before acknowledgement: {} cup for {}, ticket {}.", order.cups, order.customer, order.number),
            DurableSignalWait::Waiting => println!("Nora's order is still waiting."),
        }
        let kitchen = Effect::service(|cafe: &CoffeeShop| cafe.services.clone())?;
        println!("Sam's preparation log contains tickets {}.", kitchen.prepared.lock().expect("kitchen log").join(", "));
    }
}
/// Supply the local kitchen and execute one complete barista shift.
fn main() {
    let cafe: Arc<CoffeeShop> = Cafe::open(
        Kitchen {
            prepared: Mutex::new(Vec::new()),
        },
        nz(16),
        nz(4),
    );
    match futures_executor::block_on(sams_shift().run(cafe.clone())) {
        Exit::Success(()) => (),
        exit => panic!("Barista preparation failed: {exit:?}"),
    }
    cafe.base.finish();
}
