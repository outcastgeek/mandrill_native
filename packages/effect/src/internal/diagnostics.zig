//! Diagnostic text is presentation data. Operations and defects dispatch by
//! types/enums, never by these strings. Keep messages named at their contract.
pub const ref_modifier_contract = "Ref.modify requires a function from the stored type to a struct with result and a value of the stored type";
pub const schedule_delay = "Schedule delay must be non-negative and representable in nanoseconds";
pub const schedule_factor = "Exponential factor must be finite and at least one";
pub const schedule_jitter = "Jitter requires ordered finite non-negative factors and a sample in [0,1]";
pub const schedule_counter = "Schedule recurrence counter overflow";
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

pub const allocation_failed = "Effect allocation failed";
pub const rpc_handler_finalization_failed = "RPC handler finalization failed";
pub const suspended_binding_allocator = "Suspended binding requires an allocator supplied through its environment";
pub const owned_binding_allocator = "Runtime description construction requires an allocator supplied through its environment";
pub const provision_allocator = "Environment provision requires the bound allocator for suspended work";
pub const resource_allocator = "Resource ownership requires environment .allocator";

pub const expected_known_function = " expects a compile-time-known function";
pub const expected_typed_callback = " requires a typed callback with a declared result";
pub const expected_callback_input = " callback must accept ";
pub const continuation_environment = " and the current environment as *const ";
pub const explicit_callback_conversion = "; supply a compatible callback or explicit conversion";
pub const expected_effect = " callback must return an Effect";
pub const error_type_mismatch = " error type mismatch: expected ";
pub const environment_mismatch = " environment mismatch: expected ";
pub const actual_type = ", got ";
pub const explicit_error_mapping = "; use an explicit error mapping";
pub const explicit_provision = "; use provide or provideSome explicitly";
pub const expected_effect_contract = " callback must return Effect(A, E, R)";
pub const service_result = "service projection result must be ";
pub const sync_result = "sync callback result must be ";
pub const native_error_union = "tryIo callback must return a native error union";
pub const native_success_type = "tryIo callback success type mismatch";
pub const native_error_mapping = "tryIo error mapper must return the declared Effect error type";
pub const native_io_capability = "tryIo requires environment .io: std.Io";
pub const defer_result = "deferEffect factory must return the same Effect(A, E, R)";
pub const recovery_result = "catchAll must return the same Effect; adapt errors/environments explicitly";
pub const resource_allocator_capability = "acquireRelease requires environment .allocator: std.mem.Allocator";
pub const finalizer_contract = "acquireRelease finalizer must accept the resource and PrimaryOutcome(E), optionally followed by *const R, and return Effect(void, E, R)";
pub const projection_contract = "provideSome must return a borrowed pointer to the required environment";
pub const allocator_field = "Effect environment .allocator must be std.mem.Allocator";
pub const io_field = "Effect environment .io must be std.Io";
pub const cancellation_field = "Effect environment .cancellation must point to effect.Cancellation";
pub const traversal_allocator = "Effect traversal requires environment .allocator";
pub const protected_allocator = "Suspended cancellation protection requires environment .allocator";

// Facility contracts: presentation text only; typed cases control behavior.
pub const pool_capacity_must_be_positive = "Pool capacity must be positive";
pub const pool_idle_lifetime_must_be_non_negative = "Pool idle lifetime must be non-negative";
pub const pool_is_closed = "Pool is closed";
pub const pool_resource_ordering_is_exhausted = "Pool resource ordering is exhausted";
pub const pool_idle_duration_overflowed = "Pool idle duration overflowed";
pub const only_a_bounded_pool_accepts_a_positive_replacement_capacity = "Only a bounded pool accepts a positive replacement capacity";
pub const pool_population_target_exceeds_capacity_or_pool_is_closed = "Pool population target exceeds capacity or pool is closed";
pub const http_transfer_and_redirect_buffers_must_be_non_empty = "HTTP transfer and redirect buffers must be non-empty";
pub const invalid_http_header = "Invalid HTTP header";
pub const clock_sleep_duration_must_be_non_negative = "Clock sleep duration must be non-negative";
pub const clock_deadline_overflowed = "Clock deadline overflowed";
pub const query_refresh_interval_must_be_positive = "Query refresh interval must be positive";
pub const query_refresh_deadline_overflowed = "Query refresh deadline overflowed";
pub const manual_clock_cannot_move_backwards = "Manual clock cannot move backwards";
pub const manual_clock_timestamp_overflowed = "Manual clock timestamp overflowed";
pub const ref_values_must_be_borrowed_handles_or_ordinary_copyable_values_an_owning = "Ref values must be borrowed handles or ordinary copyable values; an owning payload requires an explicit ownership adapter";
pub const cache_ttl_must_be_non_negative = "Cache TTL must be non-negative";
pub const cache_snapshot_generation_is_exhausted = "Cache snapshot generation is exhausted";
pub const cache_is_closed = "Cache is closed";
pub const cache_in_flight_admission_capacity_is_exhausted = "Cache in-flight admission capacity is exhausted";
pub const cache_expiration_timestamp_overflowed = "Cache expiration timestamp overflowed";
pub const reusable_owned_success_requires_a_lazy_sync_fromoutcome_factory_or_a_scoped = "Reusable owned success requires a lazy sync/fromOutcome factory or a scoped resource";
pub const reusable_owned_failure_requires_a_lazy_fromoutcome_factory = "Reusable owned failure requires a lazy fromOutcome factory";
pub const sandbox_mapper_must_return_the_declared_effect_error_type = "sandbox mapper must return the declared Effect error type";
pub const effect_execution_requires_environment = "Effect execution requires environment .";
pub const race_requires_environment_i_o_for_synchronization = "Race requires environment I/O for synchronization";
pub const timeout_duration_must_be_non_negative = "Timeout duration must be non-negative";
pub const concurrency_must_be_positive = "Concurrency must be positive";
pub const concurrent_traversal_requires_environment_i_o_for_synchronization = "Concurrent traversal requires environment I/O for synchronization";
pub const scoped_acquisitions_must_expose_borrowed_resource_handles_acquire_an_owning_evaluated_by = "Scoped acquisitions must expose borrowed resource handles; acquire an owning evaluated by pointer instead";
pub const cannot_register_a_resource_in_a_closing_or_closed_scope = "Cannot register a resource in a closing or closed scope";
pub const sleep_duration_must_be_non_negative = "Sleep duration must be non-negative";
pub const sleep_requires_i_o_through_r = "Sleep requires I/O through R";
pub const sleep_requires_independent_native_progress = "Sleep requires independent native progress";
pub const logging_requires_environment_logger_effect_logger = "Logging requires environment .logger: effect.Logger";
pub const r_logger_must_be_effect_logger = "R.logger must be effect.Logger";
pub const tracing_requires_environment_tracer_effect_tracer = "Tracing requires environment .tracer: effect.Tracer";
pub const schedule_input_must_match_the_selected_effect_success_error_channel = "Schedule input must match the selected Effect success/error channel";
pub const jittered_scheduling_requires_environment_random_effect_random = "Jittered scheduling requires environment .random: effect.Random";
pub const scheduling_requires_a_clock_or_i_o_supplied_through_the_environment = "Scheduling requires a clock or I/O supplied through the environment";
pub const layer_services_must_be_borrowed_dependency_records_or_handles_install_owning_resources = "Layer services must be borrowed dependency records or handles; install owning resources with Layer.scoped";
pub const merged_layers_must_share_environment_and_error_types = "Merged layers must share environment and error types";
pub const provided_layer_input_must_match_the_first_layer_s_output_and_error = "Provided layer input must match the first layer's output and error type";
pub const stream_collection_length_overflow = "Stream collection length overflow";
pub const stream_exceeded_the_caller_s_collection_limit = "Stream exceeded the caller's collection limit";
pub const asynchronous_execution_requires_environment_i_o_for_coordination = "Asynchronous execution requires environment I/O for coordination";
pub const fork_requires_environment_i_o_for_observation = "Fork requires environment I/O for observation";
pub const r_http_transport_must_be_a_typed_http_transport = "R.http_transport must be a typed HTTP Transport";
pub const an_absolute_ws_wss_uri_is_required = "An absolute ws/wss URI is required";
pub const r_websocket_transport_must_be_a_typed_websocket_transport = "R.websocket_transport must be a typed WebSocket Transport";
pub const websocket_callback_must_accept_a_borrowed_connection_pointer_and_return_an_effect = "WebSocket callback must accept a borrowed Connection pointer and return an Effect";
pub const websocket_error_mapper_must_return_the_callback_effect_s_error_type = "WebSocket error mapper must return the callback Effect's error type";
pub const a_reusable_queue_offer_must_produce_a_fresh_owning_value_through_offer = "A reusable queue offer must produce a fresh owning value through offer's environment selector";
pub const native_asynchronous_operation_requires_independent_progress = "Native asynchronous operation requires independent progress";
pub const native_wakeable_asynchronous_operation_requires_independent_progress = "Native wakeable asynchronous operation requires independent progress";
pub const message_and_receive_buffer_limits_must_be_positive_buffer_must_fit_a = "Message and receive buffer limits must be positive; buffer must fit a complete message";
pub const close_timeout_must_be_positive = "Close timeout must be positive";
pub const acquired_websocket_must_be_open = "Acquired WebSocket must be open";
pub const invalid_utf_8_text = "Invalid UTF-8 text";
pub const websocket_provider_returned_a_fragment_larger_than_its_receive_buffer = "WebSocket provider returned a fragment larger than its receive buffer";
pub const message_type_changed_during_fragmentation = "Message type changed during fragmentation";
pub const websocket_close_requires_independent_handshake_progress = "WebSocket close requires independent handshake progress";
pub const websocket_close_requires_an_independent_timeout_clock = "WebSocket close requires an independent timeout clock";
pub const owned_effect_payload_deinit_must_accept_self_and_return_void = "Owned Effect payload deinit must accept *Self and return void";
pub const owned_effect_payload_clone_must_accept_const_self_and_return_error_outofmemory = "Owned Effect payload clone must accept *const Self and return error{OutOfMemory}!Self";
pub const weakcache_requires_gc_ephemeron_semantics_which_this_zig_port_does_not_provide = "WeakCache requires GC ephemeron semantics, which this Zig port does not provide. Choose the explicit TTL/capacity Cache contract or a consumer-owned GC adapter.";
pub const execution_provider_does_not_support_the_requested_capability = "Execution provider does not support the requested capability";
pub const native_wakeable_execution_requires_independent_progress = "Native wakeable execution requires independent progress";
pub const execution_provider_admission_capacity_is_exhausted = "Execution provider admission capacity is exhausted";
pub const native_i_o_provider_rejected_concurrent_execution = "Native I/O provider rejected concurrent execution";
pub const intersected_schedules_must_accept_the_same_input_type = "Intersected schedules must accept the same input type";
pub const native_operation_requires_independent_i_o_progress = "Native operation requires independent I/O progress";

pub const native_allocator_capability = "Native Effect execution requires environment .allocator: std.mem.Allocator";
pub const native_future_io_capability = "Native Effect execution requires environment .io: std.Io";
pub const owned_effect_outcome_can_run_only_once = "An owned Effect outcome can run only once";
