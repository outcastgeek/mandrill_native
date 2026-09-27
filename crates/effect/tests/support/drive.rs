use futures_executor::block_on;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::Poll;
use std::time::{Duration, Instant};

/// Reach an explicit execution signal without assuming a poll quantum.
pub fn until<F: Future + ?Sized>(mut future: Pin<&mut F>, reached: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    block_on(poll_fn(|cx| {
        if reached() {
            return Poll::Ready(());
        }
        assert!(
            future.as_mut().poll(cx).is_pending(),
            "execution settled before its entry signal"
        );
        if reached() {
            Poll::Ready(())
        } else {
            assert!(
                Instant::now() < deadline,
                "entry signal was not reached within five seconds"
            );
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }));
}
