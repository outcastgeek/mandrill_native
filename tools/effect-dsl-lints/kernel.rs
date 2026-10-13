//! Native execution-machine entry points that the DSL cannot implement recursively.
//! A permission identifies the callee; it never admits every call in an enclosing function.

/// Empty native builders implement std Default by delegating to their own new.
pub fn default_constructor(krate: &str, owner: &str) -> bool {
    matches!(
        (krate, owner),
        (
            "effect",
            "schema::native_codec::NativeRecordBuilder"
                | "schema::schema_builder::RecordBuilder"
                | "eventlog::projection_cases::ProjectionCases"
        ) | (
            "effect_server",
            "endpoint::Pipeline" | "reply::ReplyDraft" | "route::Router"
        ) | ("effect_server_identity", "policies::IdentityPolicies")
            | ("effect_server_web", "security::Security" | "csp::Csp")
    )
}

/// Audited native methods build Step::Suspended and forward full Exit channels through
/// runtime::finish. These exact methods need the private three-input constructor.
/// Other callers and other callees retain normal DSL enforcement.
pub fn machine(
    caller_owner: &str,
    caller: &str,
    caller_path: &str,
    callee_owner: Option<&str>,
    callee: &str,
) -> bool {
    callee_owner == Some("program::Effect")
        && callee == "new"
        && match caller_owner {
            "cache::Cache" => matches!(
                caller,
                "get_observed"
            // Inspection and shared acquisition select entries with the private
            // cancellation Context and propagate all producer Exit variants.
            | "try_get" | "get_shared"
            // Retirement and replacement settle owned generations, join cleanup
            // failures, and finish the interpreter Step. Their ordinary input and
            // uninterruptible composition are separately checked and use the DSL.
            | "invalidate_matching" | "set"
            // Shutdown joins the native driver before finishing the interpreter.
            | "shutdown"
            ),
            // Waiting transfers full owned/shared Exit variants under the private
            // cancellation Context rather than turning interruptions into data/errors.
            "deferred::Deferred" => matches!(caller, "await_value" | "await_shared"),
            // These modes consume or share a child outcome and use the private finish
            // protocol, including interruption and observation ownership defects.
            "fiber::Fiber" => matches!(caller, "join" | "owned_observation" | "shared_observation"),
            // Poll the interpreter cancellation mask before finishing a Step.
            "latch::Latch" => caller == "await_open",
            // Forward the source's full outcome under the caller's Context;
            // schema/default conversion panics stay defects and cleanup failures
            // retain their typed source mapping.
            "config::Config" => caller == "load",
            // Journal commit/retry uses the private cancellation Context and
            // maps cancellation to interruption before finishing the Step.
            "transactional::transaction::Transaction" => caller == "atomically",
            // Close/transfer the native scope owner, settling abandonment and
            // forwarding protected full cleanup outcomes through Finish.
            "scope::Scope" => {
                matches!(caller, "close" | "own_mapped" | "close_owned_with_transfer")
                    // Fiber admission registers native ownership before invoking
                    // R's provider, forwarding its private child Context/Finish.
                    || caller == "fork" && caller_path == "fiber::<impl scope::Scope<E>>::fork"
            }
            // The private buffer producer and shared pull reader use cancellation,
            // protected producer observation and complete terminal outcomes.
            "stream::internal::buffer::Buffered" => matches!(caller, "producer" | "reader"),
            // Acquisition joins late provider completion and registers the owned
            // lease with protected finalization before publishing success.
            "pool::Pool" => matches!(caller, "acquire" | "ensure_size" | "shutdown"),
            // Lease completion joins protected settlement using the original Context.
            "pool::PoolLease" => caller == "end_effect",
            // Native keyed admission forwards the selected semaphore's Context.
            "partitioned_semaphore::PartitionedSemaphore" => caller == "acquire_candidate",
            // Queue/publication admission and withdrawal distinguish interruption
            // from typed closure errors and preserve committed owned values.
            "queue::AsyncQueue" => matches!(caller, "offer_candidate" | "take"),
            "pub_sub::PubSub" => caller == "publish_candidate",
            "pub_sub::Subscription" => caller == "take",
            // Resolver settlement retains every full batch/request outcome.
            "request_resolver::RequestResolver" => matches!(caller, "resolve" | "resolve_many"),
            // Scoped acquisition registers raw ownership before cancellation.
            "resource::ResourceBinding" => caller == "acquire_scoped",
            // The pull recipe owns its initial snapshot and native cancellation
            // checkpoint before transferring to the existing subscription reader.
            "subscription_ref::SubscriptionRef" => caller == "stream",
            // Socket shutdown joins and accounts for timeout/abort settlement.
            "websocket::WebSocketConnection" => caller == "close",
            // The reusable scoped body invokes its context-aware future under
            // the existing owner, preserving full Exit and caught-panic channels.
            "internal::scoped_body::ScopedBody" => caller == "from_body",
            // The clock maps overflow/interruption to distinct Exit channels and
            // records the native tick in the interpreter's private Context.
            "program::Effect" => {
                // Native private leaf/continuation recipes forward full Exit via
                // Finish, including consumed-owned-outcome and caught panic paths.
                matches!(
                    caller,
                    "ready" | "future_exit" | "bind_exit" | "from_exit_once"
                ) || caller == "sleep"
                    && caller_path == "clock::<impl program::Effect<(), E, R>>::sleep"
                    // Keep the existing provider's cancellation/trace Context.
                    || caller == "wall_now" && caller_path == "duration::<impl program::Effect<duration::UtcMillis, E, R>>::wall_now"
                    // Publish late transport success to its resource owner and
                    // retain the private Context/full outcomes across I/O.
                    || caller == "http_request" && caller_path == "http::<impl program::Effect<http::HttpResponse, E, R>>::http_request"
                    // Select/restore native trace and poll Context; these methods
                    // preserve the existing continuation rather than bind success.
                    || matches!(caller, "with_trace_context" | "with_poll_instrumentation")
                        && caller_path == format!("native_context::<impl program::Effect<A, E, R>>::{caller}")
            }
            _ => false,
        }
}

/// The event group's alternate public constructor is the macro's native checked
/// registration bridge. Only its call to the same owner's register is recursive.
pub fn inherent_transition(
    owner: &str,
    caller: &str,
    target: Option<&str>,
    callee: &str,
    callee_path: &str,
) -> bool {
    (owner == "eventlog::event_group::EventGroup"
        && caller == "from_event"
        && target == Some(owner)
        && callee == "register")
        // Full-outcome adapters must retain defects, interruption and cleanup
        // channels. A success-only DSL bind cannot implement these transitions.
        || owner == "program::Effect"
            && matches!(
                (caller, callee_path),
                ("either" | "materialize_exit", "internal::workflow::success")
                | ("either", "internal::workflow::Workflow::from_exit")
            )
        // Protected release leaves consume their future once under the native
        // cancellation Context and keep full cleanup outcome families.
        || owner == "resource::Release"
            && matches!(caller, "try_future" | "from_exit_future")
            && callee_path == "internal::workflow::Workflow::new"
        // Native async binding/cleanup evaluates under this existing Context;
        // rebuilding it through effect! would replace its lexical scope owner.
        || owner == "internal::macro_future::MacroContext"
            && matches!((caller, callee_path),
                ("bind", "internal::macro_future::AsyncBindable::into_scoped_effect")
                | ("release", "internal::macro_future::evaluate" | "internal::macro_future::release_work"))
        // Journal leaves retain private retry/conflict/invalid decisions and
        // branch rollback. These exact native edges never admit helper calls.
        || owner == "transactional::transaction::Transaction"
            && matches!(caller, "fail_with" | "or_else" | "for_each")
            && callee_path == "transactional::transaction::Transaction::new"
        || owner == "transactional::tx_ref::TxRef"
            && matches!(caller, "read" | "write")
            && callee_path == "transactional::transaction::Transaction::new"
        // Native read recipes install observations in the supplied context.
        || owner == "reactivity::atom_program::AtomProgram"
            && matches!(caller, "read" | "read_derived")
            && callee_path == "reactivity::atom_program::AtomProgram::new"
        // These existing tracked adapters preserve the supplied tracking
        // factory. Lowering their outer boundary via atom! would add a new
        // async-workflow scope and cleanup boundary to every evaluation.
        || owner == "reactivity::atom_program::AtomProgram"
            && matches!((caller, callee_path),
                ("from_effect" | "defer" | "map" | "flat_map", "reactivity::atom_program::AtomProgram::new")
                | ("succeed_with", "reactivity::atom_program::AtomProgram::from_effect"))
        // Transfer into the private collector continuation without adding a
        // second bind/cancellation checkpoint around its complete result.
        || owner == "stream::Stream"
            && caller == "run_collect"
            && matches!(callee_path, "internal::workflow::reusable" | "internal::workflow::bind")
        // Native descriptor factories retain the externally supplied activation
        // scope; entity_resource!'s async body would introduce another boundary.
        || owner == "cluster::entity_resource::EntityResourcePlan"
            && matches!((caller, callee_path),
                ("from_effect" | "defer" | "map" | "flat_map" | "map_error", "cluster::entity_resource::EntityResourcePlan::new")
                | ("succeed_with", "cluster::entity_resource::EntityResourcePlan::from_effect"))
        || owner == "cluster::entity_resource::EntityResource"
            && caller == "plan"
            && callee_path == "cluster::entity_resource::EntityResourcePlan::new"
        // The private resource continuation installs ownership before publishing
        // acquisition, then combines all protected cleanup outcome channels.
        || owner == "resource::ResourceBinding" && caller == "bind_run"
            && callee_path == "internal::workflow::Workflow::new"
        // Creation is delayed by the enclosing native leaf. These constructors
        // initialize their native synchronization state, which effect_ref! and
        // effect_permit! consume rather than construct recursively.
        || target == Some(owner) && callee == "new"
            && matches!((owner, caller),
                ("reference::AtomicRef" | "synchronized_ref::SynchronizedRef", "make_with")
                | ("semaphore::Semaphore" | "partitioned_semaphore::PartitionedSemaphore", "make"))
        // Journal retry is a native control decision, not a callback constructor.
        || owner == "transactional::transaction::Transaction"
            && caller == "retry" && target == Some(owner) && callee == "new"
        // Preserve native scheduling decisions and the existing recursive state
        // factory. schedule! delegates its Result transition to unfold instead.
        || owner == "schedule::Schedule" && target == Some(owner) && callee == "new"
            && matches!(caller, "unfold_shared" | "up_to" | "jitter_validated" | "intersect")
        // Select a keyed native permit and execute under the supplied Context.
        || owner == "partitioned_semaphore::PartitionedSemaphore" && caller == "acquire_candidate"
            && callee_path == "semaphore::Semaphore::acquire"
        // Subscription recipes open under the stream's supplied owning scope.
        // Rebuilding their outer boundary as an emitting body would add a scope.
        || matches!(owner, "pub_sub::PubSub" | "subscription_ref::SubscriptionRef" | "reactivity::query::ReactiveQuery")
            && caller == "stream" && callee_path == "stream::Stream::new"
        // Durable's private lift connects reviewed replay operations to their
        // existing descriptions; it cannot be a general workflow! Effect bind.
        || owner == "workflow::durable::Durable" && callee_path == "workflow::durable::Durable::from_effect"
            && matches!(caller, "defer" | "succeed_with" | "succeed_once" | "map" | "flat_map" | "activity" | "signal" | "deadline" | "parallel" | "parallel_retry" | "race")
        // Child cleanup owners preserve mapped finalizer errors and the current
        // stream/service owner instead of installing a generic scoped body.
        || matches!((owner, caller, callee_path),
            ("cluster::entity_resource::EntityResourcePlan", "map_error", "scope::Scope::new")
            | ("rpc::service_middleware::RpcServiceMiddleware", "apply", "scope::Scope::new")
            | ("stream::composition::StreamContext", "append", "scope::Scope::new"))
        // Reuse the existing emitting context, retaining nested pull ownership.
        || owner == "stream::Stream" && caller == "flat_map"
            && callee_path == "stream::Stream::from_body"
        // Activity/child leaves retain recorded execution before the private lift.
        || matches!((owner, caller),
            ("workflow::activity::TypedWorkflowActivity", "call")
            | ("workflow::durable_child::DurableChild", "wait" | "run"))
            && callee_path == "workflow::durable::Durable::from_effect"
        // Existing operation wrappers retain supplied codecs, idempotency and
        // address compatibility rather than reconstructing declarations.
        || matches!((owner, caller, callee_path),
            ("rpc::service_middleware::RpcServiceMiddleware", "wrap_handler", "rpc::definition::RpcHandler::new")
            | ("eventlog::event_log_message::EventLogRpc", "handler" | "export_handler", "rpc::definition::RpcHandler::new")
            | ("cluster::cluster_workflow_engine::ClusterWorkflowEngine", "handler", "rpc::definition::RpcHandler::new")
            | ("cluster::cluster_workflow_engine::ClusterWorkflowEngine", "register", "cluster::entity::EntityDispatcher::register_addressed")
            | ("cluster::cluster_workflow_engine::ClusterWorkflowEngine", "dispatcher", "cluster::entity::EntityDispatcher::from_addressed_handler"))
        // Activation first assembles the checked definition/catalog, then fences.
        || matches!(owner, "cluster::entity_definition::EntityDefinition" | "cluster::entity_definition::ClusterDefinition")
            && caller == "activate" && target == Some(owner) && callee == "build"
        // These native admission operations transfer the raw guard into their
        // existing lifetime/Drop owner. effect_permit!'s protected release_now
        // boundary would change release accounting and must not replace it.
        || callee_path == "semaphore::Semaphore::acquire"
            && matches!((owner, caller),
                ("persistence::internal::cache_gate::CacheGate", "acquire")
                | ("reactivity::atom::Atom", "refresh" | "dispose")
                | ("reactivity::atom_cache::AtomCache", "refresh")
                | ("reactivity::atom_ref::AtomRef", "set" | "dispose" | "export" | "import")
                | ("reactivity::atom_ref::AtomReadContext", "read")
                | ("reactivity::invalidation::InvalidationRegistry", "invalidate")
                | ("rpc::client::RpcClient", "call")
                | ("semaphore::Semaphore", "acquire_scoped"))
        // These original one-shot release leaves execute directly under the
        // protected cleanup continuation. An Effect wrapper adds an operation
        // and instrumentation boundary, even if its value and type are equal.
        || callee_path == "resource::Release::sync"
            && matches!((owner, caller),
                ("pub_sub::PubSub" | "subscription_ref::SubscriptionRef" | "reactivity::invalidation::InvalidationRegistry", "subscribe_scoped")
                | ("reactivity::atom::Atom", "observe")
                | ("reactivity::query::ReactiveQuery", "stream"))
        || owner == "semaphore::Semaphore" && caller == "acquire_scoped"
            && callee_path == "resource::Release::try_sync"
        // These transactional subscriptions preserve their native pull recipe
        // and direct one-shot disposal leaf without a deferred Effect wrapper.
        || matches!(owner, "transactional::tx_pub_sub::TxPubSub" | "transactional::tx_subscription_ref::TxSubscriptionRef")
            && caller == "stream"
            && matches!(callee_path, "stream::Stream::from_pull" | "resource::Release::sync")
        // Native cache gate ownership has a direct Drop-only cleanup leaf.
        || owner == "persistence::persisted_cache::PersistedCache"
            && matches!(caller, "get" | "invalidate") && callee_path == "resource::Release::sync"
        // Preserve this native query's direct Effect factory. reactive_query!'s
        // inline evaluator installs a one-shot body continuation instead.
        || owner == "reactivity::atom_rpc::RpcQueryAtom" && caller == "new"
            && callee_path == "reactivity::query::ReactiveQuery::new"
        // Close the cache index with its direct one-shot leaf after the owning
        // scope drains producers; do not add an Effect operation to cleanup.
        || owner == "weak_cache::WeakCache" && caller == "make"
            && callee_path == "resource::Release::sync"
        // Terminal execution tags the existing durable result without nesting
        // another durable async body/scope. Recovery keeps its direct no-op leaf
        // and already effectful cleanup, preserving primary-aware close behavior.
        || owner == "workflow::workflow_types::WorkflowDefinition"
            && matches!((caller, callee_path),
                ("execute", "workflow::durable::Durable::map")
                | ("execute_resumable", "resource::Release::from_effect" | "resource::Release::sync"))
        // Primary-aware streaming cleanup retains its direct no-op leaf and
        // the already effectful close branch, preserving cleanup instrumentation.
        || owner == "rpc::streaming::RpcStreamReader" && matches!(caller, "next" | "acknowledge")
            && matches!(callee_path, "resource::Release::sync" | "resource::Release::from_effect")
}

/// Source-reviewed private bridges retain interpreter Context/Finish and sealed
/// dispatch. Each edge admits one resolved callee, not every call in its caller.
pub fn transition(caller: &str, callee: &str) -> bool {
    matches!(
        (caller, callee),
        ("internal::workflow::bind", "internal::workflow::Bindable::bind_workflow")
        | ("internal::workflow::bind_effect", "internal::workflow::bind_operation")
        | ("internal::workflow::defer_callback2", "internal::workflow::defer_callback1")
        | ("internal::workflow::success", "internal::workflow::Workflow::from_exit")
        | ("internal::workflow::defer", "internal::workflow::Workflow::new")
        | ("internal::workflow::bind_operation", "internal::workflow::Workflow::new")
        | ("internal::workflow::reusable", "program::Effect::new")
        | ("internal::workflow::once", "program::Effect::new")
        | ("internal::workflow::defer_callback0", "program::Effect::defer")
        | ("internal::workflow::defer_callback1", "program::Effect::sync_once")
        | ("internal::workflow::defer_callback1", "program::Effect::flat_map")
        | ("internal::macro_future::async_workflow", "internal::workflow::Workflow::new")
        // Run each already-registered release under its protected continuation,
        // combining every cleanup channel before returning to the interpreter.
        | ("internal::macro_future::release_work", "internal::workflow::Workflow::new")
        | ("internal::macro_future::release_work", "internal::macro_future::release_work")
        // Recursive native iteration transfers owned, potentially non-Clone
        // iterator/state values through one-shot Workflow continuations.
        | ("program::walk_unit" | "program::iterate" | "program::collect" | "program::traverse", "internal::workflow::bind" | "internal::workflow::success")
        | ("sink::fold_reader", "internal::workflow::bind" | "internal::workflow::success")
        | ("sink::fold_chunk", "internal::workflow::bind")
        | ("stream::collect", "internal::workflow::bind" | "internal::workflow::success" | "internal::workflow::Workflow::from_exit")
        | ("stream::each", "internal::workflow::bind" | "internal::workflow::success")
        | ("stream::drain_chunk", "internal::workflow::bind")
        // Evaluate a consumed Workflow using the supplied private context.
        | ("internal::macro_future::evaluate", "program::Effect::new")
        // Own the native block's scope until full-outcome protected closure.
        | ("internal::macro_future::async_workflow", "scope::Scope::new")
        // CPU/blocking macro leaves dispatch under the private context and join
        // worker settlement, cancellation and capture-destruction failures.
        | ("job::reusable", "program::Effect::new")
        | ("job::once", "program::Effect::new")
        // Read a scoped response under its cancellation Context, publish the
        // full result and retain provider ownership through body settlement.
        | ("http::text", "program::Effect::new")
        // Parallel recipes select/record outcomes and own child settlement.
        | ("parallel::contender" | "parallel::concurrent" | "parallel::worker", "program::Effect::new")
        // The batch primitive completes every request before returning Finish.
        | ("request_resolver::internal::batch_effect", "program::Effect::new")
        // Native schedule iteration carries timing and complete retry outcomes.
        | ("schedule::iterate", "program::Effect::new")
        // Socket I/O and acquisition retain full outcomes under the native
        // Context; acquisition ownership is published before cancellation.
        | ("websocket::operation" | "websocket::own_socket", "program::Effect::new")
        // Buffer recipes acquire and fork under the upstream's owning scope.
        | ("stream::internal::buffer::buffered" | "stream::internal::buffer::merged", "stream::Stream::new")
        // The named activity adapter retains its recorded activity boundary.
        | ("workflow::durable::activity", "workflow::durable::Durable::from_effect")
        // Checked views for external Rust enums cannot redeclare Result/Option.
        | ("schema::variant_schema::json_result" | "schema::variant_schema::json_option",
            "schema::enum_builder::TaggedEnumBuilder::new" | "schema::enum_builder::TaggedEnumBuilder::variant" | "schema::enum_builder::TaggedEnumBuilder::finish")
        // Native transport close retains every cleanup Exit, including retained
        // failure/interruption, instead of narrowing completion to a Result.
        | ("http::request" | "websocket::with_socket" | "websocket::with_socket_map_error", "resource::Release::from_exit_future")
    )
}

/// Actual sealed Bindable implementations dispatch to their private operation
/// bridge. Same-named user traits and other native methods receive no permission.
pub fn binding_dispatch(owner: &str, callee: &str) -> bool {
    matches!(
        (owner, callee),
        ("program::Effect", "internal::workflow::bind_effect")
            | (
                "internal::workflow::Workflow",
                "internal::workflow::bind_operation"
            )
            | (
                "resource::ResourceBinding",
                "resource::ResourceBinding::bind_run"
            )
    )
}
