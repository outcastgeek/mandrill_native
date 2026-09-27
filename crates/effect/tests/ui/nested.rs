use mandrill_effect::{Effect, effect};
fn main() { let _: Effect<usize, ()> = effect! { 1 + Effect::succeed(2)? }; }
