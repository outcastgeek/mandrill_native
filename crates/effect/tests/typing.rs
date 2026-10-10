use std::path::PathBuf;
use std::process::Command;

/// Temporary external consumer checkout removed after each compiler-contract run.
struct FixtureDirectory(
    /// Temporary consumer sources owned exclusively by this harness invocation.
    PathBuf,
);
impl Drop for FixtureDirectory {
    /// Remove the harness-owned temporary checkout after all diagnostics are inspected.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Compile existing and composable macro consumers under a renamed dependency, checking targeted errors.
#[test]
fn external_compiler_contracts() -> Result<(), Box<dyn std::error::Error>> {
    let directory = FixtureDirectory(
        std::env::temp_dir().join(format!("mandrill-effect-typing-{}", std::process::id())),
    );
    std::fs::create_dir_all(directory.0.join("src"))?;
    let manifest = format!(
        "[package]\nname = \"effect-typing-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\nrust-version = \"1.99\"\n[workspace]\n[profile.dev]\ndebug = 0\nincremental = false\n[dependencies]\nmandrill_effect = {{ package = \"effect\", path = {:?} }}\n",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::write(directory.0.join("Cargo.toml"), manifest)?;
    let cases = [
        (
            "composition_helpers",
            include_str!("ui/composition_helpers.rs"),
            None,
        ),
        (
            "transaction_terminal_effect",
            include_str!("ui/transaction_terminal_effect.rs"),
            Some("Transaction"),
        ),
        (
            "transaction_nested_effect",
            include_str!("ui/transaction_nested_effect.rs"),
            Some("Transaction"),
        ),
        (
            "workflow_terminal_effect",
            include_str!("ui/workflow_terminal_effect.rs"),
            Some("Durable"),
        ),
        (
            "workflow_nested_future",
            include_str!("ui/workflow_nested_future.rs"),
            Some("Durable"),
        ),
        (
            "span_missing_capability",
            include_str!("ui/span_missing_capability.rs"),
            Some("HasTracer"),
        ),
        (
            "composition_conflicting_fields",
            include_str!("ui/composition_conflicting_fields.rs"),
            Some("exactly one"),
        ),
        (
            "composition_missing_policy",
            include_str!("ui/composition_missing_policy.rs"),
            Some("missing descriptor field: options"),
        ),
        (
            "stream_nonunit_tail",
            include_str!("ui/stream_nonunit_tail.rs"),
            Some("E0308"),
        ),
        (
            "native_incomplete",
            include_str!("ui/native_incomplete.rs"),
            Some("native schemas require"),
        ),
        (
            "native_representation",
            include_str!("ui/native_representation.rs"),
            Some("ObjectRepresentation"),
        ),
        (
            "native_environment",
            include_str!("ui/native_environment.rs"),
            Some("E0308"),
        ),
        ("new_schema", include_str!("ui/new_schema.rs"), None),
        (
            "transaction_rejects_effect",
            include_str!("ui/transaction_rejects_effect.rs"),
            Some("Transaction"),
        ),
        (
            "workflow_rejects_effect",
            include_str!("ui/workflow_rejects_effect.rs"),
            Some("Durable"),
        ),
        (
            "workflow_rejects_future",
            include_str!("ui/workflow_rejects_future.rs"),
            Some("Durable"),
        ),
        ("services", include_str!("ui/services.rs"), None),
        ("macro_loops", include_str!("ui/macro_loops.rs"), None),
        ("no_clock", include_str!("ui/no_clock.rs"), Some("HasClock")),
        (
            "no_random",
            include_str!("ui/no_random.rs"),
            Some("HasRandom"),
        ),
        (
            "shared_value_borrow",
            include_str!("ui/shared_value_borrow.rs"),
            Some("lifetime may not live long enough"),
        ),
        ("renamed", include_str!("ui/renamed.rs"), None),
        ("defect_kind", include_str!("ui/defect_kind.rs"), None),
        ("const_defect", include_str!("ui/const_defect.rs"), None),
        ("fiber_shared", include_str!("ui/fiber_shared.rs"), None),
        (
            "no_task_spawner",
            include_str!("ui/no_task_spawner.rs"),
            Some("HasTaskSpawner"),
        ),
        (
            "shared_exit_borrow",
            include_str!("ui/shared_exit_borrow.rs"),
            Some("lifetime may not live long enough"),
        ),
        ("cancellation", include_str!("ui/cancellation.rs"), None),
        ("resource_macro", include_str!("ui/resource_macro.rs"), None),
        ("branches", include_str!("ui/branches.rs"), None),
        ("parallel", include_str!("ui/parallel.rs"), None),
        ("jobs", include_str!("ui/jobs.rs"), None),
        (
            "no_cpu_executor",
            include_str!("ui/no_cpu_executor.rs"),
            Some("HasCpuExecutor"),
        ),
        (
            "no_blocking_executor",
            include_str!("ui/no_blocking_executor.rs"),
            Some("HasBlockingExecutor"),
        ),
        (
            "job_borrow",
            include_str!("ui/job_borrow.rs"),
            Some("E0521"),
        ),
        (
            "run_future_ownership",
            include_str!("ui/run_future_ownership.rs"),
            None,
        ),
        ("queue", include_str!("ui/queue.rs"), None),
        (
            "queue_capacity",
            include_str!("ui/queue_capacity.rs"),
            Some("NonZero"),
        ),
        (
            "queue_negative_capacity",
            include_str!("ui/queue_negative_capacity.rs"),
            Some("E0600"),
        ),
        (
            "queue_clone",
            include_str!("ui/queue_clone.rs"),
            Some("Clone"),
        ),
        (
            "synchronization",
            include_str!("ui/synchronization.rs"),
            None,
        ),
        (
            "reference_borrow",
            include_str!("ui/reference_borrow.rs"),
            Some("lifetime may not live long enough"),
        ),
        (
            "reference_get_non_clone",
            include_str!("ui/reference_get_non_clone.rs"),
            Some("Clone"),
        ),
        (
            "no_parallel_provider",
            include_str!("ui/no_parallel_provider.rs"),
            Some("HasTaskSpawner"),
        ),
        (
            "zero_parallel_limit",
            include_str!("ui/zero_parallel_limit.rs"),
            Some("NonZero"),
        ),
        (
            "parallel_borrow",
            include_str!("ui/parallel_borrow.rs"),
            Some("E0521"),
        ),
        ("branch_return", include_str!("ui/branch_return.rs"), None),
        (
            "branch_mutation",
            include_str!("ui/branch_mutation.rs"),
            None,
        ),
        (
            "resource_borrow",
            include_str!("ui/resource_borrow.rs"),
            Some("lifetime may not live long enough"),
        ),
        (
            "no_cancellation",
            include_str!("ui/no_cancellation.rs"),
            Some("HasCancellation"),
        ),
        (
            "projection",
            include_str!("ui/projection.rs"),
            Some("E0308"),
        ),
        ("let_else", include_str!("ui/let_else.rs"), None),
        ("errors", include_str!("ui/errors.rs"), Some("E0308")),
        (
            "environment",
            include_str!("ui/environment.rs"),
            Some("E0308"),
        ),
        ("success", include_str!("ui/success.rs"), Some("E0308")),
        ("private", include_str!("ui/private.rs"), Some("E0603")),
        (
            "constructor",
            include_str!("ui/constructor.rs"),
            Some("E0624"),
        ),
        ("nested", include_str!("ui/nested.rs"), None),
        ("result", include_str!("ui/result.rs"), Some("Bindable")),
        ("refutable", include_str!("ui/refutable.rs"), Some("E0005")),
    ];
    for (name, source, expected) in cases {
        if std::env::var("EFFECT_TYPING_CASE")
            .ok()
            .is_some_and(|selected| !selected.split(',').any(|case| case == name))
        {
            continue;
        }
        std::fs::write(directory.0.join("src/main.rs"), source)?;
        let output = Command::new(env!("CARGO"))
            .args(["check", "--quiet", "--offline", "--manifest-path"])
            .arg(directory.0.join("Cargo.toml"))
            .env(
                "CARGO_TARGET_DIR",
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"),
            )
            .output()?;
        let diagnostics = String::from_utf8(output.stderr)?;
        match expected {
            Some(reason) => assert!(
                !output.status.success() && diagnostics.contains(reason),
                "{name}: expected {reason}; got {diagnostics}"
            ),
            None => assert!(output.status.success(), "{name}: {diagnostics}"),
        }
    }
    Ok(())
}

/// Check existing typed feature contracts using paired accepted/rejected consumers.
#[test]
fn planned_feature_type_pairs() -> Result<(), Box<dyn std::error::Error>> {
    let directory = FixtureDirectory(std::env::temp_dir().join(format!(
        "mandrill-effect-feature-typing-{}",
        std::process::id()
    )));
    std::fs::create_dir_all(directory.0.join("src"))?;
    let manifest = format!(
        "[package]\nname = \"effect-feature-typing-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\nrust-version = \"1.99\"\n[workspace]\n[profile.dev]\ndebug = 0\nincremental = false\n[dependencies]\nmandrill_effect = {{ package = \"effect\", path = {:?} }}\n",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::write(directory.0.join("Cargo.toml"), manifest)?;

    let limits = r#"
fn limits() -> mandrill_effect::ParseLimits {
    let maximum = std::num::NonZeroUsize::new(256).unwrap();
    mandrill_effect::ParseLimits {
        bytes: maximum, depth: maximum, nodes: maximum,
        collection: maximum, issues: maximum,
    }
}
"#;
    let pairs = [
        (
            "T01_wrong_schema_field_codec",
            r#"
use mandrill_effect::{schema, json_i64, json_string, ParsePolicy,
    SchemaErrorMode, UnknownFields, DuplicateFields};
struct Record { count: i64 }
fn main() {
    let policy = ParsePolicy { limits: limits(), errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject };
    let _ = schema! { struct Record { count: __TOKEN__ } policy: policy }.unwrap();
}
"#,
            "json_i64()",
            "json_string()",
            "E0308",
        ),
        (
            "T02_decode_environment_direction",
            r#"
use mandrill_effect::{Codec, Effect, JsonValue, SchemaIssue};
struct Decode;
struct Encode;
fn main() {
    let codec: Codec<i64, JsonValue, Decode, Encode> = Codec::new(
        |value: JsonValue| Effect::<i64, SchemaIssue, Decode>::sync_once(
            move || value.as_i64().unwrap()),
        |value| Effect::<JsonValue, SchemaIssue, Encode>::sync_once(
            move || JsonValue::from(value)),
    );
    let _: Effect<i64, SchemaIssue, __TOKEN__> =
        codec.decode_once(JsonValue::from(1));
}
"#,
            "Decode",
            "Encode",
            "E0308",
        ),
        (
            "T03_wrong_rpc_request_codec",
            r#"
use mandrill_effect::json_i64;
use mandrill_effect::rpc::{RpcOperation, OperationId, RpcVersion};
fn main() {
    let _: RpcOperation<__TOKEN__, i64, i64> = RpcOperation::unary(
        OperationId::new("typed-request").unwrap(),
        RpcVersion::new(std::num::NonZeroU32::MIN),
        json_i64(), json_i64(), json_i64(), limits(),
    );
}
"#,
            "i64",
            "String",
            "E0308",
        ),
        (
            "T04_wrong_activity_success_codec",
            r#"
use mandrill_effect::{json_i64, json_string, KeyDomain, KeyNamespace,
    KeySegment, StableKey};
use mandrill_effect::workflow::{WorkflowActivity, WorkflowActivityId};
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-activity"; }
fn main() {
    let namespace = KeyNamespace::<Domain>::new("typing").unwrap();
    let identity = StableKey::new(&namespace, [KeySegment::Unsigned(1)]).unwrap();
    let _: WorkflowActivity<i64, i64> = WorkflowActivity {
        id: WorkflowActivityId::from_stable(identity),
        success: __TOKEN__, error: json_i64(), limits: limits(),
    };
}
"#,
            "json_i64()",
            "json_string()",
            "E0308",
        ),
        (
            "T05_missing_workflow_engine_capability",
            r#"
use mandrill_effect::{Effect, KeyDomain, KeyNamespace, KeySegment, StableKey, json_i64};
use mandrill_effect::workflow::{HasWorkflowEngine, MemoryWorkflowEngine,
    WorkflowActivity, WorkflowActivityId, WorkflowRunId};
use std::sync::Arc;
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-workflow-env"; }
struct Environment(Arc<MemoryWorkflowEngine>);
impl HasWorkflowEngine for Environment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> { Arc::clone(&self.0) }
}
fn main() {
    let namespace = KeyNamespace::<Domain>::new("typing").unwrap();
    let identity = StableKey::new(&namespace, [KeySegment::Unsigned(1)]).unwrap();
    let activity = WorkflowActivity {
        id: WorkflowActivityId::from_stable(identity.clone()),
        success: json_i64(), error: json_i64(), limits: limits(),
    };
    let _: Effect<_, _, __TOKEN__> = activity.run(
        WorkflowRunId::from_stable(identity), std::num::NonZeroU32::MIN,
        || Effect::<Result<i64, i64>, (), __TOKEN__>::sync_once(|| Ok(1)),
    );
}
"#,
            "Environment",
            "()",
            "HasWorkflowEngine",
        ),
        (
            "T06_empty_rpc_protocol",
            r#"
use mandrill_effect::{rpc_protocol, json_i64, Effect};
use mandrill_effect::encoding::FramePolicy;
use mandrill_effect::rpc::{OperationId, RpcEnvelopePolicy, RpcHandler,
    RpcOperation, RpcProtocol, RpcVersion};
fn main() {
    let operation = RpcOperation::unary(
        OperationId::new("typed-protocol").unwrap(),
        RpcVersion::new(std::num::NonZeroU32::MIN),
        json_i64(), json_i64(), json_i64(), limits(),
    );
    let handler = RpcHandler::new(operation, |value| {
        Effect::<i64, i64, ()>::sync_once(move || value)
    });
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: std::num::NonZeroUsize::new(256).unwrap(),
        },
        json: limits(),
    };
    let _: RpcProtocol<i64, ()> = rpc_protocol! {
        policy: policy,
        handlers: [__TOKEN__],
    }.unwrap();
}
"#,
            "handler",
            "",
            "requires at least one typed handler",
        ),
        (
            "T08_volatile_provider_rejected_by_durable_queue",
            r#"
use mandrill_effect::KeyDomain;
use mandrill_effect::persistence::{DurablePersistedQueueStore,
    HasPersistedQueueStore, MemoryPersistedQueueStore};
use mandrill_effect::workflow::{DurableQueue, DurableWorkflowEngine,
    HasWorkflowEngine, MemoryWorkflowEngine};
use std::sync::Arc;
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-durable-required"; }
fn main() {
    __TOKEN__
}
"#,
            r#"
    fn accepted<R>(queue: DurableQueue<i64, Domain>)
    where
        R: HasPersistedQueueStore + HasWorkflowEngine + Send + Sync + 'static,
        <R as HasPersistedQueueStore>::Store: DurablePersistedQueueStore,
        <R as HasWorkflowEngine>::Engine: DurableWorkflowEngine,
    {
        let _ = queue.require_physical_durability::<R>();
    }
    "#,
            r#"
    struct Volatile;
    impl HasPersistedQueueStore for Volatile {
        type Store = MemoryPersistedQueueStore;
        fn persisted_queue_store(&self) -> Arc<Self::Store> { unreachable!() }
    }
    impl HasWorkflowEngine for Volatile {
        type Engine = MemoryWorkflowEngine;
        fn workflow_engine(&self) -> Arc<Self::Engine> { unreachable!() }
    }
    fn rejected(queue: DurableQueue<i64, Domain>) {
        let _ = queue.require_physical_durability::<Volatile>();
    }
    "#,
            "DurablePersistedQueueStore",
        ),
        (
            "T09_wrong_deferred_result_type",
            r#"
use mandrill_effect::{Effect, KeyDomain, KeyNamespace, KeySegment,
    StableKey, json_i64};
use mandrill_effect::workflow::{DurableDeferred, HasWorkflowEngine,
    MemoryWorkflowEngine, WorkflowRunId, WorkflowSignalId, WorkflowVersion};
use std::sync::Arc;
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-deferred"; }
struct Environment(Arc<MemoryWorkflowEngine>);
impl HasWorkflowEngine for Environment {
    type Engine = MemoryWorkflowEngine;
    fn workflow_engine(&self) -> Arc<Self::Engine> { Arc::clone(&self.0) }
}
fn main() {
    let namespace = KeyNamespace::<Domain>::new("typing").unwrap();
    let identity = StableKey::new(&namespace, [KeySegment::Unsigned(1)]).unwrap();
    let token = DurableDeferred {
        run: WorkflowRunId::from_stable(identity.clone()),
        signal: WorkflowSignalId::from_stable(identity),
        version: WorkflowVersion::new(std::num::NonZeroU32::MIN),
        codec: json_i64(), limits: limits(),
    };
    let _: Effect<_, _, Environment> = token.complete::<Environment>(__TOKEN__);
}
"#,
            "41_i64",
            "\"wrong\"",
            "E0308",
        ),
        (
            "T10_domain_error_union_required",
            r#"
use mandrill_effect::Effect;
fn main() {
    let first = Effect::<i64, &'static str>::succeed(1);
    let _: Effect<i64, usize> = first__TOKEN__.flat_map(
        |_| Effect::<i64, usize>::succeed(2));
}
"#,
            ".map_error(|_| 0_usize)",
            "",
            "E0308",
        ),
        (
            "T13_wrong_persisted_success_codec",
            r#"
use mandrill_effect::{KeyDomain, KeyNamespace, KeySegment,
    json_i64, json_string};
use mandrill_effect::persistence::{Persistable, RecordVersion};
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-record"; }
fn main() {
    let _: Persistable<u64, i64, i64, Domain> = Persistable::new(
        KeyNamespace::new("records").unwrap(),
        RecordVersion::new("v1").unwrap(),
        |value: &u64| Ok(vec![KeySegment::Unsigned(*value)]),
        __TOKEN__, json_i64(), limits(),
    );
}
"#,
            "json_i64()",
            "json_string()",
            "E0308",
        ),
        (
            "T15_wrong_reactive_write_type",
            r#"
use mandrill_effect::{KeyDomain, KeyNamespace, KeySegment, StableKey,
    PubSubOptions, PubSubOverflow, ReplayPolicy};
use mandrill_effect::reactivity::{AtomId, AtomRef, AtomRegistry};
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-reactive"; }
fn main() {
    let graph = AtomRegistry::new(PubSubOptions {
        capacity: std::num::NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    }).unwrap();
    let namespace = KeyNamespace::<Domain>::new("atoms").unwrap();
    let identity = StableKey::new(&namespace, [KeySegment::Unsigned(1)]).unwrap();
    let source = AtomRef::new(graph, AtomId::from_stable(identity), 0_i64,
        std::num::NonZeroUsize::MIN, PubSubOverflow::Backpressure).unwrap();
    let _ = source.set::<()>(__TOKEN__);
}
"#,
            "1_i64",
            "\"wrong\"",
            "E0308",
        ),
        (
            "T18_wrong_transaction_reference_type",
            r#"
use mandrill_effect::transactional::TransactionDomain;
use std::sync::Arc;
fn main() {
    let domain = TransactionDomain::new(std::num::NonZeroUsize::MIN);
    let reference = domain.reference(0_i64).unwrap();
    let _ = reference.write::<()>(Arc::new(__TOKEN__));
}
"#,
            "1_i64",
            "\"wrong\"",
            "E0308",
        ),
        (
            "T20_unpaired_rpc_handler_result",
            r#"
use mandrill_effect::{Effect, json_i64};
use mandrill_effect::rpc::{OperationId, RpcHandler, RpcOperation, RpcVersion};
fn main() {
    let operation = RpcOperation::unary(
        OperationId::new("paired").unwrap(),
        RpcVersion::new(std::num::NonZeroU32::MIN),
        json_i64(), json_i64(), json_i64(), limits(),
    );
    let _: RpcHandler<i64, i64, i64, ()> = RpcHandler::new(operation,
        |value| Effect::<__TOKEN__, i64, ()>::sync_once(move || value));
}
"#,
            "i64",
            "String",
            "E0308",
        ),
        (
            "T24_read_only_reactive_descriptor",
            r#"
use mandrill_effect::{Effect, KeyDomain, KeyNamespace, KeySegment, StableKey,
    PubSubOptions, PubSubOverflow, ReplayPolicy};
use mandrill_effect::reactivity::{Atom, AtomId, AtomRef, AtomRegistry};
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-readonly"; }
fn main() {
    let graph = AtomRegistry::new(PubSubOptions {
        capacity: std::num::NonZeroUsize::MIN,
        overflow: PubSubOverflow::Backpressure,
        replay: ReplayPolicy::None,
    }).unwrap();
    let namespace = KeyNamespace::<Domain>::new("atoms").unwrap();
    let source_id = StableKey::new(&namespace, [KeySegment::Unsigned(1)]).unwrap();
    let derived_id = StableKey::new(&namespace, [KeySegment::Unsigned(2)]).unwrap();
    let source = AtomRef::new(graph.clone(), AtomId::from_stable(source_id), 0_i64,
        std::num::NonZeroUsize::MIN, PubSubOverflow::Backpressure).unwrap();
    let derived = Atom::<i64, (), ()>::new(graph, AtomId::from_stable(derived_id),
        std::num::NonZeroUsize::MIN, PubSubOverflow::Backpressure,
        |_| Effect::<i64, (), ()>::sync_once(|| 1)).unwrap();
    let _ = __TOKEN__.set::<()>(1_i64);
}
"#,
            "source",
            "derived",
            "E0599",
        ),
        (
            "T11_effect_schema_decode_environment",
            r#"
use mandrill_effect::{Codec, Effect, EffectSchema, JsonValue,
    Schema, SchemaIssue};
use std::sync::Arc;
struct Decode;
struct Encode;
fn main() {
    let old = Schema::from_decoder(|value: &JsonValue| {
        value.as_i64().ok_or("invalid")
    });
    let _: Effect<i64, &'static str, ()> = old.decode(Arc::new(JsonValue::from(1)));
    let codec: Codec<i64, JsonValue, Decode, Encode> = Codec::new(
        |value: JsonValue| Effect::<i64, SchemaIssue, Decode>::sync_once(
            move || value.as_i64().unwrap()),
        |value| Effect::<JsonValue, SchemaIssue, Encode>::sync_once(
            move || JsonValue::from(value)),
    );
    let view: EffectSchema<i64, Decode> = codec.as_effect_schema();
    let _: Effect<i64, SchemaIssue, __TOKEN__> =
        view.decode_once(JsonValue::from(1));
}
"#,
            "Decode",
            "Encode",
            "E0308",
        ),
        (
            "T16_wrong_event_payload_codec",
            r#"
use mandrill_effect::{json_i64, json_string};
use mandrill_effect::encoding::{BinaryIdentity, BinaryLimits};
use mandrill_effect::eventlog::{EventBinaryFormats, EventCodecs,
    EventDefinition, EventGroupId, EventShape, EventTag, EventVersion};
fn main() {
    let identity = |name| BinaryIdentity::new(name,
        std::num::NonZeroU64::MIN).unwrap();
    let maximum = std::num::NonZeroUsize::new(256).unwrap();
    let _: EventDefinition<i64, i64, i64> = EventDefinition::new(
        EventShape {
            group: EventGroupId::new("typed").unwrap(),
            tag: EventTag::new("written").unwrap(),
            version: EventVersion::new(std::num::NonZeroU32::MIN),
        },
        EventCodecs { payload: __TOKEN__, success: json_i64(), error: json_i64() },
        EventBinaryFormats {
            payload: identity("payload"), success: identity("success"),
            error: identity("error"),
            limits: BinaryLimits { bytes: maximum, depth: maximum,
                nodes: maximum, collection: maximum, steps: maximum },
        },
    );
}
"#,
            "json_i64()",
            "json_string()",
            "E0308",
        ),
        (
            "T19_incomplete_schema_constructor",
            r#"
use mandrill_effect::{RecordBuilder, ParsePolicy, SchemaErrorMode,
    UnknownFields, DuplicateFields, json_i64};
struct Record { count: i64 }
fn main() {
    let policy = ParsePolicy { limits: limits(), errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject };
    let _ = RecordBuilder::<Record>::new()
        .field("count", |record| &record.count, json_i64())
        .finish(policy, __TOKEN__).unwrap();
}
"#,
            "|(count, ())| Record { count }",
            "|| Record { count: 0 }",
            "E0593",
        ),
        (
            "T21_wrong_stream_chunk_codec",
            r#"
use mandrill_effect::{json_i64, json_string};
use mandrill_effect::rpc::{OperationId, RpcStreamOperation, RpcVersion};
fn main() {
    let _: RpcStreamOperation<i64, i64, i64, i64> = RpcStreamOperation::new(
        OperationId::new("typed-stream").unwrap(),
        RpcVersion::new(std::num::NonZeroU32::MIN),
        json_i64(), __TOKEN__, json_i64(), json_i64(), limits(),
    );
}
"#,
            "json_i64()",
            "json_string()",
            "E0308",
        ),
        (
            "T25_wrong_projection_reducer_result",
            r#"
use mandrill_effect::{KeyDomain, KeyNamespace, KeySegment, StableKey};
use mandrill_effect::eventlog::{AggregateId, EventJournalId,
    JournalStream, Projection, ProjectionId, ProjectionKey, ProjectionVersion};
use std::collections::BTreeMap;
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-projection"; }
fn main() {
    let namespace = KeyNamespace::<Domain>::new("typing").unwrap();
    let identity = StableKey::new(&namespace, [KeySegment::Unsigned(1)]).unwrap();
    let key = ProjectionKey {
        id: ProjectionId::from_stable(identity.clone()),
        stream: JournalStream {
            journal: EventJournalId::from_stable(identity.clone()),
            aggregate: AggregateId::from_stable(identity),
        },
    };
    let _: Projection<i64, ()> = Projection::new(key,
        ProjectionVersion::new(std::num::NonZeroU32::MIN),
        BTreeMap::new(), || 0_i64,
        |state, _| __TOKEN__,
    );
}
"#,
            "Ok::<i64, ()>(*state + 1)",
            "Ok::<String, ()>(String::from(\"wrong\"))",
            "E0308",
        ),
        (
            "T12_public_cpu_capability_boundary",
            r#"
use mandrill_effect::{CpuExecutor, Defect, ScheduledJob};
struct Workers;
impl CpuExecutor for Workers {
    fn submit(&self, job: ScheduledJob) -> Result<(), Defect> {
        __TOKEN__
    }
}
fn main() { let _: &dyn CpuExecutor = &Workers; }
"#,
            "std::thread::Builder::new().spawn(move || { let _ = job.run(); }).map(|_| ()).map_err(Defect::from_io)",
            "let _ = job.pending; Ok(())",
            "E0616",
        ),
        (
            "T23_persistable_macro_codec_pairing",
            r#"
use mandrill_effect::{KeyDomain, KeyNamespace, json_i64,
    json_string, persistable};
use mandrill_effect::persistence::{Persistable, RecordPolicy, RecordVersion};
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-persistable-macro"; }
fn main() {
    let _: Persistable<i64, i64, i64, Domain> = persistable! {
        namespace: KeyNamespace::<Domain>::new("records").unwrap(),
        request: i64,
        key: |_: &i64| -> &'static str { "item" },
        success: __TOKEN__, error: json_i64(),
        version: RecordPolicy::new(
            RecordVersion::new("v1").unwrap(), limits()),
    };
}
"#,
            "json_i64()",
            "json_string()",
            "E0308",
        ),
        (
            "T27_private_persistable_construction",
            r#"
use mandrill_effect::{KeyDomain, KeyNamespace, KeySegment, json_i64};
use mandrill_effect::persistence::{Persistable, RecordVersion};
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-private-record"; }
fn main() {
    let base: Persistable<u64, i64, i64, Domain> = Persistable::new(
        KeyNamespace::new("records").unwrap(),
        RecordVersion::new("v1").unwrap(),
        |value: &u64| Ok(vec![KeySegment::Unsigned(*value)]),
        json_i64(), json_i64(), limits(),
    );
    let _: Persistable<u64, i64, i64, Domain> = __TOKEN__;
}
"#,
            "base",
            "Persistable { namespace: KeyNamespace::new(\"other\").unwrap(), ..base }",
            "E0451",
        ),
        (
            "T27_rpc_protocol_requires_first_handler",
            r#"
use mandrill_effect::{Effect, json_i64};
use mandrill_effect::encoding::FramePolicy;
use mandrill_effect::rpc::{OperationId, RpcEnvelopePolicy, RpcHandler,
    RpcOperation, RpcProtocol, RpcVersion};
fn main() {
    let operation = RpcOperation::unary(
        OperationId::new("typed-first-handler").unwrap(),
        RpcVersion::new(std::num::NonZeroU32::MIN),
        json_i64(), json_i64(), json_i64(), limits(),
    );
    let handler = RpcHandler::new(operation, |value| {
        Effect::<i64, i64, ()>::sync_once(move || value)
    });
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: std::num::NonZeroUsize::new(256).unwrap(),
        },
        json: limits(),
    };
    let _: RpcProtocol<i64, ()> = __TOKEN__.unwrap();
}
"#,
            "RpcProtocol::from_handler(policy, handler)",
            "RpcProtocol::new(policy)",
            "no associated function or constant named `new`",
        ),
        (
            "T27_entity_dispatcher_requires_first_handler",
            r#"
use mandrill_effect::{Effect, json_i64};
use mandrill_effect::encoding::FramePolicy;
use mandrill_effect::cluster::{ClusterEntityType, EntityDispatcher};
use mandrill_effect::rpc::{OperationId, RpcEnvelopePolicy, RpcHandler,
    RpcOperation, RpcVersion};
fn main() {
    let operation = RpcOperation::unary(
        OperationId::new("typed-entity-first-handler").unwrap(),
        RpcVersion::new(std::num::NonZeroU32::MIN),
        json_i64(), json_i64(), json_i64(), limits(),
    );
    let handler = RpcHandler::new(operation, |value| {
        Effect::<i64, i64, ()>::sync_once(move || value)
    });
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: std::num::NonZeroUsize::new(256).unwrap(),
        },
        json: limits(),
    };
    let kind = ClusterEntityType::new("typed-entity").unwrap();
    let _: EntityDispatcher<()> = __TOKEN__.unwrap();
}
"#,
            "EntityDispatcher::from_handler(policy, kind, handler)",
            "EntityDispatcher::new(policy)",
            "no associated function or constant named `new`",
        ),
        (
            "T14_rpc_middleware_error_pairing",
            r#"
use mandrill_effect::{Effect, json_i64};
use mandrill_effect::encoding::{FrameParts, FramePolicy};
use mandrill_effect::rpc::{HasRpcChannel, OperationId, RpcChannel,
    RpcClient, RpcClientMiddleware, RpcEnvelopePolicy, RpcOperation, RpcVersion};
use std::sync::Arc;
struct Channel;
impl RpcChannel for Channel {
    type Error = ();
    fn exchange(&self, _: FrameParts) -> Effect<Vec<u8>, Self::Error> {
        Effect::fail_with(|| ())
    }
}
struct Environment(Arc<Channel>);
impl HasRpcChannel for Environment {
    type Channel = Channel;
    fn rpc_channel(&self) -> Arc<Self::Channel> { Arc::clone(&self.0) }
}
fn main() {
    let operation = RpcOperation::unary(
        OperationId::new("middleware").unwrap(),
        RpcVersion::new(std::num::NonZeroU32::MIN),
        json_i64(), json_i64(), json_i64(), limits(),
    );
    let policy = RpcEnvelopePolicy {
        frame: FramePolicy {
            maximum_payload: std::num::NonZeroUsize::new(256).unwrap(),
        },
        json: limits(),
    };
    let client = RpcClient::<i64, i64, i64, ()>::new(
        operation, policy, std::num::NonZeroUsize::MIN);
    let _: RpcClientMiddleware<i64, i64, i64, i64, (), &'static str, Environment> =
        RpcClientMiddleware::new(client,
            |request| Effect::<i64, __TOKEN__, Environment>::sync_once(move || request),
            |response| Effect::<i64, &'static str, Environment>::sync_once(
                move || response));
}
"#,
            "&'static str",
            "usize",
            "E0308",
        ),
        (
            "T22_missing_queue_workflow_capability",
            r#"
use mandrill_effect::KeyDomain;
use mandrill_effect::persistence::{HasPersistedQueueStore, QueueClaimOptions};
use mandrill_effect::workflow::{DurableQueue, HasWorkflowEngine};
struct Domain;
impl KeyDomain for Domain { const TAG: &'static str = "typing-capabilities"; }
fn require<R: __TOKEN__ + Send + Sync + 'static>(
    queue: &DurableQueue<i64, Domain>, options: QueueClaimOptions,
) {
    let _ = queue.claim::<R>(options);
}
fn main() {}
"#,
            "HasWorkflowEngine + HasPersistedQueueStore",
            "HasWorkflowEngine",
            "HasPersistedQueueStore",
        ),
    ];

    for (name, source, positive, negative, diagnostic) in pairs {
        if std::env::var("EFFECT_TYPING_CASE")
            .ok()
            .is_some_and(|selected| !selected.split(',').any(|case| case == name))
        {
            continue;
        }
        for (suffix, token, expected_failure) in [
            ("positive", positive, None),
            ("negative", negative, Some(diagnostic)),
        ] {
            let case = source.replace("__TOKEN__", token) + limits;
            std::fs::write(directory.0.join("src/main.rs"), case)?;
            let output = Command::new(env!("CARGO"))
                .args(["check", "--quiet", "--offline", "--manifest-path"])
                .arg(directory.0.join("Cargo.toml"))
                .env(
                    "CARGO_TARGET_DIR",
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"),
                )
                .output()?;
            let diagnostics = String::from_utf8(output.stderr)?;
            match expected_failure {
                Some(reason) => assert!(
                    !output.status.success() && diagnostics.contains(reason),
                    "{name}_{suffix}: expected {reason}; got {diagnostics}"
                ),
                None => assert!(output.status.success(), "{name}_{suffix}: {diagnostics}"),
            }
        }
    }
    Ok(())
}

/// Compile the five real extension consumers through renamed dependencies, then
/// diagnose a finite set of distinct domain syntax/phase misuse contracts.
#[test]
fn extension_compiler_contracts() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = FixtureDirectory(
        std::env::temp_dir().join(format!("mandrill-extension-typing-{}", std::process::id())),
    );
    std::fs::create_dir_all(directory.0.join("src"))?;
    let mut manifest = String::from(
        "[package]\nname = \"extension-typing-consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\nrust-version = \"1.99\"\n[workspace]\n[profile.dev]\ndebug = 0\nincremental = false\n[features]\ndefault = [\"sqlx-turso\"]\nsqlx-turso = []\n[dependencies]\n",
    );
    for (alias, package, folder, features) in [
        ("mandrill_effect", "effect", "effect", ""),
        ("database", "effect-sql", "effect_sql", "macros,sync"),
        ("http_server", "effect-server", "effect_server", ""),
        (
            "accounts",
            "effect-server-identity",
            "effect_server_identity",
            "sqlx-turso",
        ),
        (
            "application",
            "effect-server-web",
            "effect_server_web",
            "identity,otel,sqlx-turso",
        ),
        ("telemetry", "effect-otel", "effect_otel", "server"),
        ("effect_tokio", "effect-tokio", "effect_tokio", ""),
        ("sqlx", "sqlx-turso-sqlx", "sqlx", "derive,runtime-tokio"),
    ] {
        let features = features
            .split(',')
            .filter(|value| !value.is_empty())
            .map(|value| format!("{value:?}"))
            .collect::<Vec<_>>()
            .join(",");
        manifest.push_str(&format!(
            "{alias} = {{ package = {package:?}, path = {:?}, features = [{features}] }}\n",
            root.join("crates").join(folder)
        ));
    }
    manifest.push_str("tokio = { version = \"1.53.1\", features = [\"macros\", \"rt-multi-thread\", \"net\", \"io-util\", \"sync\", \"time\", \"signal\"] }\naxum = { version = \"0.8.9\", default-features = false }\nhttp = \"1.5.0\"\nbytes = \"1.12.1\"\nhttp-body-util = \"0.1.5\"\nopentelemetry = { version = \"0.32.0\", features = [\"trace\", \"metrics\", \"logs\"] }\nopentelemetry_sdk = { version = \"0.32.1\", features = [\"trace\", \"metrics\", \"logs\", \"testing\"] }\ntracing = \"0.1.44\"\nserde_json = \"1.0.151\"\n");
    manifest.push_str("serde = { version = \"1.0.229\", features = [\"derive\"] }\ntime = { version = \"0.3.55\", features = [\"serde\", \"formatting\", \"parsing\"] }\nuuid = { version = \"1.26.1\", features = [\"v4\", \"v7\"] }\n");
    let workspace = std::fs::read_to_string(root.join("Cargo.toml"))?;
    let identity = workspace
        .lines()
        .find(|line| line.starts_with("identity_rs = "))
        .ok_or("workspace Identity dependency is missing")?;
    manifest.push_str(identity);
    manifest.push('\n');
    std::fs::write(directory.0.join("Cargo.toml"), manifest)?;

    // Server/Web reuse this existing provider module across their example trees.
    copy_renamed_consumer(
        &root.join("crates/effect_otel/examples/cafe_telemetry"),
        &directory.0.join("src/cafe_telemetry"),
    )?;
    // Credit completed consumers when retrying a later failed or unrun case.
    let skipped_consumers = std::env::var("EFFECT_EXTENSION_SKIP_CONSUMERS").unwrap_or_default();
    for (folder, example) in [
        ("effect_sql", "cafe_orders"),
        ("effect_server", "serve_cafe_orders"),
        ("effect_server_identity", "customer_account"),
        ("effect_server_web", "cafe_application"),
        ("effect_otel", "watch_order_progress"),
    ] {
        if skipped_consumers.split(',').any(|name| name == example) {
            continue;
        }
        copy_renamed_consumer(
            &root.join("crates").join(folder).join("examples"),
            &directory.0.join("src"),
        )?;
        let source =
            std::fs::read_to_string(directory.0.join("src").join(format!("{example}.rs")))?;
        std::fs::write(directory.0.join("src/main.rs"), source)?;
        let output = extension_check(&directory.0, &root)?;
        assert!(
            output.status.success(),
            "{example} renamed consumer: {}",
            String::from_utf8(output.stderr)?
        );
    }

    for (name, source, reason) in [
        (
            "sql_identifier",
            "database::sql! { identifier(\"a..b\"); }",
            "identifier parts must be nonempty",
        ),
        (
            "sql_assignment",
            "database::sql_insert! { table: \"orders\", value(\"id\", 1); value(\"id\", 2); }",
            "duplicate mutation column",
        ),
        (
            "sql_guard",
            "database::sql_update! { table: \"orders\", set(\"ready\", true); }",
            "exactly one WHERE",
        ),
        (
            "sql_projection",
            "database::sql_with! { select: (), effect: (), run: () }",
            "requires exactly one",
        ),
        (
            "endpoint_forms",
            "http_server::endpoint! { body { () } effect: () }",
            "mutually exclusive",
        ),
        (
            "reply_terminal",
            "http_server::reply! { text(\"a\"); text(\"b\"); }",
            "exactly one terminal body",
        ),
        (
            "reply_status",
            "http_server::reply! { status(()); no_content(); }",
            "bodyless terminal controls its own status",
        ),
        (
            "route_shape",
            "http_server::route! { literal(\"/{id}\"); }",
            "without captures",
        ),
        (
            "route_phase",
            "http_server::route! { capture(\"id\", ()); }",
            "before captures",
        ),
        (
            "registration",
            "http_server::registered! { endpoint: (), authorize(\"customer\"); allow_anonymous(true); }",
            "conflicts with authorize",
        ),
        (
            "representation",
            "http_server::representations! {}",
            "requires an alternative",
        ),
        (
            "router_arity",
            "http_server::router! { get(()); }",
            "requires 2 arguments",
        ),
        (
            "routes_arity",
            "http_server::routes! { methods((), ()); }",
            "requires 3 arguments",
        ),
        (
            "pipeline_callback",
            "http_server::pipeline! { before |a,b| { () } }",
            "wrong inline callback parameter count",
        ),
        (
            "server_required",
            "http_server::server! {}",
            "server requires policy",
        ),
        (
            "stream_forms",
            "http_server::stream! { source: (), body |writer| { () } }",
            "mutually exclusive",
        ),
        (
            "identity_duplicate",
            "accounts::identity_store! { backend: (), backend: () }",
            "duplicate Identity field",
        ),
        (
            "identity_required",
            "accounts::identity! {}",
            "missing Identity field: storage",
        ),
        (
            "identity_endpoint",
            "accounts::identity_endpoints! {}",
            "missing Identity endpoint field: identity",
        ),
        (
            "web_required",
            "application::web! {}",
            "missing required Web field",
        ),
        (
            "security_duplicate",
            "application::security! { body_limit: (), body_limit: () }",
            "duplicate Web descriptor field",
        ),
        (
            "csp_duplicate",
            "application::csp! { directive(\"default-src\", ()); directive(\"default-src\", ()); }",
            "duplicate CSP directive",
        ),
        (
            "cache_required",
            "application::read_through! {}",
            "missing required Web field `scope`",
        ),
        (
            "dependency_required",
            "application::dependent_read_through! {}",
            "missing required Web field `scope`",
        ),
        (
            "availability_required",
            "application::availability! {}",
            "missing required Web field `service`",
        ),
        (
            "message_duplicate",
            "application::message_arguments! { message: (), field(\"drink\", 1); field(\"drink\", 2); }",
            "empty/duplicate message field",
        ),
        (
            "localization_required",
            "application::localization! {}",
            "missing required Web field `catalogue`",
        ),
        (
            "otel_forms",
            "telemetry::otel! { name: \"order\", body { () }, effect: () }",
            "requires exactly one",
        ),
    ] {
        std::fs::write(
            directory.0.join("src/main.rs"),
            format!(
                "/// Exercise one rejected domain description at the ordinary consumer entry point.\nfn main() {{ let _ = {source}; }}"
            ),
        )?;
        let output = extension_check(&directory.0, &root)?;
        let diagnostics = String::from_utf8(output.stderr)?;
        assert!(
            !output.status.success() && diagnostics.contains(reason),
            "{name}: expected {reason}; got {diagnostics}"
        );
    }
    Ok(())
}

/// Reuse the coordinator's compilation target and offline dependency resolution.
fn extension_check(
    directory: &std::path::Path,
    root: &std::path::Path,
) -> std::io::Result<std::process::Output> {
    Command::new(env!("CARGO"))
        .args(["check", "--quiet", "--offline", "--manifest-path"])
        .arg(directory.join("Cargo.toml"))
        .env("DATABASE_URL", "turso::memory:")
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
}

/// Copy real consumers with renamed dependencies and relocate their one shared
/// telemetry support path into the temporary fixture's source tree.
fn copy_renamed_consumer(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if path.is_dir() {
            copy_renamed_consumer(&path, &target)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let mut text = std::fs::read_to_string(path)?.replace(
                "../../effect_otel/examples/cafe_telemetry/mod.rs",
                "cafe_telemetry/mod.rs",
            );
            for (original, renamed) in [
                ("effect_server_identity", "accounts"),
                ("effect_server_web", "application"),
                ("effect_server", "http_server"),
                ("effect_sql", "database"),
                ("effect_otel", "telemetry"),
                ("effect::", "mandrill_effect::"),
            ] {
                text = text.replace(original, renamed);
            }
            std::fs::write(target, text)?;
        }
    }
    Ok(())
}
