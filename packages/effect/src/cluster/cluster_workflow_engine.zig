//! Workflow commands carried by Cluster's accepted, fenced message and saved
//! reply protocol. Workflow progress remains with the selected WorkflowEngine;
//! this composition introduces no journal or storage provider of its own.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const cluster_types = @import("cluster_types.zig");
const message_store = @import("message_store.zig");
const workflow_types = @import("../workflow/workflow_types.zig");
const workflow_engine = @import("../workflow/workflow_engine.zig");
const durable_deferred = @import("../workflow/durable_deferred.zig");
const durable_clock = @import("../workflow/durable_clock.zig");

/// Cluster message selecting a workflow run or progress query.
pub const Command = union(enum) {
    run: workflow_types.Submission,
    poll: workflow_types.ExecutionId,
    resume_execution: workflow_types.ExecutionId,
    cancel_terminal: workflow_types.ExecutionId,
    interrupt: workflow_types.ExecutionId,
    lookup_activity: struct { execution: workflow_types.ExecutionId, activity: workflow_types.ActivityId },
    record_activity: struct {
        activation: workflow_types.Activation,
        activity: workflow_types.ActivityId,
        version: u32,
        outcome: []const u8,
    },
    activate: struct { execution: workflow_types.ExecutionId, owner_id: []const u8, policy: workflow_types.ActivationPolicy },
    complete: struct { activation: workflow_types.Activation, outcome: []const u8 },
    suspend_execution: workflow_types.Activation,
    abandon_activation: workflow_types.Activation,
    signal: struct { identity: durable_deferred.Identity, outcome: []const u8 },
    schedule_deadline: durable_clock.Schedule,
    cancel_deadline: durable_clock.Identity,

    /// Return the workflow execution identity.
    /// Parameters:
    /// - `self: @This()`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`workflow_types.ExecutionId`): The workflow execution identity.
    pub fn execution(self: @This()) workflow_types.ExecutionId {
        return switch (self) {
            .run => |value| value.execution,
            .poll, .resume_execution, .cancel_terminal, .interrupt => |value| value,
            .lookup_activity => |value| value.execution,
            .record_activity => |value| value.activation.execution,
            .activate => |value| value.execution,
            .complete => |value| value.activation.execution,
            .suspend_execution => |value| value.execution,
            .abandon_activation => |value| value.execution,
            .signal => |value| value.identity.execution,
            .schedule_deadline => |value| value.identity.execution,
            .cancel_deadline => |value| value.execution,
        };
    }
};

/// Typed answer to the selected workflow command.
pub const Reply = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    run: workflow_types.SubmitResult,
    poll: workflow_types.ExecutionState,
    resume_execution: workflow_types.ControlResult,
    cancel_terminal: workflow_types.ControlResult,
    interrupt: workflow_types.ControlResult,
    lookup_activity: ?workflow_types.RecordedActivity,
    record_activity: workflow_types.RecordResult,
    activate: workflow_types.ActivationResult,
    complete: workflow_types.CompletionResult,
    suspend_execution: workflow_types.SuspensionResult,
    abandon_activation: workflow_types.SuspensionResult,
    signal: durable_deferred.Completion,
    schedule_deadline: durable_clock.ScheduleResult,
    cancel_deadline: durable_clock.CancelResult,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        switch (self.*) {
            .poll => outcomes.discardPayload(&self.poll),
            .lookup_activity => outcomes.discardPayload(&self.lookup_activity),
            .activate => outcomes.discardPayload(&self.activate),
            else => {},
        }
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .poll => |value| .{ .poll = try outcomes.clonePayload(value) },
            .lookup_activity => |value| .{ .lookup_activity = try outcomes.clonePayload(value) },
            .activate => |value| .{ .activate = try outcomes.clonePayload(value) },
            else => self.*,
        };
    }
};

/// Specialize the typed exchange result outcome for cluster operations.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): typed response to a cluster/workflow command exchange.
pub fn ExchangeResult(comptime E: type) type {
    return union(enum) {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        completed: outcomes.Exit(Reply, E),
        accepted,
        uncertain,
        missing,
        expired,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            if (self.* == .completed) self.completed.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .completed => |value| blk: {
                    const copy = value.clone();
                    if (copy == .die and copy.die.kind == .out_of_memory and
                        !(value == .die and value.die.kind == .out_of_memory))
                        return error.OutOfMemory;
                    break :blk .{ .completed = copy };
                },
                else => self.*,
            };
        }
    };
}

/// Compose cluster messaging with a workflow engine and explicit durability guarantees.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `StoreProviderError: comptime type`: Compile-time error type returned by the store provider.
/// - `EngineProviderError: comptime type`: Compile-time error type returned by the selected engine provider.
/// - `SignalProviderError: comptime type`: Compile-time error type returned by the signal provider.
/// - `ClockProviderError: comptime type`: Compile-time error type returned by the clock provider.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// - `store_guarantee: comptime message_store.Guarantee`: Compile-time recoverability promised by the message store.
/// - `engine_guarantee: comptime workflow_engine.Guarantee`: Compile-time recoverability promised by the selected engine.
/// - `signal_guarantee: comptime durable_deferred.Guarantee`: Compile-time recoverability promised by the signal store.
/// - `clock_guarantee: comptime durable_clock.Guarantee`: Compile-time recoverability promised by the durable clock.
/// - `select_store: comptime fn (*const R) *const message_store.MessageStore(R, StoreProviderError, store_guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_engine: comptime fn (*const R) *const workflow_engine.WorkflowEngine(R, EngineProviderError, engine_guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_signal: comptime fn (*const R) *const durable_deferred.Store(R, SignalProviderError, signal_guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_clock: comptime fn (*const R) *const durable_clock.Store(R, ClockProviderError, clock_guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_retention: comptime fn (*const R) deadlines.Deadline`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `group: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or replayed data.
/// - `request_key: comptime fn (*const Command) []const u8`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `Codec: comptime type`: Compile-time codec type with a decoded value and encoded representation.
/// Returns (`type`): workflow engine adapter that routes through cluster messaging while preserving durability requirements.
pub fn ClusterWorkflowEngine(
    comptime R: type,
    comptime StoreProviderError: type,
    comptime EngineProviderError: type,
    comptime SignalProviderError: type,
    comptime ClockProviderError: type,
    comptime CodecError: type,
    comptime store_guarantee: message_store.Guarantee,
    comptime engine_guarantee: workflow_engine.Guarantee,
    comptime signal_guarantee: durable_deferred.Guarantee,
    comptime clock_guarantee: durable_clock.Guarantee,
    comptime select_store: fn (*const R) *const message_store.MessageStore(R, StoreProviderError, store_guarantee),
    comptime select_engine: fn (*const R) *const workflow_engine.WorkflowEngine(R, EngineProviderError, engine_guarantee),
    comptime select_signal: fn (*const R) *const durable_deferred.Store(R, SignalProviderError, signal_guarantee),
    comptime select_clock: fn (*const R) *const durable_clock.Store(R, ClockProviderError, clock_guarantee),
    comptime select_retention: fn (*const R) deadlines.Deadline,
    comptime group: []const u8,
    comptime version: u32,
    comptime request_key: fn (*const Command) []const u8,
    comptime Codec: type,
) type {
    const Store = message_store.MessageStore(R, StoreProviderError, store_guarantee);
    const Engine = workflow_engine.WorkflowEngine(R, EngineProviderError, engine_guarantee);
    const Signal = durable_deferred.Store(R, SignalProviderError, signal_guarantee);
    const Clock = durable_clock.Store(R, ClockProviderError, clock_guarantee);
    comptime {
        if (group.len == 0 or !std.unicode.utf8ValidateSlice(group) or version == 0)
            @compileError("cluster workflow identity requires a nonempty group and version");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
            !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
            @compileError("ClusterWorkflowEngine requires allocator and io in R");
        if (!@hasDecl(Codec, "encodeCommand") or !@hasDecl(Codec, "decodeCommand") or
            !@hasDecl(Codec, "encodeOutcome") or !@hasDecl(Codec, "decodeOutcome"))
            @compileError("cluster workflow codec must encode commands and full typed outcomes");
    }
    return struct {
        const Self = @This();
        /// Recoverable only when store, engine, signal, and clock providers all promise recoverability.
        pub const storage_guarantee: workflow_engine.Guarantee = if (store_guarantee == .recoverable and
            engine_guarantee == .recoverable and signal_guarantee == .recoverable and
            clock_guarantee == .recoverable) .recoverable else .@"volatile";
        /// Typed failure of a cluster-backed workflow command exchange.
        pub const Error = union(enum) {
            const Own = @This();
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = true;
            cluster: Store.Failure,
            workflow: Engine.Failure,
            signal: Signal.Failure,
            deadline: Clock.Error,
            codec: CodecError,
            invalid_identity,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *Own`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *Own) void {
                switch (self.*) {
                    .cluster => outcomes.discardPayload(&self.cluster),
                    .workflow => outcomes.discardPayload(&self.workflow),
                    .signal => outcomes.discardPayload(&self.signal),
                    .deadline => outcomes.discardPayload(&self.deadline),
                    .codec => outcomes.discardPayload(&self.codec),
                    else => {},
                }
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const Own`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!Own`): A `Own` on success; `OutOfMemory` if required allocation fails.
            pub fn clone(self: *const Own) error{OutOfMemory}!Own {
                return switch (self.*) {
                    .cluster => |value| .{ .cluster = try outcomes.clonePayload(value) },
                    .workflow => |value| .{ .workflow = try outcomes.clonePayload(value) },
                    .signal => |value| .{ .signal = try outcomes.clonePayload(value) },
                    .deadline => |value| .{ .deadline = try outcomes.clonePayload(value) },
                    .codec => |value| .{ .codec = try outcomes.clonePayload(value) },
                    .invalid_identity => .invalid_identity,
                };
            }
        };
        /// Typed command-exchange outcome for a cluster-backed workflow.
        pub const Exchange = ExchangeResult(Error);
        /// Lazy Effect for send; success, error, and environment types are fixed by
        /// `effects.Effect(Exchange, Error, R)`.
        pub const Send = effects.Effect(Exchange, Error, R);
        /// Lazy Effect for execute; success, error, and environment types are fixed by
        /// `effects.Effect(Reply, Error, R)`.
        pub const Execute = effects.Effect(Reply, Error, R);
        /// Typed terminal outcome for recorded; its success and failure types are fixed by
        /// `outcomes.Exit(Reply, Error)`.
        pub const Recorded = outcomes.Exit(Reply, Error);
        /// Lazy Effect for complete; success, error, and environment types are fixed by
        /// `effects.Effect(message_store.Completion, Error, R)`.
        pub const Complete = effects.Effect(message_store.Completion, Error, R);

        fn clusterError(value: Store.Failure) Error {
            return .{ .cluster = value };
        }
        fn workflowError(value: Engine.Failure) Error {
            return .{ .workflow = value };
        }
        fn signalError(value: Signal.Failure) Error {
            return .{ .signal = value };
        }
        fn deadlineError(value: Clock.Error) Error {
            return .{ .deadline = value };
        }
        fn codecError(value: CodecError) Error {
            return .{ .codec = value };
        }

        fn address(command: *const Command) outcomes.Result(cluster_types.Address, Error) {
            const execution = command.execution();
            if (execution.validate() == .err) return .{ .err = .invalid_identity };
            const located: cluster_types.Address = .{ .group = group, .entity_key = execution.encoded, .request_key = request_key(command) };
            return switch (located.validate()) {
                .ok => .{ .ok = located },
                .err => .{ .err = .invalid_identity },
            };
        }

        const SubmissionOwner = struct {
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = true;
            bytes: workflow_types.Bytes,
            located: cluster_types.Address,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *@This()) void {
                self.bytes.deinit();
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                return .{ .bytes = try self.bytes.clone(), .located = self.located };
            }
        };

        fn readResult(located: cluster_types.Address, acceptance: message_store.Acceptance, env: *const R) Send {
            const store = select_store(env);
            var reading = store.vtable.read_reply(store.userdata, env, located);
            var mapped = reading.mapErrorOwned(env, clusterError);
            return mapped.flatMapCapturedOwned(env, Exchange, acceptance, struct {
                fn decoded(state: *const message_store.Acceptance, reply: message_store.Reply, supplied: *const R) Send {
                    var owned = reply;
                    defer owned.deinit();
                    return switch (owned) {
                        .completed => |bytes| blk: {
                            const decoded_outcome = Codec.decodeOutcome(bytes.items, supplied);
                            break :blk switch (decoded_outcome) {
                                .success => |value| Send.fromOwnedSuccess(supplied, .{ .completed = value }),
                                else => Send.fromOwnedOutcome(supplied, outcomes.mapError(Error, outcomes.castFailure(Exchange, decoded_outcome), supplied.allocator, codecError)),
                            };
                        },
                        .pending => Send.fromOwnedSuccess(supplied, if (state.* == .possibly_accepted) .uncertain else .accepted),
                        .missing => Send.fromOwnedSuccess(supplied, if (state.* == .possibly_accepted or state.* == .already_completed) .uncertain else .missing),
                        .expired => Send.fromOwnedSuccess(supplied, .expired),
                    };
                }
            }.decoded);
        }

        fn afterAccept(owner: *const SubmissionOwner, acceptance: message_store.Acceptance, env: *const R) Send {
            return readResult(owner.located, acceptance, env);
        }

        /// Acceptance is deduplicated by the caller's stable request key;
        /// completion reads Cluster's saved reply rather than another cache.
        ///
        /// Parameters:
        /// - `select_command: comptime fn (*const R) *const Command`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`Send`): The typed operation result or description selected by this API.
        pub fn send(comptime select_command: fn (*const R) *const Command) Send {
            return Send.deferEffect(struct {
                fn start(env: *const R) Send {
                    return sendCommand(select_command(env).*, env);
                }
            }.start);
        }

        /// The caller keeps borrowed command fields valid through execution.
        /// The codec returns independent owned bytes before Cluster accepts it.
        ///
        /// Parameters:
        /// - `command: Command`: Typed command submitted to the engine.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Send`): The typed operation result or description selected by this API.
        pub fn sendCommand(command: Command, env: *const R) Send {
            const located = switch (address(&command)) {
                .ok => |value| value,
                .err => |problem| return Send.fromOwnedOutcome(env, .{ .fail = problem }),
            };
            const encoded = Codec.encodeCommand(&command, env);
            const bytes = switch (encoded) {
                .success => |value| value,
                else => return Send.fromOwnedOutcome(env, outcomes.mapError(Error, outcomes.castFailure(Exchange, encoded), env.allocator, codecError)),
            };
            const capture: SubmissionOwner = .{ .bytes = bytes, .located = located };
            const store = select_store(env);
            const submission: message_store.Submission = .{ .address = located, .definition_version = version, .encoded_request = bytes.items, .retention_until = select_retention(env) };
            var accepting = store.vtable.accept(store.userdata, env, submission);
            var mapped = accepting.mapErrorOwned(env, clusterError);
            return mapped.flatMapCapturedOwned(env, Exchange, capture, afterAccept);
        }

        const CompletionOwner = struct {
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = true;
            bytes: workflow_types.Bytes,
            claim: *const message_store.ClaimedMessage,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *@This()) void {
                self.bytes.deinit();
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                return .{ .bytes = try self.bytes.clone(), .claim = self.claim };
            }
        };

        fn completed(_: *const CompletionOwner, result: message_store.Completion, env: *const R) Complete {
            return Complete.fromOwnedSuccess(env, result);
        }

        fn record(claim: *const *const message_store.ClaimedMessage, outcome: Recorded, env: *const R) Complete {
            var owned = outcome;
            const encoded = Codec.encodeOutcome(&owned, env);
            owned.deinit();
            const bytes = switch (encoded) {
                .success => |value| value,
                else => return Complete.fromOwnedOutcome(env, outcomes.mapError(Error, outcomes.castFailure(message_store.Completion, encoded), env.allocator, codecError)),
            };
            const store = select_store(env);
            const capture: CompletionOwner = .{ .bytes = bytes, .claim = claim.* };
            var committing = store.vtable.complete(store.userdata, env, claim.*.token(), bytes.items);
            var mapped = committing.mapErrorOwned(env, clusterError);
            return mapped.flatMapCapturedOwned(env, message_store.Completion, capture, completed);
        }

        /// A Cluster owner passes one fenced claim, retained in R through the
        /// Effect. Decoding borrows from its request bytes. The codec encodes
        /// the full typed Exit into independent owned reply bytes; decoding a
        /// saved reply likewise returns an independent owned Exit.
        ///
        /// Parameters:
        /// - `select_claim: comptime fn (*const R) *const message_store.ClaimedMessage`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Complete`): The typed operation result or description selected by this API.
        pub fn serveClaim(comptime select_claim: fn (*const R) *const message_store.ClaimedMessage) Complete {
            return Complete.deferEffect(struct {
                fn start(env: *const R) Complete {
                    const claim = select_claim(env);
                    if (!std.mem.eql(u8, claim.group.items, group) or claim.definition_version != version)
                        return Complete.fromOwnedOutcome(env, .{ .fail = .invalid_identity });
                    const decoded = Codec.decodeCommand(claim.encoded_request.items, env);
                    const command = switch (decoded) {
                        .success => |value| value,
                        else => return Complete.fromOwnedOutcome(env, outcomes.mapError(Error, outcomes.castFailure(message_store.Completion, decoded), env.allocator, codecError)),
                    };
                    const located = switch (address(&command)) {
                        .ok => |value| value,
                        .err => |problem| return Complete.fromOwnedOutcome(env, .{ .fail = problem }),
                    };
                    if (!std.mem.eql(u8, located.entity_key, claim.entity_key.items) or
                        !std.mem.eql(u8, located.request_key, claim.request_key.items))
                        return Complete.fromOwnedOutcome(env, .{ .fail = .invalid_identity });
                    var running = execute(command, env);
                    var settled = running.materializeOwned(env);
                    var aligned = settled.mapErrorOwned(env, struct {
                        fn impossible(_: void) Error {
                            unreachable;
                        }
                    }.impossible);
                    return aligned.flatMapCapturedOwned(env, message_store.Completion, claim, record);
                }
            }.start);
        }

        /// Execute a cluster-backed workflow command.
        /// Parameters:
        /// - `command: Command`: Typed command submitted to the engine.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Execute`): The typed operation result or description selected by this API.
        pub fn execute(command: Command, env: *const R) Execute {
            const engine = select_engine(env);
            return switch (command) {
                .run => |value| blk: {
                    var task = engine.vtable.submit(engine.userdata, env, value);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.SubmitResult) Reply {
                            return .{ .run = result };
                        }
                    }.wrap);
                },
                .poll => |value| blk: {
                    var task = engine.vtable.poll(engine.userdata, env, value);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.ExecutionState) Reply {
                            return .{ .poll = result };
                        }
                    }.wrap);
                },
                .resume_execution => |value| blk: {
                    var task = engine.vtable.resume_execution(engine.userdata, env, value);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.ControlResult) Reply {
                            return .{ .resume_execution = result };
                        }
                    }.wrap);
                },
                .cancel_terminal => |value| blk: {
                    var task = engine.vtable.cancel_terminal(engine.userdata, env, value);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.ControlResult) Reply {
                            return .{ .cancel_terminal = result };
                        }
                    }.wrap);
                },
                .interrupt => |value| blk: {
                    var task = engine.vtable.interrupt_unsafe(engine.userdata, env, value);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.ControlResult) Reply {
                            return .{ .interrupt = result };
                        }
                    }.wrap);
                },
                .lookup_activity => |value| blk: {
                    var task = engine.vtable.lookup_activity(engine.userdata, env, value.execution, value.activity);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: ?workflow_types.RecordedActivity) Reply {
                            return .{ .lookup_activity = result };
                        }
                    }.wrap);
                },
                .record_activity => |value| blk: {
                    var task = engine.vtable.record_activity(engine.userdata, env, value.activation, value.activity, value.version, value.outcome);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.RecordResult) Reply {
                            return .{ .record_activity = result };
                        }
                    }.wrap);
                },
                .activate => |value| blk: {
                    var task = engine.vtable.activate(engine.userdata, env, value.execution, value.owner_id, value.policy);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.ActivationResult) Reply {
                            return .{ .activate = result };
                        }
                    }.wrap);
                },
                .complete => |value| blk: {
                    var task = engine.vtable.complete(engine.userdata, env, value.activation, value.outcome);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.CompletionResult) Reply {
                            return .{ .complete = result };
                        }
                    }.wrap);
                },
                .suspend_execution => |value| blk: {
                    var task = engine.vtable.suspend_execution(engine.userdata, env, value);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.SuspensionResult) Reply {
                            return .{ .suspend_execution = result };
                        }
                    }.wrap);
                },
                .abandon_activation => |value| blk: {
                    var task = engine.vtable.abandon_activation(engine.userdata, env, value);
                    var mapped = task.mapErrorOwned(env, workflowError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: workflow_types.SuspensionResult) Reply {
                            return .{ .abandon_activation = result };
                        }
                    }.wrap);
                },
                .signal => |value| blk: {
                    const service = select_signal(env);
                    var task = service.vtable.complete(service.userdata, env, value.identity, value.outcome);
                    var mapped = task.mapErrorOwned(env, signalError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: durable_deferred.Completion) Reply {
                            return .{ .signal = result };
                        }
                    }.wrap);
                },
                .schedule_deadline => |value| blk: {
                    const service = select_clock(env);
                    var task = service.vtable.schedule(service.userdata, env, value);
                    var mapped = task.mapErrorOwned(env, deadlineError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: durable_clock.ScheduleResult) Reply {
                            return .{ .schedule_deadline = result };
                        }
                    }.wrap);
                },
                .cancel_deadline => |value| blk: {
                    const service = select_clock(env);
                    var task = service.vtable.cancel(service.userdata, env, value);
                    var mapped = task.mapErrorOwned(env, deadlineError);
                    break :blk mapped.mapOwned(env, struct {
                        fn wrap(result: durable_clock.CancelResult) Reply {
                            return .{ .cancel_deadline = result };
                        }
                    }.wrap);
                },
            };
        }
    };
}
