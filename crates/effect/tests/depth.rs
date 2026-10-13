use effect::{Cancellation, Effect, Exit};
use effect::{
    effect, effect_finalizer, effect_future, effect_map, effect_map_error, effect_provide_some,
    effect_resource, effect_scope, effect_sync, effect_try_sync,
};
use futures_executor::block_on;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[path = "support/drive.rs"]
mod drive;

fn recursion(remaining: usize) -> Effect<usize, ()> {
    {
        let __dsl_factory = move || {
            if remaining == 0 {
                effect_sync! { body: { let __dsl_value = 42; move || __dsl_value }, }
            } else {
                recursion(remaining - 1)
            }
        };
        effect! { __dsl_factory()? }
    }
}

thread_local! { static THREAD_GRAPH: std::cell::RefCell<Option<Effect<usize, ()>>> = const { std::cell::RefCell::new(None) }; }

fn graph() -> Effect<usize, ()> {
    let mut program = effect_sync! { body: { let __dsl_value = 0; move || __dsl_value }, };
    for _ in 0..100_000 {
        program = effect! {
        bind;
        program: program,
        body: |value| effect_sync! { body: { let __dsl_value = value + 1; move || __dsl_value }, },
        };
    }
    program
}

#[test]
fn deep_graphs_in_subprocess() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("EFFECT_DEPTH_CHILD").is_some() {
        // Match the existing 100,000-link continuation stress workload for
        // linked cancellation propagation and destruction, not runtime policy.
        let root = Cancellation::new();
        let mut leaf = root.clone();
        for _ in 0..100_000 {
            leaf = leaf.child();
        }
        assert!(root.request());
        assert!(leaf.is_requested());
        drop(root);
        drop(leaf);
        let mut unrequested = Cancellation::new();
        for _ in 0..100_000 {
            unrequested = unrequested.child();
        }
        drop(unrequested);

        let program = graph();
        assert_eq!(block_on(program.run(Arc::new(()))), Exit::Success(100_000));
        drop(program);
        drop(graph());
        assert_eq!(
            block_on(recursion(100_000).run(Arc::new(()))),
            Exit::Success(42)
        );
        let token = Cancellation::new();
        assert!(token.request());
        assert_eq!(
            block_on(
                effect_provide_some! { program: graph(), project: |_: &Cancellation| (), }
                    .run_cancellable(Arc::new(token))
            ),
            Exit::Interrupted
        );
        let mut failed: Effect<usize, usize> = effect_try_sync! { body: { let __dsl_error = 42; move || Err(__dsl_error) }, on_error: ::core::convert::identity, };
        for _ in 0..100_000 {
            failed = effect! {
            bind;
            program: failed,
            body: |value| effect_sync! { body: { let __dsl_value = value + 1; move || __dsl_value }, },
            };
        }
        assert_eq!(block_on(failed.run(Arc::new(()))), Exit::Fail(42));
        drop(failed);
        let mut recovery: Effect<usize, usize> = effect_try_sync! { body: { let __dsl_error = 0; move || Err(__dsl_error) }, on_error: ::core::convert::identity, };
        for _ in 0..100_000 {
            recovery = effect_map_error! { program: recovery, map: |error| error + 1, };
        }
        assert_eq!(block_on(recovery.run(Arc::new(()))), Exit::Fail(100_000));
        drop(recovery);

        // Poll down to a genuinely suspended leaf before abandoning a deep
        // continuation chain, rather than only destroying an unpolled future.
        let entered = Arc::new(AtomicBool::new(false));
        let witness = Arc::clone(&entered);
        let mut pending: Effect<usize, ()> = effect_future! { body: move |_| {
            witness.store(true, Ordering::SeqCst);
            std::future::pending()
        }, };
        for _ in 0..100_000 {
            pending = effect_map! { program: pending, map: |value| value + 1, };
        }
        let mut running = Box::pin(pending.run(Arc::new(())));
        drive::until(running.as_mut(), || entered.load(Ordering::SeqCst));
        drop(running);
        drop(pending);

        struct Probe(Arc<AtomicUsize>);
        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let destroyed = Arc::new(AtomicUsize::new(0));
        let mut observed: Effect<usize, ()> =
            effect_sync! { body: { let __dsl_value = 0; move || __dsl_value }, };
        for _ in 0..100_000 {
            let probe = Probe(Arc::clone(&destroyed));
            observed = effect! {
            bind;
            program: observed,
            body: move |value| {
                           let _ = &probe;
                           effect_sync! { body: { let __dsl_value = value + 1; move || __dsl_value }, }
                       },
            };
        }
        let shared = observed.clone();
        drop(observed);
        assert_eq!(destroyed.load(Ordering::SeqCst), 0);
        drop(shared);
        assert_eq!(destroyed.load(Ordering::SeqCst), 100_000);

        let released = Arc::new(AtomicUsize::new(0));
        let mut bracketed: Effect<usize, ()> =
            effect_sync! { body: { let __dsl_value = 42; move || __dsl_value }, };
        for _ in 0..10_000 {
            let inner = bracketed;
            let witness = Arc::clone(&released);
            bracketed = effect_resource! {
            acquire: effect_sync! { body: { let __dsl_value = (); move || __dsl_value }, }, release: move |(), _| {
                            let witness = Arc::clone(&witness);
                            effect_resource! { release_sync; body: move || {
                                let _ = witness.fetch_add(1, Ordering::SeqCst);
                            }, }
                        }, body: move |_| inner.clone(),
            };
        }
        assert_eq!(block_on(bracketed.run(Arc::new(()))), Exit::Success(42));
        assert_eq!(released.load(Ordering::SeqCst), 10_000);
        drop(bracketed);

        let scoped_releases = Arc::new(AtomicUsize::new(0));
        let mut scoped: Effect<usize, ()> =
            effect_sync! { body: { let __dsl_value = 42; move || __dsl_value }, };
        for _ in 0..10_000 {
            let inner = scoped;
            let witness = Arc::clone(&scoped_releases);
            scoped = effect_scope! {
            body: move |scope| {
                            let witness = Arc::clone(&witness);
                            let registration = effect_finalizer! { release; scope: scope, release: move || {
                                let witness = Arc::clone(&witness);
                                effect_resource! { release_sync; body: move || {
                                    let _ = witness.fetch_add(1, Ordering::SeqCst);
                                }, }
                            }, };
                            let inner = inner.clone();
                            effect! {
             bind;
             program: registration,
             body: move |()| inner.clone(),
             }
                        },
            };
        }
        assert_eq!(block_on(scoped.run(Arc::new(()))), Exit::Success(42));
        assert_eq!(scoped_releases.load(Ordering::SeqCst), 10_000);
        drop(scoped);

        let iterations = Arc::new(AtomicUsize::new(0));
        let guard = Arc::clone(&iterations);
        let witness = Arc::clone(&iterations);
        let repeated: Effect<(), ()> = effect! { while_loop; guard: move || guard.load(Ordering::SeqCst) < 100_000, body: move || {
            let witness = Arc::clone(&witness);
            effect_sync! { body: move || {
                let _ = witness.fetch_add(1, Ordering::SeqCst);
            }, }
        }, };
        assert_eq!(block_on(repeated.run(Arc::new(()))), Exit::Success(()));
        assert_eq!(iterations.load(Ordering::SeqCst), 100_000);

        // This would monopolize a poll forever without cooperative handoff.
        // Keep the proof in this bounded subprocess so a regression is fatal
        // only to the child, rather than hanging the integration-test runner.
        let token = Arc::new(Cancellation::new());
        let request = Arc::clone(&token);
        let entered = Cancellation::new();
        let witness = entered.clone();
        let forever: Effect<(), (), Cancellation> = effect! { while_loop; guard: || true, body: move || {
            let witness = witness.clone();
            effect_sync! { body: move || {
                let _ = witness.request();
            }, }
        }, };
        let (outcome, ()) = block_on(futures_util::future::join(
            forever.run_cancellable(token),
            async move {
                entered.cancelled().await;
                assert!(request.request());
            },
        ));
        assert_eq!(outcome, Exit::Interrupted);

        let result =
            std::thread::spawn(|| THREAD_GRAPH.with(|slot| *slot.borrow_mut() = Some(graph())))
                .join();
        assert!(result.is_ok());
    } else {
        let mut child = std::process::Command::new(std::env::current_exe()?)
            .args(["--exact", "deep_graphs_in_subprocess", "--nocapture"])
            .env("EFFECT_DEPTH_CHILD", "1")
            .spawn()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                child.kill()?;
                let _ = child.wait()?;
                return Err("depth child exceeded its 30-second deadline".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(status.success(), "depth child failed: {status}");
    }
    Ok(())
}
