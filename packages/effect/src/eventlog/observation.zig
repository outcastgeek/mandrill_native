//! Journal/cursor progress remains authoritative; reactive invalidation is a
//! recoverable hint emitted only after an import is known applied or already
//! applied. Repeating an import repairs a notification lost after commit.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const journal = @import("event_journal.zig");
const message = @import("event_log_message.zig");
const atom_registry = @import("../reactivity/atom_registry.zig");
const local = @import("event_log.zig");

/// Reason an event-log observation invalidates its reactive dependents.
pub const ObservationCause = union(enum) {
    failure: atom_registry.Error,
    defect: outcomes.Defect,
    interrupted,
};

/// Specialize the typed eventlog error channel from its compile-time error type.
/// Parameters:
/// - `SourceError: comptime type`: Compile-time typed source failure.
/// Returns (`type`): typed failure union for event-log observation and invalidation.
pub fn Error(comptime SourceError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(SourceError);
        source: SourceError,
        observation: struct { committed: journal.Cursor, cause: ObservationCause },

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .source) outcomes.discardPayload(&self.source);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .source => |value| .{ .source = try outcomes.clonePayload(value) },
                .observation => self.*,
            };
        }
    };
}

/// The caller supplies the encoded import operation and a registry-local key.
/// A failed notification reports its already committed cursor; a caller can
/// retry the same import or read the journal/cursor to reconstruct observation.
///
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `import_effect: comptime effects.Effect(message.ImportResult, E, R)`: Lazy Effect importing a replica message.
/// - `select_registry: comptime fn (*const R) *atom_registry.Registry`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(message.ImportResult, Error(E), R)`): replica import outcome followed by invalidation of affected reactive observers.
pub fn importAndInvalidate(
    comptime E: type,
    comptime R: type,
    comptime import_effect: effects.Effect(message.ImportResult, E, R),
    comptime select_registry: fn (*const R) *atom_registry.Registry,
    comptime select_key: fn (*const R) []const u8,
) effects.Effect(message.ImportResult, Error(E), R) {
    const Out = effects.Effect(message.ImportResult, Error(E), R);
    const Adapter = struct {
        fn sourceError(problem: E) Error(E) {
            return .{ .source = problem };
        }

        fn completed(result: message.ImportResult, env: *const R) Out {
            const cursor = switch (result) {
                .applied => |value| value,
                .duplicate => |value| value,
                else => return Out.fromOwnedSuccess(env, result),
            };
            return switch (select_registry(env).invalidate(select_key(env))) {
                .success => Out.fromOwnedSuccess(env, result),
                .fail => |cause| Out.fromOwnedOutcome(env, .{ .fail = .{ .observation = .{ .committed = cursor, .cause = .{ .failure = cause } } } }),
                .die => |defect| Out.fromOwnedOutcome(env, .{ .fail = .{ .observation = .{ .committed = cursor, .cause = .{ .defect = defect } } } }),
                .interrupted => Out.fromOwnedOutcome(env, .{ .fail = .{ .observation = .{ .committed = cursor, .cause = .interrupted } } }),
                .finalization_failed => unreachable,
            };
        }

        fn run(env: *const R) Out {
            var importing = import_effect;
            var mapped = importing.mapErrorOwned(env, sourceError);
            return mapped.flatMapOwned(env, completed);
        }
    };
    return Out.deferEffect(Adapter.run);
}

/// Specialize the failure channel for append plus observer invalidation.
/// Parameters:
/// - `SourceError: comptime type`: Compile-time typed source failure.
/// - `Success: comptime type`: Success payload type represented by the returned component.
/// Returns (`type`): typed failure union combining append and observer-invalidation outcomes.
pub fn AppendObservationError(comptime SourceError: type, comptime Success: type) type {
    const Result = local.AppendResult(Success);
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        source: SourceError,
        observation: struct { committed: Result, cause: ObservationCause },

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .source => outcomes.discardPayload(&self.source),
                .observation => self.observation.committed.deinit(),
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
                .source => |value| .{ .source = try outcomes.clonePayload(value) },
                .observation => |value| .{ .observation = .{
                    .committed = try value.committed.clone(),
                    .cause = value.cause,
                } },
            };
        }
    };
}

/// Invalidate only after a local append is acknowledged committed. If the
/// notification fails, the typed error retains the committed result, which
/// can be reconciled from the journal without repeating the handler.
///
/// Parameters:
/// - `Success: comptime type`: Success payload type represented by the returned component.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `append_effect: comptime effects.Effect(local.AppendResult(Success), E, R)`: Lazy Effect appending a local event.
/// - `select_registry: comptime fn (*const R) *atom_registry.Registry`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(local.AppendResult(Success), AppendObservationError(E, Success), R)`): journal append outcome followed by invalidation of affected reactive observers.
pub fn appendAndInvalidate(
    comptime Success: type,
    comptime E: type,
    comptime R: type,
    comptime append_effect: effects.Effect(local.AppendResult(Success), E, R),
    comptime select_registry: fn (*const R) *atom_registry.Registry,
    comptime select_key: fn (*const R) []const u8,
) effects.Effect(local.AppendResult(Success), AppendObservationError(E, Success), R) {
    const Result = local.AppendResult(Success);
    const Failure = AppendObservationError(E, Success);
    const Out = effects.Effect(Result, Failure, R);
    const Adapter = struct {
        fn sourceError(problem: E) Failure {
            return .{ .source = problem };
        }

        fn completed(result: Result, env: *const R) Out {
            if (result != .committed) return Out.fromOwnedSuccess(env, result);
            return switch (select_registry(env).invalidate(select_key(env))) {
                .success => Out.fromOwnedSuccess(env, result),
                .fail => |cause| Out.fromOwnedOutcome(env, .{ .fail = .{
                    .observation = .{ .committed = result, .cause = .{ .failure = cause } },
                } }),
                .die => |defect| Out.fromOwnedOutcome(env, .{ .fail = .{
                    .observation = .{ .committed = result, .cause = .{ .defect = defect } },
                } }),
                .interrupted => Out.fromOwnedOutcome(env, .{ .fail = .{
                    .observation = .{ .committed = result, .cause = .interrupted },
                } }),
                .finalization_failed => unreachable,
            };
        }

        fn run(env: *const R) Out {
            var appending = append_effect;
            var mapped = appending.mapErrorOwned(env, sourceError);
            return mapped.flatMapOwned(env, completed);
        }
    };
    return Out.deferEffect(Adapter.run);
}
