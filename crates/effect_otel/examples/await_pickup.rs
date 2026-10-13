//! Wait for ticket 104 to become ready at either of two real outlet status files.
//!
//! Run: `cargo +1.99.0 run -p effect-otel --features server --example await_pickup -- NORTH.status SOUTH.status 2000 20`.
//! Each caller-owned UTF-8 file contains exactly `104,pending` or `104,ready`.
//! The last arguments are positive wait-window and polling milliseconds.
//! Output first shows both outlet snapshots, then names the first ready outlet.
//! Staff can change either file to ready while the customer waits. A deadline
//! returns a typed wait-window error. Only transient native reads are retried;
//! malformed/missing files fail immediately. No failure or readiness is invented.
//! The existing native environment supplies Tokio's clock, tasks and blocking I/O.
#[cfg(feature = "server")]
/// Existing native provider composition shared with the timeline host.
pub mod cafe_telemetry;
#[cfg(feature = "server")]
mod pickup {
    use super::cafe_telemetry::{Environment, TelemetryCapacity};
    use effect::{
        Deterministic, Effect, Exit, Schedule, effect, effect_all_concurrent, effect_race,
        effect_repeat, effect_retry, effect_timeout, effect_try_blocking, schedule,
    };
    use effect_otel::BorrowedProviders;
    use std::{num::NonZeroUsize, path::PathBuf, sync::Arc, time::Duration};

    /// Show both current snapshots, then await the first genuinely ready outlet.
    fn find_pickup(
        north: Arc<PathBuf>,
        south: Arc<PathBuf>,
        window: Duration,
        poll: Duration,
    ) -> Effect<StatusRow, PickupError, Environment> {
        effect! {
            let north = north.clone(); let south = south.clone();
            let snapshots = effect_all_concurrent! {
                concurrency: NonZeroUsize::MIN.saturating_add(1),
                items: vec![read_status(north.clone(), "north"), read_status(south.clone(), "south")],
            }?;
            for row in snapshots { println!("{}: ticket {}, ready={}", row.outlet, row.ticket, row.ready); }
            effect_timeout! {
                duration: window, on_timeout { PickupError::WaitWindow },
                program: effect_race! {
                    program: wait_for_ready(north, "north", poll), right: wait_for_ready(south, "south", poll),
                },
            }?
        }
    }
    /// Poll until an actual file reports ready; retries cover transient reads only.
    fn wait_for_ready(
        path: Arc<PathBuf>,
        outlet: &'static str,
        poll: Duration,
    ) -> Effect<StatusRow, PickupError, Environment> {
        let retry: Schedule<PickupError, usize, Deterministic> = schedule! {
            state: 0usize,
            step |attempt: &usize, error: &PickupError, _elapsed, _sample| {
                let transient = matches!(error, PickupError::Io(io) if matches!(io.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock));
                Ok((*attempt, (transient && *attempt < 2).then_some((poll, attempt + 1))))
            },
        };
        let until_ready: Schedule<StatusRow, (), Deterministic> = schedule! {
            state: (), step |_state: &(), row: &StatusRow, _elapsed, _sample| {
                Ok(((), (!row.ready).then_some((poll, ()))))
            },
        };
        effect_repeat! {
            schedule: until_ready,
            program: effect_retry! { schedule: retry, program: read_status(path, outlet) },
        }
    }
    /// Each selected blocking job reads the caller's current record, including later staff edits.
    fn read_status(
        path: Arc<PathBuf>,
        outlet: &'static str,
    ) -> Effect<StatusRow, PickupError, Environment> {
        effect_try_blocking! {
            body |_cancel| {
                let text = std::fs::read_to_string(path.as_ref()).map_err(PickupError::Io)?;
                let (ticket, state) = text.trim().split_once(',').ok_or(PickupError::Record)?;
                if ticket != "104" { return Err(PickupError::Record); }
                let ready = match state { "pending" => false, "ready" => true, _ => return Err(PickupError::Record) };
                Ok(StatusRow { outlet, ticket: ticket.to_owned(), ready })
            },
        }
    }
    /// Readiness belongs to a supplied ticket record, not an attempt counter.
    #[derive(Debug)]
    struct StatusRow {
        /// Selected outlet whose file supplied this result.
        outlet: &'static str,
        /// Actual ticket identifier decoded from the file.
        ticket: String,
        /// Actual pending/ready state decoded from the file.
        ready: bool,
    }
    /// File problems and elapsed business wait remain distinct.
    #[derive(Debug)]
    enum PickupError {
        /// Original native file-read error.
        Io(std::io::Error),
        /// Input is not the declared ticket/status record.
        Record,
        /// Neither outlet reported ready within the caller's wait window.
        WaitWindow,
    }
    /// Borrow native providers, await cancellation/settlement, then join their original owners.
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mut args = std::env::args().skip(1);
        let north = Arc::new(PathBuf::from(
            args.next().ok_or("provide north status file")?,
        ));
        let south = Arc::new(PathBuf::from(
            args.next().ok_or("provide south status file")?,
        ));
        let window = positive_duration(args.next())?;
        let poll = positive_duration(args.next())?;
        if args.next().is_some() {
            return Err("unexpected argument".into());
        }
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?,
        );
        let traces = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let metrics = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
        let logs = opentelemetry_sdk::logs::SdkLoggerProvider::builder().build();
        let maximum = NonZeroUsize::MIN.saturating_add(7);
        let env = Arc::new(Environment::new(
            runtime.clone(),
            BorrowedProviders::new(&traces, &metrics, &logs),
            TelemetryCapacity {
                active_spans: maximum,
                metric_descriptors: maximum,
                series_per_descriptor: maximum,
            },
        )?);
        let result = runtime.block_on(find_pickup(north, south, window, poll).run(env.clone()));
        runtime.block_on(env.tasks.stop().wait());
        runtime.block_on(env.blocking.stop().wait());
        logs.shutdown()?;
        metrics.shutdown()?;
        traces.shutdown()?;
        match result {
            Exit::Success(row) => {
                use std::io::Write;
                writeln!(
                    std::io::stdout().lock(),
                    "Ticket {} is ready at {}.",
                    row.ticket,
                    row.outlet
                )?;
                Ok(())
            }
            other @ (Exit::Fail(_)
            | Exit::Die(_)
            | Exit::Interrupted
            | Exit::FinalizationFailed(_)) => Err(format!("pickup wait ended: {other:?}").into()),
        }
    }
    /// Reject zero instead of creating an unbounded spin or an ambiguous wait budget.
    fn positive_duration(value: Option<String>) -> Result<Duration, Box<dyn std::error::Error>> {
        let millis: std::num::NonZeroU64 = value.ok_or("provide positive milliseconds")?.parse()?;
        Ok(Duration::from_millis(millis.get()))
    }
}
/// Execute the caller-selected native pickup wait.
#[cfg(feature = "server")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    pickup::run()
}
/// Without native server provider support this example remains inert.
#[cfg(not(feature = "server"))]
fn main() {}
