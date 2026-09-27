use effect::{Effect, Exit, effect};
use std::future::poll_fn;
use std::sync::Arc;
use std::task::Poll;

fn workflow() -> Effect<usize, ()> {
    effect! {
        let value = Effect::from_future(|_| {
            let mut suspended = false;
            poll_fn(move |cx| {
                if suspended { Poll::Ready(21) }
                else {
                    suspended = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            })
        })?;
        value * 2
    }
}

fn main() {
    assert_eq!(
        futures_executor::block_on(workflow().run(Arc::new(()))),
        Exit::Success(42)
    );
}
