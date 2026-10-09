//! Compatible helpers compose under a renamed dependency without cloning payloads.
use mandrill_effect::cluster::EntityResourcePlan;
use mandrill_effect::reactivity::AtomProgram;
use mandrill_effect::{Effect, Stream, effect, service, stream};
use std::cell::Cell;
/// A sendable payload with neither Clone nor Sync.
struct Owned(
    /// Interior-mutable data transferred through each typed bind.
    Cell<i64>,
);
/// Produce one fresh owned payload per evaluation.
fn helper<R: Send + Sync + 'static>() -> Effect<Owned, (), R> {
    Effect::sync(|| Owned(Cell::new(3)))
}
/// Type-check shared helper admission while retaining the caller's environment.
fn main() {
    let _: Effect<i64, (), ()> =
        service! {select:|_:&()|2i64,run |selected|{let value=helper()?;value.0.get()+*selected}};
    let _: Stream<i64, (), ()> = stream! {let prefix={yield 0i64;1i64};yield prefix;let value=helper()?;yield value.0.get();yield_from!(Stream::from_iterable(||[4].into_iter()));};
    let _: AtomProgram<i64, (), ()> = mandrill_effect::atom! {let value=helper()?;value.0.get()};
    let _: EntityResourcePlan<i64, (), ()> =
        mandrill_effect::entity_resource! {let value=helper()?;value.0.get()};
    let _: Effect<i64, (), ()> =
        effect! {let value=helper()?;match value.0.get(){v if v>0=>v,_=>0}};
}
