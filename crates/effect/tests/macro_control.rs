use effect::*;
use futures_executor::block_on;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[path = "support/drive.rs"]
mod drive;
fn resource(index: usize, releases: Arc<Mutex<Vec<usize>>>) -> ResourceBinding<usize, (), ()> {
    effect_resource! { binding; acquire: effect_sync! { body: move || index, }, release: move |index, _| {
        let releases = Arc::clone(&releases);
        effect_resource! { release_sync; body: move || {
            releases
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(index)
        }, }
    }, }
}
#[test]
fn loop_continue_and_labelled_break_await_each_lexical_release() {
    let releases = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&releases);
    let program: Effect<(), ()> = effect! {
        let releases = Arc::clone(&releases);
        'outer: for index in 0..3 {
            let _outer = resource(index, Arc::clone(&releases))?;
            if index == 0 { continue; }
            loop {
                let _inner = resource(10, Arc::clone(&releases))?;
                break 'outer;
            }
        }
        assert_eq!(releases.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_slice(), &[0,10,1]);
    };
    assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(()));
    assert_eq!(
        observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_slice(),
        &[0, 10, 1]
    );
}
#[test]
fn nested_returns_let_else_branch_mutation_and_nonclone_state_follow_rust_flow() {
    struct Owned(String);
    let program: Effect<String, ()> = effect! {
        let mut owned = Owned(String::new());
        for index in 0..3 {
            let value = effect_sync! { body: move || if index == 2 { None } else { Some(index) }, }?;
            let Some(value) = value else { return owned.0; };
            if value == 0 { owned.0.push('a'); } else { owned.0.push('b'); }
        }
        owned.0
    };
    for _ in 0..2 {
        assert_eq!(
            block_on(program.run(Arc::new(()))),
            Exit::Success(String::from("ab"))
        );
    }
}
#[test]
fn return_keeps_expected_primary_and_all_release_failures() {
    let program: Effect<(), usize> = effect! {
        for index in 0..2 {
            let _resource = effect_resource! { binding; acquire: effect_sync! { body: move || index, }, release: |index,_| effect_resource! { release_try_sync; body: move || Err(index), }, }?;
            if index == 0 { effect_try_sync! { body: { let __dsl_error = || 99; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }?; }
        }
    };
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(99),
            first: CleanupFailure::ReleaseFailure(0),
            rest: Vec::new()
        })
    );
}
#[test]
fn pure_macro_loops_cooperate_and_observe_cancellation_without_a_magic_batch() {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&count);
    let env = Arc::new(Cancellation::new());
    let program: Effect<(), (), Cancellation> = effect! {
        let count = Arc::clone(&count);
        loop { count.fetch_add(1,Ordering::SeqCst); }
    };
    let mut run = Box::pin(program.run_cancellable(Arc::clone(&env)));
    drive::until(run.as_mut(), || observed.load(Ordering::SeqCst) > 0);
    assert!(env.request());
    assert_eq!(block_on(run), Exit::Interrupted);
}

/// Native bodies remain delayed; once input ownership and Result propagation stay native.
#[test]
fn named_native_roles_preserve_delayed_once_ownership() {
    struct Owned(String);
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let input = Owned(String::from("Maya"));
    let program: Effect<String, &'static str> = effect_try_sync! {
        once;
        body {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(input.0)
        },
    };
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::Success(String::from("Maya"))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(block_on(program.run(Arc::new(()))), Exit::Die(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// Primary-aware release inspects a borrow then retains owned cleanup failures in order.
#[test]
fn named_resource_factory_retains_primary_and_cleanup() {
    let inspected = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&inspected);
    let program: Effect<(), usize> = effect_resource! {
        acquire: effect_sync! { once; body { String::from("104") } },
        body |_ticket| { { let __dsl_program: Effect < () , usize > = effect_try_sync! { body: { let __dsl_error = || 7; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; __dsl_program }? },
        release |ticket: String, primary| {
            assert!(matches!(primary, PrimaryOutcomeRef::Failed(&7)));
            observed.fetch_add(1, Ordering::SeqCst);
            effect_try_sync! { once; body { drop(ticket); Err(9usize) }, }
        },
    };
    assert_eq!(inspected.load(Ordering::SeqCst), 0);
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(7),
            first: CleanupFailure::ReleaseFailure(9),
            rest: Vec::new(),
        })
    );
    assert_eq!(inspected.load(Ordering::SeqCst), 1);
}

/// Mapping a stream's error channel maps retained cleanup without copying the primary.
#[test]
fn named_stream_mapping_retains_cleanup_owner() {
    let released = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&released);
    let source: Stream<usize, usize, ()> = stream! {
        pull: effect_sync! { body { String::from("104") } },
        read |_ticket| { { let __dsl_program: Effect < Chunk < usize > , usize > = effect_try_sync! { body: { let __dsl_error = || 7; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; __dsl_program }? },
        release |ticket: String| {
            let observed = observed.clone();
            drop(ticket); observed.fetch_add(1, Ordering::SeqCst);
            { let __dsl_program: Effect < () , usize > = effect_try_sync! { body: { let __dsl_error = || 9; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; __dsl_program }?
        },
    };
    let mapped = stream! { stream: source, map_error |error| { format!("ticket-{error}") }, };
    assert_eq!(released.load(Ordering::SeqCst), 0);
    assert_eq!(
        block_on(mapped.run_collect(None).run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(String::from("ticket-7")),
            first: CleanupFailure::ReleaseFailure(String::from("ticket-9")),
            rest: Vec::new(),
        })
    );
    assert_eq!(released.load(Ordering::SeqCst), 1);
}
