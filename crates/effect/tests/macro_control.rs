use effect::*;
use futures_executor::block_on;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[path = "support/drive.rs"]
mod drive;
fn resource(index: usize, releases: Arc<Mutex<Vec<usize>>>) -> ResourceBinding<usize, (), ()> {
    ResourceBinding::new(Effect::sync(move || index), move |index, _| {
        let releases = Arc::clone(&releases);
        Release::sync(move || {
            releases
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(index)
        })
    })
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
            let value = Effect::sync(move || if index == 2 { None } else { Some(index) })?;
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
            let _resource = ResourceBinding::new(Effect::sync(move || index), |index,_| Release::try_sync(move || Err(index)))?;
            if index == 0 { Effect::fail_with(|| 99)?; }
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
