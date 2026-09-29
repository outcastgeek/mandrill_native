//! Explicitly volatile WorkflowEngine. Definitions and activities are encoded
//! data, while application code stays in typed registries outside the engine.
//! A recoverable engine must supply its own durable atomic transitions.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const types = @import("workflow_types.zig");
const contract = @import("workflow_engine.zig");

/// Construct an in-memory workflow engine provider with caller-owned runtime resources.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `now: comptime fn (*const R) deadlines.WallInstant`: Compile-time caller-supplied time or cursor boundary used for ordering and retention.
/// Returns (`type`): in-memory workflow engine using caller-selected allocation and runtime capabilities.
pub fn MemoryWorkflowEngine(
    comptime R: type,
    comptime now: fn (*const R) deadlines.WallInstant,
) type {
    return struct {
        const Self = @This();
        const Engine = contract.WorkflowEngine(R, void, .@"volatile");
        const Failure = contract.Error(void);
        const Bytes = outcomes.OwnedSlice(u8);
        const State = enum { queued, running, suspended, completed, terminal_cancelled };
        const Item = struct {
            execution_key: Bytes,
            definition_name: Bytes,
            definition_version: u32,
            payload_schema_version: u32,
            success_schema_version: u32,
            error_schema_version: u32,
            payload: Bytes,
            state: State = .queued,
            completed: ?Bytes = null,
            owner_id: ?Bytes = null,
            generation: u64 = 0,
            lease_until: deadlines.Deadline = .never,
            wake_pending: bool = false,
            cancel_requested: bool = false,

            fn deinit(self: *Item) void {
                self.execution_key.deinit();
                self.definition_name.deinit();
                self.payload.deinit();
                if (self.completed) |*value| value.deinit();
                if (self.owner_id) |*value| value.deinit();
                self.* = undefined;
            }
        };
        const Activity = struct {
            execution_key: Bytes,
            activity_key: Bytes,
            attempt: u64,
            version: u32,
            encoded_outcome: Bytes,

            fn deinit(self: *Activity) void {
                self.execution_key.deinit();
                self.activity_key.deinit();
                self.encoded_outcome.deinit();
                self.* = undefined;
            }
        };
        const SubmitCapture = struct { owner: *Self, submission: types.Submission };
        const ExecutionCapture = struct { owner: *Self, execution: types.ExecutionId };
        const ActivityCapture = struct { owner: *Self, execution: types.ExecutionId, activity: types.ActivityId };
        const RecordCapture = struct {
            owner: *Self,
            activation: types.Activation,
            activity: types.ActivityId,
            version: u32,
            outcome: []const u8,
        };
        const ActivateCapture = struct {
            owner: *Self,
            execution: types.ExecutionId,
            owner_id: []const u8,
            policy: types.ActivationPolicy,
        };
        const CompleteCapture = struct { owner: *Self, activation: types.Activation, outcome: []const u8 };
        const SuspendCapture = struct { owner: *Self, activation: types.Activation };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        executions: std.array_list.Managed(Item),
        activities: std.array_list.Managed(Activity),
        closed: bool = false,

        /// Create an in-memory workflow engine using the caller environment.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Self`): a workflow engine whose state uses the supplied capabilities.
        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemoryWorkflowEngine requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemoryWorkflowEngine requires caller io in R");
            return .{
                .allocator = env.allocator,
                .io = env.io,
                .executions = std.array_list.Managed(Item).init(env.allocator),
                .activities = std.array_list.Managed(Activity).init(env.allocator),
            };
        }

        /// Expose this implementation through its capability interface.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`Engine`): A workflow-engine capability backed by this in-memory provider.
        pub fn capability(self: *Self) Engine {
            return Engine.fromProvider(self, &.{
                .submit = submitEffect,
                .poll = pollEffect,
                .resume_execution = resumeEffect,
                .cancel_terminal = cancelEffect,
                .interrupt_unsafe = interruptUnsafeEffect,
                .lookup_activity = lookupEffect,
                .record_activity = recordEffect,
                .activate = activateEffect,
                .complete = completeEffect,
                .suspend_execution = suspendEffect,
                .abandon_activation = abandonEffect,
            });
        }

        fn bytes(allocator: std.mem.Allocator, value: []const u8) error{OutOfMemory}!Bytes {
            return .{ .items = try allocator.dupe(u8, value), .allocator = allocator };
        }

        fn find(self: *Self, execution: []const u8) ?*Item {
            for (self.executions.items) |*item| {
                if (std.mem.eql(u8, item.execution_key.items, execution)) return item;
            }
            return null;
        }

        fn findActivity(self: *Self, execution: []const u8, activity: types.ActivityId) ?*Activity {
            for (self.activities.items) |*record| {
                if (record.attempt == activity.attempt and
                    std.mem.eql(u8, record.execution_key.items, execution) and
                    std.mem.eql(u8, record.activity_key.items, activity.encoded)) return record;
            }
            return null;
        }

        fn submitted(self: *Self, submission: types.Submission) error{OutOfMemory}!Item {
            var execution_key = try bytes(self.allocator, submission.execution.encoded);
            errdefer execution_key.deinit();
            var definition_name = try bytes(self.allocator, submission.definition.name);
            errdefer definition_name.deinit();
            return .{
                .execution_key = execution_key,
                .definition_name = definition_name,
                .definition_version = submission.definition.version,
                .payload_schema_version = submission.definition.payload_schema_version,
                .success_schema_version = submission.definition.success_schema_version,
                .error_schema_version = submission.definition.error_schema_version,
                .payload = try bytes(self.allocator, submission.encoded_payload),
            };
        }

        fn sameSubmission(item: *const Item, submission: types.Submission) bool {
            return std.mem.eql(u8, item.definition_name.items, submission.definition.name) and
                item.definition_version == submission.definition.version and
                item.payload_schema_version == submission.definition.payload_schema_version and
                item.success_schema_version == submission.definition.success_schema_version and
                item.error_schema_version == submission.definition.error_schema_version and
                std.mem.eql(u8, item.payload.items, submission.encoded_payload);
        }

        fn submitEffect(data: ?*anyopaque, env: *const R, submission: types.Submission) Engine.SubmitEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.SubmitEffect.fromOwned(env, SubmitCapture{ .owner = owner, .submission = submission }, struct {
                fn run(capture: *const SubmitCapture, _: *const R) outcomes.Exit(types.SubmitResult, Failure) {
                    return capture.owner.submitNow(capture.submission);
                }
            }.run);
        }

        fn submitNow(self: *Self, submission: types.Submission) outcomes.Exit(types.SubmitResult, Failure) {
            if (!submission.definition.valid() or submission.execution.validate() == .err)
                return .{ .fail = .{ .operation = .malformed_record } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            if (self.find(submission.execution.encoded)) |existing| {
                if (!sameSubmission(existing, submission)) return .{ .fail = .{ .operation = .definition_conflict } };
                return .{ .success = .already_present };
            }
            var prepared = self.submitted(submission) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep = false;
            defer if (!keep) prepared.deinit();
            self.executions.append(prepared) catch return .{ .die = outcomes.Defect.out_of_memory };
            keep = true;
            return .{ .success = .accepted };
        }

        fn pollEffect(data: ?*anyopaque, env: *const R, execution: types.ExecutionId) Engine.PollEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.PollEffect.fromOwned(env, ExecutionCapture{ .owner = owner, .execution = execution }, struct {
                fn run(capture: *const ExecutionCapture, read_env: *const R) outcomes.Exit(types.ExecutionState, Failure) {
                    return capture.owner.pollNow(read_env.allocator, capture.execution);
                }
            }.run);
        }

        fn pollNow(self: *Self, allocator: std.mem.Allocator, execution: types.ExecutionId) outcomes.Exit(types.ExecutionState, Failure) {
            if (execution.validate() == .err) return .{ .fail = .{ .operation = .malformed_record } };
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                return .{ .fail = .{ .operation = .unavailable } };
            }
            const item = self.find(execution.encoded) orelse {
                self.mutex.unlock(self.io);
                return .{ .success = .absent };
            };
            switch (item.state) {
                .queued => {
                    self.mutex.unlock(self.io);
                    return .{ .success = .queued };
                },
                .running => {
                    const observed: types.ExecutionState = if (item.cancel_requested)
                        .cancellation_requested
                    else
                        .running;
                    self.mutex.unlock(self.io);
                    return .{ .success = observed };
                },
                .suspended => {
                    self.mutex.unlock(self.io);
                    return .{ .success = .suspended };
                },
                .terminal_cancelled => {
                    self.mutex.unlock(self.io);
                    return .{ .success = .terminal_cancelled };
                },
                .completed => {},
            }
            const copied = bytes(allocator, item.completed.?.items) catch {
                self.mutex.unlock(self.io);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            self.mutex.unlock(self.io);
            return .{ .success = .{ .completed = copied } };
        }

        fn resumeEffect(data: ?*anyopaque, env: *const R, execution: types.ExecutionId) Engine.ControlEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.ControlEffect.fromOwned(env, ExecutionCapture{ .owner = owner, .execution = execution }, struct {
                fn run(capture: *const ExecutionCapture, _: *const R) outcomes.Exit(types.ControlResult, Failure) {
                    return capture.owner.controlNow(capture.execution, false);
                }
            }.run);
        }

        fn cancelEffect(data: ?*anyopaque, env: *const R, execution: types.ExecutionId) Engine.ControlEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.ControlEffect.fromOwned(env, ExecutionCapture{ .owner = owner, .execution = execution }, struct {
                fn run(capture: *const ExecutionCapture, _: *const R) outcomes.Exit(types.ControlResult, Failure) {
                    return capture.owner.controlNow(capture.execution, true);
                }
            }.run);
        }

        fn interruptUnsafeEffect(data: ?*anyopaque, env: *const R, execution: types.ExecutionId) Engine.ControlEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.ControlEffect.fromOwned(env, ExecutionCapture{ .owner = owner, .execution = execution }, struct {
                fn run(capture: *const ExecutionCapture, _: *const R) outcomes.Exit(types.ControlResult, Failure) {
                    return capture.owner.interruptUnsafeNow(capture.execution);
                }
            }.run);
        }

        fn interruptUnsafeNow(self: *Self, execution: types.ExecutionId) outcomes.Exit(types.ControlResult, Failure) {
            if (execution.validate() == .err) return .{ .fail = .{ .operation = .malformed_record } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(execution.encoded) orelse return .{ .success = .absent };
            if (item.state == .terminal_cancelled or item.state == .completed) return .{ .success = .already_recorded };
            settleCancellation(item);
            return .{ .success = .recorded };
        }

        fn controlNow(self: *Self, execution: types.ExecutionId, terminal: bool) outcomes.Exit(types.ControlResult, Failure) {
            if (execution.validate() == .err) return .{ .fail = .{ .operation = .malformed_record } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(execution.encoded) orelse return .{ .success = .absent };
            if (terminal) {
                if (item.state == .terminal_cancelled or item.state == .completed) return .{ .success = .already_recorded };
                if (item.state == .running) {
                    if (item.cancel_requested) return .{ .success = .already_recorded };
                    item.cancel_requested = true;
                    return .{ .success = .recorded };
                }
                settleCancellation(item);
                return .{ .success = .recorded };
            }
            switch (item.state) {
                .running => {
                    if (item.wake_pending) return .{ .success = .already_recorded };
                    item.wake_pending = true;
                    return .{ .success = .recorded };
                },
                .suspended => {
                    item.state = .queued;
                    return .{ .success = .recorded };
                },
                .queued, .completed, .terminal_cancelled => return .{ .success = .already_recorded },
            }
        }

        fn activateEffect(data: ?*anyopaque, env: *const R, execution: types.ExecutionId, owner_id: []const u8, policy: types.ActivationPolicy) Engine.ActivationEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.ActivationEffect.fromOwned(env, ActivateCapture{ .owner = owner, .execution = execution, .owner_id = owner_id, .policy = policy }, struct {
                fn run(capture: *const ActivateCapture, read_env: *const R) outcomes.Exit(types.ActivationResult, Failure) {
                    return capture.owner.activateNow(read_env.allocator, capture.*, now(read_env));
                }
            }.run);
        }

        fn activateNow(self: *Self, allocator: std.mem.Allocator, request: ActivateCapture, instant: deadlines.WallInstant) outcomes.Exit(types.ActivationResult, Failure) {
            if (request.execution.validate() == .err or request.owner_id.len == 0 or
                !request.policy.valid() or request.policy.lease_until.isDue(instant))
                return .{ .fail = .{ .operation = .deadline_invalid } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(request.execution.encoded) orelse return .{ .fail = .{ .operation = .execution_missing } };
            if (item.state == .completed or item.state == .terminal_cancelled) return .{ .success = .already_terminal };
            if (item.state == .running and !item.lease_until.isDue(instant)) return .{ .success = .unavailable };
            if (item.cancel_requested) {
                settleCancellation(item);
                return .{ .success = .already_terminal };
            }
            if (item.generation == std.math.maxInt(u64)) return .{ .fail = .{ .operation = .generation_exhausted } };
            var stored_owner = bytes(self.allocator, request.owner_id) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_owner = false;
            defer if (!keep_owner) stored_owner.deinit();
            var returned_key = bytes(allocator, request.execution.encoded) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_key = false;
            defer if (!keep_key) returned_key.deinit();
            var returned_owner = bytes(allocator, request.owner_id) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep_returned_owner = false;
            defer if (!keep_returned_owner) returned_owner.deinit();
            if (item.owner_id) |*old| old.deinit();
            item.owner_id = stored_owner;
            keep_owner = true;
            item.generation += 1;
            item.lease_until = request.policy.lease_until;
            item.state = .running;
            keep_key = true;
            keep_returned_owner = true;
            return .{ .success = .{ .activated = .{
                .execution_key = returned_key,
                .owner_id = returned_owner,
                .generation = item.generation,
            } } };
        }

        fn current(item: *const Item, activation: types.Activation, instant: deadlines.WallInstant) bool {
            const owner = item.owner_id orelse return false;
            return item.state == .running and item.generation == activation.generation and
                std.mem.eql(u8, owner.items, activation.owner_id) and !item.lease_until.isDue(instant);
        }

        fn settleCancellation(item: *Item) void {
            item.state = .terminal_cancelled;
            item.cancel_requested = false;
            item.wake_pending = false;
            if (item.owner_id) |*owner| owner.deinit();
            item.owner_id = null;
            item.lease_until = .never;
        }

        fn lookupEffect(data: ?*anyopaque, env: *const R, execution: types.ExecutionId, activity: types.ActivityId) Engine.ActivityEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.ActivityEffect.fromOwned(env, ActivityCapture{ .owner = owner, .execution = execution, .activity = activity }, struct {
                fn run(capture: *const ActivityCapture, read_env: *const R) outcomes.Exit(?types.RecordedActivity, Failure) {
                    return capture.owner.lookupNow(read_env.allocator, capture.execution, capture.activity);
                }
            }.run);
        }

        fn lookupNow(self: *Self, allocator: std.mem.Allocator, execution: types.ExecutionId, activity: types.ActivityId) outcomes.Exit(?types.RecordedActivity, Failure) {
            if (execution.validate() == .err or activity.validate() == .err) return .{ .fail = .{ .operation = .malformed_record } };
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                return .{ .fail = .{ .operation = .unavailable } };
            }
            if (self.find(execution.encoded) == null) {
                self.mutex.unlock(self.io);
                return .{ .fail = .{ .operation = .execution_missing } };
            }
            const record = self.findActivity(execution.encoded, activity) orelse {
                self.mutex.unlock(self.io);
                return .{ .success = null };
            };
            const copied = bytes(allocator, record.encoded_outcome.items) catch {
                self.mutex.unlock(self.io);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            const version = record.version;
            self.mutex.unlock(self.io);
            return .{ .success = .{
                .outcome = copied,
                .activity_version = version,
            } };
        }

        fn recordEffect(data: ?*anyopaque, env: *const R, activation: types.Activation, activity: types.ActivityId, version: u32, outcome: []const u8) Engine.RecordEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.RecordEffect.fromOwned(env, RecordCapture{
                .owner = owner,
                .activation = activation,
                .activity = activity,
                .version = version,
                .outcome = outcome,
            }, struct {
                fn run(capture: *const RecordCapture, read_env: *const R) outcomes.Exit(types.RecordResult, Failure) {
                    return capture.owner.recordNow(capture.*, now(read_env));
                }
            }.run);
        }

        fn preparedActivity(self: *Self, request: RecordCapture) error{OutOfMemory}!Activity {
            var execution_key = try bytes(self.allocator, request.activation.execution.encoded);
            errdefer execution_key.deinit();
            var activity_key = try bytes(self.allocator, request.activity.encoded);
            errdefer activity_key.deinit();
            return .{
                .execution_key = execution_key,
                .activity_key = activity_key,
                .attempt = request.activity.attempt,
                .version = request.version,
                .encoded_outcome = try bytes(self.allocator, request.outcome),
            };
        }

        fn recordNow(self: *Self, request: RecordCapture, instant: deadlines.WallInstant) outcomes.Exit(types.RecordResult, Failure) {
            if (!request.activation.valid() or request.activity.validate() == .err)
                return .{ .fail = .{ .operation = .malformed_record } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(request.activation.execution.encoded) orelse return .{ .fail = .{ .operation = .execution_missing } };
            if (!current(item, request.activation, instant)) return .{ .success = .stale_activation };
            if (self.findActivity(request.activation.execution.encoded, request.activity)) |record| {
                if (record.version == request.version and std.mem.eql(u8, record.encoded_outcome.items, request.outcome)) {
                    if (item.cancel_requested) settleCancellation(item);
                    return .{ .success = .already_recorded };
                }
                return .{ .success = .conflicting_record };
            }
            var prepared = self.preparedActivity(request) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep = false;
            defer if (!keep) prepared.deinit();
            self.activities.append(prepared) catch return .{ .die = outcomes.Defect.out_of_memory };
            keep = true;
            if (item.cancel_requested) settleCancellation(item);
            return .{ .success = .recorded };
        }

        fn completeEffect(data: ?*anyopaque, env: *const R, activation: types.Activation, outcome: []const u8) Engine.CompletionEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.CompletionEffect.fromOwned(env, CompleteCapture{ .owner = owner, .activation = activation, .outcome = outcome }, struct {
                fn run(capture: *const CompleteCapture, read_env: *const R) outcomes.Exit(types.CompletionResult, Failure) {
                    return capture.owner.completeNow(capture.activation, capture.outcome, now(read_env));
                }
            }.run);
        }

        fn completeNow(self: *Self, activation: types.Activation, outcome: []const u8, instant: deadlines.WallInstant) outcomes.Exit(types.CompletionResult, Failure) {
            if (!activation.valid())
                return .{ .fail = .{ .operation = .malformed_record } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(activation.execution.encoded) orelse return .{ .fail = .{ .operation = .execution_missing } };
            if (item.state == .completed) {
                if (std.mem.eql(u8, item.completed.?.items, outcome)) return .{ .success = .already_committed };
                return .{ .success = .conflicting_completion };
            }
            if (!current(item, activation, instant)) return .{ .success = .stale_activation };
            if (item.cancel_requested) {
                settleCancellation(item);
                return .{ .success = .stale_activation };
            }
            var prepared = bytes(self.allocator, outcome) catch return .{ .die = outcomes.Defect.out_of_memory };
            var keep = false;
            defer if (!keep) prepared.deinit();
            item.completed = prepared;
            item.state = .completed;
            item.wake_pending = false;
            keep = true;
            return .{ .success = .committed };
        }

        fn suspendEffect(data: ?*anyopaque, env: *const R, activation: types.Activation) Engine.SuspensionEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.SuspensionEffect.fromOwned(env, SuspendCapture{ .owner = owner, .activation = activation }, struct {
                fn run(capture: *const SuspendCapture, read_env: *const R) outcomes.Exit(types.SuspensionResult, Failure) {
                    return capture.owner.suspendNow(capture.activation, now(read_env));
                }
            }.run);
        }

        fn suspendNow(self: *Self, activation: types.Activation, instant: deadlines.WallInstant) outcomes.Exit(types.SuspensionResult, Failure) {
            if (!activation.valid())
                return .{ .fail = .{ .operation = .malformed_record } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(activation.execution.encoded) orelse return .{ .fail = .{ .operation = .execution_missing } };
            if (item.state == .suspended and item.generation == activation.generation and
                item.owner_id != null and std.mem.eql(u8, item.owner_id.?.items, activation.owner_id))
                return .{ .success = .already_recorded };
            if (!current(item, activation, instant)) return .{ .success = .stale_activation };
            if (item.cancel_requested) {
                settleCancellation(item);
                return .{ .success = .stale_activation };
            }
            if (item.wake_pending) {
                item.wake_pending = false;
                item.state = .queued;
            } else item.state = .suspended;
            return .{ .success = .recorded };
        }

        fn abandonEffect(data: ?*anyopaque, env: *const R, activation: types.Activation) Engine.SuspensionEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Engine.SuspensionEffect.fromOwned(env, SuspendCapture{ .owner = owner, .activation = activation }, struct {
                fn run(capture: *const SuspendCapture, read_env: *const R) outcomes.Exit(types.SuspensionResult, Failure) {
                    return capture.owner.abandonNow(capture.activation, now(read_env));
                }
            }.run);
        }

        fn abandonNow(self: *Self, activation: types.Activation, instant: deadlines.WallInstant) outcomes.Exit(types.SuspensionResult, Failure) {
            if (!activation.valid()) return .{ .fail = .{ .operation = .malformed_record } };
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .{ .operation = .unavailable } };
            const item = self.find(activation.execution.encoded) orelse return .{ .fail = .{ .operation = .execution_missing } };
            if (!current(item, activation, instant)) return .{ .success = .stale_activation };
            if (item.cancel_requested) {
                settleCancellation(item);
                return .{ .success = .recorded };
            }
            if (item.owner_id) |*owner| owner.deinit();
            item.owner_id = null;
            item.lease_until = .never;
            item.wake_pending = false;
            item.state = .queued;
            return .{ .success = .recorded };
        }

        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.mutex.unlock(self.io);
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.shutdown();
            for (self.executions.items) |*item| item.deinit();
            for (self.activities.items) |*activity| activity.deinit();
            self.executions.deinit();
            self.activities.deinit();
            self.* = undefined;
        }
    };
}
