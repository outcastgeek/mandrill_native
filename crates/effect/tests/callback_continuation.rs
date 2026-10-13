//! Continuation syntax preserves native provider-first owned input and reusable errors.

use effect::{Effect, Exit};
use effect::{effect, effect_future, effect_resource, effect_sync, effect_try_sync, effect_using};
use futures_executor::block_on;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// An owned payload deliberately supplies no Clone implementation.
struct Payload(String);

/// A non-Clone resource records disposal through its native Drop implementation.
struct DropResource(Arc<AtomicUsize>);
impl Drop for DropResource {
    /// Observe one release of this acquired resource.
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
/// An inert binding acquires and drops a fresh owned resource for each evaluation.
fn resource_binding_stays_deferred_and_releases_on_reuse() {
    let acquired = Arc::new(AtomicUsize::new(0));
    let released = Arc::new(AtomicUsize::new(0));
    let acquisition = Arc::clone(&acquired);
    let disposal = Arc::clone(&released);
    let binding = effect_using! {
        binding;
        acquire: effect_sync! {
            body: move || {
                acquisition.fetch_add(1, Ordering::SeqCst);
                DropResource(Arc::clone(&disposal))
            },
        },
    };
    let program: Effect<(), &'static str> = effect! {
        let binding = binding.clone();
        let resource = binding?;
        assert!(resource.with(|_| ()).is_ok());
    };
    assert_eq!(acquired.load(Ordering::SeqCst), 0);
    assert_eq!(released.load(Ordering::SeqCst), 0);
    for expected in 1..=2 {
        assert!(matches!(
            block_on(program.run(Arc::new(()))),
            Exit::Success(())
        ));
        assert_eq!(acquired.load(Ordering::SeqCst), expected);
        assert_eq!(released.load(Ordering::SeqCst), expected);
    }
}

#[test]
/// Provider failure leaves the slot available; later reuse retains the typed error.
fn provider_failure_does_not_consume_the_continuation_input() {
    let calls = Arc::new(AtomicUsize::new(0));
    let source_calls = Arc::clone(&calls);
    let source: Effect<(), &'static str> = effect_try_sync! {
        body: move || {
            if source_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Err("provider")
            } else {
                Ok(())
            }
        },
    };
    let input = Mutex::new(Some(Payload(String::from("owned"))));
    let program: Effect<Payload, &'static str> = effect! {
        bind;
        program: source,
        body: move |_| {
            let value = input.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            effect_try_sync! {
                once;
                body: move || value.ok_or("consumed"),
            }
        },
    };
    assert!(matches!(
        block_on(program.run(Arc::new(()))),
        Exit::Fail("provider")
    ));
    let result = block_on(program.run(Arc::new(())));
    assert!(matches!(result, Exit::Success(Payload(value)) if value == "owned"));
    assert!(matches!(
        block_on(program.run(Arc::new(()))),
        Exit::Fail("consumed")
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
/// A native full-exit future remains deferred and its cleanup retains both errors.
fn native_exit_and_primary_cleanup_preserve_failure_channels() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observe = Arc::clone(&calls);
    let native: Effect<(), &'static str> = effect_future! {
        exit_with_context;
        body: move |_, context| {
            assert!(!context.cancellation().is_requested());
            observe.fetch_add(1, Ordering::SeqCst);
            async move { Exit::Fail("primary") }
        },
    };
    let cleanup = Arc::clone(&calls);
    let program = effect_resource! {
        ensuring;
        program: native,
        release: move |primary| {
            assert!(matches!(primary, effect::PrimaryOutcomeRef::Failed(&"primary")));
            let cleanup = Arc::clone(&cleanup);
            effect_try_sync! {
                body: move || { cleanup.fetch_add(1, Ordering::SeqCst); Err("cleanup") },
            }
        },
    };
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let result = block_on(program.run(Arc::new(())));
    assert!(
        matches!(result, Exit::FinalizationFailed(effect::Finalization {
        primary: effect::PrimaryOutcome::Failed("primary"),
        first: effect::CleanupFailure::ReleaseFailure("cleanup"),
        rest,
    }) if rest.is_empty())
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
