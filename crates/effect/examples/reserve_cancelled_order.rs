//! Reserve two cups, cancel the order, and record the stock return once.
//!
//! Run: `cargo +1.99.0 run -p effect --example reserve_cancelled_order`.
//! Input: eight available cups and a two-cup reservation. Output: stock 6 after
//! reservation, stock 8 after cancellation, and stock 8 after compensation replay.
//! The stock and workflow history are local to this process.

use effect::{effect_die, effect_map_error, effect_provide_some, effect_try_sync};
pub mod cafe_support;
use cafe_support::{Cafe, key, limits, parse_policy};
use effect::workflow::{
    CompensationRunError, WorkflowCodecError, WorkflowCompensation, WorkflowCompensationId,
    WorkflowDefinitionId, WorkflowEngine, WorkflowEngineError, WorkflowHandle, WorkflowRunId,
    WorkflowVersion,
};
use effect::{
    AtomicRef, Effect, Exit, effect, effect_ref, json_i64, schema_enum, service, workflow,
    workflow_definition,
};
use std::num::NonZeroUsize;
use std::{num::NonZeroU32, sync::Arc};

/// A recorded compensation reverses the actual reservation without a second return.
fn cancel_reservation() -> Effect<(), ReservationError, Cafe<AtomicRef<i64>>> {
    effect! {
        let stock = service! { env |env: &Cafe<AtomicRef<i64>>| { env.services.as_ref().clone() }, }?;
        let reserved = effect_ref! {
            modify; reference: stock.clone(),
            modify |available: &i64| {
                if *available >= 2 { (Ok(2i64), available - 2) }
                else { (Err(StockError::SoldOut), *available) }
            },
        }?;
        let cups = effect_map_error! { program: effect_try_sync! { once; body: { let __dsl_result = reserved; move || __dsl_result }, on_error: ::core::convert::identity, }, map: ReservationError::Stock, }?;
        let available = stock.get()?;
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Reserved {cups} cups; stock {available}."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        let workflow_id = match WorkflowDefinitionId::new("cancel-reservation") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("definition: {error:?}")), }?, };
        let reservation_errors = match stock_errors() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let definition = workflow_definition! {
            id: workflow_id,
            version: WorkflowVersion::new(NonZeroU32::MIN), input: json_i64(), success: json_i64(),
            error: reservation_errors, compatibility: Arc::from(&b"cup-reservation-v1"[..]), limits: limits(),
        };
        let run = WorkflowRunId::from_stable(match key("cancel-ticket-107") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, });
        let engine = service! { env |env: &Cafe<AtomicRef<i64>>| { env.workflow.clone() }, }?;
        effect_map_error! { program: effect_provide_some! { program: engine.register(definition.registration()), project: |_: &Cafe<AtomicRef<i64>>| (), }, map: ReservationError::Workflow, }?;
        let input = effect_map_error! { program: effect_try_sync! { once; body: { let __dsl_result = definition.encode_input_bytes(&cups); move || __dsl_result }, on_error: ::core::convert::identity, }, map: ReservationError::Codec, }?;
        effect_map_error! { 
            program: effect_provide_some! { 
            program: engine.start(WorkflowHandle {
            run: run.clone(), definition: definition.id,
            version: definition.version }, input), project: |_: &Cafe<AtomicRef<i64>>| (), }, map: ReservationError::Workflow, }?;
        let returned_stock = stock.clone();
        let return_cups_key = match key("return-reserved-cups") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let compensation_errors = match stock_errors() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let compensation = workflow! {
            compensate;
            compensation: WorkflowCompensation {
                identity: WorkflowCompensationId::from_stable(return_cups_key),
                order: NonZeroU32::MIN, success: json_i64(), error: compensation_errors, limits: limits(),
            },
            run: run.clone(),
            body |_identity| {
                let stock = returned_stock.clone();
                effect_ref! { update; reference: stock, update |available: &i64| { available + cups }, }?;
                Ok::<_, StockError>(cups)
            },
        };
        let first = effect_map_error! { program: compensation.clone(), map: ReservationError::Compensation, }?;
        assert!(matches!(first, Ok(2)));
        let available = stock.get()?;
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Cancelled; stock {available}."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        let replay = effect_map_error! { program: compensation, map: ReservationError::Compensation, }?;
        assert!(matches!(replay, Ok(2)));
        let available = stock.get()?;
        { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Compensation replay; stock {available}."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } };
        effect_map_error! { program: effect_provide_some! { program: engine.cancel(run), project: |_: &Cafe<AtomicRef<i64>>| (), }, map: ReservationError::Workflow, }?;
    }
}
/// A reservation cannot consume cups that are not available.
#[derive(Debug)]
pub enum StockError {
    /// The requested stock is unavailable.
    SoldOut,
}
/// Domain and recording failures retain their respective boundaries.
#[derive(Debug)]
pub enum ReservationError {
    /// Stock admission rejected the request.
    Stock(StockError),
    /// Workflow registration/start/cancellation failed.
    Workflow(WorkflowEngineError),
    /// The reservation input could not be encoded.
    Codec(WorkflowCodecError),
    /// Recording or executing the rollback failed.
    Compensation(CompensationRunError<WorkflowEngineError, StockError>),
}
/// Record the actual domain rejection as a tagged value.
fn stock_errors() -> Result<effect::PureCodec<StockError, effect::JsonValue>, effect::Defect> {
    schema_enum! {
        enum StockError { SoldOut => "sold-out" }
        tag_field: "problem", content_field: "details", policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("stock codec: {error:?}")))
}
/// Supply stock beside workflow authority, then run and join the provider boundary.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env = Cafe::open(
        AtomicRef::new(8),
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(cancel_reservation().run(env.clone()));
    env.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => (),
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("reservation rollback failed: {other:?}").into());
        }
    }
    Ok(())
}
