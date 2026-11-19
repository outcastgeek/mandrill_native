//! Resolve requested pickup records in one batch, retaining unknown-ticket results.
//!
//! Run: `cargo +1.100.0 run -p effect --example resolve_pickup_tickets`.
//! Input: ticket numbers 104, 105 and 999. Output: Maya's two cups, Lee's one cup,
//! then an unknown-ticket result. Results retain request order and shared records.

use effect::effect_die;
pub mod cafe_support;
use cafe_support::{Cafe, key};
use effect::memory::{Bytes, RegionId};
use effect::{
    Effect, Exit, PreparedRequestBatch, PreparedResolverCapacity, PreparedResolverIndex, Request,
    RequestGroup, RequestKey, RequestResolverError, Scope, effect_scope, effect_try_sync, memory,
    request_resolver, service,
};
use std::num::NonZeroUsize;
use std::{collections::BTreeMap, sync::Arc};

/// A scoped resolver shares one record lookup across the requested batch.
fn resolve_tickets(
    index: PreparedResolverIndex<u64, Arc<Pickup>, UnknownTicket>,
    group: RequestGroup,
    keys: [(RequestKey, u64); 3],
) -> Effect<(), RequestResolverError<UnknownTicket>, Cafe<Records>> {
    let index = Arc::new(index);
    effect_scope! {
        body |scope: Scope<RequestResolverError<UnknownTicket>>| {
            let index = index.try_clone();
            let requests = keys.clone().map(|(key, number)| Request::new(key, group.clone(), number));
            let records = service! { env |env: &Cafe<Records>| { env.services.clone() }, }?;
            let index = effect_try_sync! {
                once;
                body: move || index,
                on_error: RequestResolverError::Memory,
            }?;
            let resolver = request_resolver! {
                scope: scope,
                memory: index,
                on_capacity |error| { RequestResolverError::Memory(error) },
                run_prepared |_group, mut batch: PreparedRequestBatch<u64, Arc<Pickup>, UnknownTicket>| {
                    let records = records.clone();
                    while let Some(number) = batch.next_request() {
                        let outcome = match records.get(&number) {
                            Some(ticket) => Exit::Success(ticket.clone()),
                            None => Exit::Fail(UnknownTicket(number)),
                        };
                        if batch.push(outcome).is_err() {
                            break;
                        }
                    }
                    batch
                },
            }?;
            for outcome in resolver.resolve_many_prepared(requests)? {
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
    let setup_key = |name| key(name).map_err(|error| error.message().to_owned());
    let group = RequestGroup::from_stable(setup_key("pickup-desk")?);
    let keys = [
        (RequestKey::from_stable(setup_key("ticket-104")?), 104),
        (RequestKey::from_stable(setup_key("ticket-105")?), 105),
        (RequestKey::from_stable(setup_key("ticket-999")?), 999),
    ];
    let capacity = PreparedResolverCapacity {
        pending: 8,
        invocations: 2,
        requests_per_batch: 4,
        key_bytes: 96,
        group_bytes: 96,
        retained_values: 16,
        cleanup_copies: 48,
        cleanup_retained: 2,
        child_cancellation_listeners: 1,
    };
    let resolver_bytes =
        PreparedResolverIndex::<u64, Arc<Pickup>, UnknownTicket>::storage_bytes_with_batches::<
            Cafe<Records>,
        >(capacity)?
        .checked_add(Bytes::new(16 * 1024))?;
    let total = resolver_bytes.checked_add(Bytes::new(64 * 1024))?;
    let memory = memory! {
        total: total,
        mode: prepared,
        metadata: metadata,
        partition metadata: 32 KiB,
        partition resolver: resolver_bytes,
    }?
    .prepare()?;
    let index = PreparedResolverIndex::prepare_with_batches::<Cafe<Records>>(
        &memory,
        RegionId::named("resolver"),
        capacity,
    )?;
    let _memory = memory.seal();
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
    let result = futures_executor::block_on(resolve_tickets(index, group, keys).run(env.clone()));
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
