use crate::schedule;
extern crate effect as fx;
use effect::*;
use futures_executor::block_on;
use futures_util::future::join;
use std::cell::Cell;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll};
use std::time::Duration;
#[path = "support/drive.rs"]
mod drive;
#[path = "support/task_cases.rs"]
pub mod tasks;

struct PendingItems(Arc<AtomicUsize>);

impl futures_util::stream::Stream for PendingItems {
    type Item = Result<usize, ()>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

#[test]
fn async_iterable_pull_settles_and_disposes_when_cancelled() {
    let environment = Arc::new(Env::default());
    let entered = Arc::new(AtomicUsize::new(0));
    let disposed = Arc::new(AtomicUsize::new(0));
    let stream: Stream<usize, (), Env> = stream! { async_iterable; async_iterable: {
        let entered = Arc::clone(&entered);
        move || PendingItems(Arc::clone(&entered))
    }, on_error: |error| error,  release: {
        let disposed = Arc::clone(&disposed);
        move |items| {
            let disposed = Arc::clone(&disposed);
            effect_resource! { release_sync; body: move || {
                drop(items);
                disposed.fetch_add(1, Ordering::SeqCst);
            }, }
        }
    }, };
    let mut running = Box::pin(
        stream
            .run_collect(None)
            .run_cancellable(Arc::clone(&environment)),
    );
    drive::until(running.as_mut(), || entered.load(Ordering::SeqCst) > 0);
    assert!(environment.cancel.request());
    assert!(matches!(block_on(running), Exit::Interrupted));
    assert_eq!(disposed.load(Ordering::SeqCst), 1);
}

#[test]
fn subscription_snapshot_and_publication_share_one_boundary() -> Result<(), String> {
    const RACE_ROUNDS: usize = 1_000;
    let environment = Arc::new(());
    let capacity = NonZeroUsize::MIN.saturating_add(1);

    for _ in 0..RACE_ROUNDS {
        let reference = SubscriptionRef::new(0_usize, capacity, PubSubOverflow::Backpressure);
        let start = Arc::new(Barrier::new(2));
        let (snapshot, published) = std::thread::scope(|threads| -> Result<_, String> {
            let subscribe = threads.spawn(|| {
                start.wait();
                block_on(reference.subscribe().run(Arc::clone(&environment)))
            });
            let publish = threads.spawn(|| {
                start.wait();
                block_on(reference.set(1).run(Arc::clone(&environment)))
            });
            let snapshot = subscribe.join();
            let published = publish.join();
            Ok((
                snapshot.map_err(|error| format!("subscriber thread: {error:?}"))?,
                published.map_err(|error| format!("publisher thread: {error:?}"))?,
            ))
        })?;
        assert!(matches!(published, Exit::Success(_)));
        let Exit::Success(snapshot) = snapshot else {
            return Err(String::from("subscription must succeed"));
        };

        match *snapshot.value {
            0 => {
                let next = block_on(snapshot.subscription.take().run(Arc::clone(&environment)));
                assert!(
                    matches!(next, Exit::Success(SubscriptionEvent::Item(value)) if *value == 1)
                );
            }
            1 => {}
            other => return Err(format!("unexpected snapshot value {other}")),
        }

        assert!(matches!(
            block_on(reference.set(2).run(Arc::clone(&environment))),
            Exit::Success(_)
        ));
        let next = block_on(snapshot.subscription.take().run(Arc::clone(&environment)));
        assert!(matches!(next, Exit::Success(SubscriptionEvent::Item(value)) if *value == 2));
    }
    Ok(())
}

#[derive(Default)]
struct Env {
    clock: TestClock,
    cancel: Cancellation,
    tasks: tasks::Threads,
}
impl HasClock for Env {
    type Clock = TestClock;
    fn clock(&self) -> &TestClock {
        &self.clock
    }
}
impl HasCancellation for Env {
    fn cancellation(&self) -> &Cancellation {
        &self.cancel
    }
}
impl HasTaskSpawner for Env {
    type Tasks = tasks::Threads;
    fn task_spawner(&self) -> &tasks::Threads {
        &self.tasks
    }
}
impl HasCpuExecutor for Env {
    type Executor = tasks::Threads;
    fn cpu_executor(&self) -> &tasks::Threads {
        &self.tasks
    }
}

#[test]
fn virtual_advance_follows_new_deadlines_through_macro_loop_and_protection() {
    let env = Arc::new(Env::default());
    let program: Effect<Vec<Duration>, (), Env> = effect! {
        let mut times = Vec::new();
        for _ in 0..3 {
            effect_uninterruptible! { program: Effect::sleep(Duration::from_secs(2)), }?;
            times.push(Effect::clock_now()?);
        }
        times
    };
    let mut run = Box::pin(program.run(Arc::clone(&env)));
    drive::until(run.as_mut(), || env.clock.pending() == 1);
    let (outcome, advanced) = block_on(join(run, env.clock.advance(Duration::from_secs(6))));
    assert!(advanced.is_ok());
    assert_eq!(
        outcome,
        Exit::Success(vec![
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(6)
        ])
    );
    assert_eq!(env.clock.pending(), 0);
}
#[test]
fn virtual_advance_is_fifo_at_invocation_and_sleep_cancel_drop_overflow_are_explicit() {
    let clock = TestClock::default();
    let first = clock.advance(Duration::from_secs(2));
    let second = clock.advance(Duration::from_secs(3));
    let (second, first) = block_on(join(second, first));
    assert!(first.is_ok() && second.is_ok());
    assert_eq!(clock.now(), Duration::from_secs(5));
    let cancelled = Cancellation::new();
    let mut sleeper = clock.sleep(Duration::from_secs(1), cancelled.clone());
    drive::until(sleeper.as_mut(), || clock.pending() == 1);
    assert!(cancelled.request());
    assert!(matches!(block_on(sleeper), Err(ClockError::Interrupted)));
    assert_eq!(clock.pending(), 0);
    let mut sleeper = clock.sleep(Duration::from_secs(1), Cancellation::new());
    drive::until(sleeper.as_mut(), || clock.pending() == 1);
    drop(sleeper);
    assert_eq!(clock.pending(), 0);
    assert!(matches!(
        block_on(TestClock::new(Duration::MAX).advance(Duration::from_nanos(1))),
        Err(ClockError::Overflow)
    ));
}
#[test]
fn retry_repeat_and_jitter_follow_injected_time_and_select_only_their_channel() -> Result<(), String>
{
    let env = Arc::new(Env::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&calls);
    let attempt: Effect<usize, usize, Env> = effect_try_sync! { body: move || {
        let index = count.fetch_add(1, Ordering::SeqCst);
        if index < 2 { Err(index) } else { Ok(index) }
    }, on_error: |error| error, };
    let policy = Schedule::recurs(2).intersect(Schedule::spaced(Duration::from_secs(2)));
    let mut run =
        Box::pin(effect_retry! { program: attempt, schedule: policy, }.run(Arc::clone(&env)));
    drive::until(run.as_mut(), || env.clock.pending() == 1);
    let (outcome, advanced) = block_on(join(run, env.clock.advance(Duration::from_secs(4))));
    assert!(advanced.is_ok());
    assert_eq!(outcome, Exit::Success(2));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let count = Arc::clone(&calls);
    let repeated: Effect<usize, usize, Env> =
        effect_sync! { body: move || count.fetch_add(1, Ordering::SeqCst), };
    assert_eq!(
        block_on(
            effect_repeat! { program: repeated, schedule: Schedule::recurs(2), }
                .run(Arc::clone(&env))
        ),
        Exit::Success(5)
    );
    let defect = Defect::from(LibraryDefect::ProviderClosed);
    let failed: Effect<(), usize, Env> = effect_die! { defect: defect.clone(), };
    assert_eq!(
        block_on(effect_retry! { program: failed, schedule: Schedule::recurs(10), }.run(env)),
        Exit::Die(defect)
    );
    let factor =
        NonnegativeFactor::new(1.0).map_err(|error| format!("invalid fixture: {error:?}"))?;
    let policy = Schedule::<(), usize>::spaced(Duration::from_secs(4))
        .jitter(factor, factor)
        .map_err(|error| format!("invalid fixture: {error:?}"))?;
    assert!(
        matches!(policy.step(&(), Duration::ZERO, Some(UnitInterval::new(0.5).map_err(|error| format!("invalid fixture: {error:?}"))?)), ScheduleDecision::Continue { delay, .. } if delay == Duration::from_secs(4))
    );
    assert!(GrowthFactor::new(0.5).is_err());
    assert!(UnitInterval::new(f64::NAN).is_err());
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum DecodeError {
    Syntax,
    Domain,
}
#[test]
fn schemas_are_lazy_reusable_and_invalid_input_never_starts_downstream_work() {
    let calls = Arc::new(AtomicUsize::new(0));
    let decoded = Arc::clone(&calls);
    let schema = Schema::from_decoder(move |value| {
        decoded.fetch_add(1, Ordering::SeqCst);
        value.as_i64().map(Cell::new).ok_or(DecodeError::Domain)
    });
    let good: Effect<Cell<i64>, DecodeError> = schema.parse("42", |_| DecodeError::Syntax);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for _ in 0..2 {
        assert_eq!(
            block_on(good.run(Arc::new(()))),
            Exit::Success(Cell::new(42))
        );
    }
    let downstream = Arc::new(AtomicUsize::new(0));
    for (text, expected) in [("{", DecodeError::Syntax), ("[]", DecodeError::Domain)] {
        let downstream = Arc::clone(&downstream);
        let bad = effect_map! { program: schema.parse(text, |_| DecodeError::Syntax), map: move |_| {
            downstream.fetch_add(1, Ordering::SeqCst);
        }, };
        assert_eq!(block_on(bad.run(Arc::new(()))), Exit::Fail(expected));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(downstream.load(Ordering::SeqCst), 0);
}
#[test]
fn layers_keep_scoped_dependencies_alive_and_streams_move_nonclone_data_and_await_disposal() {
    let releases = Arc::new(AtomicUsize::new(0));
    let released = Arc::clone(&releases);
    let layer: Layer<(), (), ResourceValue<String>> = layer! { scoped; acquire: effect_sync! { body: || String::from("owned"), }, release: move |resource, _| {
        let released = Arc::clone(&released);
        effect_resource! { release_future; body: move |_| async move {
            drop(resource);
            released.fetch_add(1, Ordering::SeqCst);
        }, }
    }, };
    let read = service! { env: |value: &ResourceValue<String>| value.with(String::len), };
    assert_eq!(
        block_on(effect_with_layer! { layer: layer, program: read, }.run(Arc::new(()))),
        Exit::Success(Ok(5))
    );
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    let stream: Stream<Cell<usize>, (), ()> = stream! { async_iterable; async_iterable: || futures_util::stream::iter(vec![Ok::<_, ()>(Cell::new(1)), Ok(Cell::new(2))]), on_error: |error| error,  release: move |iterator| {
        let released = Arc::clone(&releases);
        effect_resource! { release_future; body: move |_| async move {
            drop(iterator);
            released.fetch_add(1, Ordering::SeqCst);
        }, }
    }, };
    let output = block_on(stream.run_collect(Some(2)).run(Arc::new(())));
    assert!(
        matches!(output, Exit::Success(values) if values.iter().map(Cell::get).collect::<Vec<_>>() == vec![1,2])
    );
    assert!(
        matches!(block_on(stream.run_collect(Some(1)).run(Arc::new(()))), Exit::Die(defect) if defect.kind() == DefectKind::Library(LibraryDefect::StreamCollectionLimitExceeded))
    );
}

struct Observed {
    starts: Mutex<Vec<(String, Option<SpanId>, Annotations)>>,
    finishes: Mutex<Vec<(SpanOutcome, bool)>>,
    logs: Mutex<Vec<TraceContext>>,
}
struct TraceEnv {
    base: Env,
    logger: Logger,
    tracer: Tracer,
}
impl HasClock for TraceEnv {
    type Clock = TestClock;
    fn clock(&self) -> &TestClock {
        &self.base.clock
    }
}
impl HasTaskSpawner for TraceEnv {
    type Tasks = tasks::Threads;
    fn task_spawner(&self) -> &tasks::Threads {
        &self.base.tasks
    }
}
impl HasCpuExecutor for TraceEnv {
    type Executor = tasks::Threads;
    fn cpu_executor(&self) -> &tasks::Threads {
        &self.base.tasks
    }
}
impl HasLogger for TraceEnv {
    fn logger(&self) -> &Logger {
        &self.logger
    }
}
impl HasTracer for TraceEnv {
    fn tracer(&self) -> &Tracer {
        &self.tracer
    }
}
#[test]
fn tracing_context_crosses_children_and_cpu_jobs_restores_and_isolates_backend_failures() {
    let observed = Arc::new(Observed {
        starts: Mutex::new(Vec::new()),
        finishes: Mutex::new(Vec::new()),
        logs: Mutex::new(Vec::new()),
    });
    let diagnostics = Diagnostics::new(
        DiagnosticCapacity::Bounded(NonZeroUsize::MIN),
        DiagnosticOverflow::DiscardOldest,
    );
    let starts = Arc::clone(&observed);
    let finishes = Arc::clone(&observed);
    let logs = Arc::clone(&observed);
    let redact = |annotations: &Annotations| {
        Ok(annotations
            .iter()
            .filter(|(key, _)| key.as_ref() != "secret")
            .map(|(key, value)| (Arc::clone(key), Arc::clone(value)))
            .collect())
    };
    let tracer = Tracer::new(
        diagnostics.clone(),
        redact,
        move |event| {
            let mut starts = starts
                .starts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let id = SpanId::new(format!("{}", starts.len()));
            starts.push((
                event.name.to_string(),
                event.parent.clone(),
                event.annotations.as_ref().clone(),
            ));
            Ok(Some(id))
        },
        move |event| {
            finishes
                .finishes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((event.outcome, event.cleanup_failed));
            Err(Defect::from(LibraryDefect::ProviderClosed))
        },
    );
    let logger = Logger::new(LogLevel::Info, diagnostics.clone(), redact, move |event| {
        logs.logs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event.context.clone());
        Ok(())
    });
    let env = Arc::new(TraceEnv {
        base: Env::default(),
        logger,
        tracer,
    });
    let program: Effect<Option<SpanId>, (), TraceEnv> = span! { annotations: Annotations::from([(Arc::from("secret"),Arc::from("value")), (Arc::from("public"),Arc::from("visible"))]), effect: span! { name: "parent", effect: effect_scope! {
    body: |scope| effect! {
            let fiber = scope.fork(span! { name: "child", effect: effect_cpu! { with_context; body: |_, context| context.span, }, })?;
            let child = fiber.join()?;
            Effect::log(LogLevel::Info, "message")?;
            let current = Effect::trace_context()?;
            assert_ne!(child, current.span);
            current.span
        },
    }, }, };
    assert_eq!(
        block_on(program.run(Arc::clone(&env))),
        Exit::Success(Some(SpanId::new("0")))
    );
    let restored: Effect<TraceContext, (), TraceEnv> = Effect::trace_context();
    let restored = block_on(restored.run(Arc::clone(&env)));
    assert!(
        matches!(restored, Exit::Success(context) if context.span.is_none() && context.annotations.is_empty())
    );
    let starts = observed
        .starts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(starts.len(), 2);
    assert_eq!(
        starts.get(1).map(|(_, parent, _)| parent.clone()),
        Some(Some(SpanId::new("0")))
    );
    assert!(
        starts
            .iter()
            .all(|(_, _, annotations)| !annotations.contains_key("secret"))
    );
    assert_eq!(
        observed
            .finishes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_slice(),
        &[
            (SpanOutcome::Succeeded, false),
            (SpanOutcome::Succeeded, false)
        ]
    );
    assert_eq!(diagnostics.drain().len(), 1);
    assert_eq!(diagnostics.discarded(), 1);
    env.base.tasks.finish();
}

#[test]
fn schedule_exhaustion_delay_cancellation_and_timeout_cleanup_keep_full_outcomes() {
    let env = Arc::new(Env::default());
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&attempts);
    let failing: Effect<(), usize, Env> = effect_try_sync! { body: { let __dsl_error = move || count.fetch_add(1, Ordering::SeqCst); move || Err(__dsl_error()) }, on_error: ::core::convert::identity, };
    assert_eq!(
        block_on(
            effect_retry! { program: failing, schedule: Schedule::recurs(2), }
                .run(Arc::clone(&env))
        ),
        Exit::Fail(2)
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    let finalized: Effect<(), usize, Env> = effect_resource! {
    ensuring; program: effect_try_sync! { body: { let __dsl_error = || 5; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }, release: { let __dsl_cleanup = effect_try_sync! { body: { let __dsl_error = || 7; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; move |_| __dsl_cleanup.clone() },
    };
    assert_eq!(
        block_on(
            effect_retry! { program: finalized, schedule: Schedule::recurs(2), }
                .run(Arc::clone(&env))
        ),
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(5),
            first: CleanupFailure::ReleaseFailure(7),
            rest: Vec::new()
        })
    );
    let retry: Effect<(), usize, Env> = effect_retry! { program: effect_try_sync! { body: { let __dsl_error = || 5; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }, schedule: Schedule::recurs(2).intersect(Schedule::spaced(Duration::from_secs(1))), };
    let mut run = Box::pin(retry.run_cancellable(Arc::clone(&env)));
    drive::until(run.as_mut(), || env.clock.pending() == 1);
    assert!(env.cancel.request());
    assert_eq!(block_on(run), Exit::Interrupted);
    assert_eq!(env.clock.pending(), 0);
    let env = Arc::new(Env::default());
    let pending: Effect<usize, usize, Env> = effect_resource! {
    ensuring; program: effect_future! { body: |cancel| async move {
            cancel.cancelled().await;
            1usize
        }, }, release: { let __dsl_cleanup = effect_try_sync! { body: { let __dsl_error = || 7; move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; move |_| __dsl_cleanup.clone() },
    };
    let mut run = Box::pin(
        effect_timeout! { program: pending, duration: Duration::from_secs(1), on_timeout: || 99, }
            .run(Arc::clone(&env)),
    );
    drive::until(run.as_mut(), || env.clock.pending() == 1);
    let (outcome, advanced) = block_on(join(run, env.clock.advance(Duration::from_secs(1))));
    assert!(advanced.is_ok());
    assert_eq!(
        outcome,
        Exit::FinalizationFailed(Finalization {
            primary: PrimaryOutcome::Failed(99),
            first: CleanupFailure::ReleaseFailure(7),
            rest: Vec::new()
        })
    );
    env.tasks.finish();
}

#[test]
fn failed_schedule_decision_preserves_nonclone_primary_and_nested_cleanup_order() {
    struct Owned(String);
    let env = Arc::new(Env::default());
    let schedule = {
        let __dsl_program: Schedule<Owned, ()> = schedule! {
        state: (), step: |_, _, _, _| {
                Err(Defect::from(LibraryDefect::ProviderClosed))
            },
        };
        __dsl_program
    };
    let effect = effect_resource! {
    ensuring; program: effect_retry! { program: { let __dsl_program: Effect < () , Owned , Env > = effect_try_sync! { body: { let __dsl_error = || Owned(String::from("primary")); move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; __dsl_program }, schedule: schedule, }, release: { let __dsl_cleanup = effect_resource! {
    ensuring; program: effect_try_sync! { body: { let __dsl_error = || Owned(String::from("outer")); move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }, release: { let __dsl_cleanup = effect_try_sync! { body: { let __dsl_error = || Owned(String::from("inner")); move || Err(__dsl_error()) }, on_error: ::core::convert::identity, }; move |_| __dsl_cleanup.clone() },
    }; move |_| __dsl_cleanup.clone() },
    };
    let outcome = block_on(effect.run(env));
    assert!(
        matches!(outcome,Exit::FinalizationFailed(ref details) if matches!(&details.primary,PrimaryOutcome::Failed(Owned(message)) if message=="primary")&&matches!(&details.first,CleanupFailure::ReleaseDefect(defect) if defect.kind()==DefectKind::Library(LibraryDefect::ProviderClosed))&&matches!(details.rest.as_slice(),[CleanupFailure::ReleaseFailure(Owned(outer)),CleanupFailure::ReleaseFailure(Owned(inner))] if outer=="outer"&&inner=="inner"))
    );
}
