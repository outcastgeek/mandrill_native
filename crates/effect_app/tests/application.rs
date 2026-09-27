use effect::{
    Defect, DiagnosticCapacity, DiagnosticOverflow, Diagnostics, Exit, LibraryDefect, LogLevel,
    Logger,
};
use effect_app::{AppConfig, AppError, Files, LiveApp, application, describe};
use std::{
    num::NonZeroUsize,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "effect-app-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir(&path).unwrap_or_else(fixture_failure);
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[allow(clippy::panic)]
fn fixture_failure<T>(error: impl std::fmt::Debug) -> T {
    panic!("fixture failed: {error:?}")
}
fn nz(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap_or_else(|| fixture_failure(value))
}
fn app(directory: &Directory) -> LiveApp {
    LiveApp::new(AppConfig {
        files: Files {
            input: directory.0.join("input.json"),
            staging: directory.0.join("staging"),
            output: directory.0.join("output"),
        },
        async_workers: nz(2),
        cpu_workers: nz(2),
        blocking_workers: nz(2),
        task_admission: nz(8),
        cpu_admission: nz(4),
        blocking_admission: nz(4),
        telemetry_capacity: nz(8),
        diagnostic_overflow: DiagnosticOverflow::DiscardOldest,
        metadata_capacity: nz(2),
        metadata_ttl: Duration::from_secs(1),
        retries: 2,
        retry_delay: Duration::ZERO,
        log_level: LogLevel::Error,
    })
    .unwrap_or_else(fixture_failure)
}
fn run(app: &LiveApp) -> Exit<effect_app::Report, effect::SharedValue<AppError>> {
    app.runtime.block_on(async {
        let outcome = application().run_cancellable(Arc::clone(&app.env)).await;
        app.drain().await;
        outcome
    })
}
#[test]
fn actual_file_application_commits_result_and_telemetry_failure_preserves_success() {
    let directory = Directory::new();
    let mut app = app(&directory);
    std::fs::write(&app.env.files.input, r#"{"samples":[1,2,3]}"#).unwrap_or_else(fixture_failure);
    let diagnostics = Diagnostics::new(
        DiagnosticCapacity::Unbounded,
        DiagnosticOverflow::DiscardOldest,
    );
    Arc::get_mut(&mut app.env)
        .unwrap_or_else(|| fixture_failure("unique environment"))
        .logger = Logger::new(
        LogLevel::Debug,
        diagnostics.clone(),
        |annotations| Ok(annotations.clone()),
        |_| Err(Defect::from(LibraryDefect::ProviderClosed)),
    );
    Arc::get_mut(&mut app.env)
        .unwrap_or_else(|| fixture_failure("unique environment"))
        .tracer = effect::Tracer::new(
        diagnostics.clone(),
        |annotations| Ok(annotations.clone()),
        |event| Ok(Some(effect::SpanId::new(Arc::clone(&event.name)))),
        |_| Err(Defect::from(LibraryDefect::ProviderClosed)),
    );
    let outcome = run(&app);
    assert!(matches!(outcome,Exit::Success(report) if report.sum==6));
    assert_eq!(
        std::fs::read_to_string(&app.env.files.output).unwrap_or_else(fixture_failure),
        "6\n"
    );
    assert!(!app.env.files.staging.exists());
    // Exercise the logger failure boundary directly because the application does
    // not need to manufacture log events in its file-processing path.
    let logged: effect::Effect<(), effect::SharedValue<AppError>, effect_app::AppEnv> =
        effect::Effect::log(LogLevel::Error, "fixture event");
    assert!(matches!(
        app.runtime.block_on(logged.run(Arc::clone(&app.env))),
        Exit::Success(())
    ));
    assert_eq!(diagnostics.drain().len(), 2);
}
#[test]
fn malformed_domain_input_exhausted_io_and_precancellation_start_no_commit() {
    for text in [Some("{"), Some(r#"{"samples":[]}"#), None] {
        let directory = Directory::new();
        let app = app(&directory);
        if let Some(text) = text {
            std::fs::write(&app.env.files.input, text).unwrap_or_else(fixture_failure);
        }
        let outcome = run(&app);
        assert!(matches!(outcome, Exit::Fail(_)));
        assert!(!app.env.files.output.exists());
        assert!(!app.env.files.staging.exists());
    }
    let directory = Directory::new();
    let app = app(&directory);
    assert!(app.env.cancellation.request());
    assert!(matches!(run(&app), Exit::Interrupted));
    assert!(!app.env.files.output.exists());
}
#[test]
fn provider_child_failure_is_reported_and_every_cleanup_detail_is_rendered() {
    let directory = Directory::new();
    let app = app(&directory);
    std::fs::write(&app.env.files.input, r#"{"samples":[1]}"#).unwrap_or_else(fixture_failure);
    app.runtime.block_on(app.env.cpu.stop().wait());
    let outcome = run(&app);
    assert!(
        matches!(outcome,Exit::Die(defect) if defect.kind()==effect::DefectKind::Library(LibraryDefect::ProviderClosed))
    );
    assert!(!app.env.files.staging.exists());
    let outcome = Exit::FinalizationFailed(effect::Finalization {
        primary: effect::PrimaryOutcome::Failed(effect::SharedValue::new(
            AppError::ArithmeticOverflow,
        )),
        first: effect::CleanupFailure::ReleaseDefect(Defect::from(LibraryDefect::ProviderClosed)),
        rest: vec![
            effect::CleanupFailure::ReleaseFailure(effect::SharedValue::new(
                AppError::EmptySamples,
            )),
            effect::CleanupFailure::ReleaseInterrupted,
        ],
    });
    let rendered = describe(outcome);
    assert!(
        rendered.contains("ArithmeticOverflow")
            && rendered.contains("EmptySamples")
            && rendered.contains("Interrupted")
    );
}
