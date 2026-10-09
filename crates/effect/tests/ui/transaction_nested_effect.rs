//! Every transactional initializer remains on the journal boundary.
use mandrill_effect::transactional::Transaction;
use mandrill_effect::{Effect, transaction};
/// Native nesting cannot hide an incompatible Effect bind.
fn main() {
    let _: Transaction<i64, ()> = transaction! {let value={Effect::<i64,()>::sync(||1)?};value};
}
