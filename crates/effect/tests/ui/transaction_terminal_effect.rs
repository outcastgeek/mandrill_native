//! External Effects cannot enter terminal transactional branches.
use mandrill_effect::transactional::Transaction;
use mandrill_effect::{Effect, transaction};
/// The terminal try is checked against Transaction even in a nested branch.
fn main() {
    let _: Transaction<i64, ()> = transaction! {if true {Effect::<i64,()>::sync(||1)?}else{0}};
}
