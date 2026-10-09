//! Nested native futures require a recorded activity boundary.
use mandrill_effect::workflow;
use mandrill_effect::workflow::Durable;
/// Async syntax is not an unrestricted durable step.
fn main() {
    let _: Durable<i64, (), ()> = workflow! {let value={async {1i64}?};value};
}
