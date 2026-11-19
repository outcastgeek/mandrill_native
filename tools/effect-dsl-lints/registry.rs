//! Macro-owned native construction/composition APIs, matched after compiler resolution.
use rustc_hir::def::DefKind;

/// Native facades re-export the actual approved procedural definition identities.
pub fn facade(name: &str) -> bool {
    matches!(
        name,
        "effect"
            | "effect_sql"
            | "effect_server"
            | "effect_server_identity"
            | "effect_server_web"
            | "effect_otel"
            | "effect_cms"
    )
}
pub const MACROS: &[&str] = &[
    "memory",
    "memory_lease",
    "memory_vec",
    "memory_write",
    "effect_drop_guard",
    "effect",
    "schema",
    "schema_enum",
    "persistable",
    "transaction",
    "event",
    "event_group",
    "projection",
    "workflow",
    "activity",
    "workflow_definition",
    "activity_definition",
    "rpc_operation",
    "rpc_stream_operation",
    "rpc_protocol",
    "rpc_stream_protocol",
    "rpc_middleware",
    "rpc_stream_middleware",
    "entity",
    "reactive_source",
    "reactive_derived",
    "reactive_query",
    "reactive_bridge",
    "span",
    "service",
    "cache",
    "dependency_cache",
    "atom",
    "hydration",
    "entity_resource",
    "singleton",
    "cluster_schedule",
    "persisted_queue",
    "durable_queue",
    "stream",
    "handled_event",
    "cluster",
    "sql",
    "sql_insert",
    "sql_update",
    "row",
    "sql_with",
    "server",
    "endpoint",
    "registered",
    "reply",
    "representations",
    "route",
    "router",
    "routes",
    "pipeline",
    "server_stream",
    "identity_store",
    "identity",
    "identity_endpoints",
    "web",
    "security",
    "csp",
    "read_through",
    "dependent_read_through",
    "availability",
    "message_arguments",
    "localization",
    "otel",
    "cms",
    "effect_sync",
    "effect_try_sync",
    "effect_future",
    "effect_die",
    "effect_fail",
    "effect_try_future",
    "effect_cpu",
    "effect_try_cpu",
    "effect_blocking",
    "effect_try_blocking",
    "effect_map",
    "effect_map_error",
    "effect_provide_some",
    "effect_sandbox",
    "effect_from_option",
    "effect_from_result",
    "effect_timeout",
    "effect_recover",
    "effect_for_each",
    "effect_for_each_concurrent",
    "effect_provided",
    "effect_race",
    "effect_uninterruptible",
    "effect_on_interrupt",
    "effect_all",
    "effect_all_concurrent",
    "effect_retry",
    "effect_repeat",
    "effect_resource",
    "effect_using",
    "effect_scope",
    "effect_prepare",
    "effect_scope_acquire",
    "effect_finalizer",
    "layer",
    "effect_with_layer",
    "effect_permit",
    "schedule",
    "effect_ref",
    "pool",
    "weak_cache",
    "request_resolver",
    "sink",
    "effect_stream_for_each",
    "effect_socket",
    "codec",
    "effect_schema_parse",
];

/// Public lowerings shared by the Effect block, callback and ownership macros.
const EFFECT: &[&str] = &[
    "new",
    "succeed",
    "fail",
    "fail_with",
    "from_result",
    "from_result_once",
    "die",
    "sync",
    "sync_once",
    "try_sync",
    "try_sync_once",
    "from_future",
    "from_prepared_future",
    "from_future_once",
    "from_future_with_env",
    "try_future",
    "try_future_once",
    "try_future_with_env",
    "cpu",
    "cpu_once",
    "cpu_with_context",
    "cpu_once_with_context",
    "try_cpu",
    "try_cpu_once",
    "try_cpu_with_context",
    "blocking",
    "blocking_once",
    "blocking_with_context",
    "blocking_once_with_context",
    "try_blocking",
    "try_blocking_once",
    "try_blocking_with_context",
    "from_exit_future",
    "from_exit_future_with_env",
    "from_exit_future_with_context",
    "from_prepared_exit_future_with_context",
    "service",
    "defer",
    "flat_map",
    "map",
    "map_error",
    "provide_some",
    "provide_some_arc",
    "sandbox",
    "catch_all",
    "from_option",
    "from_option_once",
    "all",
    "all_concurrent",
    "for_each",
    "for_each_sequential",
    "for_each_concurrent",
    "for_each_concurrent_shared",
    "timeout",
    "provide",
    "race",
    "uninterruptible",
    "on_interrupt",
    "retry",
    "repeat",
    "with_span",
    "annotate",
    "ensuring",
    "ensuring_with",
    "zip",
    "while_loop",
];

/// Each row names the defining native owner and its macro-provided entry points.
/// Receiver resolution handles renamed imports, re-exports and inherent UFCS.
const OWNERS: &[(&str, &[&str])] = &[
    ("Workflow", &["new", "from_exit"]),
    (
        "Transaction",
        &[
            "from_body",
            "succeed",
            "fail",
            "sync",
            "flat_map",
            "map",
            "map_error",
        ],
    ),
    ("TxRef", &["update"]),
    ("TransactionDomain", &["prepare"]),
    (
        "AtomProgram",
        &["from_body", "map_error", "map", "flat_map", "defer"],
    ),
    (
        "Durable",
        &[
            "from_result_once",
            "from_body",
            "from_body_once",
            "for_each",
            "retry",
            "map_error",
            "map",
            "flat_map",
            "defer",
        ],
    ),
    ("WorkflowCompensation", &["run", "on_failure"]),
    ("EntityResourcePlan", &["from_body"]),
    (
        "Stream",
        &[
            "new",
            "from_body",
            "from_pull",
            "from_iterable",
            "from_async_iterable",
            "from_async_iterable_effect",
            "map_effect",
            "map_error",
            "provide_some",
            "run_for_each",
        ],
    ),
    ("Sink", &["for_each", "fold"]),
    ("PreparedStream", &["new", "from_iterable", "map", "rechunk", "collect", "fold"]),
    ("ResourceBinding", &["new", "use_with"]),
    ("Release", &["from_effect", "sync", "try_sync", "new"]),
    (
        "Scope",
        &["scoped", "acquire", "add_release", "add_finalizer"],
    ),
    (
        "Layer",
        &["new", "scoped", "merge", "provide", "provide_to"],
    ),
    ("PreparedScope", &["close", "admit_close", "reserve_closed"]),
    ("ScopePermit", &["reserve", "bind", "try_bind"]),
    ("PreparedScopeOwner", &["prepare", "prepare_with_cleanup", "into_effect", "reserve_close"]),
    ("PreparedRunPlan", &["new", "prepare"]),
    ("PreparedRun", &["admit", "admit_with_prepared_trace"]),
    ("PreparedRunContext", &["native", "reserve_native"]),
    ("PreparedNativeReservation", &["native"]),
    (
        "PreparedGraphPlan",
        &["new", "new_with_input", "prepare", "with_construction"],
    ),
    ("ConstructedGraphPlan", &["prepare"]),
    ("ConstructedGraph", &["admit", "admit_with_prepared_trace"]),
    ("PreparedGraph", &["admit", "admit_with_prepared_trace"]),
    ("PreparedScopePool", &["prepare", "scoped", "try_scoped", "admit_scoped"]),
    ("PreparedResourceBinding", &["acquire_scoped", "admit_acquire_scoped"]),
    ("PreparedFutureResourceBinding", &["acquire_scoped", "admit_acquire_scoped"]),
    ("PreparedCheckpoint", &["release"]),
    ("PreparedRelease", &["prepare", "register", "admit_registration"]),
    ("PreparedResourcePlan", &["new"]),
    ("PreparedFutureResourcePlan", &["new"]),
    ("PreparedReleasePlan", &["new"]),
    ("Semaphore", &["acquire"]),
    ("PartitionedSemaphore", &["acquire", "acquire_with"]),
    ("Schedule", &["unfold"]),
    ("AtomicRef", &["modify", "modify_in_place", "update"]),
    ("PreparedAtomicRef", &["modify_now", "modify_in_place_now", "update_now"]),
    ("PreparedSynchronizedRef", &["admit_write"]),
    ("PreparedSynchronizedWrite", &["snapshot", "commit", "release"]),
    ("PreparedQueue", &["offer_try_with_context", "take_with_context"]),
    ("PreparedPubSub", &["admit_publication"]),
    ("PreparedPublication", &["run_candidate_with_context"]),
    ("PreparedSubscription", &["take_with_context"]),
    ("PreparedSubscriptionRef", &["admit_set", "read_now", "subscribe_now"]),
    ("PreparedExitMapper", &["admit", "map"]),
    ("SynchronizedRef", &["modify_effect"]),
    ("Pool", &["make", "make_with_prepared_storage", "exists_idle", "exists_idle_prepared"]),
    ("WeakCache", &["get", "get_partition", "make_with_prepared_storage", "make_with_prepared_refusals"]),
    ("PartitionedWeakCache", &["get", "get_partition"]),
    (
        "RequestResolver",
        &["make", "make_with_prepared_index", "make_with_prepared_batches"],
    ),
    ("ServiceOperation", &["new", "try_sync", "try_future"]),
    ("ServiceResource", &["new"]),
    ("ServiceBody", &["from_body"]),
    ("Cache", &["make_with_options", "make_with_shared_key", "make_with_prepared_index", "make_prepared", "make_prepared_with_refusals", "make_prepared_independent"]),
    ("DependencyCache", &["make", "make_with_shared_key", "make_with_prepared_index"]),
    ("PreparedDependencyCache", &["make", "make_prepared", "make_prepared_with_refusals", "make_prepared_independent"]),
    ("HydrationSet", &["new", "insert"]),
    ("ClusterSingleton", &["run"]),
    ("ClusterSchedule", &["new"]),
    ("PersistedQueue", &["new"]),
    (
        "DurableQueue",
        &["new", "handle", "require_physical_durability"],
    ),
    ("Persistable", &["new", "with_policy"]),
    ("RpcOperation", &["unary", "with_idempotency_key"]),
    ("PreparedRpcOperation", &["new"]),
    ("MemoryClusterMessageStore", &["prepare"]),
    ("MemoryWorkflowEngine", &["prepare"]),
    ("RpcHandler", &["new", "provide_some"]),
    ("RpcStreamOperation", &["new"]),
    ("RpcStreamHandler", &["new", "provide_some"]),
    (
        "RpcProtocol",
        &["from_handler", "new", "register", "with_schema"],
    ),
    (
        "RpcStreamProtocol",
        &["from_handler", "new", "register", "with_schema"],
    ),
    ("RpcServerMiddleware", &["wrap"]),
    ("RpcServiceMiddleware", &["new", "from_body"]),
    ("RpcStreamMiddleware", &["wrap", "serve_stream"]),
    ("EventDefinition", &["new"]),
    ("AggregateEvent", &["new"]),
    ("HandledEvent", &["new", "with_context", "from_bound"]),
    (
        "EventGroup",
        &[
            "from_definition",
            "register_definition",
            "new",
            "register",
            "register_aggregate",
            "register_handled",
        ],
    ),
    ("Projection", &["new"]),
    ("ProjectionCases", &["register", "new", "case", "finish"]),
    ("Atom", &["new"]),
    ("AtomRef", &["new"]),
    ("ReactiveQuery", &["new"]),
    ("EffectStreamSource", &["new"]),
    ("EntityDefinition", &["new", "from_definition"]),
    (
        "EntityDispatcher",
        &[
            "from_handler",
            "from_addressed_handler",
            "register",
            "register_addressed",
        ],
    ),
    (
        "ClusterDefinition",
        &["finish", "new", "register", "register_scoped"],
    ),
    ("TypedWorkflowDefinition", &["new"]),
    ("TypedWorkflowActivity", &["new"]),
    (
        "Codec",
        &[
            "new",
            "recursive",
            "with_context",
            "from_shared_context",
            "with_ast",
            "transform",
            "transform_effectful",
            "parse",
        ],
    ),
    (
        "PureCodec",
        &[
            "new",
            "recursive",
            "with_ast",
            "from_shared_context",
            "transform",
            "parse",
        ],
    ),
    ("ConfiguredCodec", &["parse", "parse_prepared"]),
    ("EffectSchema", &["parse"]),
    (
        "RecordBuilder",
        &[
            "new",
            "field",
            "optional",
            "optional_with_default",
            "finish",
        ],
    ),
    ("PreparedRecordBuilder", &["new", "field", "optional", "optional_with_default", "finish"]),
    ("PreparedEffectRecordBuilder", &["new", "field", "optional", "default_with", "default_with_context", "finish", "finish_in"]),
    ("PreparedGenericRecordBacking", &["new"]),
    (
        "NativeRecordBuilder",
        &["new", "field", "optional", "default_with_context", "finish"],
    ),
    ("TaggedEnumBuilder", &["new", "variant", "finish"]),
    ("PreparedTaggedEnumBuilder", &["new", "variant", "finish"]),
    ("SqlStatement", &["builder", "trusted"]),
    ("PreparedSqlRowsPool", &["prepare"]),
    ("PreparedSqlRowLanes", &["prepare"]),
    ("PreparedSqlStatementPool", &["prepare", "reserve"]),
    ("PreparedSqlSessionPool", &["prepare"]),
    ("PreparedSqlBatchPool", &["prepare", "reserve"]),
    ("PreparedSqlBatchBuilder", &["try_push", "finish"]),
    ("PreparedSqlStatementBuilder", &["text", "identifier", "null", "integer", "real", "text_param", "blob", "finish"]),
    ("Sql", &["execute", "query", "first", "scalar"]),
    ("SqlClient", &[
        "execute", "query", "first", "first_prepared_read", "scalar", "with_session", "open",
        "with_session_pool", "batch",
        "transaction", "transaction_once", "fold_rows", "fold_rows_effect",
        "query_into_rows_atomic", "batch_into_rows_atomic", "query_bytes_into_rows_atomic", "batch_bytes_into_rows_atomic",
    ]),
    ("SqlSession", &["execute", "query", "first", "scalar", "native", "fold_rows", "fold_rows_effect", "batch"]),
    ("SqlTransaction", &[
        "execute", "query", "first", "scalar", "native", "savepoint", "savepoint_once", "batch",
        "fold_rows", "fold_rows_effect", "query_into_rows_atomic", "batch_into_rows_atomic",
        "query_bytes_into_rows_atomic", "batch_bytes_into_rows_atomic",
    ]),
    ("SqlKernel", &[
        "reserve_prepared_session", "reserve_prepared_rows", "with_connection", "transaction",
        "connection_operation", "transaction_operation", "reserve_transaction_operation",
    ]),
    ("NativeSession", &["with_native"]),
    ("NativeTransactionReservation", &["run"]),
    ("SqlMigrator", &["new", "initialize", "up"]),
    (
        "StatementBuilder",
        &[
            "text",
            "param",
            "identifier",
            "ident",
            "fragment",
            "predicate",
            "finish",
        ],
    ),
    ("MutationBuilder", &["new", "value", "update", "insert"]),
    ("RowDecoder", &["from_program", "pure", "pure_with_env"]),
    ("PreparedEndpointPlan", &["from_graph"]),
    ("PreparedInputEndpointPlan", &["from_graph"]),
    (
        "Endpoint",
        &[
            "from_body",
            "from_effect",
            "pipe_through",
            "on_error",
            "preconditions",
        ],
    ),
    (
        "Reply",
        &[
            "empty",
            "text",
            "html",
            "json",
            "json_codec",
            "bytes",
            "lease",
            "frame",
            "stream",
            "representations",
            "redirect",
            "render",
            "problem",
            "validation",
            "sse",
            "negotiate",
            "native",
            "native_owned",
            "file_path",
            "no_content",
        ],
    ),
    (
        "ReplyDraft",
        &[
            "new",
            "header",
            "append_header",
            "status",
            "remove_header",
            "location",
            "created",
            "content_language",
            "cache_control",
            "vary",
            "validators",
            "conditional_get",
            "trailer",
            "set_cookie",
            "delete_cookie",
            "cookie",
            "finish",
            "finish_bodyless",
        ],
    ),
    (
        "Route",
        &[
            "literal",
            "segment",
            "capture_first",
            "constrained_first",
            "optional_first",
            "default_first",
            "catch_all_first",
            "capture",
            "capture_constrained",
            "optional_capture",
            "default_capture",
            "catch_all",
            "name",
            "tags",
            "configure",
        ],
    ),
    (
        "CapturedRoute",
        &[
            "segment",
            "capture",
            "capture_constrained",
            "optional_capture",
            "default_capture",
            "catch_all",
            "name",
            "tags",
            "configure",
        ],
    ),
    (
        "Router",
        &[
            "new",
            "include",
            "group",
            "group_configured",
            "pipe_through",
            "websocket",
            "native",
            "configure",
        ],
    ),
    (
        "Pipeline",
        &[
            "new",
            "before",
            "after",
            "on_exit",
            "guard",
            "respond",
            "set_header",
            "stage",
            "prepared_stage",
            "append",
        ],
    ),
    ("Registration", &["default"]),
    ("Registered", &["new", "pipe_through"]),
    (
        "Representation",
        &["new", "text", "html", "stream", "json_codec", "language"],
    ),
    ("Representations", &["new"]),
    (
        "Server",
        &[
            "new",
            "configure_app",
            "configure_builder",
            "middleware",
            "before_environment",
            "before_routing",
            "after_response",
            "around_request",
            "around_selection",
            "websockets",
            "with_http1_header_timeout",
            "with_overload_reply",
            "with_host",
            "run",
        ],
    ),
    ("RequestBinding", &["new"]),
    ("ServerPolicy", &["new", "on_exit"]),
    ("PreparedServer", &[
        "prepare_with_request_controls", "with_host", "with_http_timeouts", "with_request_queue",
        "with_response_limits", "with_http_limits", "with_http2_reset_limits", "with_body_channels",
        "with_native_bytes_pool", "with_json_writer_pool", "with_native_tasks", "with_request_storage",
        "with_request_byte_pools", "with_request_control_admission", "with_body_completion_admission",
        "with_connection_admission", "with_observer_batch_admission", "with_terminal_admission",
        "with_response_headers_pool", "with_connection_tasks",
    ]),
    ("BodyProgram", &["new", "from_body", "source", "leases", "frames"]),
    ("BodyWriter", &["source"]),
    (
        "IdentityStores",
        &[
            "new",
            "with_accounts",
            "with_sessions",
            "with_keys",
            "with_roles",
            "with_permissions",
            "with_challenges",
            "with_queries",
            "with_audit",
            "with_consent",
            "with_delivery",
            "with_notifications",
            "with_jobs",
            "with_personal_data",
            "with_native_user_queries",
            "with_native_role_queries",
        ],
    ),
    (
        "IdentityPolicies",
        &[
            "new",
            "with_passwords",
            "with_authenticator",
            "with_external",
            "with_passkeys",
            "with_email_confirmation",
            "with_browser",
            "with_renderers",
            "with_validators",
            "with_normalizer",
            "with_principal_factory",
            "with_tokens",
            "with_tickets",
            "with_key_ring",
        ],
    ),
    (
        "IdentityDefinition",
        &[
            "new",
            "authorize_once",
            "invoke",
            "invoke_once",
            "invoke_capability_once",
            "native_http",
            "accounts_page",
            "roles_page",
            "administer_account",
            "administer_role",
            "create_role",
            "rename_role",
            "delete_role",
            "add_role_claim",
            "remove_role_claim",
            "add_account_to_role",
            "remove_account_from_role",
            "mutate_account_snapshot",
            "deliver",
            "notify",
            "request_personal_data_job",
            "authorization_stage",
            "invoke_prepared_once",
            "with_prepared_admission",
            "register",
            "login",
            "account",
            "change_password",
            "remove_password",
            "password_recovery_token",
            "recover_password",
            "complete_authenticator",
            "complete_recovery_code",
            "complete_external_authenticator",
            "complete_external_recovery_code",
            "complete_registry_factor",
            "complete_external_registry_factor",
            "pending_factor_providers",
            "pending_factor_token",
            "reset_authenticator",
            "disable_two_factor",
            "enable_authenticator",
            "regenerate_recovery_codes",
            "begin_passkey_enrollment",
            "enroll_passkey",
            "passkey_login_options",
            "passkey_login",
            "remove_passkey",
            "verify_external",
            "external_login",
            "link_external",
            "unlink_external",
            "email_confirmation_token",
            "confirm_email",
            "set_email",
            "set_phone_number",
            "sign_in_cookie",
            "authenticate_cookie",
            "sign_in_bearer",
            "refresh_bearer",
            "logout_bearer",
            "store_session",
            "authenticate_stored_session",
            "revoke_session",
            "preferences",
            "update_preferences",
            "account_snapshot",
            "delete_account",
            "personal_data_job",
            "cookie_stage",
            "prepared_cookie_stage",
            "selected_bearer_stage",
            "prepared_bearer_stage",
            "retain_pending_browser",
            "resume_pending_browser",
            "issue_antiforgery",
            "selected_antiforgery_stage",
            "logout_cookie",
            "reserve_prepared_invocation",
            "bearer_stage",
            "antiforgery_stage",
        ],
    ),
    ("SqlxIdentityStore", &["new", "from_kernel", "initialize", "with_prepared_invocations"]),
    ("PreparedIdentityInvocations", &["prepare"]),
    ("AtomicAccounts", &["new"]),
    ("SqlRecordAdapter", &["new"]),
    (
        "IdentityEndpoints",
        &[
            "new",
            "group",
            "group_with",
            "workflow_route",
            "definition",
            "include",
            "pipe_through",
            "on_error",
            "finish",
        ],
    ),
    ("EndpointGroup", &["new"]),
    ("Csp", &["new", "directive", "replace", "append"]),
    ("Security", &["new", "validate"]),
    ("ReadThrough", &["scoped", "scoped_prepared"]),
    ("DependentReadThrough", &["scoped", "scoped_by", "scoped_prepared", "scoped_prepared_by"]),
    ("NamedArgument", &["new"]),
    ("Localization", &["new"]),
    ("Probe", &["new", "process_liveness"]),
    ("Availability", &["new", "with_dashboard", "with_prepared_output"]),
    ("PreparedWeb", &["with_host", "run"]),
    (
        "Web",
        &[
            "new",
            "with_memory",
            "security",
            "localization",
            "observe",
            "otel",
            "availability",
            "output_cache",
            "configure_builder",
            "configure_app",
            "prepare",
            "with_host",
            "run",
        ],
    ),
    ("Otel", &["span"]),
];

/// These public representations are constructed by macro lowerings. Struct aliases
/// and field-by-field equivalents resolve to the same compiler definition identities.
const REPRESENTATIONS: &[&str] = &[
    "WorkflowDefinition",
    "WorkflowActivity",
    "RpcProtocolGroup",
    "EventShape",
    "EventCodecs",
    "EventBinaryFormats",
    "SpanOptions",
    "Registration",
    "Security",
];

/// Only named native lowerings own an implementation boundary. Consumer rejection
/// also covers alternate constructors, but those names do not grant permissions
/// to an enclosing helper just because it starts with from_ or is named build.
fn native_owner(krate: &str, owner: &str) -> bool {
    // These are defining module paths, including #[path] module names. A
    // same-named local type in a test/helper module owns no such boundary.
    let owners: &[&str] = match krate {
        "effect" => &[
            "program::Effect",
            "internal::workflow::Workflow",
            "internal::scoped_body::ScopedBody",
            "workflow::workflow_types::WorkflowDefinition",
            "workflow::activity::WorkflowActivity",
            "rpc::service_middleware::RpcProtocolGroup",
            "eventlog::event_builder::EventShape",
            "eventlog::event_builder::EventCodecs",
            "eventlog::event_builder::EventBinaryFormats",
            "transactional::transaction::Transaction",
            "transactional::tx_ref::TxRef",
            "transactional::tx_ref::TransactionDomain",
            "reactivity::atom_program::AtomProgram",
            "workflow::durable::Durable",
            "workflow::compensation::WorkflowCompensation",
            "cluster::entity_resource::EntityResourcePlan",
            "stream::Stream",
            "stream::prepared::PreparedStream",
            "sink::Sink",
            "resource::ResourceBinding",
            "scope::prepared::PreparedScope",
            "scope::prepared::PreparedScopeOwner",
            "internal::runtime::prepared::PreparedRunPlan",
            "internal::runtime::prepared::PreparedRun",
            "internal::runtime::prepared::PreparedRunContext",
            "internal::runtime::prepared::PreparedNativeReservation",
            "internal::runtime::prepared::graph::run::PreparedGraphPlan",
            "internal::runtime::prepared::graph::run::PreparedGraph",
            "internal::runtime::prepared::graph::run::ConstructedGraphPlan",
            "internal::runtime::prepared::graph::run::ConstructedGraph",
            "internal::runtime::prepared::graph::scope_binding::ScopePermit",
            "scope::prepared::pool::PreparedScopePool",
            "scope::prepared::resource::PreparedResourceBinding",
            "scope::prepared::resource::future::PreparedFutureResourceBinding",
            "scope::prepared::PreparedCheckpoint",
            "scope::prepared::PreparedRelease",
            "scope::prepared::PreparedReleasePlan",
            "scope::prepared::resource::PreparedResourcePlan",
            "scope::prepared::resource::future::PreparedFutureResourcePlan",
            "resource::Release",
            "scope::Scope",
            "layer::Layer",
            "semaphore::Semaphore",
            "partitioned_semaphore::PartitionedSemaphore",
            "schedule::Schedule",
            "reference::AtomicRef",
            "synchronized_ref::SynchronizedRef",
            "pool::Pool",
            "weak_cache::WeakCache",
            "request_resolver::RequestResolver",
            "service::ServiceOperation",
            "service::ServiceResource",
            "service::ServiceBody",
            "cache::Cache",
            "dependency_cache::DependencyCache",
            "dependency_cache::prepared::PreparedDependencyCache",
            "reactivity::hydration::HydrationSet",
            "cluster::singleton::ClusterSingleton",
            "cluster::schedule::ClusterSchedule",
            "persistence::persisted_queue::PersistedQueue",
            "workflow::durable_queue::DurableQueue",
            "persistence::persistable::Persistable",
            "rpc::definition::RpcOperation",
            "rpc::prepared_client::PreparedRpcOperation",
            "rpc::definition::RpcHandler",
            "rpc::streaming::RpcStreamOperation",
            "rpc::streaming_server::RpcStreamHandler",
            "rpc::protocol::RpcProtocol",
            "rpc::stream_protocol::RpcStreamProtocol",
            "rpc::middleware::RpcServerMiddleware",
            "rpc::service_middleware::RpcServiceMiddleware",
            "rpc::middleware::RpcStreamMiddleware",
            "eventlog::event_builder::EventDefinition",
            "eventlog::handled_event::AggregateEvent",
            "eventlog::handled_event::HandledEvent",
            "eventlog::event_group::EventGroup",
            "eventlog::projection::Projection",
            "eventlog::projection_cases::ProjectionCases",
            "reactivity::atom::Atom",
            "reactivity::atom_ref::AtomRef",
            "reactivity::query::ReactiveQuery",
            "reactivity::source::EffectStreamSource",
            "cluster::entity_definition::EntityDefinition",
            "cluster::entity::EntityDispatcher",
            "cluster::entity_definition::ClusterDefinition",
            "cluster::memory_store::MemoryClusterMessageStore",
            "workflow::execution::TypedWorkflowDefinition",
            "workflow::memory_workflow_engine::MemoryWorkflowEngine",
            "workflow::activity::TypedWorkflowActivity",
            "schema::codec::Codec",
            "schema::codec::PureCodec",
            "schema::codec::EffectSchema",
            "schema::parse_context::ConfiguredCodec",
            "schema::schema_builder::RecordBuilder",
            "schema::prepared_record::PreparedRecordBuilder",
            "schema::prepared_effect_record::PreparedEffectRecordBuilder",
            "schema::prepared_effect_record::PreparedGenericRecordBacking",
            "schema::native_codec::NativeRecordBuilder",
            "schema::enum_builder::TaggedEnumBuilder",
            "schema::prepared_enum::PreparedTaggedEnumBuilder",
        ],
        "effect_sql" => &[
            "prepared_rows::PreparedSqlRowsPool",
            "prepared_rows::PreparedSqlRowLanes",
            "prepared_statement::PreparedSqlStatementPool",
            "prepared_statement::PreparedSqlStatementBuilder",
            "kernel::PreparedSqlSessionPool",
            "prepared_batch::PreparedSqlBatchPool",
            "prepared_batch::PreparedSqlBatchBuilder",
            "client::Sql",
            "client::SqlClient",
            "client::SqlSession",
            "client::SqlTransaction",
            "kernel::SqlKernel",
            "kernel::NativeSession",
            "kernel::NativeTransactionReservation",
            "migration::SqlMigrator",
            "statement::SqlStatement",
            "statement::StatementBuilder",
            "statement::MutationBuilder",
            "row::RowDecoder",
        ],
        "effect_server" => &[
            "endpoint::Endpoint",
            "prepared_endpoint::PreparedEndpointPlan",
            "prepared_endpoint::PreparedInputEndpointPlan",
            "reply::Reply",
            "reply::ReplyDraft",
            "route::Route",
            "route::Router",
            "endpoint::Pipeline",
            "endpoint::Registration",
            "endpoint::Registered",
            "reply::Representation",
            "reply::Representations",
            "host::Server",
            "host::RequestBinding",
            "host::ServerPolicy",
            "prepared_terminal::PreparedServer",
            "body::BodyProgram",
            "body::BodyWriter",
        ],
        "effect_server_identity" => &[
            "capabilities::IdentityStores",
            "policies::IdentityPolicies",
            "definition::IdentityDefinition",
            "store::SqlxIdentityStore",
            "store::invocation::PreparedIdentityInvocations",
            "store::accounts::AtomicAccounts",
            "store::records::SqlRecordAdapter",
            "endpoints::IdentityEndpoints",
            "endpoints::EndpointGroup",
        ],
        "effect_server_web" => &[
            "csp::Csp",
            "security::Security",
            "cache::ReadThrough",
            "cache::DependentReadThrough",
            "localization::message::NamedArgument",
            "localization::reload::Localization",
            "availability::Probe",
            "availability::Availability",
            "application::Web",
            "application::PreparedWeb",
        ],
        "effect_otel" => &["signals::Otel", "signals::SpanOptions"],
        _ => &[],
    };
    let prefix = format!("{krate}::");
    owners.contains(&owner.trim_prefix(&prefix))
}

pub fn implementation_entry(krate: &str, owner: &str, name: &str) -> bool {
    if !native_owner(krate, owner) {
        return false;
    }
    let Some(owner) = owner.rsplit("::").next() else {
        return false;
    };
    if owner == "Effect" {
        EFFECT.contains(&name)
    } else {
        OWNERS
            .iter()
            .any(|(ty, methods)| *ty == owner && methods.contains(&name))
    }
}

/// Name the public DSL form for a resolved lowering, retaining useful mode hints.
pub fn intended_dsl(owner: Option<&str>, method: &str) -> String {
    let owner = owner.and_then(|owner| owner.rsplit("::").next());
    if owner == Some("Effect") {
        let form = match method {
            "fail" | "fail_with" => "effect_fail",
            "from_result" | "from_result_once" => "effect_from_result",
            "sync" | "sync_once" => "effect_sync",
            "try_sync" | "try_sync_once" => "effect_try_sync",
            "from_future"
            | "from_prepared_future"
            | "from_future_once"
            | "from_future_with_env"
            | "from_exit_future_with_context"
            | "from_prepared_exit_future_with_context" => "effect_future",
            "try_future" | "try_future_once" | "try_future_with_env" => "effect_try_future",
            "map" => "effect_map",
            "map_error" => "effect_map_error",
            "provide_some" | "provide_some_arc" => "effect_provide_some",
            "provide" => "effect_provided",
            "sandbox" => "effect_sandbox",
            "catch_all" => "effect_recover",
            "timeout" => "effect_timeout",
            "race" => "effect_race",
            "retry" => "effect_retry",
            "repeat" => "effect_repeat",
            "service" => "service",
            "with_span" | "annotate" => "span",
            "ensuring" | "ensuring_with" => "effect_resource",
            _ => "effect",
        };
        return format!("{form}!");
    }
    match owner {
        Some("Sql" | "SqlClient" | "SqlSession" | "SqlTransaction" | "SqlKernel"
            | "NativeSession" | "NativeTransactionReservation" | "SqlMigrator") => return "sql!".into(),
        Some("Server" | "PreparedServer" | "ServerPolicy" | "RequestBinding") => return "server!".into(),
        Some("SpanOptions" | "Otel") => return "otel!".into(),
        Some("Codec" | "PureCodec" | "LazyLock") => return "codec!".into(),
        Some("Release") | None => {
            return "effect_resource!, effect_scope_acquire! or effect_finalizer!".into();
        }
        Some("Scope") => {
            return match method {
                "acquire" => "effect_scope_acquire!",
                "add_release" | "add_finalizer" => "effect_finalizer!",
                _ => "effect_scope!",
            }
            .into();
        }
        Some("PreparedStream") if matches!(method, "collect" | "fold") => return "sink!".into(),
        _ => {}
    }
    MACRO_OWNERS
        .iter()
        .find(|(_, owners)| owner.is_some_and(|owner| owners.contains(&owner)))
        .map_or_else(
            || "the owning DSL macro".into(),
            |(name, _)| format!("{name}!"),
        )
}

/// Match canonical defining owners, never caller spelling or imports.
pub fn protected(krate: &str, owner: Option<&str>, name: &str, kind: DefKind, path: &str) -> bool {
    if !matches!(
        krate,
        "effect"
            | "effect_sql"
            | "effect_server"
            | "effect_server_identity"
            | "effect_server_web"
            | "effect_otel"
    ) {
        return false;
    }
    if owner.is_some_and(|owner| !native_owner(krate, owner)) {
        return false;
    }
    let owner = owner.and_then(|owner| owner.rsplit("::").next());
    if owner == Some("Effect") {
        return EFFECT.contains(&name);
    }
    if owner.is_some_and(|ty| REPRESENTATIONS.contains(&ty)) && name == "default" {
        return true;
    }
    if path.contains("::__private::") {
        return matches!(kind, DefKind::Fn | DefKind::AssocFn);
    }
    if let Some(owner) = owner {
        // These public owners reserve only their explicitly listed lowering
        // entry points for the DSL. Their other methods remain callable.
        if matches!(
            owner,
            "Sql" | "SqlClient" | "SqlSession" | "SqlTransaction" | "SqlKernel"
                | "NativeSession" | "NativeTransactionReservation" | "SqlMigrator"
                | "PreparedInputEndpointPlan"
                | "PreparedResourcePlan" | "PreparedFutureResourcePlan" | "PreparedReleasePlan"
                | "PreparedStream" | "TransactionDomain" | "PreparedRpcOperation"
                | "MemoryClusterMessageStore" | "MemoryWorkflowEngine"
        ) {
            return OWNERS
                .iter()
                .any(|(ty, methods)| *ty == owner && methods.contains(&name));
        }
        if owner == "ScopedBody" {
            return matches!(name, "from_body" | "from_effect");
        }
        // State macros compose transitions on an existing reference. Initializing
        // an ordinary reference value does not construct those Effect descriptions.
        if matches!(owner, "AtomicRef" | "SynchronizedRef") {
            return OWNERS
                .iter()
                .any(|(ty, methods)| *ty == owner && methods.contains(&name));
        }
        return OWNERS.iter().any(|(ty, methods)| {
            *ty == owner
                && (methods.contains(&name)
                    || matches!(name, "new" | "default" | "build")
                    || name.starts_with("from_"))
        }) || matches!(owner, "IdentityStores" | "IdentityPolicies")
            && name.starts_with("with_");
    }
    if matches!(kind, DefKind::Struct | DefKind::Union) && REPRESENTATIONS.contains(&name) {
        return native_owner(krate, path);
    }
    match krate {
        "effect" => {
            matches!(kind, DefKind::Fn | DefKind::AssocFn)
                && (path.starts_with("effect::internal::workflow::")
                    || path.starts_with("effect::internal::macro_future::"))
                || matches!(
                    path,
                    "effect::internal::runtime::prepared::graph::success"
                        | "effect::internal::runtime::prepared::graph::ready"
                        | "effect::internal::runtime::prepared::graph::sync"
                        | "effect::internal::runtime::prepared::graph::try_sync"
                        | "effect::internal::runtime::prepared::graph::outcome::either"
                        | "effect::internal::runtime::prepared::graph::outcome::capture"
                        | "effect::internal::runtime::prepared::graph::outcome::map_error"
                        | "effect::internal::runtime::prepared::graph::looping::while_loop"
                        | "effect::internal::runtime::prepared::graph::future::native_future"
                        | "effect::internal::runtime::prepared::graph::provider_result::provider_result"
                        | "effect::internal::runtime::prepared::graph::import::import"
                        | "effect::internal::runtime::prepared::graph::import::environment::import_with_environment"
                        | "effect::internal::runtime::prepared::graph::scheduled::scheduled"
                        | "effect::internal::runtime::prepared::graph::scoped::scope"
                        | "effect::internal::runtime::prepared::graph::scoped::scope_construct"
                        | "effect::internal::runtime::prepared::graph::fork::race::race"
                        | "effect::internal::runtime::prepared::graph::fork::race::race_on"
                        | "effect::internal::runtime::prepared::graph::fork::PreparedFiber::join"
                        | "effect::internal::runtime::prepared::graph::service"
                        | "effect::internal::runtime::prepared::graph::resource::resource"
                        | "effect::internal::runtime::prepared::graph::layer::build"
                        | "effect::internal::runtime::prepared::graph::layer::construct::construct"
                        | "effect::internal::runtime::prepared::graph::layer::construct::try_construct"
                        | "effect::internal::runtime::prepared::graph::scope_binding::recipe"
                        | "effect::internal::runtime::prepared::graph::scope_binding::finalizer_recipe"
                        | "effect::internal::runtime::prepared::graph::scope_binding::construction"
                        | "effect::internal::runtime::prepared::graph::layer::with_layer"
                        | "effect::internal::runtime::prepared::graph::layer::scoped::scoped"
                        | "effect::internal::runtime::prepared::graph::layer::composition::merge"
                        | "effect::internal::runtime::prepared::graph::layer::composition::provide"
                        | "effect::internal::runtime::prepared::graph::environment::provide_some"
                        | "effect::internal::runtime::prepared::graph::bind"
                        | "effect::internal::runtime::prepared::graph::control::defer"
                        | "effect::internal::runtime::prepared::graph::control::branch_left"
                        | "effect::internal::runtime::prepared::graph::control::branch_right"
                        | "effect::workflow::durable::activity"
                        | "effect::websocket::with_connection"
                        | "effect::websocket::with_connection_map_error"
                        | "effect::websocket::with_socket"
                        | "effect::websocket::with_socket_map_error"
                )
        }
        "effect_sql" => matches!(
            path,
            "effect_sql::prepared_rows::prepare_native_rows"
                | "effect_sql::client::sql_with"
                | "effect_sql::row::column"
                | "effect_sql::row::native_column"
                | "effect_sql::row::optional_column"
                | "effect_sql::row::refine"
                | "effect_sql::row::validate"
                | "effect_sql::row::choice_text"
        ),
        "effect_server" => path == "effect_server::route::mount_route",
        "effect_server_identity" => matches!(
            path,
            "effect_server_identity::definition::definition_with_factory"
                | "effect_server_identity::definition::error_mapper"
                | "effect_server_identity::endpoints::authorized_guard"
        ),
        _ => false,
    }
}

/// Exact export-to-native-rule reconciliation, shared across every mode and role form.
pub const MACRO_OWNERS: &[(&str, &[&str])] = &[
    (
        "memory",
        &[
            "MemoryConfiguration",
            "HasMemory",
            "PreparedSchemaMemory",
            "PreparedIssueMemory",
            "PreparedJsonArrayPool",
            "PreparedArrayOperationPool",
            "PreparedMapOperationPool",
            "PreparedSequenceOperationPool",
            "PreparedObjectOperationPool",
            "PreparedRecordOperationPool",
            "PreparedBinaryUnionPool",
            "PreparedNumericScratchPool",
            "PreparedMemoryConfigSource",
            "PreparedConfig",
            "PreparedEffectConfig",
            "PreparedConfigStoreSource",
        ],
    ),
    ("memory_lease", &["Effect", "HasMemoryPool", "SlotPool"]),
    ("memory_vec", &["RegionAllocator"]),
    ("memory_write", &["Text", "SlotLease"]),
    ("effect_drop_guard", &["DropGuard"]),
    ("effect", &["Effect", "Workflow", "@workflow"]),
    ("schema", &["RecordBuilder", "NativeRecordBuilder", "PreparedRecordBuilder", "PreparedEffectRecordBuilder", "PreparedGenericRecordBacking", "PreparedTaggedEnumBuilder", "PureCodec", "PreparedJsonArrayCodec", "PreparedJsonMapCodec", "PreparedSequenceCodec", "PreparedObjectCodec", "PreparedBinaryCodec"]),
    ("schema_enum", &["TaggedEnumBuilder", "PureCodec"]),
    ("persistable", &["Persistable"]),
    ("transaction", &["Transaction", "TxRef", "TransactionDomain"]),
    ("event", &["EventDefinition", "AggregateEvent"]),
    ("event_group", &["EventGroup"]),
    ("projection", &["Projection", "ProjectionCases"]),
    (
        "workflow",
        &[
            "Durable",
            "TypedWorkflowDefinition",
            "WorkflowDefinition",
            "WorkflowCompensation",
            "MemoryWorkflowEngine",
        ],
    ),
    (
        "activity",
        &[
            "TypedWorkflowActivity",
            "WorkflowActivity",
            "@durable_activity",
        ],
    ),
    ("workflow_definition", &["WorkflowDefinition"]),
    ("activity_definition", &["WorkflowActivity"]),
    ("rpc_operation", &["RpcOperation", "RpcHandler", "PreparedRpcOperation"]),
    (
        "rpc_stream_operation",
        &["RpcStreamOperation", "RpcStreamHandler"],
    ),
    (
        "rpc_protocol",
        &["RpcProtocol", "RpcStreamProtocol", "RpcProtocolGroup"],
    ),
    ("rpc_stream_protocol", &["RpcStreamProtocol"]),
    (
        "rpc_middleware",
        &["RpcServerMiddleware", "RpcServiceMiddleware"],
    ),
    ("rpc_stream_middleware", &["RpcStreamMiddleware"]),
    ("entity", &["EntityDefinition", "EntityDispatcher"]),
    ("reactive_source", &["AtomRef"]),
    ("reactive_derived", &["Atom", "AtomProgram"]),
    ("reactive_query", &["ReactiveQuery"]),
    ("reactive_bridge", &["EffectStreamSource"]),
    ("span", &["Effect"]),
    (
        "service",
        &[
            "Effect",
            "ServiceOperation",
            "ServiceResource",
            "ServiceBody",
        ],
    ),
    ("cache", &["Cache"]),
    ("cms", &["Service", "ResourcePageRenderer", "@new_definition_service_set", "@prepared_cms_search_stream", "@prepared_resource_search_stream"]),
    ("dependency_cache", &["DependencyCache", "PreparedDependencyCache"]),
    ("atom", &["AtomProgram"]),
    ("hydration", &["HydrationSet"]),
    ("entity_resource", &["EntityResourcePlan"]),
    ("singleton", &["ClusterSingleton"]),
    ("cluster_schedule", &["ClusterSchedule"]),
    ("persisted_queue", &["PersistedQueue"]),
    ("durable_queue", &["DurableQueue"]),
    ("stream", &["Stream", "PreparedStream"]),
    ("handled_event", &["HandledEvent", "AggregateEvent"]),
    ("cluster", &["ClusterDefinition", "MemoryClusterMessageStore"]),
    ("sql", &["@prepare_native_rows", "SqlStatement", "StatementBuilder", "PreparedSqlRowsPool", "PreparedSqlRowLanes", "PreparedSqlStatementPool", "PreparedSqlStatementBuilder", "PreparedSqlSessionPool", "PreparedSqlBatchPool", "PreparedSqlBatchBuilder", "Sql", "SqlClient", "SqlSession", "SqlTransaction", "SqlKernel", "NativeSession", "NativeTransactionReservation", "SqlMigrator"]),
    ("sql_insert", &["MutationBuilder"]),
    ("sql_update", &["MutationBuilder"]),
    ("row", &["RowDecoder", "@row"]),
    ("sql_with", &["@sql_with", "Effect"]),
    ("server", &["Server", "PreparedServer", "ServerPolicy", "RequestBinding"]),
    ("endpoint", &["Endpoint", "PreparedEndpointPlan", "PreparedInputEndpointPlan", "PreparedGraphPlan"]),
    ("registered", &["Registered", "Registration"]),
    ("reply", &["Reply", "ReplyDraft"]),
    ("representations", &["Representation", "Representations"]),
    ("route", &["Route"]),
    ("router", &["Router", "@mount_route"]),
    ("routes", &["Router", "@mount_route"]),
    ("pipeline", &["Pipeline"]),
    (
        "server_stream",
        &["BodyProgram", "BodyWriter", "@server_stream"],
    ),
    ("identity_store", &["IdentityStores", "SqlxIdentityStore", "PreparedIdentityInvocations", "AtomicAccounts", "SqlRecordAdapter"]),
    (
        "identity",
        &["IdentityDefinition", "IdentityPolicies", "@identity"],
    ),
    (
        "identity_endpoints",
        &[
            "IdentityEndpoints",
            "EndpointGroup",
            "Router",
            "Pipeline",
            "@identity",
        ],
    ),
    ("web", &["Web", "PreparedWeb"]),
    ("security", &["Security"]),
    ("csp", &["Csp"]),
    ("read_through", &["ReadThrough"]),
    ("dependent_read_through", &["DependentReadThrough"]),
    ("availability", &["Availability", "Probe"]),
    ("message_arguments", &["NamedArgument"]),
    ("localization", &["Localization"]),
    ("otel", &["Otel", "SpanOptions", "Effect"]),
    ("effect_sync", &["Effect"]),
    ("effect_try_sync", &["Effect"]),
    ("effect_future", &["Effect", "PreparedQueue", "PreparedPubSub", "PreparedPublication", "PreparedSubscription", "PreparedSubscriptionRef", "PreparedExitMapper"]),
    ("effect_die", &["Effect"]),
    ("effect_fail", &["Effect"]),
    ("effect_try_future", &["Effect"]),
    ("effect_cpu", &["Effect"]),
    ("effect_try_cpu", &["Effect"]),
    ("effect_blocking", &["Effect"]),
    ("effect_try_blocking", &["Effect"]),
    ("effect_map", &["Effect"]),
    ("effect_map_error", &["Effect"]),
    ("effect_provide_some", &["Effect"]),
    ("effect_sandbox", &["Effect"]),
    ("effect_from_option", &["Effect"]),
    ("effect_from_result", &["Effect"]),
    ("effect_timeout", &["Effect"]),
    ("effect_recover", &["Effect"]),
    ("effect_for_each", &["Effect"]),
    ("effect_for_each_concurrent", &["Effect"]),
    ("effect_provided", &["Effect"]),
    ("effect_race", &["Effect"]),
    ("effect_uninterruptible", &["Effect"]),
    ("effect_on_interrupt", &["Effect"]),
    ("effect_all", &["Effect"]),
    ("effect_all_concurrent", &["Effect"]),
    ("effect_retry", &["Effect"]),
    ("effect_repeat", &["Effect"]),
    ("effect_resource", &["ResourceBinding", "Release", "PreparedResourcePlan", "PreparedFutureResourcePlan"]),
    ("effect_using", &["ResourceBinding", "Release"]),
    (
        "effect_prepare",
        &[
            "PreparedRunPlan",
            "PreparedRun",
            "PreparedRunContext",
            "PreparedNativeReservation",
            "PreparedGraphPlan",
            "PreparedGraph",
            "ConstructedGraphPlan",
            "ConstructedGraph",
            "Ready",
        ],
    ),
    (
        "effect_scope",
        &[
            "Scope",
            "PreparedScope",
            "PreparedScopeOwner",
            "PreparedScopePool",
            "PreparedCheckpoint",
        ],
    ),
    (
        "effect_scope_acquire",
        &[
            "ScopePermit",
            "Scope",
            "PreparedResourceBinding",
            "PreparedFutureResourceBinding",
        ],
    ),
    ("effect_finalizer", &["Scope", "PreparedRelease", "PreparedReleasePlan"]),
    ("layer", &["Layer"]),
    ("effect_with_layer", &["Layer"]),
    (
        "effect_permit",
        &[
            "Semaphore",
            "PartitionedSemaphore",
            "ResourceBinding",
            "Release",
        ],
    ),
    ("schedule", &["Schedule"]),
    ("effect_ref", &["AtomicRef", "SynchronizedRef", "PreparedAtomicRef", "PreparedSynchronizedRef", "PreparedSynchronizedWrite", "PreparedSubscriptionRef"]),
    ("pool", &["Pool"]),
    ("weak_cache", &["WeakCache", "PartitionedWeakCache"]),
    ("request_resolver", &["RequestResolver"]),
    ("sink", &["Sink", "PreparedStream"]),
    ("effect_stream_for_each", &["Stream"]),
    (
        "effect_socket",
        &["@websocket", "ResourceBinding", "Release"],
    ),
    ("codec", &["Codec", "PureCodec", "@lazy_codec"]),
    (
        "effect_schema_parse",
        &["Codec", "PureCodec", "ConfiguredCodec", "EffectSchema"],
    ),
];
