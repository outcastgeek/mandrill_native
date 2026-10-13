//! An executable example that composes asynchronous, CPU, and blocking work.
//!
//! [`AppConfig`] requires explicit worker counts, admission capacities, file
//! paths, retry policy, and telemetry bounds. [`LiveApp::new`] constructs the
//! providers and shared [`AppEnv`]; [`application`] describes the work without
//! executing it. The environment supplies Tokio tasks and blocking work, Rayon
//! CPU execution, and logging/tracing in one `R`. The final [`Exit`] is rendered
//! by [`describe`], preserving the distinction between expected errors,
//! defects, interruption, and cleanup failure.

use effect::{effect_map_error, span};
pub mod native;
use effect::{
    AdmissionCapacity, Cache, CacheCapacity, CacheConfig, Cancellation, Console,
    DiagnosticCapacity, DiagnosticOverflow, Diagnostics, Effect, Exit, HasBlockingExecutor,
    HasCancellation, HasClock, HasCpuExecutor, HasLogger, HasTaskSpawner, HasTracer, LogLevel,
    Logger, Schema, Scope, SharedValue, SpanId, StandardConsole, Tracer, effect, effect_retry,
    effect_scope, effect_scope_acquire, effect_try_blocking, effect_try_cpu, effect_try_future,
    effect_try_sync, service,
};
use effect_rayon::RayonCpu;
use effect_tokio::{TokioBlocking, TokioClock, TokioTasks};
use serde::Deserialize;
use std::fs::File;
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
/// Application failures retain I/O, JSON, arithmetic, and resource causes.
pub enum AppError {
    /// Filesystem or stream I/O failed. Carries the original error.
    Io(std::io::Error),
    /// The input contains invalid JSON syntax.
    JsonSyntax(serde_json::Error),
    /// The JSON document could not be decoded as application input.
    Document(serde_json::Error),
    /// The input contains no samples to process.
    EmptySamples,
    /// Sample arithmetic exceeded its supported range.
    ArithmeticOverflow,
    /// Resource acquisition, use, or release failed.
    Resource(effect::Defect),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
/// Input document containing the samples processed by this application.
pub struct Input {
    /// Samples read from the input document. Type: `Vec<i64>`.
    pub samples: Vec<i64>,
}
/// Paths for the input, final output, and staging output files.
pub struct Files {
    /// Path to the input document. Type: `PathBuf`.
    pub input: PathBuf,
    /// Path to the final output document. Type: `PathBuf`.
    pub output: PathBuf,
    /// Path used while writing output before it is finalized. Type: `PathBuf`.
    pub staging: PathBuf,
}
/// Explicit worker, admission, telemetry, cache, retry, and file policies.
pub struct AppConfig {
    /// Input, final output, and staging paths. Type: `Files`.
    pub files: Files,
    /// Number of asynchronous runtime workers. Type: `NonZeroUsize`.
    pub async_workers: NonZeroUsize,
    /// Number of Rayon CPU workers. Type: `NonZeroUsize`.
    pub cpu_workers: NonZeroUsize,
    /// Maximum number of Tokio blocking workers. Type: `NonZeroUsize`.
    pub blocking_workers: NonZeroUsize,
    /// Maximum admitted asynchronous tasks. Type: `NonZeroUsize`.
    pub task_admission: NonZeroUsize,
    /// Maximum admitted CPU jobs. Type: `NonZeroUsize`.
    pub cpu_admission: NonZeroUsize,
    /// Maximum admitted blocking jobs. Type: `NonZeroUsize`.
    pub blocking_admission: NonZeroUsize,
    /// The capacity allocated to telemetry. Type: `NonZeroUsize`.
    pub telemetry_capacity: NonZeroUsize,
    /// Behavior when diagnostic capacity is exhausted. Type: `DiagnosticOverflow`.
    pub diagnostic_overflow: DiagnosticOverflow,
    /// The capacity allocated to metadata. Type: `NonZeroUsize`.
    pub metadata_capacity: NonZeroUsize,
    /// Time to retain cached metadata. Type: `Duration`.
    pub metadata_ttl: Duration,
    /// Number of retries after an initial attempt. Type: `usize`.
    pub retries: usize,
    /// Delay between retry attempts. Type: `Duration`.
    pub retry_delay: Duration,
    /// Minimum application log level. Type: `LogLevel`.
    pub log_level: LogLevel,
}
/// Application environment selecting Tokio tasks, Rayon CPU work, and Tokio
/// blocking work together. These concrete providers use static trait dispatch;
/// a vtable is involved only when an application selects a trait object.
pub struct AppEnv {
    /// The token that requests cancellation. Type: `Cancellation`.
    pub cancellation: Cancellation,
    /// Tokio task spawning capability. Type: `TokioTasks`.
    pub tasks: TokioTasks,
    /// Rayon CPU execution capability. Type: `RayonCpu`.
    pub cpu: RayonCpu,
    /// Tokio blocking execution capability. Type: `TokioBlocking`.
    pub blocking: TokioBlocking,
    /// Clock and scheduling capability. Type: `TokioClock`.
    pub clock: TokioClock,
    /// Application logging capability. Type: `Logger`.
    pub logger: Logger,
    /// Application tracing capability. Type: `Tracer`.
    pub tracer: Tracer,
    /// Shared file paths for application effects. Type: `Arc<Files>`.
    pub files: Arc<Files>,
    /// Number of retries after an initial attempt. Type: `usize`.
    pub retries: usize,
    /// Delay between retry attempts. Type: `Duration`.
    pub retry_delay: Duration,
    /// The capacity allocated to metadata. Type: `NonZeroUsize`.
    pub metadata_capacity: NonZeroUsize,
    /// Time to retain cached metadata. Type: `Duration`.
    pub metadata_ttl: Duration,
}
impl HasCancellation for AppEnv {
    ///
    /// # Returns
    /// Type: `&Cancellation`.
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}
impl HasTaskSpawner for AppEnv {
    /// Type alias for tasks in lib.
    type Tasks = TokioTasks;
    ///
    /// # Returns
    /// Type: `&TokioTasks`.
    fn task_spawner(&self) -> &TokioTasks {
        &self.tasks
    }
}
impl HasCpuExecutor for AppEnv {
    /// Type alias for executor in lib.
    type Executor = RayonCpu;
    ///
    /// # Returns
    /// Type: `&RayonCpu`.
    fn cpu_executor(&self) -> &RayonCpu {
        &self.cpu
    }
}
impl HasBlockingExecutor for AppEnv {
    /// Type alias for executor in lib.
    type Executor = TokioBlocking;
    ///
    /// # Returns
    /// Type: `&TokioBlocking`.
    fn blocking_executor(&self) -> &TokioBlocking {
        &self.blocking
    }
}
impl HasClock for AppEnv {
    /// Type alias for clock in lib.
    type Clock = TokioClock;
    ///
    /// # Returns
    /// Type: `&TokioClock`.
    fn clock(&self) -> &TokioClock {
        &self.clock
    }
}
impl HasLogger for AppEnv {
    ///
    /// # Returns
    /// Type: `&Logger`.
    fn logger(&self) -> &Logger {
        &self.logger
    }
}
impl HasTracer for AppEnv {
    ///
    /// # Returns
    /// Type: `&Tracer`.
    fn tracer(&self) -> &Tracer {
        &self.tracer
    }
}
/// Owns the configured application environment, runtime, and diagnostics.
pub struct LiveApp {
    /// Shared application capability environment. Type: `Arc<AppEnv>`.
    pub env: Arc<AppEnv>,
    /// Runtime that executes asynchronous application effects. Type: `Arc<tokio::runtime::Runtime>`.
    pub runtime: Arc<tokio::runtime::Runtime>,
    /// Diagnostic events emitted during execution. Type: `Diagnostics`.
    pub diagnostics: Diagnostics,
}
impl LiveApp {
    /// Construct the owned application runtime and environment from explicit worker, admission and service policies; invalid runtime construction returns its defect.
    pub fn new(config: AppConfig) -> Result<Self, effect::Defect> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(config.async_workers.get())
                .max_blocking_threads(config.blocking_workers.get())
                .enable_all()
                .build()
                .map_err(effect::Defect::from_io)?,
        );
        let pool = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(config.cpu_workers.get())
                .build()
                .map_err(|error| effect::Defect::from_io(std::io::Error::other(error)))?,
        );
        let diagnostics = Diagnostics::new(
            DiagnosticCapacity::Bounded(config.telemetry_capacity),
            config.diagnostic_overflow,
        );
        let logger = Logger::new(
            config.log_level,
            diagnostics.clone(),
            |annotations| Ok(annotations.clone()),
            |event| StandardConsole.write(effect::ConsoleChannel::Error, &event.message),
        );
        let span_count = std::sync::atomic::AtomicUsize::new(usize::default());
        let tracer = Tracer::new(
            diagnostics.clone(),
            |annotations| Ok(annotations.clone()),
            move |_| {
                let ordinal = span_count
                    .try_update(
                        std::sync::atomic::Ordering::Relaxed,
                        std::sync::atomic::Ordering::Relaxed,
                        |count| count.checked_add(NonZeroUsize::MIN.get()),
                    )
                    .map_err(|_| effect::Defect::from(effect::LibraryDefect::TraceCountOverflow))?;
                Ok(Some(SpanId::new(ordinal.to_string())))
            },
            |_| Ok(()),
        );
        let env = Arc::new(AppEnv {
            cancellation: Cancellation::new(),
            tasks: TokioTasks::new(
                Arc::clone(&runtime),
                AdmissionCapacity::Bounded(config.task_admission),
            )?,
            cpu: RayonCpu::new(
                pool,
                AdmissionCapacity::Bounded(config.cpu_admission),
                diagnostics.clone(),
            ),
            blocking: TokioBlocking::new(
                Arc::clone(&runtime),
                AdmissionCapacity::Bounded(config.blocking_admission),
                diagnostics.clone(),
            ),
            clock: TokioClock::new(Arc::clone(&runtime)),
            logger,
            tracer,
            files: Arc::new(config.files),
            retries: config.retries,
            retry_delay: config.retry_delay,
            metadata_capacity: config.metadata_capacity,
            metadata_ttl: config.metadata_ttl,
        });
        Ok(Self {
            env,
            runtime,
            diagnostics,
        })
    }
    /// Wait for accepted work to settle and collect retained outcomes.
    ///
    /// # Returns
    /// Type: `()`.
    pub async fn drain(&self) {
        self.env.tasks.stop().wait().await;
        self.env.cpu.stop().wait().await;
        self.env.blocking.stop().wait().await;
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
/// Stable identity used by lib operations.
enum DocumentId {
    Input,
}
/// The computed sample sum and size of the input document after a successful run.
pub struct Report {
    /// Sum of all validated input samples. Type: `i128`.
    pub sum: i128,
    /// The encoded bytes for input. Type: `u64`.
    pub input_bytes: u64,
}
/// Type alias for error in lib.
type Error = SharedValue<AppError>;
///
/// # Returns
/// Type: `Schema<Input, AppError>`.
/// Returns `Schema<Input, AppError>`.
fn schema() -> Schema<Input, AppError> {
    Schema::from_decoder(|value| {
        let input = Input::deserialize(value).map_err(AppError::Document)?;
        if input.samples.is_empty() {
            return Err(AppError::EmptySamples);
        }
        Ok(input)
    })
}
///
/// # Returns
/// Type: `Effect<Vec<u8>, Error, AppEnv>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn fetch() -> Effect<Vec<u8>, Error, AppEnv> {
    effect_try_future! { with_env; body: |env: Arc<AppEnv>, _| async move { tokio::fs::read(&env.files.input).await }, on_error: |error| SharedValue::new(AppError::Io(error)), }
}
///
/// # Returns
/// Type: `Effect<u64, AppError, AppEnv>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
fn metadata() -> Effect<u64, AppError, AppEnv> {
    effect_try_future! { with_env; body: |env: Arc<AppEnv>, _| async move {
        tokio::fs::metadata(&env.files.input)
            .await
            .map(|metadata| metadata.len())
    }, on_error: AppError::Io, }
}
/// Reads a JSON document, validates it, computes on Rayon, shares a concurrent
/// metadata load, writes on the blocking provider, and atomically renames the
/// supplied staging file. Both file paths and every scheduling limit are input.
///
/// # Returns
/// Type: `Effect<Report, Error, AppEnv>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn application() -> Effect<Report, Error, AppEnv> {
    span! {
        name: stringify!(application),
        effect: effect_scope! {
        body |scope: Scope<Error>| {
            let scope = scope.clone();
            let files = service! { env |env: &AppEnv| { Arc::clone(&env.files) }, }?;
            let policy = service! { env |env: &AppEnv| { (env.retries, env.retry_delay, env.metadata_capacity, env.metadata_ttl) }, }?;
            let (retries, delay, capacity, ttl) = policy;
            let bytes = effect_retry! {
                program: fetch(), schedule: effect::Schedule::recurs(retries).intersect(effect::Schedule::spaced(delay)),
            }?;
            let json = effect_try_sync! {
                once; body { String::from_utf8(bytes) },
                on_error |error| { SharedValue::new(AppError::Resource(effect::Defect::from_utf8(error))) },
            }?;
            let input = effect_map_error! { program: schema().parse(Arc::<str>::from(json), AppError::JsonSyntax), map: SharedValue::new, }?;
            let cache = Cache::make(scope.clone(), CacheConfig { equal: Arc::new(|left: &DocumentId, right: &DocumentId| left == right), ttl: Some(ttl), capacity: CacheCapacity::Bounded(capacity), cache_failure: Arc::new(|_: &AppError| false) }, |_| metadata())?;
            let compute = scope.fork(effect_try_cpu! {
                once;
                body |_cancel| {
                    input.samples.into_iter().try_fold(i128::default(), |sum, sample| {
                        sum.checked_add(i128::from(sample)).ok_or(AppError::ArithmeticOverflow)
                    })
                },
                on_error: SharedValue::new,
            })?;
            let inspect = scope.fork(cache.get(DocumentId::Input))?;
            let sum = compute.join()?;
            let input_bytes = inspect.join()?;
            let input_bytes = effect_try_sync! {
                body { input_bytes.with(|bytes| *bytes) },
                on_error |defect| { SharedValue::new(AppError::Resource(defect)) },
            }?;
            let create_files = Arc::clone(&files);
            let acquire = effect_try_blocking! {
                body |_cancel| { File::options().write(true).create_new(true).open(&create_files.staging) },
                on_error |error| { SharedValue::new(AppError::Io(error)) },
            };
            let release_files = Arc::clone(&files);
            let staging = effect_scope_acquire! {
                scope: scope, acquire: acquire,
                release |file: File| {
                    let files = Arc::clone(&release_files);
                    effect_try_future! {
                        once;
                        body |_cancel| {
                            async move {
                                drop(file);
                                match tokio::fs::remove_file(&files.staging).await {
                                    Ok(()) => Ok(()),
                                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                                    Err(error) => Err(SharedValue::new(AppError::Io(error))),
                                }
                            }
                        },
                    }?;
                },
            }?;
            let stage = staging.clone();
            effect_try_blocking! {
                once;
                body |_cancel| {
                    stage.with(|file| {
                        let mut file = file;
                        writeln!(file, "{sum}").and_then(|()| file.sync_all())
                    }).map_err(AppError::Resource)?.map_err(AppError::Io)
                },
                on_error: SharedValue::new,
            }?;
            let commit_files = Arc::clone(&files);
            effect_try_future! {
                with_env;
                body |_env: Arc<AppEnv>, _cancel| {
                    let files = Arc::clone(&commit_files);
                    async move { tokio::fs::rename(&files.staging, &files.output).await }
                },
                on_error |error| { SharedValue::new(AppError::Io(error)) },
            }?;
            Report { sum, input_bytes }
        },
        },
    }
}
#[must_use]
/// Render every terminal channel as a human-readable command-line result.
/// This consumes the complete exit, including retained cleanup failures.
///
/// # Parameters
/// - `outcome` (`Exit<Report, Error>`): Complete terminal outcome, including expected errors and cleanup failures.
///
/// # Returns
/// Type: `String`.
pub fn describe(outcome: Exit<Report, Error>) -> String {
    match outcome {
        Exit::Success(report) => format!("sum={} input_bytes={}", report.sum, report.input_bytes),
        Exit::Fail(error) => error
            .with(|error| format!("{error:?}"))
            .unwrap_or_else(|defect| format!("{defect:?}")),
        Exit::Die(defect) => format!("{defect:?}"),
        Exit::Interrupted => String::from(stringify!(Interrupted)),
        Exit::FinalizationFailed(details) => {
            let primary = match details.primary {
                effect::PrimaryOutcome::Succeeded => String::from(stringify!(Succeeded)),
                effect::PrimaryOutcome::Failed(error) => describe_error(&error),
                effect::PrimaryOutcome::Defected(defect) => format!("{defect:?}"),
                effect::PrimaryOutcome::WasInterrupted => String::from(stringify!(Interrupted)),
            };
            let mut failures = vec![describe_cleanup(details.first)];
            failures.extend(details.rest.into_iter().map(describe_cleanup));
            format!("cleanup failed after {primary}: {failures:?}")
        }
    }
}
///
/// # Parameters
/// - `error` (`&Error`): Expected or provider error preserved in the returned outcome.
///
/// # Returns
/// Type: `String`.
/// Returns `String`.
fn describe_error(error: &Error) -> String {
    error
        .with(|error| format!("{error:?}"))
        .unwrap_or_else(|defect| format!("{defect:?}"))
}
///
/// # Parameters
/// - `failure` (`effect::CleanupFailure<Error>`): Expected error or cleanup failure retained in this outcome.
///
/// # Returns
/// Type: `String`.
/// Returns `String`.
fn describe_cleanup(failure: effect::CleanupFailure<Error>) -> String {
    match failure {
        effect::CleanupFailure::ReleaseFailure(error) => describe_error(&error),
        effect::CleanupFailure::ReleaseDefect(defect) => format!("{defect:?}"),
        effect::CleanupFailure::ReleaseInterrupted => String::from(stringify!(Interrupted)),
    }
}

/// Sequential application state across async CPU boundaries uses ordinary Rust
/// mutation. Each document and each partial sum moves into its next step.
///
/// # Parameters
/// - `factory` (`impl Fn() -> Vec<Input> + Send + Sync + 'static`): Factory invoked to create fresh data for an execution.
///
/// # Returns
/// Type: `Effect<i128, Error, AppEnv>`.
/// Returns a lazy `Effect` with the declared success, error, and environment types.
pub fn sum_documents(
    factory: impl Fn() -> Vec<Input> + Send + Sync + 'static,
) -> Effect<i128, Error, AppEnv> {
    let factory = Arc::new(factory);
    effect! {
        let documents = factory();
        let mut total = i128::default();
        for input in documents {
            let partial = effect_try_cpu! { once; body: move |_| input.samples.into_iter().try_fold(i128::default(), |sum, sample| sum.checked_add(i128::from(sample)).ok_or(AppError::ArithmeticOverflow)), on_error: SharedValue::new, }?;
            total = effect_try_sync! { once; body: move || total.checked_add(partial).ok_or(AppError::ArithmeticOverflow), on_error: SharedValue::new, }?;
        }
        total
    }
}
