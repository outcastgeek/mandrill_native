use effect::{Defect, Effect, Exit, Release, ResourceBinding, effect};
use futures_executor::block_on;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Message(String);

#[test]
fn value_producing_if_else_and_match_move_non_clone_values() {
    for selector in [0, 1, 2] {
        let program: Effect<Message, &'static str> = effect! {
            let selector = Effect::succeed(selector)?;
            let message = if selector == 0 {
                let text = Effect::sync(|| String::from("zero"))?;
                Message(text)
            } else if selector == 1 {
                Effect::sync(|| Message(String::from("one")))?
            } else {
                Message(String::from("two"))
            };
            let message = match message {
                Message(text) if text == "one" => {
                    let text = Effect::sync(move || format!("{text}!"))?;
                    Message(text)
                },
                Message(text) => Message(text),
            };
            message
        };
        let text = match block_on(program.run(Arc::new(()))) {
            Exit::Success(Message(text)) => Some(text),
            Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_) => None,
        };
        assert_eq!(
            text.as_deref(),
            Some(match selector {
                0 => "zero",
                1 => "one!",
                _ => "two",
            })
        );
    }
}

#[test]
fn only_selected_branch_runs_and_failure_skips_the_outer_continuation() {
    let called = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&called);
    let program: Effect<usize, &'static str> = effect! {
        let witness = Arc::clone(&witness);
        let other = Arc::clone(&witness);
        let choice = Effect::succeed(true)?;
        let number = if choice {
            Effect::<usize, &'static str>::fail("chosen")?
        } else {
            let _ = other.fetch_add(1, Ordering::SeqCst);
            Effect::succeed(42)?
        };
        let _ = witness.fetch_add(1, Ordering::SeqCst);
        number
    };
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Fail("chosen"));
    assert_eq!(called.load(Ordering::SeqCst), 0);
}

#[test]
fn tail_branches_blocks_and_unit_statements_use_the_same_effect_channels() {
    let conditional: Effect<usize, ()> = effect! {
        if true { Effect::succeed(42)? } else { 0 }
    };
    assert_eq!(block_on(conditional.run(Arc::new(()))), Exit::Success(42));
    let matched: Effect<usize, ()> = effect! {
        match 1 {
            1 => Effect::succeed(42)?,
            _ => 0,
        }
    };
    assert_eq!(block_on(matched.run(Arc::new(()))), Exit::Success(42));
    let blocked: Effect<usize, ()> = effect! {
        let value = { let number = Effect::succeed(40)?; number + 2 };
        if value == 42 { Effect::<(), ()>::succeed(())?; }
        value
    };
    assert_eq!(block_on(blocked.run(Arc::new(()))), Exit::Success(42));
}

#[test]
fn pure_rust_control_flow_preserves_mutation_and_nested_native_closures() {
    let program: Effect<usize, ()> = effect! {
        let mut value = 0;
        if true { value = 40; }
        let extra = { value += 1; value };
        let native = (|| -> Result<_, ()> { let delta = Ok::<_, ()>(1)?; Ok(extra + delta) })();
        let finish = Effect::from_result(native)?;
        finish
    };
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(42));
}

#[test]
fn a_resource_bound_in_a_branch_releases_before_the_branch_result_is_published() {
    let count = Arc::new(AtomicUsize::new(0));
    let witness = Arc::clone(&count);
    let program: Effect<usize, ()> = effect! {
        let witness = Arc::clone(&witness);
        let size = if true {
            let resource = ResourceBinding::new(
                Effect::sync(|| String::from("branch")),
                move |value, _| {
                    let witness = Arc::clone(&witness);
                    Release::sync(move || { drop(value); let _ = witness.fetch_add(1, Ordering::SeqCst); })
                },
            )?;
            resource.access(String::len)?
        } else { 0 };
        size
    };
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(6));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
#[allow(clippy::panic)] // Branch computation is part of the per-run factory boundary.
fn branch_panics_are_defects() {
    fn fail(value: usize) -> usize {
        panic!("branch {value}")
    }
    let program: Effect<usize, ()> = effect! {
        if true {
            let value = Effect::succeed(42)?;
            fail(value)
        } else { 0 }
    };
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("branch 42")))
    );
}
