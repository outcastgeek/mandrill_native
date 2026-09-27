use mandrill_effect::{Effect, effect};
fn main() { let _: Effect<usize, ()> = effect! { let value = Ok::<usize, ()>(2)?; value }; }
