use mandrill_effect::{Effect, effect};
fn main() {
    let _program: Effect<usize, ()> = effect! {
        let mut value = 0;
        if true { let delta = Effect::succeed(1)?; value += delta; }
        value
    };
}
