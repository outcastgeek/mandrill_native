use effect::{Cancellation, Effect, Exit, HasCancellation, effect};
use std::sync::Arc;

struct Application {
    port: u16,
    cancellation: Cancellation,
}

impl HasCancellation for Application {
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AppError {
    InvalidPort,
}

fn endpoint() -> Effect<String, AppError, Application> {
    effect! {
        let port = Effect::service(|env: &Application| env.port)?;
        Effect::try_sync(move || {
            if port == 0 { Err(AppError::InvalidPort) } else { Ok(()) }
        }, |error| error)?;
        format!("127.0.0.1:{port}")
    }
}

fn main() {
    let env = Arc::new(Application {
        port: 8080,
        cancellation: Cancellation::new(),
    });
    let program = endpoint();
    assert_eq!(
        futures_executor::block_on(program.run_cancellable(Arc::clone(&env))),
        Exit::Success(String::from("127.0.0.1:8080"))
    );
    assert!(env.cancellation.request());
    assert_eq!(
        futures_executor::block_on(program.run_cancellable(env)),
        Exit::Interrupted
    );
}
