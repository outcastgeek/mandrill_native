//! Public, provider-neutral encoded WorkflowEngine contract. It does not
//! depend on Cluster or expose a database object to workflow definitions.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const types = @import("workflow_types.zig");
const workflow_error = @import("workflow_error.zig");

/// Storage or provider durability promised by this capability. `recoverable`
/// permits compositions that must survive process restart; `volatile` does not.
pub const Guarantee = enum { @"volatile", recoverable };

/// Specialize the typed workflow error channel from its compile-time error type.
/// Parameters:
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): typed engine failure union separating workflow operation and provider errors.
pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        operation: workflow_error.OperationError,
        provider: ProviderError,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .provider) outcomes.discardPayload(&self.provider);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `guarantee: comptime Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// Returns (`type`): provider-neutral encoded workflow engine with explicit durability guarantee.
pub fn WorkflowEngine(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        /// Compile-time volatility or recoverability promised by this provider.
        pub const storage_guarantee = guarantee;
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = Error(ProviderError);
        /// Lazy Effect for submit effect; success, error, and environment types are fixed by
        /// `effects.Effect(types.SubmitResult, Failure, R)`.
        pub const SubmitEffect = effects.Effect(types.SubmitResult, Failure, R);
        /// Lazy Effect for poll effect; success, error, and environment types are fixed by
        /// `effects.Effect(types.ExecutionState, Failure, R)`.
        pub const PollEffect = effects.Effect(types.ExecutionState, Failure, R);
        /// Lazy Effect for control effect; success, error, and environment types are fixed by
        /// `effects.Effect(types.ControlResult, Failure, R)`.
        pub const ControlEffect = effects.Effect(types.ControlResult, Failure, R);
        /// Lazy Effect for activity effect; success, error, and environment types are fixed by
        /// `effects.Effect(?types.RecordedActivity, Failure, R)`.
        pub const ActivityEffect = effects.Effect(?types.RecordedActivity, Failure, R);
        /// Lazy Effect for record effect; success, error, and environment types are fixed by
        /// `effects.Effect(types.RecordResult, Failure, R)`.
        pub const RecordEffect = effects.Effect(types.RecordResult, Failure, R);
        /// Lazy Effect for activation effect; success, error, and environment types are fixed by
        /// `effects.Effect(types.ActivationResult, Failure, R)`.
        pub const ActivationEffect = effects.Effect(types.ActivationResult, Failure, R);
        /// Lazy Effect for completion effect; success, error, and environment types are fixed by
        /// `effects.Effect(types.CompletionResult, Failure, R)`.
        pub const CompletionEffect = effects.Effect(types.CompletionResult, Failure, R);
        /// Lazy Effect for suspension effect; success, error, and environment types are fixed by
        /// `effects.Effect(types.SuspensionResult, Failure, R)`.
        pub const SuspensionEffect = effects.Effect(types.SuspensionResult, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Submit the supplied work.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.Submission`: typed domain value consumed by this hook.
            /// Returns (`SubmitEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            submit: *const fn (?*anyopaque, *const R, types.Submission) SubmitEffect,
            /// Observe current state.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ExecutionId`: stable typed identity for the selected operation.
            /// Returns (`PollEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            poll: *const fn (?*anyopaque, *const R, types.ExecutionId) PollEffect,
            /// Resume execution through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ResumeRequest`: borrowed observed revision and optional terminal reply, compared atomically at storage reset.
            /// Returns (`ControlEffect`): typed compare-and-set acknowledgement, preserving provider failure/cleanup channels.
            resume_execution: *const fn (?*anyopaque, *const R, types.ResumeRequest) ControlEffect,
            /// Cancel terminal through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ExecutionId`: stable typed identity for the selected operation.
            /// Returns (`ControlEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            cancel_terminal: *const fn (?*anyopaque, *const R, types.ExecutionId) ControlEffect,
            /// May fence a running activation before its owned work has joined.
            /// Callers must opt into this separate, explicitly unsafe action.
            /// Interrupt unsafe through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ExecutionId`: stable typed identity for the selected operation.
            /// Returns (`ControlEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            interrupt_unsafe: *const fn (?*anyopaque, *const R, types.ExecutionId) ControlEffect,
            /// Find a recorded activity.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ExecutionId`: stable typed identity for the selected operation.
            /// - `4: types.ActivityId`: stable typed identity for the selected operation.
            /// Returns (`ActivityEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            lookup_activity: *const fn (?*anyopaque, *const R, types.ExecutionId, types.ActivityId) ActivityEffect,
            /// Store an activity result.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.Activation`: typed domain value consumed by this hook.
            /// - `4: types.ActivityId`: stable typed identity for the selected operation.
            /// - `5: u32`: recorded activity version.
            /// - `6: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`RecordEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            record_activity: *const fn (?*anyopaque, *const R, types.Activation, types.ActivityId, u32, []const u8) RecordEffect,
            /// Acquire a workflow activation.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.ExecutionId`: stable typed identity for the selected operation.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `5: types.ActivationPolicy`: typed domain value consumed by this hook.
            /// Returns (`ActivationEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            activate: *const fn (?*anyopaque, *const R, types.ExecutionId, []const u8, types.ActivationPolicy) ActivationEffect,
            /// Record completion.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.Activation`: typed domain value consumed by this hook.
            /// - `4: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`CompletionEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            complete: *const fn (?*anyopaque, *const R, types.Activation, []const u8) CompletionEffect,
            /// Suspend execution through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.Activation`: typed domain value consumed by this hook.
            /// Returns (`SuspensionEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            suspend_execution: *const fn (?*anyopaque, *const R, types.Activation) SuspensionEffect,
            /// Release only this local activation. The execution remains
            /// queued for recovery and its completed activity records remain.
            /// Abandon activation through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: types.Activation`: typed domain value consumed by this hook.
            /// Returns (`SuspensionEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            abandon_activation: *const fn (?*anyopaque, *const R, types.Activation) SuspensionEffect,
        };

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context passed unchanged to every hook.
        ///   Its owner keeps it alive through any pending effect or operation.
        /// - `vtable: *const VTable`: Borrowed table of exactly typed callbacks; keep it
        ///   alive for every use of this capability, including deferred execution.
        /// Returns (`Self`): A non-owning adapter referencing `data` and `vtable`; no
        ///   provider state is allocated or transferred by this constructor.
        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        /// Submit typed work to the selected provider.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_submission: comptime fn (*const R) types.Submission`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`SubmitEffect`): durable workflow submission acknowledgement keyed by execution identity.
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

        /// Observe the current completion state.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`PollEffect`): current recorded workflow state or completed outcome.
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

        /// Poll and resume the observed running/suspended checkpoint with its revision fence.
        /// Completed/queued/terminal runs return already_recorded and are not reset or replayed.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ControlEffect`): typed acknowledgement of resuming the selected suspended execution.
        pub fn resumeExecution(comptime select_engine: fn (*const R) *const Self, comptime select_execution: fn (*const R) types.ExecutionId) ControlEffect {
            return ControlEffect.deferEffect(struct {
                fn start(env: *const R) ControlEffect {
                    var observed = poll(select_engine, select_execution);
                    return observed.flatMapOwned(env, struct {
                        fn resumeObserved(state: types.ExecutionState, current: *const R) ControlEffect {
                            var owned = state;
                            defer owned.deinit();
                            const revision = switch (state) {
                                .suspended, .running, .cancellation_requested => |value| value,
                                .absent => return ControlEffect.succeed(.absent),
                                else => return ControlEffect.succeed(.already_recorded),
                            };
                            const engine = select_engine(current);
                            return engine.vtable.resume_execution(engine.userdata, current, .{ .execution = select_execution(current), .expected_revision = revision });
                        }
                    }.resumeObserved);
                }
            }.start);
        }
        /// Lazily submit the caller’s observed checkpoint revision and optional terminal bytes.
        /// Borrow request identity/bytes until join; the provider atomically compares them with its reset.
        /// A stale comparison returns stale_revision without mutation; other provider Exit channels remain intact.
        pub fn resumeChecked(comptime select_engine: fn (*const R) *const Self, comptime select_request: fn (*const R) types.ResumeRequest) ControlEffect {
            return ControlEffect.deferEffect(struct {
                fn start(env: *const R) ControlEffect {
                    const request = select_request(env);
                    if (request.execution.validate() == .err) return ControlEffect.fail(.{ .operation = .malformed_record });
                    const engine = select_engine(env);
                    return engine.vtable.resume_execution(engine.userdata, env, request);
                }
            }.start);
        }

        /// Request cooperative terminal cancellation.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ControlEffect`): typed acknowledgement of terminal cancellation.
        pub fn cancelTerminal(comptime select_engine: fn (*const R) *const Self, comptime select_execution: fn (*const R) types.ExecutionId) ControlEffect {
            return control(select_engine, select_execution, true);
        }

        /// Immediately fence an activation; external work may still run.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ControlEffect`): typed acknowledgement of an explicit unsafe interruption request.
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
                    if (!terminal) @compileError("resume requires an expected checkpoint revision");
                    return engine.vtable.cancel_terminal(engine.userdata, env, execution);
                }
            }.start);
        }

        /// Look up a previously recorded workflow activity.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_activity: comptime fn (*const R) types.ActivityId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`ActivityEffect`): recorded activity result for the supplied stable activity identity.
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

        /// Record an activity result under its activation fence.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_activation: comptime fn (*const R) types.Activation`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_activity: comptime fn (*const R) types.ActivityId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_version: comptime fn (*const R) u32`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_outcome: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`RecordEffect`): typed acknowledgement after recording an activity outcome.
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

        /// Claim a workflow activation under the selected policy.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_execution: comptime fn (*const R) types.ExecutionId`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_owner_id: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_policy: comptime fn (*const R) types.ActivationPolicy`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`ActivationEffect`): lease-fenced activation of workflow work ready to run.
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

        /// Record successful completion of the operation.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_activation: comptime fn (*const R) types.Activation`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_outcome: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`CompletionEffect`): terminal workflow outcome committed under the current activation.
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

        /// Suspend the selected workflow execution.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_activation: comptime fn (*const R) types.Activation`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`SuspensionEffect`): typed acknowledgement after suspending the active workflow.
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

        /// Release a local activation for later recovery.
        /// Parameters:
        /// - `select_engine: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_activation: comptime fn (*const R) types.Activation`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`SuspensionEffect`): typed acknowledgement after releasing a stale or failed activation.
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
