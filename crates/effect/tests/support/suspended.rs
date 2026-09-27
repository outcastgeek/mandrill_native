use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Wake, Waker};
use std::time::{Duration, Instant};

/// Reach an actually blocked future, beyond cooperative self-wakes. These
/// callers have a native wait that cannot finish until the test signals it.
pub fn suspended<F: Future + ?Sized>(mut future: Pin<&mut F>) {
    struct Wakes(AtomicUsize);
    impl Wake for Wakes {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            let _ = self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let wakes = Arc::new(Wakes(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wakes));
    let mut context = Context::from_waker(&waker);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let before = wakes.0.load(Ordering::SeqCst);
        assert!(
            future.as_mut().poll(&mut context).is_pending(),
            "expected a blocked native wait"
        );
        if wakes.0.load(Ordering::SeqCst) == before {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "native suspension not reached within five seconds"
        );
    }
}
