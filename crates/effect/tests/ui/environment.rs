use mandrill_effect::{Effect, effect};
struct Environment;
fn main() {
    let _: Effect<usize, (), ()> = effect! {
        let value = Effect::<usize, (), Environment>::service(|_| 1)?;
        value
    };
}
