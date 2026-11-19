//! Validate the explicitly selected fixed CPU worker, saturation, and retained output.

#[path = "../../effect/tests/fixtures/shared/prepared_provider.rs"]
mod fixture;

use effect::memory::MemoryError;
use effect::prepared_provider::{ProviderRefusal, ProviderSettlement, SynchronousJob};
use effect_rayon::RayonCpu;
use fixture::{Diagnostics, NATIVE, REGION, ScalarWork};

#[test]
fn accepted_cpu_work_settles_after_stop_and_output_retains_slot() -> Result<(), String> {
    let memory = fixture::memory().map_err(|error| format!("{error:?}"))?;
    let diagnostics = Diagnostics::prepare(&memory).map_err(|error| format!("{error:?}"))?;
    let mut cpu = RayonCpu::prepare::<_, _, ScalarWork>(&memory, REGION, 1, NATIVE)
        .map_err(|error| format!("{error:?}"))?;
    let _memory = memory.seal();
    let submitter = cpu.submitter().map_err(|error| format!("{error:?}"))?;
    let reserved = submitter.reserve().map_err(|error| format!("{error:?}"))?;
    assert!(matches!(
        submitter.reserve(),
        Err(ProviderRefusal::Memory(MemoryError::NoSlot))
    ));

    cpu.stop();
    let caller = std::thread::current().id();
    let receipt = reserved
        .submit(
            || diagnostics.budget(ScalarWork),
            |_, cancellation| {
                SynchronousJob::new(
                    move |_| (caller != std::thread::current().id(), 23),
                    cancellation,
                )
            },
        )
        .map_err(|error| format!("{error:?}"))?;
    cpu.join();
    let retained = receipt
        .retain_result()
        .map_err(|error| format!("{error:?}"))?
        .ok_or_else(|| "CPU completion missing".to_owned())?;
    drop(receipt);
    assert_eq!(submitter.active(), 1);
    assert!(matches!(
        retained.result().settlement(),
        Some(ProviderSettlement::Returned((true, 23)))
    ));
    drop(retained);
    assert_eq!(submitter.active(), 0);
    assert!(matches!(submitter.reserve(), Err(ProviderRefusal::Closed)));
    assert_eq!(diagnostics.deliveries(), 0);
    assert!(cpu.worker_panics().payloads().is_empty());
    Ok(())
}
