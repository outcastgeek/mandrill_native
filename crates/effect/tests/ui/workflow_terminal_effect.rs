//! Terminal durable branches reject unrecorded external Effects.
use mandrill_effect::workflow::Durable;
use mandrill_effect::{Effect, workflow};
/// A native match retains durable admission checking.
fn main() {
    let _: Durable<i64, (), ()> =
        workflow! {match true{true=>Effect::<i64,()>::sync(||1)?,false=>0}};
}
