use effect::{
    Cancellation, CleanupFailure, Defect, Effect, Exit, Finalization, PrimaryOutcome, effect,
};
use futures_executor::block_on;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn run<A: Send + 'static, E: Send + 'static>(program: &Effect<A, E>) -> Exit<A, E> {
    block_on(program.run(Arc::new(())))
}

#[derive(Debug, PartialEq, Eq)]
struct Owned(String);

#[test]
fn reusable_macro_moves_non_clone_values_and_defers_all_statements() {
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let program: Effect<Owned, Owned> = effect! {
        let witness = Arc::clone(&witness);
        let Owned(value) = Effect::sync(move || Owned(witness.fetch_add(1, Ordering::SeqCst).to_string()))?;
        let suffix = Effect::sync(|| String::from("!"))?;
        let mut combined = value;
        combined.push_str(&suffix);
        Owned(combined)
    };
    let pending = program.run(Arc::new(()));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(block_on(pending), Exit::Success(Owned(String::from("0!"))));
    assert_eq!(run(&program), Exit::Success(Owned(String::from("1!"))));
}

#[test]
fn statements_tail_delegation_patterns_and_nested_scopes() {
    let program: Effect<usize, ()> = effect! {
        let ordinary = || -> Result<usize, ()> {
            let value = 3usize.checked_add(1).ok_or(())?;
            Ok(value + 1)
        };
        let (left, right) = Effect::succeed((2, 3))?;
        Effect::sync(|| ())?;
        let amount = ordinary().unwrap_or_default();
        return left + right + amount;
    };
    assert_eq!(run(&program), Exit::Success(10));
    let delegated: Effect<usize, ()> = effect! { Effect::succeed(42)? };
    assert_eq!(run(&delegated), Exit::Success(42));
    let empty: Effect<(), ()> = effect! {};
    assert_eq!(run(&empty), Exit::Success(()));
    let nested: Effect<usize, ()> = effect! {
        let value = effect! { Effect::succeed(7)? }?;
        value
    };
    assert_eq!(run(&nested), Exit::Success(7));
}

#[test]
fn macro_and_explicit_combinators_have_the_same_lazy_evaluation_order() {
    fn record(trace: &Mutex<Vec<&'static str>>, event: &'static str) {
        trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
    let macro_trace = Arc::new(Mutex::new(Vec::new()));
    let trace = Arc::clone(&macro_trace);
    let macro_program: Effect<usize, ()> = effect! {
        let first = Arc::clone(&trace);
        let second = Arc::clone(&trace);
        let tail = Arc::clone(&trace);
        record(&trace, "start");
        let left = Effect::sync(move || { record(&first, "first"); 2 })?;
        let right = Effect::sync(move || { record(&second, "second"); 3 })?;
        record(&tail, "tail");
        left + right
    };
    let explicit_trace = Arc::new(Mutex::new(Vec::new()));
    let trace = Arc::clone(&explicit_trace);
    let explicit_program: Effect<usize, ()> = Effect::defer(move || {
        record(&trace, "start");
        let first = Arc::clone(&trace);
        let second = Arc::clone(&trace);
        let tail = Arc::clone(&trace);
        Effect::sync(move || {
            record(&first, "first");
            2
        })
        .flat_map(move |left| {
            let second = Arc::clone(&second);
            let tail = Arc::clone(&tail);
            Effect::sync(move || {
                record(&second, "second");
                3
            })
            .map(move |right| {
                record(&tail, "tail");
                left + right
            })
        })
    });
    assert!(
        macro_trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
    assert!(
        explicit_trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
    for _ in 0..2 {
        assert_eq!(run(&macro_program), Exit::Success(5));
        assert_eq!(run(&explicit_program), Exit::Success(5));
    }
    let expected = vec![
        "start", "first", "second", "tail", "start", "first", "second", "tail",
    ];
    assert_eq!(
        *macro_trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        expected
    );
    assert_eq!(
        *explicit_trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        expected
    );
}

#[test]
fn stored_result_option_and_empty_traversal_preserve_their_channels() {
    let success: Effect<usize, String> = Effect::from_result(Ok(42));
    let failure: Effect<usize, String> = Effect::from_result(Err(String::from("expected")));
    assert_eq!(run(&success), Exit::Success(42));
    assert_eq!(
        run(&failure.result()),
        Exit::Success(Err(String::from("expected")))
    );
    let present: Effect<usize, String> = Effect::from_option(Some(42), || String::from("missing"));
    assert_eq!(run(&present), Exit::Success(42));
    let absent: Effect<usize, String> = Effect::from_option(None, || String::from("missing"));
    assert_eq!(run(&absent), Exit::Fail(String::from("missing")));
    assert_eq!(
        run(&Effect::<usize, ()>::all(Vec::new())),
        Exit::Success(Vec::new())
    );
}

#[test]
fn typed_environment_and_projections() {
    struct Config {
        port: u16,
    }
    struct Application {
        port: u16,
        label: String,
    }
    let program: Effect<String, (), Config> = effect! {
        let port = Effect::service(|config: &Config| config.port)?;
        format!("localhost:{port}")
    };
    assert_eq!(
        block_on(
            program
                .clone()
                .provide(Arc::new(Config { port: 443 }))
                .run(Arc::new(()))
        ),
        Exit::Success(String::from("localhost:443"))
    );
    let projected = program.provide_some(|app: &Application| Config { port: app.port });
    let app = Arc::new(Application {
        port: 8080,
        label: String::from("app"),
    });
    assert_eq!(app.label, "app");
    assert_eq!(
        block_on(projected.run(app)),
        Exit::Success(String::from("localhost:8080"))
    );
}

#[test]
fn expected_error_conversion_and_non_clone_error_factories() {
    let failure: Effect<usize, Owned> = Effect::fail_with(|| Owned(String::from("expected")));
    assert_eq!(
        run(&failure.clone().map_error(|error| error.0.len())),
        Exit::Fail(8)
    );
    assert_eq!(
        run(&failure
            .clone()
            .catch_all(|error| Effect::succeed(error.0.len()))),
        Exit::Success(8)
    );
    assert_eq!(
        run(&failure.clone().either()),
        Exit::Success(Err(Owned(String::from("expected"))))
    );
    assert_eq!(run(&failure.option()), Exit::Success(None));
    let defect: Effect<usize, String> = Effect::die(Defect::new("boom"));
    assert_eq!(
        run(&defect.clone().catch_all(|_| Effect::succeed(3))),
        Exit::Die(Defect::new("boom"))
    );
    assert_eq!(
        run(&defect.sandbox(|defect| defect.message().to_owned())),
        Exit::Fail(String::from("boom"))
    );
}

#[test]
fn error_mapping_preserves_every_finalization_error() {
    let exit: Exit<(), usize> = Exit::FinalizationFailed(Finalization {
        primary: PrimaryOutcome::Failed(1),
        first: CleanupFailure::ReleaseFailure(2),
        rest: vec![
            CleanupFailure::ReleaseFailure(3),
            CleanupFailure::ReleaseDefect(Defect::new("release")),
            CleanupFailure::ReleaseInterrupted,
        ],
    });
    assert_eq!(
        exit.map_error(|value| value.to_string()),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(String::from("1")),
            first: CleanupFailure::ReleaseFailure(String::from("2")),
            rest: vec![
                CleanupFailure::ReleaseFailure(String::from("3")),
                CleanupFailure::ReleaseDefect(Defect::new("release")),
                CleanupFailure::ReleaseInterrupted
            ],
        })
    );
}

#[test]
fn sequential_traversal_is_ordered_lazy_and_short_circuits_callback_creation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let program = Effect::<usize, usize>::for_each(vec![0, 1, 2, 3], move |value| {
        let _ = witness.fetch_add(1, Ordering::SeqCst);
        if value == 2 {
            Effect::fail(value)
        } else {
            Effect::succeed(value)
        }
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(run(&program), Exit::Fail(2));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let zipped: Effect<(Owned, Owned), ()> =
        Effect::sync(|| Owned(String::from("a"))).zip(Effect::sync(|| Owned(String::from("b"))));
    assert_eq!(
        run(&zipped),
        Exit::Success((Owned(String::from("a")), Owned(String::from("b"))))
    );
    let collected: Effect<Vec<usize>, ()> =
        Effect::all(vec![Effect::succeed(1), Effect::succeed(2)]);
    assert_eq!(run(&collected), Exit::Success(vec![1, 2]));
}

#[test]
fn cancellation_skips_start_and_success_continuation() {
    let token = Cancellation::new();
    assert!(token.request());
    let calls = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&calls);
    let program: Effect<usize, ()> = Effect::sync(move || witness.fetch_add(1, Ordering::SeqCst));
    assert_eq!(
        block_on(
            program
                .provide_some(|_: &Cancellation| ())
                .run_cancellable(Arc::new(token))
        ),
        Exit::Interrupted
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let token = Cancellation::new();
    let requester = token.clone();
    let witness = Arc::clone(&calls);
    let program: Effect<usize, ()> = effect! {
        let requester = requester.clone();
        let witness = Arc::clone(&witness);
        let value = Effect::sync(move || { let _ = requester.request(); 7 })?;
        let _ = witness.fetch_add(1, Ordering::SeqCst);
        value
    };
    assert_eq!(
        block_on(
            program
                .provide_some(|_: &Cancellation| ())
                .run_cancellable(Arc::new(token))
        ),
        Exit::Interrupted
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cooperative_yield_allows_sibling_progress() {
    let entered = Arc::new(AtomicBool::new(false));
    let witness = Arc::clone(&entered);
    let mut program: Effect<usize, ()> = Effect::succeed(0);
    for _ in 0..2000 {
        program = program.map(|value| value + 1);
    }
    let observed = Arc::clone(&entered);
    let program = program.map(move |value| (value, observed.load(Ordering::SeqCst)));
    let pending = program.run(Arc::new(()));
    let (outcome, ()) = block_on(futures_util::future::join(pending, async move {
        witness.store(true, Ordering::SeqCst);
    }));
    assert_eq!(outcome, Exit::Success((2000, true)));
    assert!(entered.load(Ordering::SeqCst));
}
