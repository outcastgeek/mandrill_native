use mandrill_effect::workflow::Durable;
use mandrill_effect::{Effect, workflow};

fn main() {
    let _: Durable<usize, (), ()> = workflow! {
        let value = Effect::<usize, ()>::sync(|| 1)?;
        value
    };
}
