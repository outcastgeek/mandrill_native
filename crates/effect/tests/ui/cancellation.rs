use mandrill_effect::{Cancellation, Effect, Exit, HasCancellation, effect};
use std::sync::Arc;

struct Application { value: usize, cancellation: Cancellation }
impl HasCancellation for Application {
    fn cancellation(&self) -> &Cancellation { &self.cancellation }
}

fn main() {
    let program: Effect<usize, (), Application> = effect! {
        let value = Effect::service(|env: &Application| env.value)?;
        value + 1
    };
    let env = Arc::new(Application { value: 41, cancellation: Cancellation::new() });
    let _future: Box<dyn std::future::Future<Output = Exit<usize, ()>> + Send> =
        Box::new(program.run_cancellable(env));
}
