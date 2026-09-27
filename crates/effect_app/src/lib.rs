pub mod native;
use effect::{
    AdmissionCapacity, Cache, CacheCapacity, CacheConfig, Cancellation, Console,
    DiagnosticCapacity, DiagnosticOverflow, Diagnostics, Effect, Exit, HasBlockingExecutor,
    HasCancellation, HasClock, HasCpuExecutor, HasLogger, HasTaskSpawner, HasTracer, LogLevel,
    Logger, Release, Schema, Scope, SharedValue, SpanId, StandardConsole, Tracer, effect,
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
pub enum AppError {
    Io(std::io::Error),
    JsonSyntax(serde_json::Error),
    Document(serde_json::Error),
    EmptySamples,
    ArithmeticOverflow,
    Resource(effect::Defect),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub samples: Vec<i64>,
}
pub struct Files {
    pub input: PathBuf,
    pub output: PathBuf,
    pub staging: PathBuf,
}
pub struct AppConfig {
    pub files: Files,
    pub async_workers: NonZeroUsize,
    pub cpu_workers: NonZeroUsize,
    pub blocking_workers: NonZeroUsize,
    pub task_admission: NonZeroUsize,
    pub cpu_admission: NonZeroUsize,
    pub blocking_admission: NonZeroUsize,
    pub telemetry_capacity: NonZeroUsize,
    pub diagnostic_overflow: DiagnosticOverflow,
    pub metadata_capacity: NonZeroUsize,
    pub metadata_ttl: Duration,
    pub retries: usize,
    pub retry_delay: Duration,
    pub log_level: LogLevel,
}
pub struct AppEnv {
    pub cancellation: Cancellation,
    pub tasks: TokioTasks,
    pub cpu: RayonCpu,
    pub blocking: TokioBlocking,
    pub clock: TokioClock,
    pub logger: Logger,
    pub tracer: Tracer,
    pub files: Arc<Files>,
    pub retries: usize,
    pub retry_delay: Duration,
    pub metadata_capacity: NonZeroUsize,
    pub metadata_ttl: Duration,
}
impl HasCancellation for AppEnv {
    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}
impl HasTaskSpawner for AppEnv {
    type Tasks = TokioTasks;
    fn task_spawner(&self) -> &TokioTasks {
        &self.tasks
    }
}
impl HasCpuExecutor for AppEnv {
    type Executor = RayonCpu;
    fn cpu_executor(&self) -> &RayonCpu {
        &self.cpu
    }
}
impl HasBlockingExecutor for AppEnv {
    type Executor = TokioBlocking;
    fn blocking_executor(&self) -> &TokioBlocking {
        &self.blocking
    }
}
impl HasClock for AppEnv {
    type Clock = TokioClock;
    fn clock(&self) -> &TokioClock {
        &self.clock
    }
}
impl HasLogger for AppEnv {
    fn logger(&self) -> &Logger {
        &self.logger
    }
}
impl HasTracer for AppEnv {
    fn tracer(&self) -> &Tracer {
        &self.tracer
    }
}
pub struct LiveApp {
    pub env: Arc<AppEnv>,
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub diagnostics: Diagnostics,
}
impl LiveApp {
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
                    .fetch_update(
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
    pub async fn drain(&self) {
        self.env.tasks.stop().wait().await;
        self.env.cpu.stop().wait().await;
        self.env.blocking.stop().wait().await;
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum DocumentId {
    Input,
}
pub struct Report {
    pub sum: i128,
    pub input_bytes: u64,
}
type Error = SharedValue<AppError>;
fn schema() -> Schema<Input, AppError> {
    Schema::from_decoder(|value| {
        let input = Input::deserialize(value).map_err(AppError::Document)?;
        if input.samples.is_empty() {
            return Err(AppError::EmptySamples);
        }
        Ok(input)
    })
}
fn fetch() -> Effect<Vec<u8>, Error, AppEnv> {
    Effect::try_future_with_env(
        |env: Arc<AppEnv>, _| async move { tokio::fs::read(&env.files.input).await },
        |error| SharedValue::new(AppError::Io(error)),
    )
}
fn metadata() -> Effect<u64, AppError, AppEnv> {
    Effect::try_future_with_env(
        |env: Arc<AppEnv>, _| async move {
            tokio::fs::metadata(&env.files.input)
                .await
                .map(|metadata| metadata.len())
        },
        AppError::Io,
    )
}
/// Reads a JSON document, validates it, computes on Rayon, shares a concurrent
/// metadata load, writes on the blocking provider, and atomically renames the
/// supplied staging file. Both file paths and every scheduling limit are input.
pub fn application() -> Effect<Report, Error, AppEnv> {
    Scope::scoped(|scope| {
        effect! {
            let scope = scope.clone();
            let files = Effect::service(|env: &AppEnv| Arc::clone(&env.files))?;
            let policy = Effect::service(|env: &AppEnv| (env.retries, env.retry_delay, env.metadata_capacity, env.metadata_ttl))?;
            let (retries, delay, capacity, ttl) = policy;
            let bytes = fetch().retry(effect::Schedule::recurs(retries).intersect(effect::Schedule::spaced(delay)))?;
            let json = Effect::try_sync_once(move || String::from_utf8(bytes), |error| SharedValue::new(AppError::Resource(effect::Defect::from_utf8(error))))?;
            let input = schema().parse(Arc::<str>::from(json), AppError::JsonSyntax).map_error(SharedValue::new)?;
            let cache = Cache::make(scope.clone(), CacheConfig { equal: Arc::new(|left: &DocumentId, right: &DocumentId| left == right), ttl: Some(ttl), capacity: CacheCapacity::Bounded(capacity), cache_failure: Arc::new(|_: &AppError| false) }, |_| metadata())?;
            let compute = scope.fork(Effect::try_cpu_once(move |_| input.samples.into_iter().try_fold(i128::default(), |sum, sample| sum.checked_add(i128::from(sample)).ok_or(AppError::ArithmeticOverflow)), SharedValue::new))?;
            let inspect = scope.fork(cache.get(DocumentId::Input))?;
            let sum = compute.join()?;
            let input_bytes = inspect.join()?;
            let input_bytes = Effect::try_sync(move || input_bytes.with(|bytes| *bytes), |defect| SharedValue::new(AppError::Resource(defect)))?;
            let create_files = Arc::clone(&files);
            let acquire = Effect::try_blocking(move |_| File::options().write(true).create_new(true).open(&create_files.staging), |error| SharedValue::new(AppError::Io(error)));
            let release_files = Arc::clone(&files);
            let staging = scope.acquire(acquire, move |file| {
                let files = Arc::clone(&release_files);
                Release::try_future(move |_| async move {
                    drop(file);
                    match tokio::fs::remove_file(&files.staging).await { Ok(()) => Ok(()), Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()), Err(error) => Err(SharedValue::new(AppError::Io(error))) }
                })
            })?;
            let stage = staging.clone();
            Effect::try_blocking_once(move |_| {
                stage.with(|file| { let mut file = file; writeln!(file, "{sum}").and_then(|()| file.sync_all()) }).map_err(AppError::Resource)?.map_err(AppError::Io)
            }, SharedValue::new)?;
            let commit_files = Arc::clone(&files);
            Effect::try_future_with_env(move |_: Arc<AppEnv>, _| { let files = Arc::clone(&commit_files); async move { tokio::fs::rename(&files.staging, &files.output).await } }, |error| SharedValue::new(AppError::Io(error)))?;
            Report { sum, input_bytes }
        }
    }).with_span(stringify!(application))
}
#[must_use]
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
fn describe_error(error: &Error) -> String {
    error
        .with(|error| format!("{error:?}"))
        .unwrap_or_else(|defect| format!("{defect:?}"))
}
fn describe_cleanup(failure: effect::CleanupFailure<Error>) -> String {
    match failure {
        effect::CleanupFailure::ReleaseFailure(error) => describe_error(&error),
        effect::CleanupFailure::ReleaseDefect(defect) => format!("{defect:?}"),
        effect::CleanupFailure::ReleaseInterrupted => String::from(stringify!(Interrupted)),
    }
}

/// Sequential application state across async CPU boundaries uses ordinary Rust
/// mutation. Each document and each partial sum moves into its next step.
pub fn sum_documents(
    factory: impl Fn() -> Vec<Input> + Send + Sync + 'static,
) -> Effect<i128, Error, AppEnv> {
    let factory = Arc::new(factory);
    effect! {
        let documents = factory();
        let mut total = i128::default();
        for input in documents {
            let partial = Effect::try_cpu_once(move |_| input.samples.into_iter().try_fold(i128::default(), |sum, sample| sum.checked_add(i128::from(sample)).ok_or(AppError::ArithmeticOverflow)), SharedValue::new)?;
            total = Effect::try_sync_once(move || total.checked_add(partial).ok_or(AppError::ArithmeticOverflow), SharedValue::new)?;
        }
        total
    }
}
