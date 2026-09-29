//! Diagnostic text is presentation data. Operations and defects dispatch by
//! types/enums, never by these strings. Keep messages named at their contract.
/// Diagnostic text for ref modifier contract.
pub const ref_modifier_contract = "Ref.modify requires a function from the stored type to a struct with result and a value of the stored type";
/// Diagnostic text for schedule delay.
pub const schedule_delay = "Schedule delay must be non-negative and representable in nanoseconds";
/// Diagnostic text for schedule factor.
pub const schedule_factor = "Exponential factor must be finite and at least one";
/// Diagnostic text for schedule jitter.
pub const schedule_jitter = "Jitter requires ordered finite non-negative factors and a sample in [0,1]";
/// Diagnostic text for schedule counter.
pub const schedule_counter = "Schedule recurrence counter overflow";
/// `Operation` cases: `service`, `sync`, `map`, `flatMap`, `flatMapOwned`, `catchAll`, `tryIo`,
/// `tryIo_error_mapper` and further declared cases.
pub const Operation = enum {
    service,
    sync,
    map,
    flatMap,
    flatMapOwned,
    catchAll,
    tryIo,
    tryIo_error_mapper,
    deferEffect,
    acquireRelease_body,
    acquireRelease_finalizer,
    provideSome,
    EffectBuilder_bind,
    mapError,
    sandbox,
    zip,
    traverse,

    /// Return the diagnostic label for this operation.
    /// Parameters:
    /// - `self: comptime Operation`: Compile-time current value passed by value; ownership follows this operation’s
    ///   contract.
    /// Returns (`[]const u8`): A borrowed slice; keep the owner alive while using it.
    pub fn label(comptime self: Operation) []const u8 {
        return switch (self) {
            .tryIo_error_mapper => "tryIo error mapper",
            .acquireRelease_body => "acquireRelease body",
            .acquireRelease_finalizer => "acquireRelease finalizer",
            .EffectBuilder_bind => "EffectBuilder.bind",
            else => @tagName(self),
        };
    }
};

/// Diagnostic text for allocation failed.
pub const allocation_failed = "Effect allocation failed";
/// Diagnostic text for rpc handler finalization failed.
pub const rpc_handler_finalization_failed = "RPC handler finalization failed";
/// Diagnostic text for suspended binding allocator.
pub const suspended_binding_allocator = "Suspended binding requires an allocator supplied through its environment";
/// Diagnostic text for owned binding allocator.
pub const owned_binding_allocator = "Runtime description construction requires an allocator supplied through its environment";
/// Diagnostic text for provision allocator.
pub const provision_allocator = "Environment provision requires the bound allocator for suspended work";
/// Diagnostic text for resource allocator.
pub const resource_allocator = "Resource ownership requires environment .allocator";

/// Diagnostic text for expected known function.
pub const expected_known_function = " expects a compile-time-known function";
/// Diagnostic text for expected typed callback.
pub const expected_typed_callback = " requires a typed callback with a declared result";
/// Diagnostic text for expected callback input.
pub const expected_callback_input = " callback must accept ";
/// Diagnostic text for continuation environment.
pub const continuation_environment = " and the current environment as *const ";
/// Diagnostic text for explicit callback conversion.
pub const explicit_callback_conversion = "; supply a compatible callback or explicit conversion";
/// Diagnostic text for expected effect.
pub const expected_effect = " callback must return an Effect";
/// Diagnostic text for error type mismatch.
pub const error_type_mismatch = " error type mismatch: expected ";
/// Diagnostic text for environment mismatch.
pub const environment_mismatch = " environment mismatch: expected ";
/// Diagnostic text for actual type.
pub const actual_type = ", got ";
/// Diagnostic text for explicit error mapping.
pub const explicit_error_mapping = "; use an explicit error mapping";
/// Diagnostic text for explicit provision.
pub const explicit_provision = "; use provide or provideSome explicitly";
/// Diagnostic text for expected effect contract.
pub const expected_effect_contract = " callback must return Effect(A, E, R)";
/// Diagnostic text for service result.
pub const service_result = "service projection result must be ";
/// Diagnostic text for sync result.
pub const sync_result = "sync callback result must be ";
/// Diagnostic text for native error union.
pub const native_error_union = "tryIo callback must return a native error union";
/// Diagnostic text for native success type.
pub const native_success_type = "tryIo callback success type mismatch";
/// Diagnostic text for native error mapping.
pub const native_error_mapping = "tryIo error mapper must return the declared Effect error type";
/// Diagnostic text for native io capability.
pub const native_io_capability = "tryIo requires environment .io: std.Io";
/// Diagnostic text for defer result.
pub const defer_result = "deferEffect factory must return the same Effect(A, E, R)";
/// Diagnostic text for recovery result.
pub const recovery_result = "catchAll must return the same Effect; adapt errors/environments explicitly";
/// Diagnostic text for resource allocator capability.
pub const resource_allocator_capability = "acquireRelease requires environment .allocator: std.mem.Allocator";
/// Diagnostic text for finalizer contract.
pub const finalizer_contract = "acquireRelease finalizer must accept the resource and PrimaryOutcome(E), optionally followed by *const R, and return Effect(void, E, R)";
/// Diagnostic text for projection contract.
pub const projection_contract = "provideSome must return a borrowed pointer to the required environment";
/// Diagnostic text for allocator field.
pub const allocator_field = "Effect environment .allocator must be std.mem.Allocator";
/// Diagnostic text for io field.
pub const io_field = "Effect environment .io must be std.Io";
/// Diagnostic text for cancellation field.
pub const cancellation_field = "Effect environment .cancellation must point to effect.Cancellation";
/// Diagnostic text for traversal allocator.
pub const traversal_allocator = "Effect traversal requires environment .allocator";
/// Diagnostic text for protected allocator.
pub const protected_allocator = "Suspended cancellation protection requires environment .allocator";

// Facility contracts: presentation text only; typed cases control behavior.
/// Diagnostic text for pool capacity must be positive.
pub const pool_capacity_must_be_positive = "Pool capacity must be positive";
/// Diagnostic text for pool idle lifetime must be non negative.
pub const pool_idle_lifetime_must_be_non_negative = "Pool idle lifetime must be non-negative";
/// Diagnostic text for pool is closed.
pub const pool_is_closed = "Pool is closed";
/// Diagnostic text for pool resource ordering is exhausted.
pub const pool_resource_ordering_is_exhausted = "Pool resource ordering is exhausted";
/// Diagnostic text for pool idle duration overflowed.
pub const pool_idle_duration_overflowed = "Pool idle duration overflowed";
/// Diagnostic text for only a bounded pool accepts a positive replacement capacity.
pub const only_a_bounded_pool_accepts_a_positive_replacement_capacity = "Only a bounded pool accepts a positive replacement capacity";
/// Diagnostic text for pool population target exceeds capacity or pool is closed.
pub const pool_population_target_exceeds_capacity_or_pool_is_closed = "Pool population target exceeds capacity or pool is closed";
/// Diagnostic text for http transfer and redirect buffers must be non empty.
pub const http_transfer_and_redirect_buffers_must_be_non_empty = "HTTP transfer and redirect buffers must be non-empty";
/// Diagnostic text for invalid http header.
pub const invalid_http_header = "Invalid HTTP header";
/// Diagnostic text for clock sleep duration must be non negative.
pub const clock_sleep_duration_must_be_non_negative = "Clock sleep duration must be non-negative";
/// Diagnostic text for clock deadline overflowed.
pub const clock_deadline_overflowed = "Clock deadline overflowed";
/// Diagnostic text for query refresh interval must be positive.
pub const query_refresh_interval_must_be_positive = "Query refresh interval must be positive";
/// Diagnostic text for query refresh deadline overflowed.
pub const query_refresh_deadline_overflowed = "Query refresh deadline overflowed";
/// Diagnostic text for manual clock cannot move backwards.
pub const manual_clock_cannot_move_backwards = "Manual clock cannot move backwards";
/// Diagnostic text for manual clock timestamp overflowed.
pub const manual_clock_timestamp_overflowed = "Manual clock timestamp overflowed";
/// Diagnostic text for ref values must be borrowed handles or ordinary copyable values an owning.
pub const ref_values_must_be_borrowed_handles_or_ordinary_copyable_values_an_owning = "Ref values must be borrowed handles or ordinary copyable values; an owning payload requires an explicit ownership adapter";
/// Diagnostic text for cache ttl must be non negative.
pub const cache_ttl_must_be_non_negative = "Cache TTL must be non-negative";
/// Diagnostic text for cache snapshot generation is exhausted.
pub const cache_snapshot_generation_is_exhausted = "Cache snapshot generation is exhausted";
/// Diagnostic text for cache is closed.
pub const cache_is_closed = "Cache is closed";
/// Diagnostic text for cache in flight admission capacity is exhausted.
pub const cache_in_flight_admission_capacity_is_exhausted = "Cache in-flight admission capacity is exhausted";
/// Diagnostic text for cache expiration timestamp overflowed.
pub const cache_expiration_timestamp_overflowed = "Cache expiration timestamp overflowed";
/// Diagnostic text for reusable owned success requires a lazy sync fromoutcome factory or a scoped.
pub const reusable_owned_success_requires_a_lazy_sync_fromoutcome_factory_or_a_scoped = "Reusable owned success requires a lazy sync/fromOutcome factory or a scoped resource";
/// Diagnostic text for reusable owned failure requires a lazy fromoutcome factory.
pub const reusable_owned_failure_requires_a_lazy_fromoutcome_factory = "Reusable owned failure requires a lazy fromOutcome factory";
/// Diagnostic text for sandbox mapper must return the declared effect error type.
pub const sandbox_mapper_must_return_the_declared_effect_error_type = "sandbox mapper must return the declared Effect error type";
/// Diagnostic text for effect execution requires environment.
pub const effect_execution_requires_environment = "Effect execution requires environment .";
/// Diagnostic text for race requires environment i o for synchronization.
pub const race_requires_environment_i_o_for_synchronization = "Race requires environment I/O for synchronization";
/// Diagnostic text for timeout duration must be non negative.
pub const timeout_duration_must_be_non_negative = "Timeout duration must be non-negative";
/// Diagnostic text for concurrency must be positive.
pub const concurrency_must_be_positive = "Concurrency must be positive";
/// Diagnostic text for concurrent traversal requires environment i o for synchronization.
pub const concurrent_traversal_requires_environment_i_o_for_synchronization = "Concurrent traversal requires environment I/O for synchronization";
/// Diagnostic text for scoped acquisitions must expose borrowed resource handles acquire an owning evaluated by.
pub const scoped_acquisitions_must_expose_borrowed_resource_handles_acquire_an_owning_evaluated_by = "Scoped acquisitions must expose borrowed resource handles; acquire an owning evaluated by pointer instead";
/// Diagnostic text for cannot register a resource in a closing or closed scope.
pub const cannot_register_a_resource_in_a_closing_or_closed_scope = "Cannot register a resource in a closing or closed scope";
/// Diagnostic text for sleep duration must be non negative.
pub const sleep_duration_must_be_non_negative = "Sleep duration must be non-negative";
/// Diagnostic text for sleep requires i o through r.
pub const sleep_requires_i_o_through_r = "Sleep requires I/O through R";
/// Diagnostic text for sleep requires independent native progress.
pub const sleep_requires_independent_native_progress = "Sleep requires independent native progress";
/// Diagnostic text for logging requires environment logger effect logger.
pub const logging_requires_environment_logger_effect_logger = "Logging requires environment .logger: effect.Logger";
/// Diagnostic text for r logger must be effect logger.
pub const r_logger_must_be_effect_logger = "R.logger must be effect.Logger";
/// Diagnostic text for tracing requires environment tracer effect tracer.
pub const tracing_requires_environment_tracer_effect_tracer = "Tracing requires environment .tracer: effect.Tracer";
/// Diagnostic text for schedule input must match the selected effect success error channel.
pub const schedule_input_must_match_the_selected_effect_success_error_channel = "Schedule input must match the selected Effect success/error channel";
/// Diagnostic text for jittered scheduling requires environment random effect random.
pub const jittered_scheduling_requires_environment_random_effect_random = "Jittered scheduling requires environment .random: effect.Random";
/// Diagnostic text for scheduling requires a clock or i o supplied through the environment.
pub const scheduling_requires_a_clock_or_i_o_supplied_through_the_environment = "Scheduling requires a clock or I/O supplied through the environment";
/// Diagnostic text for layer services must be borrowed dependency records or handles install owning resources.
pub const layer_services_must_be_borrowed_dependency_records_or_handles_install_owning_resources = "Layer services must be borrowed dependency records or handles; install owning resources with Layer.scoped";
/// Diagnostic text for merged layers must share environment and error types.
pub const merged_layers_must_share_environment_and_error_types = "Merged layers must share environment and error types";
/// Diagnostic text for provided layer input must match the first layer s output and error.
pub const provided_layer_input_must_match_the_first_layer_s_output_and_error = "Provided layer input must match the first layer's output and error type";
/// Diagnostic text for stream collection length overflow.
pub const stream_collection_length_overflow = "Stream collection length overflow";
/// Diagnostic text for stream exceeded the caller s collection limit.
pub const stream_exceeded_the_caller_s_collection_limit = "Stream exceeded the caller's collection limit";
/// Diagnostic text for asynchronous execution requires environment i o for coordination.
pub const asynchronous_execution_requires_environment_i_o_for_coordination = "Asynchronous execution requires environment I/O for coordination";
/// Diagnostic text for fork requires environment i o for observation.
pub const fork_requires_environment_i_o_for_observation = "Fork requires environment I/O for observation";
/// Diagnostic text for r http transport must be a typed http transport.
pub const r_http_transport_must_be_a_typed_http_transport = "R.http_transport must be a typed HTTP Transport";
/// Diagnostic text for an absolute ws wss uri is required.
pub const an_absolute_ws_wss_uri_is_required = "An absolute ws/wss URI is required";
/// Diagnostic text for r websocket transport must be a typed websocket transport.
pub const r_websocket_transport_must_be_a_typed_websocket_transport = "R.websocket_transport must be a typed WebSocket Transport";
/// Diagnostic text for websocket callback must accept a borrowed connection pointer and return an effect.
pub const websocket_callback_must_accept_a_borrowed_connection_pointer_and_return_an_effect = "WebSocket callback must accept a borrowed Connection pointer and return an Effect";
/// Diagnostic text for websocket error mapper must return the callback effect s error type.
pub const websocket_error_mapper_must_return_the_callback_effect_s_error_type = "WebSocket error mapper must return the callback Effect's error type";
/// Diagnostic text for a reusable queue offer must produce a fresh owning value through offer.
pub const a_reusable_queue_offer_must_produce_a_fresh_owning_value_through_offer = "A reusable queue offer must produce a fresh owning value through offer's environment selector";
/// Diagnostic text for native asynchronous operation requires independent progress.
pub const native_asynchronous_operation_requires_independent_progress = "Native asynchronous operation requires independent progress";
/// Diagnostic text for native wakeable asynchronous operation requires independent progress.
pub const native_wakeable_asynchronous_operation_requires_independent_progress = "Native wakeable asynchronous operation requires independent progress";
/// Diagnostic text for message and receive buffer limits must be positive buffer must fit a.
pub const message_and_receive_buffer_limits_must_be_positive_buffer_must_fit_a = "Message and receive buffer limits must be positive; buffer must fit a complete message";
/// Diagnostic text for close timeout must be positive.
pub const close_timeout_must_be_positive = "Close timeout must be positive";
/// Diagnostic text for acquired websocket must be open.
pub const acquired_websocket_must_be_open = "Acquired WebSocket must be open";
/// Diagnostic text for invalid utf 8 text.
pub const invalid_utf_8_text = "Invalid UTF-8 text";
/// Diagnostic text for websocket provider returned a fragment larger than its receive buffer.
pub const websocket_provider_returned_a_fragment_larger_than_its_receive_buffer = "WebSocket provider returned a fragment larger than its receive buffer";
/// Diagnostic text for message type changed during fragmentation.
pub const message_type_changed_during_fragmentation = "Message type changed during fragmentation";
/// Diagnostic text for websocket close requires independent handshake progress.
pub const websocket_close_requires_independent_handshake_progress = "WebSocket close requires independent handshake progress";
/// Diagnostic text for websocket close requires an independent timeout clock.
pub const websocket_close_requires_an_independent_timeout_clock = "WebSocket close requires an independent timeout clock";
/// Diagnostic text for owned effect payload deinit must accept self and return void.
pub const owned_effect_payload_deinit_must_accept_self_and_return_void = "Owned Effect payload deinit must accept *Self and return void";
/// Diagnostic text for owned effect payload clone must accept const self and return error outofmemory.
pub const owned_effect_payload_clone_must_accept_const_self_and_return_error_outofmemory = "Owned Effect payload clone must accept *const Self and return error{OutOfMemory}!Self";
/// Diagnostic text for weakcache requires gc ephemeron semantics which this zig port does not provide.
pub const weakcache_requires_gc_ephemeron_semantics_which_this_zig_port_does_not_provide = "WeakCache requires GC ephemeron semantics, which this Zig port does not provide. Choose the explicit TTL/capacity Cache contract or a consumer-owned GC adapter.";
/// Diagnostic text for execution provider does not support the requested capability.
pub const execution_provider_does_not_support_the_requested_capability = "Execution provider does not support the requested capability";
/// Diagnostic text for native wakeable execution requires independent progress.
pub const native_wakeable_execution_requires_independent_progress = "Native wakeable execution requires independent progress";
/// Diagnostic text for execution provider admission capacity is exhausted.
pub const execution_provider_admission_capacity_is_exhausted = "Execution provider admission capacity is exhausted";
/// Diagnostic text for native i o provider rejected concurrent execution.
pub const native_i_o_provider_rejected_concurrent_execution = "Native I/O provider rejected concurrent execution";
/// Diagnostic text for intersected schedules must accept the same input type.
pub const intersected_schedules_must_accept_the_same_input_type = "Intersected schedules must accept the same input type";
/// Diagnostic text for native operation requires independent i o progress.
pub const native_operation_requires_independent_i_o_progress = "Native operation requires independent I/O progress";

/// Diagnostic text for native allocator capability.
pub const native_allocator_capability = "Native Effect execution requires environment .allocator: std.mem.Allocator";
/// Diagnostic text for native future io capability.
pub const native_future_io_capability = "Native Effect execution requires environment .io: std.Io";
/// Diagnostic text for owned effect outcome can run only once.
pub const owned_effect_outcome_can_run_only_once = "An owned Effect outcome can run only once";
