extern crate effect as fx;
use effect::*;
use effect_tracing::TracingBackend;
use std::num::NonZeroUsize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
#[path = "../../effect/tests/support/task_cases.rs"]
pub mod tasks;
#[derive(Default)]
struct Records {
    ids: AtomicU64,
    spans: Mutex<Vec<(Option<u64>, String)>>,
    events: Mutex<Vec<(Option<u64>, String)>>,
    changes: Mutex<Vec<String>>,
}
#[derive(Clone)]
struct Subscriber(Arc<Records>);
struct Fields(String);
impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(&mut self.0, "{field}={value:?};");
    }
}
impl tracing::Subscriber for Subscriber {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut fields = Fields(String::new());
        attrs.record(&mut fields);
        self.0
            .spans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((attrs.parent().map(tracing::span::Id::into_u64), fields.0));
        tracing::span::Id::from_u64(self.0.ids.fetch_add(1, Ordering::SeqCst) + 1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut fields = Fields(String::new());
        values.record(&mut fields);
        self.0
            .changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(fields.0);
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((event.parent().map(tracing::span::Id::into_u64), fields.0));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}
struct Env {
    clock: TestClock,
    tasks: tasks::Threads,
    logger: Logger,
    tracer: Tracer,
}
impl HasClock for Env {
    type Clock = TestClock;
    fn clock(&self) -> &TestClock {
        &self.clock
    }
}
impl HasTaskSpawner for Env {
    type Tasks = tasks::Threads;
    fn task_spawner(&self) -> &tasks::Threads {
        &self.tasks
    }
}
impl HasLogger for Env {
    fn logger(&self) -> &Logger {
        &self.logger
    }
}
impl HasTracer for Env {
    fn tracer(&self) -> &Tracer {
        &self.tracer
    }
}
#[test]
fn configured_dispatch_survives_threads_with_explicit_parent_redaction_and_retirement() {
    let records = Arc::new(Records::default());
    let diagnostics = Diagnostics::new(
        DiagnosticCapacity::Unbounded,
        DiagnosticOverflow::DiscardOldest,
    );
    let backend = TracingBackend::new(
        tracing::Dispatch::new(Subscriber(Arc::clone(&records))),
        NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN),
        diagnostics.clone(),
    );
    let redact = |annotations: &Annotations| {
        Ok(annotations
            .iter()
            .filter(|(key, _)| key.as_ref() != "secret")
            .map(|(key, value)| (Arc::clone(key), Arc::clone(value)))
            .collect())
    };
    let env = Arc::new(Env {
        clock: TestClock::default(),
        tasks: tasks::Threads::default(),
        logger: backend.logger(LogLevel::Info, redact),
        tracer: backend.tracer(redact),
    });
    let program: Effect<(), (), Env> = Scope::scoped(|scope| {
        scope
            .fork(Effect::log(LogLevel::Info, "child event").with_span("child"))
            .flat_map(|fiber| fiber.join())
    })
    .with_span("root")
    .annotate(Annotations::from([(
        Arc::from("secret"),
        Arc::from("private"),
    )]));
    assert_eq!(
        futures_executor::block_on(program.run(Arc::clone(&env))),
        Exit::Success(())
    );
    env.tasks.finish();
    assert_eq!(backend.active(), 0);
    assert!(diagnostics.drain().is_empty());
    let spans = records
        .spans
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(spans.len(), 2);
    assert_eq!(spans.get(1).map(|(parent, _)| *parent), Some(Some(1)));
    assert!(spans.iter().all(|(_, fields)| !fields.contains("private")));
    let events = records
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(events.len(), 1);
    assert_eq!(events.first().map(|(parent, _)| *parent), Some(Some(2)));
    assert!(
        records
            .changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|fields| fields.contains("Succeeded"))
    );
}
