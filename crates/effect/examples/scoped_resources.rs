use effect::{Effect, Exit, Release, Scope, effect};
use futures_executor::block_on;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn main() {
    let releases = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&releases);
    let program: Effect<usize, String> = Scope::scoped(move |scope| {
        let witness = Arc::clone(&witness);
        let acquisition = scope.acquire(Effect::sync(|| String::from("document")), move |value| {
            let witness = Arc::clone(&witness);
            Release::from_future(move |masked| async move {
                assert!(!masked.is_requested());
                drop(value);
                let _ = witness.fetch_add(1, Ordering::SeqCst);
            })
        });
        effect! {
            let acquisition = acquisition.clone();
            let handle = acquisition?;
            let size = handle.access(String::len)?;
            if size > 0 { Effect::succeed(size)? } else { 0 }
        }
    });
    for _ in 0..2 {
        let (driver, observer) = program.owned_run(Arc::new(()));
        block_on(driver);
        assert_eq!(block_on(observer), Exit::Success(8));
    }
    assert_eq!(releases.load(Ordering::SeqCst), 2);
}
