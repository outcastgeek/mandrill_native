use effect::workflow::{
    HasWorkflowEngine, MemoryWorkflowEngine, TypedWorkflowDefinition, WorkflowDefinitionId,
    WorkflowRunId, WorkflowRunResult, WorkflowVersion,
};
use effect::{
    Exit, JsonValue, KeyDomain, KeyNamespace, KeySegment, ParseLimits, PureCodec, SchemaIssue,
    SchemaIssueKind, StableKey, workflow,
};
use futures_executor::block_on;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Domain;
impl KeyDomain for Domain {
    const TAG: &'static str = "workflow-macro-definition-test";
}

struct Environment(Arc<MemoryWorkflowEngine>);
impl HasWorkflowEngine for Environment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> {
        Arc::clone(&self.0)
    }
}

struct Payload(String);

#[test]
fn typed_macro_replays_a_non_clone_payload_without_rerunning_the_body() {
    let text = PureCodec::new(
        |json| match json {
            JsonValue::String(value) => Ok(value.clone()),
            _ => Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
        },
        |value: &String| Ok(JsonValue::String(value.clone())),
    );
    let payload = PureCodec::new(
        |json| match json {
            JsonValue::String(value) => Ok(Payload(value.clone())),
            _ => Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
        },
        |value: &Payload| Ok(JsonValue::String(value.0.clone())),
    );
    let budget = NonZeroUsize::new(1024).unwrap();
    let limits = ParseLimits {
        bytes: budget,
        depth: budget,
        nodes: budget,
        collection: budget,
        issues: budget,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let definition: TypedWorkflowDefinition<
        Payload,
        String,
        String,
        std::convert::Infallible,
        Environment,
    > = workflow! {
        name: WorkflowDefinitionId::new("typed-workflow").unwrap(),
        version: WorkflowVersion::new(NonZeroU32::new(1).unwrap()),
        payload: payload,
        success: text.clone(),
        error: text,
        compatibility: Arc::from(&b"typed-workflow-v1"[..]),
        limits: limits,
        run |input| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(input.0)
        },
    };
    let namespace = KeyNamespace::<Domain>::new("runs").unwrap();
    let key = StableKey::new(&namespace, [KeySegment::Text(Arc::from("first"))]).unwrap();
    let run = WorkflowRunId::from_stable(key);
    let environment = Arc::new(Environment(Arc::new(MemoryWorkflowEngine::new())));
    for _ in 0..2 {
        let result = block_on(
            definition
                .run(run.clone(), Payload("paid".into()))
                .run(Arc::clone(&environment)),
        );
        assert!(
            matches!(result, Exit::Success(WorkflowRunResult::Completed(Ok(value)))
            if value == "paid")
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
