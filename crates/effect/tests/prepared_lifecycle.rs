//! Real downstream prepared root, resource and cancellation contracts.

use effect::memory::{MemoryError, PreparedMemory, RegionId};
use effect::{
    CleanupFailure, DynamicRunContract, Exit, PreparedCancellationWait, PreparedCleanupConfig,
    PreparedCleanupPool, PreparedRunContext, PrimaryOutcome, PrimaryOutcomeRef, effect_prepare,
    memory,
};
use futures_executor::block_on;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Waker};

const WORK: RegionId = RegionId::named("work");

fn memory() -> Result<PreparedMemory<'static>, MemoryError> {
    memory! { total: 256 KiB, mode: prepared, metadata: work, alignment: 16, partition work: 256 KiB, }?.prepare()
}

#[derive(Clone, Copy)]
enum Behavior {
    Ready,
    Wait,
    Panic,
    Refuse,
}

struct NativeEnv {
    behavior: Behavior,
    entries: Arc<AtomicUsize>,
}

#[test]
fn native_root_holds_output_and_cancellation_owners_until_retirement() -> Result<(), MemoryError> {
    let plan = effect_prepare! {
        plan;
        body: |env: Arc<NativeEnv>, _: PreparedRunContext<usize, !>, wait: PreparedCancellationWait<usize, !>| async move {
            env.entries.fetch_add(1, Ordering::Relaxed);
            match env.behavior {
                Behavior::Ready => Exit::Success(19),
                Behavior::Wait => { let _ = wait.await; Exit::Success(23) }
                Behavior::Panic => panic!("prepared attempt panicked"),
                Behavior::Refuse => unreachable!("admission refusal must skip the native body"),
            }
        },
        reserve: |env: &Arc<NativeEnv>, context: &PreparedRunContext<usize, !>| {
            if matches!(env.behavior, Behavior::Refuse) { return Err(MemoryError::NoSlot); }
            context.cancelled()
        },
        contract: DynamicRunContract::ConcreteFuture { clock_ticks: 0, contexts: 1, listeners: 1, bridges: 0 },
    };
    let demand = plan.demand(1)?;
    let prepared = memory()?;
    let frames = prepared.frames(WORK, demand.frames_per_run, demand.frame_layout)?;
    let run = effect_prepare! { prepare; plan: plan, memory: prepared, region: WORK, capacity: 1, frames: frames, }?;
    let _memory = prepared.seal();
    let entries = Arc::new(AtomicUsize::new(0));
    let env = |behavior| {
        Arc::new(NativeEnv {
            behavior,
            entries: entries.clone(),
        })
    };

    assert!(matches!(
        effect_prepare! { admit; run: &run, environment: env(Behavior::Refuse), },
        Err(MemoryError::NoSlot)
    ));
    assert_eq!(run.active(), 0);
    assert_eq!(entries.load(Ordering::Relaxed), 0);
    let abandoned = effect_prepare! { admit; run: &run, environment: env(Behavior::Ready), }?;
    drop(abandoned);
    assert_eq!(run.active(), 0);
    assert_eq!(entries.load(Ordering::Relaxed), 0);

    let invocation = effect_prepare! { admit; run: &run, environment: env(Behavior::Ready), }?;
    let output = block_on(invocation.run());
    let reader = output.try_clone()?;
    assert!(
        output
            .with(|exit| matches!(exit, Exit::Success(19)))
            .is_ok_and(|matched| matched)
    );
    drop(output);
    assert!(matches!(
        effect_prepare! { admit; run: &run, environment: env(Behavior::Ready), },
        Err(MemoryError::NoSlot)
    ));
    assert!(
        reader
            .with(|exit| matches!(exit, Exit::Success(19)))
            .is_ok_and(|matched| matched)
    );
    drop(reader);
    assert_eq!(run.active(), 0);

    let invocation = effect_prepare! { admit; run: &run, environment: env(Behavior::Wait), }?;
    let cancellation = invocation.context()?;
    assert!(cancellation.request_cancellation());
    let before_poll = block_on(invocation.run());
    assert!(
        before_poll
            .with(|exit| matches!(exit, Exit::Interrupted))
            .is_ok_and(|matched| matched)
    );
    assert_eq!(entries.load(Ordering::Relaxed), 1);
    drop(before_poll);
    drop(cancellation);

    let invocation = effect_prepare! { admit; run: &run, environment: env(Behavior::Wait), }?;
    let cancellation = invocation.context()?;
    let mut driver = invocation.into_driver();
    // The first turn can prepare a continuation without polling the native
    // future. Drive the bounded setup through to the actual wait.
    let mut context = Context::from_waker(Waker::noop());
    for _ in 0..8 {
        assert!(Pin::new(&mut driver).poll(&mut context).is_pending());
        if entries.load(Ordering::Relaxed) == 2 {
            break;
        }
    }
    assert_eq!(entries.load(Ordering::Relaxed), 2);
    assert!(cancellation.request_cancellation());
    let pending_output = block_on(driver);
    assert!(
        pending_output
            .with(|exit| matches!(exit, Exit::Interrupted))
            .is_ok_and(|matched| matched)
    );
    drop(pending_output);
    assert_eq!(run.active(), 1);
    drop(cancellation);

    let invocation = effect_prepare! { admit; run: &run, environment: env(Behavior::Panic), }?;
    let panic = block_on(invocation.run());
    assert!(panic.with(|exit| matches!(exit, Exit::Die(defect) if defect.message().contains("prepared attempt panicked"))).is_ok_and(|matched| matched));
    run.close();
    drop(panic);
    assert_eq!(run.active(), 0);
    assert!(matches!(
        effect_prepare! { admit; run: &run, environment: env(Behavior::Ready), },
        Err(MemoryError::Closed)
    ));
    Ok(())
}

struct OwnedResource(u8);

#[test]
fn explicit_prepared_using_release_preserves_primary_and_cleanup_order() -> Result<(), MemoryError>
{
    let released = Arc::new(AtomicUsize::new(0));
    let report_release = released.clone();
    let plan = effect_prepare! {
        graph;
        program: effect::effect! {
            let report_release = Arc::clone(&report_release);
            let outcome = effect_using! {
                acquire: effect_sync! { body: || OwnedResource(7), },
            release: move |resource: OwnedResource, primary: PrimaryOutcomeRef<'_, &'static str>| {
                assert!(matches!(primary, PrimaryOutcomeRef::Failed(&"primary")));
                effect_resource! {
                    release_try_sync;
                    body: move || {
                        report_release.fetch_add(usize::from(resource.0), Ordering::Relaxed);
                        Err("release")
                    },
                }
            },
                body: move |_resource| effect_try_sync! { body: || Err::<u8, _>("primary"), },
            }?;
            outcome
        },
        contract: DynamicRunContract::GeneratedGraph { clock_ticks: 0, contexts: 2, listeners: 0, bridges: 0 },
    };
    let demand = plan.demand(1)?;
    let prepared = memory()?;
    let frames = prepared.frames(WORK, demand.frames_per_run, demand.frame_layout)?;
    let cleanup = PreparedCleanupPool::prepare(&prepared, WORK, demand.cleanup_sites, 1)?;
    let cleanup_frames = prepared.frames(
        WORK,
        demand.cleanup_sites,
        PreparedCleanupPool::<&'static str>::frame_layout(),
    )?;
    let plan = plan.with_cleanup(PreparedCleanupConfig::new(cleanup, cleanup_frames));
    let run = effect_prepare! { prepare; plan: plan, memory: prepared, region: WORK, capacity: 1, frames: frames, }?;
    let _memory = prepared.seal();
    assert_eq!(released.load(Ordering::Relaxed), 0);
    let invocation = effect_prepare! { admit; run: &run, environment: Arc::new(()), }?;
    assert_eq!(released.load(Ordering::Relaxed), 0);
    let outcome = block_on(invocation.run());
    assert!(
        outcome
            .with(|exit| matches!(
                exit,
                Exit::FinalizationFailed(details)
                    if details.primary == PrimaryOutcome::Failed("primary")
                        && details.first == CleanupFailure::ReleaseFailure("release")
                        && details.rest.is_empty()
            ))
            .is_ok_and(|matched| matched)
    );
    assert_eq!(released.load(Ordering::Relaxed), 7);
    drop(outcome);
    Ok(())
}

#[test]
fn contextual_exit_mode_refuses_before_factory_without_selected_frames()
-> Result<(), Box<dyn std::error::Error>> {
    use effect::memory::RunFrames;
    use effect::{Effect, NativeContext};
    let entries = Arc::new(AtomicUsize::new(0));
    let entered = entries.clone();
    let expressed = AtomicUsize::new(0);
    let program: Effect<u8, MemoryError> = effect::effect_future! {
        exit_with_context;
        on_capacity: {
            assert_eq!(expressed.fetch_add(1, Ordering::Relaxed), 0);
            std::convert::identity
        },
        body: {
            assert_eq!(expressed.fetch_add(1, Ordering::Relaxed), 1);
            move |_: Arc<()>, context: NativeContext| {
                entered.fetch_add(1, Ordering::Relaxed);
                async move {
                    assert!(!context.cancellation().is_requested());
                    Exit::Success(9)
                }
            }
        },
    };
    assert_eq!(expressed.load(Ordering::Relaxed), 2);
    assert_eq!(entries.load(Ordering::Relaxed), 0);
    assert_eq!(
        block_on(program.run(Arc::new(()))),
        Exit::Fail(MemoryError::MissingRuntimeStorage)
    );
    assert_eq!(entries.load(Ordering::Relaxed), 0);
    let prepared = memory()?;
    let layout = std::alloc::Layout::from_size_align(4096, 16)?;
    let frames = RunFrames::new(
        prepared.frames(WORK, 2, layout)?,
        prepared.frames(WORK, 1, layout)?,
    )?;
    let _sealed = prepared.seal();
    assert_eq!(
        block_on(program.run_with_frames(Arc::new(()), frames)),
        Exit::Success(9)
    );
    assert_eq!(entries.load(Ordering::Relaxed), 1);
    Ok(())
}
