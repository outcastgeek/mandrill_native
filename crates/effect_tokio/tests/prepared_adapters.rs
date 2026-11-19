//! Actual Tokio reactor, physical wait ownership, and fixed-worker task settlement.

#[path = "../../effect/tests/fixtures/shared/prepared_provider.rs"]
mod fixture;

use effect::memory::MemoryError;
use effect::prepared_provider::{ProviderRefusal, ProviderSettlement};
use effect_tokio::{PreparedTokioClockError, TokioBlocking, TokioClock, TokioTasks};
use fixture::{Diagnostics, NATIVE, REGION, ScalarWork};
use std::future::poll_fn;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

type TestResult = Result<(), String>;

fn runtime() -> Result<Arc<tokio::runtime::Runtime>, String> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .map(Arc::new)
        .map_err(|err| err.to_string())
}

#[test]
fn native_sleep_frame_refuses_and_reuses_after_pending_drop() -> TestResult {
    let runtime = runtime()?;
    let memory = fixture::memory().map_err(|error| format!("{error:?}"))?;
    let clock = TokioClock::prepare(runtime.clone(), &memory, REGION, NonZeroUsize::MIN)
        .map_err(|error| format!("{error:?}"))?;
    let _memory = memory.seal();

    let mut pending = clock
        .reserve_sleep(Duration::from_secs(30))
        .map_err(|error| format!("{error:?}"))?;
    let saturation = clock.reserve_sleep(Duration::ZERO);
    assert!(matches!(
        saturation,
        Err(PreparedTokioClockError::Memory(MemoryError::NoSlot))
    ));
    runtime.block_on(poll_fn(|context| {
        use std::future::Future;
        let observed = std::pin::Pin::new(&mut pending).poll(context);
        assert!(observed.is_pending());
        Poll::Ready(())
    }));
    drop(pending);
    let recovered = clock
        .reserve_sleep(Duration::ZERO)
        .map_err(|error| format!("{error:?}"))?;
    runtime
        .block_on(recovered)
        .map_err(|error| format!("{error:?}"))?;
    assert!(clock.reserve_sleep(Duration::ZERO).is_ok());
    Ok(())
}

#[test]
fn dedicated_tokio_task_and_blocking_workers_keep_accepted_work() -> TestResult {
    let runtime = runtime()?;
    let memory = fixture::memory().map_err(|error| format!("{error:?}"))?;
    let diagnostics = Diagnostics::prepare(&memory).map_err(|error| format!("{error:?}"))?;
    let mut tasks =
        TokioTasks::prepare::<_, _, ScalarWork>(runtime.clone(), &memory, REGION, 1, NATIVE)
            .map_err(|error| format!("{error:?}"))?;
    let mut blocking =
        TokioBlocking::prepare::<_, _, ScalarWork>(runtime, &memory, REGION, 1, NATIVE)
            .map_err(|error| format!("{error:?}"))?;
    let _memory = memory.seal();

    let task_submitter = tasks.submitter().map_err(|error| format!("{error:?}"))?;
    let task_slot = task_submitter
        .reserve()
        .map_err(|error| format!("{error:?}"))?;
    assert!(matches!(
        task_submitter.reserve(),
        Err(ProviderRefusal::Memory(MemoryError::NoSlot))
    ));
    tasks.stop();
    let task = task_slot
        .submit(
            || diagnostics.budget(ScalarWork),
            |_, _| async {
                assert!(tokio::runtime::Handle::try_current().is_ok());
                11
            },
        )
        .map_err(|error| format!("{error:?}"))?;
    tasks.join();
    let output = task
        .result()
        .ok_or_else(|| "task completion missing".to_owned())?;
    assert!(matches!(
        output.settlement(),
        Some(ProviderSettlement::Returned(11))
    ));
    drop(output);
    drop(task);
    assert!(matches!(
        task_submitter.reserve(),
        Err(ProviderRefusal::Closed)
    ));

    let blocking_submitter = blocking.submitter().map_err(|error| format!("{error:?}"))?;
    let blocking_slot = blocking_submitter
        .reserve()
        .map_err(|error| format!("{error:?}"))?;
    let operation = blocking_slot
        .submit(
            || diagnostics.budget(ScalarWork),
            |_, cancel| {
                effect::prepared_provider::SynchronousJob::new(
                    |_| {
                        assert!(tokio::runtime::Handle::try_current().is_ok());
                        17
                    },
                    cancel,
                )
            },
        )
        .map_err(|error| format!("{error:?}"))?;
    blocking.join();
    let output = operation
        .result()
        .ok_or_else(|| "blocking completion missing".to_owned())?;
    assert!(matches!(
        output.settlement(),
        Some(ProviderSettlement::Returned(17))
    ));
    assert!(tasks.worker_panics().payloads().is_empty());
    assert!(blocking.worker_panics().payloads().is_empty());
    assert_eq!(diagnostics.deliveries(), 0);
    Ok(())
}
