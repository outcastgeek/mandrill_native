use effect::{Defect, Effect, Exit, effect};
use effect::{effect_resource, effect_sync, effect_try_sync};
use futures_executor::block_on;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Message(String);

#[test]
fn value_producing_if_else_and_match_move_non_clone_values() {
    for selector in [0, 1, 2] {
        let program: Effect<Message, &'static str> = effect! {
            let selector = effect_sync! { body: { let __dsl_value = selector; move || __dsl_value }, }?;
            let message = if selector == 0 {
                let text = effect_sync! { body: || String::from("zero"), }?;
                Message(text)
            } else if selector == 1 {
                effect_sync! { body: || Message(String::from("one")), }?
            } else {
                Message(String::from("two"))
            };
            let message = match message {
                Message(text) if text == "one" => {
                    let text = effect_sync! { body: move || format!("{text}!"), }?;
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
        let choice = effect_sync! { body: { let __dsl_value = true; move || __dsl_value }, }?;
        let number = if choice {
            { let __dsl_program: Effect < usize , & 'static str > = effect_try_sync! { body: move || Err("chosen"), on_error: ::core::convert::identity, }; __dsl_program }?
        } else {
            let _ = other.fetch_add(1, Ordering::SeqCst);
            effect_sync! { body: { let __dsl_value = 42; move || __dsl_value }, }?
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
        if true { effect_sync! { body: { let __dsl_value = 42; move || __dsl_value }, }? } else { 0 }
    };
    assert_eq!(block_on(conditional.run(Arc::new(()))), Exit::Success(42));
    let matched: Effect<usize, ()> = effect! {
        match 1 {
            1 => effect_sync! { body: { let __dsl_value = 42; move || __dsl_value }, }?,
            _ => 0,
        }
    };
    assert_eq!(block_on(matched.run(Arc::new(()))), Exit::Success(42));
    let blocked: Effect<usize, ()> = effect! {
        let value = { let number = effect_sync! { body: { let __dsl_value = 40; move || __dsl_value }, }?; number + 2 };
        if value == 42 { { let __dsl_program: Effect < () , () > = effect_sync! { body: { let __dsl_value = (); move || __dsl_value }, }; __dsl_program }?; }
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
        let finish = effect_try_sync! { body: { let __dsl_result = native; move || __dsl_result }, on_error: ::core::convert::identity, }?;
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
            let resource = effect_resource! { binding; acquire: effect_sync! { body: || String::from("branch"), }, release: move |value, _| {
                    let witness = Arc::clone(&witness);
                    effect_resource! { release_sync; body: move || { drop(value); let _ = witness.fetch_add(1, Ordering::SeqCst); }, }
                }, }?;
            resource.access(String::len)?
        } else { 0 };
        size
    };
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(6));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn branch_panics_are_defects() {
    fn fail(value: usize) -> usize {
        std::panic::resume_unwind(Box::new(format!("branch {value}")))
    }
    let program: Effect<usize, ()> = effect! {
        if true {
            let value = effect_sync! { body: { let __dsl_value = 42; move || __dsl_value }, }?;
            fail(value)
        } else { 0 }
    };
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::Die(Defect::from_panic(Box::new("branch 42")))
    );
}
