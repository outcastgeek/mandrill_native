use mandrill_effect::Effect;
use std::sync::Arc;

fn main() {
    let program: Effect<usize, (), ()> = Effect::succeed(42);
    let _future = program.run_cancellable(Arc::new(()));
}
