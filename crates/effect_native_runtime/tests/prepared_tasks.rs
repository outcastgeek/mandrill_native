//! Exercise native parent/child authority together with the physical task owner.

#[path = "../../effect/tests/fixtures/shared/prepared_provider.rs"]
mod fixture;

use effect::prepared_provider::ProviderSettlement;
use effect_native_runtime::NativeTasks;
use fixture::{Diagnostics, NATIVE, REGION, ScalarWork};
use native_async_runtime::RuntimeOwner;
use std::future::ready;

#[test]
fn native_parent_drains_preheld_child_while_result_outlives_both() -> Result<(), String> {
    let runtime = RuntimeOwner::new(2, 1).map_err(|error| error.to_string())?;
    let service = runtime.controller(1).map_err(|error| error.to_string())?;
    let memory = fixture::memory().map_err(|error| format!("{error:?}"))?;
    let diagnostics = Diagnostics::prepare(&memory).map_err(|error| format!("{error:?}"))?;
    let mut tasks =
        NativeTasks::prepare::<_, _, _, ScalarWork>(service.handle(), &memory, REGION, 1, NATIVE)
            .map_err(|error| format!("{error:?}"))?;
    let _memory = memory.seal();
    let submitter = tasks.submitter().map_err(|error| format!("{error:?}"))?;
    let accepted = submitter.reserve().map_err(|error| format!("{error:?}"))?;
    assert_eq!(runtime.active(), 1);
    assert!(submitter.reserve().is_err());
    let drained = runtime.stop();
    let receipt = accepted
        .submit(|| diagnostics.budget(ScalarWork), |_| move |_| ready(31))
        .map_err(|error| format!("{error:?}"))?;
    tasks.join();
    runtime.block_on_sync(drained.wait());
    assert_eq!(runtime.active(), 0);
    assert_eq!(submitter.active(), 1);
    drop(runtime);
    let completion = receipt
        .result()
        .ok_or_else(|| "native completion missing".to_owned())?;
    assert!(matches!(
        completion.settlement(),
        Some(ProviderSettlement::Returned(31))
    ));
    drop(completion);
    drop(receipt);
    assert_eq!(submitter.active(), 0);
    assert!(tasks.worker_panics().payloads().is_empty());
    assert_eq!(diagnostics.deliveries(), 0);
    Ok(())
}
