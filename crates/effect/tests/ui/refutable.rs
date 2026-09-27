use mandrill_effect::{Effect, effect};
fn main() { let _: Effect<usize, ()> = effect! { let Some(value) = Effect::succeed(Some(2))?; value }; }
