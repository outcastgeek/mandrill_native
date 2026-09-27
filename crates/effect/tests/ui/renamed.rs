use mandrill_effect::{Effect, effect};
use std::sync::Arc;
struct Owned(String);
fn main() {
    let program: Effect<Owned, ()> = effect! {
        let value = Effect::sync(|| Owned(String::from("owned")))?;
        let suffix = Effect::succeed("!")?;
        Owned(value.0 + suffix)
    };
    let _ = program.run(Arc::new(()));
}
