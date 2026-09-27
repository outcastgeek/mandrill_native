use fx::{
    Cancellation, Defect, DefectKind, Effect, Exit, HasCancellation, LibraryDefect, Release,
    ResourceBinding, ResourceValue, Scope, effect,
};
use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Poll;

#[derive(Default)]
pub struct Environment {
    pub offset: usize,
    pub cancellation: Cancellation,
}

impl HasCancellation for Environment {
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}

pub fn workflow() -> Effect<String, String, Environment> {
    effect! {
        let offset = Effect::service(|env: &Environment| env.offset)?;
        let value = Effect::from_future(|_| {
            let mut suspended = false;
            poll_fn(move |cx| {
                if suspended { Poll::Ready(7) }
                else { suspended = true; cx.waker().wake_by_ref(); Poll::Pending }
            })
        })?;
        format!("result-{}", value + offset)
    }
}

pub async fn portable_cases() {
    let program = workflow();
    let env = Arc::new(Environment {
        offset: 3,
        ..Environment::default()
    });
    assert_eq!(
        program.run(Arc::clone(&env)).await,
        Exit::Success(String::from("result-10"))
    );
    assert_eq!(
        program.run(Arc::clone(&env)).await,
        Exit::Success(String::from("result-10"))
    );
    assert!(env.cancellation.request());
    assert_eq!(program.run_cancellable(env).await, Exit::Interrupted);
    let failure: Effect<String, String> = Effect::fail(String::from("expected"));
    assert_eq!(
        failure.run(Arc::new(())).await,
        Exit::Fail(String::from("expected"))
    );
    let defect: Effect<String, String> = Effect::die(Defect::new("unexpected"));
    assert_eq!(
        defect.run(Arc::new(())).await,
        Exit::Die(Defect::new("unexpected"))
    );
    let releases = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&releases);
    let resource_program: Effect<usize, String, Environment> = effect! {
        let witness = Arc::clone(&witness);
        let resource = ResourceBinding::new(Effect::sync(|| String::from("owned")), move |value, _reason| {
            let witness = Arc::clone(&witness);
            Release::from_future(move |masked| async move {
                assert!(!masked.is_requested());
                drop(value);
                let _ = witness.fetch_add(1, Ordering::SeqCst);
            })
        })?;
        let length = resource.access(String::len)?;
        length
    };
    let env = Arc::new(Environment {
        offset: 3,
        ..Environment::default()
    });
    let (driver, observer) = resource_program.owned_run_cancellable(env);
    driver.await;
    assert_eq!(observer.await, Exit::Success(5));
    assert_eq!(releases.load(Ordering::SeqCst), 1);

    let count = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&count);
    let scoped: Effect<usize, String, Environment> = Scope::scoped(move |scope| {
        let witness = Arc::clone(&witness);
        let acquired = scope.acquire(Effect::sync(|| String::from("portable")), move |value| {
            let witness = Arc::clone(&witness);
            Release::from_future(move |masked| async move {
                assert!(!masked.is_requested());
                drop(value);
                let _ = witness.fetch_add(1, Ordering::SeqCst);
            })
        });
        effect! {
            let acquired = acquired.clone();
            let handle = acquired?;
            let size = handle.access(String::len)?;
            if size > 0 { Effect::succeed(size)? } else { 0 }
        }
    });
    let env = Arc::new(Environment::default());
    for _ in 0..2 {
        assert_eq!(
            scoped.run_cancellable(Arc::clone(&env)).await,
            Exit::Success(8)
        );
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);

    // Every host exposes the same typed failure for an escaped scoped handle;
    // diagnostic text is not a matching contract or a substitute identity.
    let escaped: Effect<ResourceValue<String>, String> = Scope::scoped(|scope| {
        scope.acquire(Effect::sync(String::new), |value| {
            Release::sync(move || drop(value))
        })
    });
    let actual = escaped.run(Arc::new(())).await;
    assert!(matches!(actual, Exit::Success(_)));
    if let Exit::Success(handle) = actual {
        let failure = handle.with(String::len);
        assert_eq!(
            failure,
            Err(Defect::from(LibraryDefect::ResourceScopeEnded))
        );
        if let Err(defect) = failure {
            assert_eq!(
                defect.kind(),
                DefectKind::Library(LibraryDefect::ResourceScopeEnded)
            );
            assert_ne!(defect, Defect::new(defect.message()));
        }
    }
}

pub fn assert_send_future(_: impl Future + Send) {}
