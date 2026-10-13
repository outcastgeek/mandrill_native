//! Resolve requested pickup records in one batch, retaining unknown-ticket results.
//!
//! Run: `cargo +1.99.0 run -p effect --example resolve_pickup_tickets`.
//! Input: ticket numbers 104, 105 and 999. Output: Maya's two cups, Lee's one cup,
//! then an unknown-ticket result. Results retain request order and shared records.

use effect::effect_die;
pub mod cafe_support;
use cafe_support::{Cafe, key};
use effect::{
    Effect, Exit, Request, RequestGroup, RequestKey, RequestResolverError, Scope, effect_scope,
    request_resolver, service,
};
use std::num::NonZeroUsize;
use std::{collections::BTreeMap, sync::Arc};

/// A scoped resolver shares one record lookup across the requested batch.
fn resolve_tickets() -> Effect<(), RequestResolverError<UnknownTicket>, Cafe<Records>> {
    effect_scope! {
        body |scope: Scope<RequestResolverError<UnknownTicket>>| {
            let records = service! { env |env: &Cafe<Records>| { env.services.clone() }, }?;
            let resolver = request_resolver! {
                scope: scope,
                run |_group, numbers: Vec<u64>| {
                    let records = records.clone();
                    numbers.into_iter().map(|number| match records.get(&number) {
                        Some(ticket) => Exit::Success(ticket.clone()),
                        None => Exit::Fail(UnknownTicket(number)),
                    }).collect::<Vec<_>>()
                },
            }?;
            let group = RequestGroup::from_stable(match key("pickup-desk") { Ok(value) => value, Err(error) => effect_die! { defect: error, }?, });
            let requests = [104, 105, 999].into_iter().map(|number| {
                let key = key(&format!("ticket-{number}"))?;
                Ok(Request::new(RequestKey::from_stable(key), group.clone(), number))
            }).collect::<Result<Vec<_>, effect::Defect>>();
            let requests = match requests { Ok(requests) => requests, Err(error) => effect_die! { defect: error, }?, };
            for outcome in resolver.resolve_many(requests)? {
                let text = match outcome {
                    Exit::Success(record) => record.with(|ticket| format!("{}: {} cup(s).", ticket.customer, ticket.cups)),
                    Exit::Fail(RequestResolverError::RequestFailed(error)) => error.with(|unknown| format!("Unknown ticket {}.", unknown.0)),
                    other @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => effect_die! { defect: effect::Defect::new(format!("lookup did not settle: {other:?}")), }?,
                };
                let text = match text { Ok(text) => text, Err(error) => effect_die! { defect: error, }?, };
                use std::io::Write;
                let written = writeln!(std::io::stdout().lock(), "{text}");
                match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, };
            }
        },
    }
}
/// The example's actual pickup records, indexed by printed number.
type Records = BTreeMap<u64, Arc<Pickup>>;
/// Immutable pickup data; sharing a record requires no payload Clone.
#[derive(Debug)]
struct Pickup {
    /// Customer called at the desk.
    customer: String,
    /// Cups waiting for this customer.
    cups: usize,
}
/// A missing requested record, retained independently from other request outcomes.
#[derive(Debug)]
struct UnknownTicket(#[doc = "Requested pickup number."] u64);
/// Supply two records and join the resolver's native work before returning.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let records = BTreeMap::from([
        (
            104,
            Arc::new(Pickup {
                customer: "Maya".into(),
                cups: 2,
            }),
        ),
        (
            105,
            Arc::new(Pickup {
                customer: "Lee".into(),
                cups: 1,
            }),
        ),
    ]);
    let env = Cafe::open(
        records,
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(resolve_tickets().run(env.clone()));
    env.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => (),
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("ticket lookup failed: {other:?}").into());
        }
    }
    Ok(())
}
