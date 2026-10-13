//! Deliver Nora's pickup ticket from a persisted queue to one barista worker.
//!
//! Run: `cargo +1.99.0 run -p effect --example queue_coffee_orders`.
//! Input: ticket 106 for one cup. Output: one set of pickup instructions, its recorded
//! result, and an empty claim after acknowledgement. Stores are process-local.

use effect::{effect_die, effect_map_error, effect_provide_some, effect_try_sync};
pub mod cafe_support;
use cafe_support::tickets::{Pickup, Ticket, TicketError, error_codec, pickup_codec, ticket_codec};
use cafe_support::{Cafe, CafeKeys, binary, binary_limits, key, limits};
use effect::persistence::{
    MemoryQueueStoreError, PersistedQueue, PersistedQueueName, QueueClaimOptions,
    QueueDecodeFailure, QueueMessagePolicy, QueueOwner,
};
use effect::workflow::{
    DurableDeferred, DurableQueueError, DurableQueueWorkerError, DurableSignalError,
    DurableSignalWait, WorkflowCodecError, WorkflowDefinitionId, WorkflowEngine,
    WorkflowEngineError, WorkflowHandle, WorkflowRunId, WorkflowSignalId, WorkflowVersion,
};
use effect::{
    Effect, Exit, UtcMillis, durable_queue, effect, persisted_queue, service, workflow_definition,
};
use std::num::NonZeroUsize;
use std::{num::NonZeroU32, sync::Arc, time::Duration};

/// The producer admits a ticket; the consumer records pickup before acknowledging it.
fn serve_queued_ticket() -> Effect<(), DeliveryError, Cafe> {
    effect! {
        let workflow_id = match WorkflowDefinitionId::new("queued-pickup") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("definition: {error:?}")), }?, };
        let input_codec = match ticket_codec() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let result_codec = match pickup_codec() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let validation_codec = match error_codec() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let definition = workflow_definition! {
            id: workflow_id,
            version: WorkflowVersion::new(NonZeroU32::MIN),
            input: input_codec, success: result_codec, error: validation_codec,
            compatibility: Arc::from(&b"queued-pickup-v1"[..]), limits: limits(),
        };
        let ticket = Ticket { number: "106".into(), customer: "Nora".into(), cups: 1 };
        let run = WorkflowRunId::from_stable(match key("delivery-106") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, });
        let engine = service! { env |env: &Cafe| { env.workflow.clone() }, }?;
        effect_map_error! { program: effect_provide_some! { program: engine.register(definition.registration()), project: |_: &Cafe| (), }, map: DeliveryError::Workflow, }?;
        let input = effect_map_error! { program: effect_try_sync! { once; body: { let __dsl_result = definition.encode_input_bytes(&ticket); move || __dsl_result }, on_error: ::core::convert::identity, }, map: DeliveryError::Codec, }?;
        effect_map_error! { 
            program: effect_provide_some! { 
            program: engine.start(WorkflowHandle {
            run: run.clone(), definition: definition.id,
            version: definition.version }, input), project: |_: &Cafe| (), }, map: DeliveryError::Workflow, }?;
        let pickup = DurableDeferred {
            run, signal: WorkflowSignalId::from_stable(match key("pickup-106") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, }),
            version: definition.version, codec: match pickup_codec() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, }, limits: limits() };
        let queue_name = match PersistedQueueName::new("pickup-tickets") { Ok(name) => name, Err(error) => effect_die! { defect: effect::Defect::new(format!("queue: {error:?}")), }?, };
        let queued_ticket_codec = match ticket_codec() { Ok(codec) => codec, Err(error) => effect_die! { defect: error, }?, };
        let queue_identity = match binary("pickup-ticket") { Ok(identity) => identity, Err(error) => effect_die! { defect: error, }?, };
        let queue: PersistedQueue<Ticket, CafeKeys> = persisted_queue! {
            name: queue_name,
            payload: queued_ticket_codec, identity: queue_identity, limits: binary_limits(),
            policy: QueueMessagePolicy {
                maximum_attempts: NonZeroU32::MIN,
                acknowledged_retention: Duration::from_secs(60) },
        };
        let queue = Arc::new(durable_queue! { queue: queue });
        let delivery_key = match key("delivery-106") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        effect_map_error! { 
            program: queue.enqueue(delivery_key, move || ticket.clone(), &pickup,
            UtcMillis::new(0), UtcMillis::new(0)), map: DeliveryError::Queue, }?;
        let owner = match QueueOwner::new("barista") { Ok(owner) => owner, Err(error) => effect_die! { defect: effect::Defect::new(format!("queue owner: {error:?}")), }?, };
        let claim = match QueueClaimOptions::new(owner, UtcMillis::new(0), UtcMillis::new(100), NonZeroUsize::MIN) { Ok(claim) => claim, Err(error) => effect_die! { defect: effect::Defect::new(format!("claim policy: {error:?}")), }?, };
        let mut claims = effect_map_error! { program: queue.claim(claim.clone()), map: DeliveryError::Queue, }?;
        let claimed = match claims.pop() { Some(claimed) => claimed, None => effect_die! { defect: effect::Defect::new("queued ticket"), }?, };
        let delivery = effect_map_error! { program: effect_try_sync! { once; body: { let __dsl_result = queue.decode_claim(claimed); move || __dsl_result }, on_error: ::core::convert::identity, }, map: DeliveryError::Decode, }?;
        effect_map_error! { 
            program: durable_queue! {
            queue: queue.clone(), delivery: delivery, deferred: pickup.clone(),
            acknowledged_at: UtcMillis::new(1),
            handler |ticket: Ticket| {
                use std::io::Write;
                let (ticket, written) = {
                    let written = writeln!(std::io::stdout().lock(), "Issue pickup instructions: {} cup for {}, ticket {}.", ticket.cups, ticket.customer, ticket.number);
                    (ticket, written)
                };
                match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
                Pickup { number: ticket.number, customer: ticket.customer, cups: ticket.cups }
            },
        }, map: DeliveryError::Worker, }?;
        match effect_map_error! { program: pickup.wait(), map: DeliveryError::Pickup, }? {
            DurableSignalWait::Ready(pickup) => {
                use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Recorded pickup: {} cup for {}, ticket {}.",
                pickup.cups, pickup.customer, pickup.number); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } },
            DurableSignalWait::Waiting => effect_die! { defect: effect::Defect::new("worker returned before recording pickup"), }?,
        }
        let remaining = effect_map_error! { program: queue.claim(claim), map: DeliveryError::Queue, }?;
        assert!(remaining.is_empty());
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Acknowledged; no ticket remains to claim."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
    }
}
/// Each queue phase retains its own failure type.
#[derive(Debug)]
pub enum DeliveryError {
    /// Registration/start failed before delivery.
    Workflow(WorkflowEngineError),
    /// Input did not match the registered codec.
    Codec(WorkflowCodecError),
    /// Queue admission/claim failed.
    Queue(DurableQueueError<MemoryQueueStoreError, WorkflowEngineError>),
    /// A claimed payload could not be decoded.
    Decode(QueueDecodeFailure),
    /// Preparation, recording or acknowledgement failed.
    Worker(DurableQueueWorkerError<MemoryQueueStoreError, WorkflowEngineError, TicketError>),
    /// The recorded pickup could not be read.
    Pickup(DurableSignalError<WorkflowEngineError>),
}
/// Run once with local queue/workflow stores, then join provider work.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env = Cafe::open(
        (),
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(serve_queued_ticket().run(env.clone()));
    env.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => (),
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("queued pickup failed: {other:?}").into());
        }
    }
    Ok(())
}
