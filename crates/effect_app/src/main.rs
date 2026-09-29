//! Native application entry point.

use clap::{Parser, ValueEnum};
use effect::{Console, ConsoleChannel, Exit, LogLevel, StandardConsole};
use effect_app::{AppConfig, Files, LiveApp, application, describe};
use futures_util::future::{Either, select};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::ExitCode;
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
}
/// Parse configuration, run the application, request cancellation on Ctrl-C,
/// drain accepted work, then print the terminal outcome and diagnostics.
///
/// # Returns
/// Type: `Result<ExitCode, effect::Defect>`.
fn main() -> Result<ExitCode, effect::Defect> {
    let args = Arguments::parse();
    if args.input == args.staging || args.output == args.staging || args.input == args.output {
        return Err(effect::Defect::from_io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Input, output, and staging paths must be distinct",
        )));
    }
    let app = LiveApp::new(AppConfig {
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
    })?;
    let outcome = app.runtime.block_on(async {
        let run = application().run_cancellable(std::sync::Arc::clone(&app.env));
        let outcome = match select(Box::pin(run), Box::pin(tokio::signal::ctrl_c())).await {
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
    });
    let succeeded = matches!(outcome, Exit::Success(_));
    StandardConsole.write(ConsoleChannel::Output, &describe(outcome))?;
    for diagnostic in app.diagnostics.drain() {
        StandardConsole.write(ConsoleChannel::Error, &format!("{diagnostic:?}"))?;
    }
    Ok(if succeeded {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
