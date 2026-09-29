use mandrill_effect::workflow::Durable;
use mandrill_effect::workflow;

fn main() {
    let _: Durable<usize, (), ()> = workflow! {
        let value = async { 1usize }?;
        value
    };
}
