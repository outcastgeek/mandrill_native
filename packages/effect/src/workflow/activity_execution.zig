//! Replays completed activity outcomes through a caller-supplied engine, or
//! executes and records one full typed Exit under a fenced activation.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const engine_module = @import("workflow_engine.zig");
const types = @import("workflow_types.zig");

/// Specialize the workflow failure variants from their typed channels.
/// Parameters:
/// - `EngineError: comptime type`: Compile-time typed engine failure.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// Returns (`type`): typed activity execution failure distinguishing domain and recorded-work errors.
pub fn RunError(comptime EngineError: type, comptime CodecError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(EngineError) or outcomes.ownsPayload(CodecError);
        engine: EngineError,
        codec: CodecError,
        invalid_identity,
        incompatible_version,
        conflicting_record,
        stale_activation,
        uncertain_record,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .engine => outcomes.discardPayload(&self.engine),
                .codec => outcomes.discardPayload(&self.codec),
                else => {},
            }
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .engine => |value| .{ .engine = try outcomes.clonePayload(value) },
                .codec => |value| .{ .codec = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

/// Specialize the typed recorded outcome for workflow operations.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): activity result together with its durable execution record.
pub fn Recorded(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        source: enum { executed, replayed },
        exit: outcomes.Exit(A, E),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.exit.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            const copy = self.exit.clone();
            if (copy == .die and copy.die.kind == .out_of_memory and
                !(self.exit == .die and self.exit.die.kind == .out_of_memory))
                return error.OutOfMemory;
            return .{ .source = self.source, .exit = copy };
        }
    };
}

/// A completed record is decoded instead of rerunning its body. An absent
/// record permits execution; only an acknowledged record is replayable.
/// `possibly_recorded` remains explicit uncertainty for caller read-back.
///
/// Parameters:
/// - `handler: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where static.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `CodecError: comptime type`: Failure type returned by the selected encoder or decoder.
/// - `guarantee: comptime engine_module.Guarantee`: Compile-time volatility or recoverability promised by this provider; durable compositions check it before compiling.
/// - `select_engine: comptime fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_activation: comptime fn (*const R) types.Activation`: Compile-time typed selector from the current environment; its result
///   must remain valid through use.
/// - `select_activity: comptime fn (*const R) types.ActivityId`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_payload: comptime fn (*const R) *const @TypeOf(handler).ActivityDefinition.Payload`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `encode_exit: comptime fn (*const outcomes.Exit(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// - `decode_exit: comptime fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError), CodecError)`: Compile-time typed function selected at comptime; no runtime callback lookup
///   is required.
/// Returns (`effects.Effect(Recorded(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError), RunError(engine_module.WorkflowEngine(R, ProviderError, guarantee).Failure, CodecError), R)`): activity outcome obtained from a recorded result or a newly committed run.
pub fn runRecorded(
    comptime handler: anytype,
    comptime R: type,
    comptime ProviderError: type,
    comptime CodecError: type,
    comptime guarantee: engine_module.Guarantee,
    comptime select_engine: fn (*const R) *const engine_module.WorkflowEngine(R, ProviderError, guarantee),
    comptime select_activation: fn (*const R) types.Activation,
    comptime select_activity: fn (*const R) types.ActivityId,
    comptime select_payload: fn (*const R) *const @TypeOf(handler).ActivityDefinition.Payload,
    comptime encode_exit: fn (*const outcomes.Exit(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError), *const R) outcomes.Exit(outcomes.OwnedSlice(u8), CodecError),
    comptime decode_exit: fn ([]const u8, *const R) outcomes.Exit(outcomes.Exit(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError), CodecError),
) effects.Effect(Recorded(@TypeOf(handler).ActivityDefinition.Success, @TypeOf(handler).ActivityDefinition.DomainError), RunError(engine_module.WorkflowEngine(R, ProviderError, guarantee).Failure, CodecError), R) {
    const Definition = @TypeOf(handler).ActivityDefinition;
    const A = Definition.Success;
    const E = Definition.DomainError;
    const Engine = engine_module.WorkflowEngine(R, ProviderError, guarantee);
    const Failure = RunError(Engine.Failure, CodecError);
    const Output = Recorded(A, E);
    const Out = effects.Effect(Output, Failure, R);
    const Encoded = outcomes.OwnedSlice(u8);
    const Capture = struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        outcome: outcomes.Exit(A, E),
        encoded: Encoded,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *@This()) void {
            self.outcome.deinit();
            self.encoded.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const copy = self.outcome.clone();
            if (copy == .die and copy.die.kind == .out_of_memory and
                !(self.outcome == .die and self.outcome.die.kind == .out_of_memory))
                return error.OutOfMemory;
            var owned = copy;
            errdefer owned.deinit();
            return .{ .outcome = owned, .encoded = try self.encoded.clone() };
        }
    };
    const Adapter = struct {
        fn engineError(problem: Engine.Failure) Failure {
            return .{ .engine = problem };
        }

        fn codecError(problem: CodecError) Failure {
            return .{ .codec = problem };
        }

        fn impossible(_: void) Failure {
            unreachable;
        }

        fn replay(record: types.RecordedActivity, env: *const R) Out {
            var owned = record;
            if (owned.activity_version != handler.definition.version) {
                owned.deinit();
                return Out.fromOwnedOutcome(env, .{ .fail = .incompatible_version });
            }
            const decoded = decode_exit(owned.outcome.items, env);
            owned.deinit();
            const mapped = outcomes.mapError(Failure, decoded, env.allocator, codecError);
            return switch (mapped) {
                .success => |exit| Out.fromOwnedSuccess(env, .{ .source = .replayed, .exit = exit }),
                else => Out.fromOwnedOutcome(env, outcomes.castFailure(Output, mapped)),
            };
        }

        fn committed(captured: *const Capture, status: types.RecordResult, env: *const R) Out {
            return switch (status) {
                .recorded, .already_recorded => blk: {
                    const copy = captured.outcome.clone();
                    if (copy == .die and copy.die.kind == .out_of_memory and
                        !(captured.outcome == .die and captured.outcome.die.kind == .out_of_memory))
                        break :blk Out.die(outcomes.Defect.out_of_memory);
                    break :blk Out.fromOwnedSuccess(env, .{ .source = .executed, .exit = copy });
                },
                .conflicting_record => Out.fromOwnedOutcome(env, .{ .fail = .conflicting_record }),
                .stale_activation => Out.fromOwnedOutcome(env, .{ .fail = .stale_activation }),
                .possibly_recorded => Out.fromOwnedOutcome(env, .{ .fail = .uncertain_record }),
            };
        }

        fn executed(value: outcomes.Exit(A, E), env: *const R) Out {
            // A locally interrupted attempt has not completed. Replay must
            // run it again rather than memoizing a suspension/cancellation.
            if (value == .interrupted) return Out.fromOwnedOutcome(env, .interrupted);
            var owned = value;
            const prepared = encode_exit(&owned, env);
            const encoded = switch (prepared) {
                .success => |bytes| bytes,
                else => {
                    owned.deinit();
                    const mapped = outcomes.mapError(Failure, outcomes.castFailure(Encoded, prepared), env.allocator, codecError);
                    return Out.fromOwnedOutcome(env, outcomes.castFailure(Output, mapped));
                },
            };
            const capture: Capture = .{ .outcome = owned, .encoded = encoded };
            const engine = select_engine(env);
            var recording = engine.vtable.record_activity(
                engine.userdata,
                env,
                select_activation(env),
                select_activity(env),
                handler.definition.version,
                capture.encoded.items,
            );
            var mapped = recording.mapErrorOwned(env, engineError);
            return mapped.flatMapCapturedOwned(env, Output, capture, committed);
        }

        fn loaded(record: ?types.RecordedActivity, env: *const R) Out {
            if (record) |existing| return replay(existing, env);
            var body = handler.body(select_payload(env), env);
            var settled = body.materializeOwned(env);
            var aligned = settled.mapErrorOwned(env, impossible);
            return aligned.flatMapOwned(env, executed);
        }
    };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            const activation = select_activation(env);
            const activity = select_activity(env);
            if (!activation.valid() or activity.validate() == .err)
                return Out.fromOwnedOutcome(env, .{ .fail = .invalid_identity });
            const engine = select_engine(env);
            var reading = engine.vtable.lookup_activity(engine.userdata, env, activation.execution, activity);
            var mapped = reading.mapErrorOwned(env, Adapter.engineError);
            return mapped.flatMapOwned(env, Adapter.loaded);
        }
    }.start);
}
