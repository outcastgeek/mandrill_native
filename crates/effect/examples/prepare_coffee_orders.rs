//! Issue pickup labels for ticket 104, then retrieve the same recorded result.
//!
//! Run: `cargo +1.99.0 run -p effect --example prepare_coffee_orders`.
//! Input: Maya requests two cups. Output: one label-issuance message and two
//! identical pickup results. Replaying the run does not issue the labels again.
//! The supplied in-memory workflow engine records only within this process.

use effect::effect_die;
pub mod cafe_support;
use cafe_support::tickets::{Pickup, Ticket, TicketError, error_codec, pickup_codec, ticket_codec};
use cafe_support::{Cafe, key, limits};
use effect::workflow::{
    ActivityRunError, WorkflowDefinitionId, WorkflowEngineError, WorkflowRunError, WorkflowRunId,
    WorkflowRunResult, WorkflowVersion,
};
use effect::{Effect, Exit, activity, activity_definition, effect, effect_try_sync, workflow};
use std::num::NonZeroU32;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// One recorded preparation request is also the request used to retrieve its result.
fn prepare_and_replay() -> Effect<
    (),
    WorkflowRunError<WorkflowEngineError, ActivityRunError<WorkflowEngineError, TicketError>>,
    Cafe,
> {
    effect! {
        let run = WorkflowRunId::from_stable(match key("ticket-104") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, });
        let activity_run = run.clone();
        let workflow_id = match WorkflowDefinitionId::new("issue-pickup-labels") { Ok(value) => value, Err(error) => effect_die! { defect: effect::Defect::new(format!("workflow identity: {error:?}")), }?, };
        let input_codec = match ticket_codec() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let result_codec = match pickup_codec() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let validation_codec = match error_codec() { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, };
        let label_key = match key("print-labels") { Ok(key) => key, Err(error) => effect_die! { defect: error, }?, };
        let label_success = match pickup_codec() { Ok(codec) => codec, Err(error) => effect_die! { defect: error, }?, };
        let label_error = match error_codec() { Ok(codec) => codec, Err(error) => effect_die! { defect: error, }?, };
        let label_activity = activity_definition! {
            id: effect::workflow::WorkflowActivityId::from_stable(label_key),
            success: label_success, error: label_error, limits: limits(),
        };
        let prepare = workflow! {
            name: workflow_id,
            version: WorkflowVersion::new(NonZeroU32::MIN),
            payload: input_codec, success: result_codec, error: validation_codec,
            compatibility: Arc::from(&b"pickup-labels-v1"[..]), limits: limits(),
            run |ticket: Ticket| {
                let run = activity_run.clone();
                let ticket = Arc::new(ticket);
                let description = label_activity.clone();
                activity! {
                    description: description,
                    run: run, attempt: NonZeroU32::MIN,
                    body || {
                        let ticket = ticket.clone();
                        let outcome = effect_try_sync! {
                            body {
                                if !(1..=4).contains(&ticket.cups) {
                                    return Ok(Err(TicketError::InvalidCups));
                                }
                                Ok::<_, TicketError>(Ok(Pickup {
                                    number: ticket.number.clone(), customer: ticket.customer.clone(),
                                    cups: ticket.cups,
                                }))
                            },
                        }?;
                        use std::io::Write;
                        let (outcome, written) = {
                            let written = match &outcome {
                                Ok(pickup) => writeln!(std::io::stdout().lock(), "Issue {} pickup labels: {} / ticket {}.", pickup.cups, pickup.customer, pickup.number),
                                Err(_) => Ok(()),
                            };
                            (outcome, written)
                        };
                        match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
                        outcome
                    },
                }?
            },
        };
        let requested = prepare.run(run, Ticket {
            number: "104".into(), customer: "Maya".into(), cups: 2,
        });
        match show_pickup(requested.clone()?) { Ok(()) => (), Err(error) => effect_die! { defect: error, }?, };
        match show_pickup(requested?) { Ok(()) => (), Err(error) => effect_die! { defect: error, }?, };
    }
}
/// Both normal completion and recorded replay expose the same owned pickup value.
fn show_pickup(result: WorkflowRunResult<Pickup, TicketError>) -> Result<(), effect::Defect> {
    match result {
        WorkflowRunResult::Completed(Ok(pickup)) => {
            use std::io::Write;
            writeln!(
                std::io::stdout().lock(),
                "Pickup: {} cups for {}, ticket {}.",
                pickup.cups,
                pickup.customer,
                pickup.number
            )
            .map_err(effect::Defect::from_io)?;
        }
        WorkflowRunResult::Completed(Err(error)) => {
            return Err(effect::Defect::new(format!("pickup rejected: {error:?}")));
        }
        WorkflowRunResult::Busy => return Err(effect::Defect::new("pickup run is busy")),
        WorkflowRunResult::Ready => {
            return Err(effect::Defect::new("pickup run is ready to continue"));
        }
        WorkflowRunResult::Suspended => {
            return Err(effect::Defect::new("pickup run is suspended"));
        }
        WorkflowRunResult::Cancelled => {
            return Err(effect::Defect::new("pickup run was cancelled"));
        }
        WorkflowRunResult::Abandoned => {
            return Err(effect::Defect::new("pickup ownership was abandoned"));
        }
    }
    Ok(())
}
/// R selects the local workflow store; the outer boundary runs and joins providers.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cafe = Cafe::open(
        (),
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(prepare_and_replay().run(cafe.clone()));
    cafe.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => (),
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("pickup preparation failed: {other:?}").into());
        }
    }
    Ok(())
}
