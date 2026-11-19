//! Native application entry point.

use clap::{Parser, ValueEnum};
use effect::memory::Bytes;
use effect::{
    Console, ConsoleChannel, Effect, Exit, HasCancellation, LogLevel, SharedValue, StandardConsole,
};
use effect_app::{
    AppConfig, AppMemoryLimits, AppMemoryMode, Files, LiveApp, ManagedLiveApp, ManagedStartupError,
    OutputMemoryError, Report, application, describe, managed_application,
};
use futures_util::future::{Either, select};
use std::fmt;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::pin::pin;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
#[derive(Clone, Copy, ValueEnum)]
/// Command-line choices for the minimum emitted log level.
enum Verbosity {
    Debug,
    Info,
    Warn,
    Error,
}
#[derive(Clone, Copy, ValueEnum)]
/// Command-line choices for behavior when diagnostics reach capacity.
enum Overflow {
    DiscardOldest,
    DiscardNewest,
}

#[derive(Clone, Copy, ValueEnum)]
enum MemoryMode {
    System,
    BoundedAllocation,
    PreparedReuse,
}
#[derive(Parser)]
/// Required file paths, worker counts, admission bounds, and application
/// policies. No runtime or retry count is inferred from hidden defaults.
struct Arguments {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    staging: PathBuf,
    #[arg(long)]
    async_workers: NonZeroUsize,
    #[arg(long)]
    cpu_workers: NonZeroUsize,
    #[arg(long)]
    blocking_workers: NonZeroUsize,
    #[arg(long)]
    task_admission: NonZeroUsize,
    #[arg(long)]
    cpu_admission: NonZeroUsize,
    #[arg(long)]
    blocking_admission: NonZeroUsize,
    #[arg(long)]
    telemetry_capacity: NonZeroUsize,
    #[arg(long)]
    metadata_capacity: NonZeroUsize,
    #[arg(long)]
    diagnostic_overflow: Overflow,
    #[arg(long)]
    metadata_ttl_millis: u64,
    #[arg(long)]
    retries: usize,
    #[arg(long)]
    retry_delay_millis: u64,
    #[arg(long)]
    log_level: Verbosity,
    /// Application data storage policy. The normal system path is the default.
    #[arg(long, value_enum, default_value = "system")]
    memory_mode: MemoryMode,
    /// Total region backing, including metadata and padding, in bytes.
    #[arg(long)]
    memory_bytes: Option<NonZeroUsize>,
    /// Maximum accepted file size in bytes.
    #[arg(long)]
    input_bytes: Option<NonZeroUsize>,
    /// Byte capacity of each staged or console message.
    #[arg(long)]
    output_bytes: Option<NonZeroUsize>,
    /// Maximum decoded samples per application run.
    #[arg(long)]
    samples: Option<NonZeroUsize>,
    /// Number of data workspaces to provision.
    #[arg(long)]
    memory_runs: Option<NonZeroUsize>,
}

enum CliError {
    Configuration(&'static str),
    Provider(effect::Defect),
    ManagedStartup(ManagedStartupError),
    Output(OutputMemoryError),
}

impl fmt::Debug for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) => formatter.write_str(message),
            Self::Provider(error) => fmt::Debug::fmt(error, formatter),
            Self::ManagedStartup(error) => fmt::Debug::fmt(error, formatter),
            Self::Output(error) => fmt::Debug::fmt(error, formatter),
        }
    }
}

impl Arguments {
    fn managed_memory(&self) -> Result<Option<(AppMemoryMode, AppMemoryLimits)>, CliError> {
        let mode = match self.memory_mode {
            MemoryMode::BoundedAllocation => AppMemoryMode::BoundedAllocation,
            MemoryMode::PreparedReuse => AppMemoryMode::PreparedReuse,
            MemoryMode::System => {
                if self.memory_bytes.is_some()
                    || self.input_bytes.is_some()
                    || self.output_bytes.is_some()
                    || self.samples.is_some()
                    || self.memory_runs.is_some()
                {
                    return Err(CliError::Configuration(
                        "Managed limits require --memory-mode bounded-allocation or prepared-reuse",
                    ));
                }
                return Ok(None);
            }
        };
        let total = self.memory_bytes.ok_or(CliError::Configuration(
            "Managed memory requires --memory-bytes",
        ))?;
        Ok(Some((
            mode,
            AppMemoryLimits {
                total_bytes: Bytes::new(total.get()),
                input_bytes: self.input_bytes.ok_or(CliError::Configuration(
                    "Managed memory requires --input-bytes",
                ))?,
                output_bytes: self.output_bytes.ok_or(CliError::Configuration(
                    "Managed memory requires --output-bytes",
                ))?,
                samples: self
                    .samples
                    .ok_or(CliError::Configuration("Managed memory requires --samples"))?,
                runs: self.memory_runs.ok_or(CliError::Configuration(
                    "Managed memory requires --memory-runs",
                ))?,
            },
        )))
    }
}

/// Parse configuration, run the application, request cancellation on Ctrl-C,
/// drain accepted work, then print the terminal outcome and diagnostics.
fn main() -> Result<ExitCode, CliError> {
    let args = Arguments::parse();
    if args.input == args.staging || args.output == args.staging || args.input == args.output {
        return Err(CliError::Configuration(
            "Input, output, and staging paths must be distinct",
        ));
    }
    let memory = args.managed_memory()?;
    let config = AppConfig {
        files: Files {
            input: args.input,
            output: args.output,
            staging: args.staging,
        },
        async_workers: args.async_workers,
        cpu_workers: args.cpu_workers,
        blocking_workers: args.blocking_workers,
        task_admission: args.task_admission,
        cpu_admission: args.cpu_admission,
        blocking_admission: args.blocking_admission,
        telemetry_capacity: args.telemetry_capacity,
        diagnostic_overflow: match args.diagnostic_overflow {
            Overflow::DiscardOldest => effect::DiagnosticOverflow::DiscardOldest,
            Overflow::DiscardNewest => effect::DiagnosticOverflow::DiscardNewest,
        },
        metadata_capacity: args.metadata_capacity,
        metadata_ttl: Duration::from_millis(args.metadata_ttl_millis),
        retries: args.retries,
        retry_delay: Duration::from_millis(args.retry_delay_millis),
        log_level: match args.log_level {
            Verbosity::Debug => LogLevel::Debug,
            Verbosity::Info => LogLevel::Info,
            Verbosity::Warn => LogLevel::Warn,
            Verbosity::Error => LogLevel::Error,
        },
    };
    let succeeded = if let Some((mode, limits)) = memory {
        let app = ManagedLiveApp::new(config, mode, limits).map_err(CliError::ManagedStartup)?;
        let outcome = app.app.runtime.block_on(run_application(
            &app.app,
            Arc::clone(&app.env),
            managed_application(),
        ));
        let succeeded = matches!(outcome, Exit::Success(_));
        let text = app.format_outcome(&outcome).map_err(CliError::Output)?;
        write_console(ConsoleChannel::Output, text.as_str())?;
        drop(text);
        for diagnostic in app.app.diagnostics.drain() {
            let text = app
                .format_message(format_args!("{diagnostic:?}"))
                .map_err(CliError::Output)?;
            write_console(ConsoleChannel::Error, text.as_str())?;
        }
        succeeded
    } else {
        let app = LiveApp::new(config).map_err(CliError::Provider)?;
        let outcome =
            app.runtime
                .block_on(run_application(&app, Arc::clone(&app.env), application()));
        let succeeded = matches!(outcome, Exit::Success(_));
        write_console(ConsoleChannel::Output, &describe(outcome))?;
        for diagnostic in app.diagnostics.drain() {
            write_console(ConsoleChannel::Error, &format!("{diagnostic:?}"))?;
        }
        succeeded
    };
    Ok(if succeeded {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn write_console(channel: ConsoleChannel, text: &str) -> Result<(), CliError> {
    StandardConsole
        .write(channel, text)
        .map_err(CliError::Provider)
}

async fn run_application<E: Send + 'static, R: HasCancellation + Send + Sync + 'static>(
    app: &LiveApp,
    environment: Arc<R>,
    program: Effect<Report, SharedValue<E>, R>,
) -> Exit<Report, SharedValue<E>> {
    let run = pin!(program.run_cancellable(environment));
    let signal = pin!(tokio::signal::ctrl_c());
    let outcome = match select(run, signal).await {
        Either::Left((outcome, _)) => outcome,
        Either::Right((signal, remaining)) => {
            if let Err(error) = signal {
                app.diagnostics.record(effect::Defect::from_io(error));
            }
            let _ = app.env.cancellation.request();
            remaining.await
        }
    };
    app.drain().await;
    outcome
}
