use effect::{
    CleanupFailure, Defect, DefectKind, Effect, Exit, Finalization, LibraryDefect, PrimaryOutcome,
    Release, ResourceBinding,
};
use futures_executor::block_on;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

#[derive(Debug, PartialEq, Eq)]
enum Expected {
    Rejected,
}

#[derive(Debug, PartialEq, Eq)]
enum Translated {
    Rejected,
}

#[derive(Debug)]
struct PanicPayload;

// The compiler must evaluate these expressions, rather than merely being free
// to optimize a normal call. This is a typed identity, not numeric policy.
const LIBRARY_CONDITION: &Defect = &Defect::library(LibraryDefect::RunDriverDropped);
const LIBRARY_KIND: DefectKind = LIBRARY_CONDITION.kind();

#[test]
fn typed_library_conditions_are_constructible_and_inspectable_at_compile_time() {
    assert_eq!(
        LIBRARY_KIND,
        DefectKind::Library(LibraryDefect::RunDriverDropped)
    );
    assert_eq!(
        LIBRARY_CONDITION,
        &Defect::from(LibraryDefect::RunDriverDropped)
    );
}

#[test]
fn diagnostic_text_cannot_impersonate_a_library_failure_or_panic() {
    let library = Defect::from(LibraryDefect::ResourceScopeEnded);
    let custom = Defect::new(library.message());
    let panic = Defect::from_panic(Box::new(library.message().to_owned()));
    assert_eq!(
        library.kind(),
        DefectKind::Library(LibraryDefect::ResourceScopeEnded)
    );
    assert_eq!(custom.kind(), DefectKind::Custom);
    assert_eq!(panic.kind(), DefectKind::CallbackPanic);
    assert_ne!(library, custom);
    assert_ne!(library, panic);
    assert_ne!(custom, panic);
    assert_eq!(library.clone(), library);
    assert_eq!(panic.clone(), panic);
    assert_eq!(custom.clone(), custom);
}

#[test]
#[allow(clippy::panic)] // A non-string user panic must retain its typed panic classification.
fn a_non_string_panic_has_a_typed_identity_through_the_real_callback_boundary() {
    let program: Effect<(), Expected> = Effect::sync(|| std::panic::panic_any(PanicPayload));
    let actual = block_on(program.run(Arc::new(())));
    assert_eq!(
        actual,
        Exit::Die(Defect::from_panic(Box::new(PanicPayload)))
    );
    if let Exit::Die(defect) = actual {
        assert_eq!(defect.kind(), DefectKind::CallbackPanic);
        assert_ne!(defect, Defect::new(defect.message()));
    }
}

#[test]
#[allow(clippy::panic)] // Cleanup must retain both E and the panic identity.
fn typed_defect_identity_survives_finalization_and_error_mapping() {
    let body: Effect<(), Expected> = ResourceBinding::new(Effect::succeed(()), |(), _| {
        Release::sync(|| std::panic::panic_any(PanicPayload))
    })
    .use_with(|_| Effect::fail_with(|| Expected::Rejected));
    let program = body.map_error(|error| match error {
        Expected::Rejected => Translated::Rejected,
    });
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(Translated::Rejected),
            first: CleanupFailure::ReleaseDefect(Defect::from_panic(Box::new(PanicPayload))),
            rest: Vec::new(),
        })
    );
}

#[test]
fn a_consumed_observer_reports_a_typed_library_defect() {
    let (driver, mut observer) = Effect::<(), Expected>::succeed(()).owned_run(Arc::new(()));
    block_on(driver);
    assert_eq!(block_on(&mut observer), Exit::Success(()));
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(
        Pin::new(&mut observer).poll(&mut context),
        Poll::Ready(Exit::Die(Defect::from(LibraryDefect::RunOutcomeConsumed)))
    );
}
