use mandrill_effect::transactional::Transaction;
use mandrill_effect::{Effect, transaction};

fn main() {
    let _: Transaction<usize, ()> = transaction! {
        let value = Effect::<usize, ()>::sync(|| 1)?;
        value
    };
}
