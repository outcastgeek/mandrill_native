use effect::{Cancellation, Defect, Effect, Exit, HasCancellation, effect};
use futures_executor::block_on;
use futures_util::future::join;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Environment {
    value: usize,
    cancellation: Cancellation,
    accesses: Arc<AtomicUsize>,
}

impl HasCancellation for Environment {
    fn cancellation(&self) -> &Cancellation {
        let _ = self.accesses.fetch_add(1, Ordering::SeqCst);
        &self.cancellation
    }
}

fn environment(value: usize) -> Arc<Environment> {
    Arc::new(Environment {
        value,
        cancellation: Cancellation::new(),
        accesses: Arc::new(AtomicUsize::new(0)),
    })
}

#[test]
fn cancellation_capability_is_read_lazily_once_per_run() {
    let env = environment(7);
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let program: Effect<usize, (), Environment> = Effect::sync(move || {
        let _ = witness.fetch_add(1, Ordering::SeqCst);
        42
    });
    let pending = program.run_cancellable(Arc::clone(&env));
    assert_eq!(env.accesses.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(env.cancellation.request());
    assert_eq!(block_on(pending), Exit::Interrupted);
    assert_eq!(env.accesses.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn concurrent_runs_share_recipe_without_sharing_cancellation() {
    let entered = Cancellation::new();
    let entered_witness = entered.clone();
    let program: Effect<usize, (), Environment> = effect! {
        let entered_witness = entered_witness.clone();
        let value = Effect::service(|env: &Environment| env.value)?;
        Effect::from_future(move |cancel| {
            let entered_witness = entered_witness.clone();
            async move {
                if value == 1 {
                    let _ = entered_witness.request();
                    cancel.cancelled().await;
                }
                value
            }
        })?
    };
    let first = environment(1);
    let second = environment(2);
    let request = first.cancellation.clone();
    let ((cancelled, successful), ()) = block_on(join(
        join(
            program.run_cancellable(Arc::clone(&first)),
            program.run_cancellable(Arc::clone(&second)),
        ),
        async move {
            entered.cancelled().await;
            assert!(request.request());
        },
    ));
    assert_eq!(cancelled, Exit::Interrupted);
    assert_eq!(successful, Exit::Success(2));
    assert!(!second.cancellation.is_requested());
    assert_eq!(first.accesses.load(Ordering::SeqCst), 1);
    assert_eq!(second.accesses.load(Ordering::SeqCst), 1);
}

#[test]
fn projection_preserves_the_current_run_token_instead_of_restarting_it() {
    let root = environment(7);
    let other = Cancellation::new();
    assert!(other.request());
    let projected_accesses = Arc::new(AtomicUsize::new(0));
    let accesses = Arc::clone(&projected_accesses);
    let initially_cancelled = Arc::new(AtomicBool::new(true));
    let witnessed = Arc::clone(&initially_cancelled);
    let settled = Arc::new(AtomicBool::new(false));
    let finished = Arc::clone(&settled);
    let entered = Cancellation::new();
    let entered_witness = entered.clone();
    let program: Effect<usize, (), Environment> = effect! {
        let entered_witness = entered_witness.clone();
        let witnessed = Arc::clone(&witnessed);
        let finished = Arc::clone(&finished);
        let value = Effect::service(|env: &Environment| env.value)?;
        Effect::from_future(move |cancel| {
            let witnessed = Arc::clone(&witnessed);
            let finished = Arc::clone(&finished);
            let entered_witness = entered_witness.clone();
            async move {
                witnessed.store(cancel.is_requested(), Ordering::SeqCst);
                let _ = entered_witness.request();
                cancel.cancelled().await;
                finished.store(true, Ordering::SeqCst);
                value
            }
        })?
    };
    let projected = program.provide_some(move |env: &Environment| Environment {
        value: env.value + 1,
        cancellation: other.clone(),
        accesses: Arc::clone(&accesses),
    });
    let request = root.cancellation.clone();
    let (outcome, ()) = block_on(join(
        projected.run_cancellable(Arc::clone(&root)),
        async move {
            entered.cancelled().await;
            assert!(request.request());
        },
    ));
    assert_eq!(outcome, Exit::Interrupted);
    assert!(!initially_cancelled.load(Ordering::SeqCst));
    assert!(settled.load(Ordering::SeqCst));
    assert_eq!(root.accesses.load(Ordering::SeqCst), 1);
    assert_eq!(projected_accesses.load(Ordering::SeqCst), 0);
}

#[test]
fn uncancellable_boundary_requires_no_capability_and_does_not_probe_one() {
    struct Config {
        value: usize,
    }
    let plain: Effect<usize, (), Config> = Effect::service(|config: &Config| config.value);
    assert_eq!(
        block_on(plain.run(Arc::new(Config { value: 42 }))),
        Exit::Success(42)
    );

    let env = environment(7);
    assert!(env.cancellation.request());
    let program: Effect<usize, (), Environment> = Effect::service(|env: &Environment| env.value);
    assert_eq!(block_on(program.run(Arc::clone(&env))), Exit::Success(7));
    assert_eq!(env.accesses.load(Ordering::SeqCst), 0);
}

#[test]
#[allow(clippy::panic)] // An application-supplied capability accessor is a user-code boundary.
fn accessor_panics_are_reported_as_defects() {
    struct Broken;
    impl HasCancellation for Broken {
        fn cancellation(&self) -> &Cancellation {
            panic!("broken accessor")
        }
    }
    let program: Effect<usize, (), Broken> = Effect::succeed(42);
    assert_eq!(
        block_on(program.run_cancellable(Arc::new(Broken))),
        Exit::Die(Defect::from_panic(Box::new("broken accessor")))
    );
}
