//! Public, provider-neutral encoded WorkflowEngine contract. It does not
//! depend on Cluster or expose a database object to workflow definitions.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");
const workflow_error = @import("workflow_error.zig");

pub const Guarantee = enum { @"volatile", recoverable };

pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        operation: workflow_error.OperationError,
        provider: ProviderError,

        pub fn deinit(self: *Self) void {
            if (self.* == .provider) outcomes.discardPayload(&self.provider);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

/// The provider copies borrowed submission/control buffers before retaining
/// them. Returned records are owned by the caller's allocator from R. A
/// recordActivity transition is atomic for its execution, identity, attempt
/// and activation fence. `possibly_recorded` requires lookup before retrying
/// an external side effect; it must never masquerade as `recorded`.
/// `cancel_terminal` records a cooperative cancellation request for running
/// work and reaches a terminal state at a fenced activity/suspension boundary
/// or after lease recovery. `interrupt_unsafe` may fence it immediately while
/// external effects still run. Providers must keep these operations distinct.
pub fn WorkflowEngine(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        pub const storage_guarantee = guarantee;
        pub const Failure = Error(ProviderError);
        pub const SubmitEffect = effects.Effect(types.SubmitResult, Failure, R);
        pub const PollEffect = effects.Effect(types.ExecutionState, Failure, R);
        pub const ControlEffect = effects.Effect(types.ControlResult, Failure, R);
        pub const ActivityEffect = effects.Effect(?types.RecordedActivity, Failure, R);
        pub const RecordEffect = effects.Effect(types.RecordResult, Failure, R);
        pub const ActivationEffect = effects.Effect(types.ActivationResult, Failure, R);
        pub const CompletionEffect = effects.Effect(types.CompletionResult, Failure, R);
        pub const SuspensionEffect = effects.Effect(types.SuspensionResult, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            submit: *const fn (?*anyopaque, *const R, types.Submission) SubmitEffect,
            poll: *const fn (?*anyopaque, *const R, types.ExecutionId) PollEffect,
            resume_execution: *const fn (?*anyopaque, *const R, types.ExecutionId) ControlEffect,
            cancel_terminal: *const fn (?*anyopaque, *const R, types.ExecutionId) ControlEffect,
            /// May fence a running activation before its owned work has joined.
            /// Callers must opt into this separate, explicitly unsafe action.
            interrupt_unsafe: *const fn (?*anyopaque, *const R, types.ExecutionId) ControlEffect,
            lookup_activity: *const fn (?*anyopaque, *const R, types.ExecutionId, types.ActivityId) ActivityEffect,
            record_activity: *const fn (?*anyopaque, *const R, types.Activation, types.ActivityId, u32, []const u8) RecordEffect,
            activate: *const fn (?*anyopaque, *const R, types.ExecutionId, []const u8, types.ActivationPolicy) ActivationEffect,
            complete: *const fn (?*anyopaque, *const R, types.Activation, []const u8) CompletionEffect,
            suspend_execution: *const fn (?*anyopaque, *const R, types.Activation) SuspensionEffect,
            /// Release only this local activation. The execution remains
            /// queued for recovery and its completed activity records remain.
            abandon_activation: *const fn (?*anyopaque, *const R, types.Activation) SuspensionEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn submit(comptime select_engine: fn (*const R) *const Self, comptime select_submission: fn (*const R) types.Submission) SubmitEffect {
            return SubmitEffect.deferEffect(struct {
                fn start(env: *const R) SubmitEffect {
                    const submission = select_submission(env);
                    if (!submission.definition.valid() or submission.execution.validate() == .err)
                        return SubmitEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.submit(engine.userdata, env, submission);
                }
            }.start);
        }

        pub fn poll(comptime select_engine: fn (*const R) *const Self, comptime select_execution: fn (*const R) types.ExecutionId) PollEffect {
            return PollEffect.deferEffect(struct {
                fn start(env: *const R) PollEffect {
                    const execution = select_execution(env);
                    if (execution.validate() == .err) return PollEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.poll(engine.userdata, env, execution);
                }
            }.start);
        }

        pub fn resumeExecution(comptime select_engine: fn (*const R) *const Self, comptime select_execution: fn (*const R) types.ExecutionId) ControlEffect {
            return control(select_engine, select_execution, false);
        }

        pub fn cancelTerminal(comptime select_engine: fn (*const R) *const Self, comptime select_execution: fn (*const R) types.ExecutionId) ControlEffect {
            return control(select_engine, select_execution, true);
        }

        pub fn interruptUnsafe(comptime select_engine: fn (*const R) *const Self, comptime select_execution: fn (*const R) types.ExecutionId) ControlEffect {
            return ControlEffect.deferEffect(struct {
                fn start(env: *const R) ControlEffect {
                    const execution = select_execution(env);
                    if (execution.validate() == .err) return ControlEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.interrupt_unsafe(engine.userdata, env, execution);
                }
            }.start);
        }

        fn control(comptime select_engine: fn (*const R) *const Self, comptime select_execution: fn (*const R) types.ExecutionId, comptime terminal: bool) ControlEffect {
            return ControlEffect.deferEffect(struct {
                fn start(env: *const R) ControlEffect {
                    const execution = select_execution(env);
                    if (execution.validate() == .err) return ControlEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return if (terminal)
                        engine.vtable.cancel_terminal(engine.userdata, env, execution)
                    else
                        engine.vtable.resume_execution(engine.userdata, env, execution);
                }
            }.start);
        }

        pub fn lookupActivity(comptime select_engine: fn (*const R) *const Self, comptime select_execution: fn (*const R) types.ExecutionId, comptime select_activity: fn (*const R) types.ActivityId) ActivityEffect {
            return ActivityEffect.deferEffect(struct {
                fn start(env: *const R) ActivityEffect {
                    const execution = select_execution(env);
                    const activity = select_activity(env);
                    if (execution.validate() == .err or activity.validate() == .err)
                        return ActivityEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.lookup_activity(engine.userdata, env, execution, activity);
                }
            }.start);
        }

        pub fn recordActivity(
            comptime select_engine: fn (*const R) *const Self,
            comptime select_activation: fn (*const R) types.Activation,
            comptime select_activity: fn (*const R) types.ActivityId,
            comptime select_version: fn (*const R) u32,
            comptime select_outcome: fn (*const R) []const u8,
        ) RecordEffect {
            return RecordEffect.deferEffect(struct {
                fn start(env: *const R) RecordEffect {
                    const activation = select_activation(env);
                    const activity = select_activity(env);
                    if (!activation.valid() or activity.validate() == .err)
                        return RecordEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.record_activity(engine.userdata, env, activation, activity, select_version(env), select_outcome(env));
                }
            }.start);
        }

        pub fn activate(
            comptime select_engine: fn (*const R) *const Self,
            comptime select_execution: fn (*const R) types.ExecutionId,
            comptime select_owner_id: fn (*const R) []const u8,
            comptime select_policy: fn (*const R) types.ActivationPolicy,
        ) ActivationEffect {
            return ActivationEffect.deferEffect(struct {
                fn start(env: *const R) ActivationEffect {
                    const execution = select_execution(env);
                    const owner_id = select_owner_id(env);
                    const policy = select_policy(env);
                    if (execution.validate() == .err or owner_id.len == 0 or !policy.valid())
                        return ActivationEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.activate(engine.userdata, env, execution, owner_id, policy);
                }
            }.start);
        }

        pub fn complete(
            comptime select_engine: fn (*const R) *const Self,
            comptime select_activation: fn (*const R) types.Activation,
            comptime select_outcome: fn (*const R) []const u8,
        ) CompletionEffect {
            return CompletionEffect.deferEffect(struct {
                fn start(env: *const R) CompletionEffect {
                    const activation = select_activation(env);
                    if (!activation.valid())
                        return CompletionEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.complete(engine.userdata, env, activation, select_outcome(env));
                }
            }.start);
        }

        pub fn suspendExecution(
            comptime select_engine: fn (*const R) *const Self,
            comptime select_activation: fn (*const R) types.Activation,
        ) SuspensionEffect {
            return SuspensionEffect.deferEffect(struct {
                fn start(env: *const R) SuspensionEffect {
                    const activation = select_activation(env);
                    if (!activation.valid())
                        return SuspensionEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.suspend_execution(engine.userdata, env, activation);
                }
            }.start);
        }

        pub fn abandonActivation(
            comptime select_engine: fn (*const R) *const Self,
            comptime select_activation: fn (*const R) types.Activation,
        ) SuspensionEffect {
            return SuspensionEffect.deferEffect(struct {
                fn start(env: *const R) SuspensionEffect {
                    const activation = select_activation(env);
                    if (!activation.valid())
                        return SuspensionEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.abandon_activation(engine.userdata, env, activation);
                }
            }.start);
        }
    };
}
