use mandrill_effect::{Effect, Scope};
fn main() {
    let _: Effect<_, (), ()> = Scope::scoped(|scope| scope.fork(Effect::sync(|| ())));
}
