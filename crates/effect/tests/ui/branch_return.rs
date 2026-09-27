use mandrill_effect::{Effect, effect};
fn main() {
    let _program: Effect<usize, ()> = effect! {
        let value = if true { let value = Effect::succeed(1)?; return value; } else { 0 };
        value + 1
    };
}
