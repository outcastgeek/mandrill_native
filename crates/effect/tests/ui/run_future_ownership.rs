use mandrill_effect::{Cancellation, Effect, Exit};
use std::{cell::Cell, future::Future, sync::Arc};

struct Data(Cell<usize>); // Neither Clone nor Sync.

fn plain() -> impl Future<Output = Exit<Data, Data>> + Send + 'static {
    let description: Effect<Data, Data> = Effect::sync(|| Data(Cell::new(0)));
    description.run(Arc::new(()))
}

fn cancellable() -> impl Future<Output = Exit<Data, Data>> + Send + 'static {
    let description: Effect<Data, Data, Cancellation> = Effect::sync(|| Data(Cell::new(0)));
    description.run_cancellable(Arc::new(Cancellation::new()))
}

fn generic<A: Send + 'static, E: Send + 'static, R: Send + Sync + 'static>(
    description: &Effect<A, E, R>,
    environment: Arc<R>,
) -> impl Future<Output = Exit<A, E>> + Send + 'static + use<A, E, R> {
    description.run(environment)
}

fn main() {
    drop(plain());
    drop(cancellable());
    let description: Effect<Data, Data> = Effect::sync(|| Data(Cell::new(0)));
    let future = generic(&description, Arc::new(()));
    drop(description);
    drop(future);
}
