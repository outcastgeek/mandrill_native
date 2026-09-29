//! Journal/cursor progress remains authoritative; reactive invalidation is a
//! recoverable hint emitted only after an import is known applied or already
//! applied. Repeating an import repairs a notification lost after commit.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const journal = @import("event_journal.zig");
const message = @import("event_log_message.zig");
const atom_registry = @import("../reactivity/atom_registry.zig");
const local = @import("event_log.zig");

pub const ObservationCause = union(enum) {
    failure: atom_registry.Error,
    defect: outcomes.Defect,
    interrupted,
};

pub fn Error(comptime SourceError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(SourceError);
        source: SourceError,
        observation: struct { committed: journal.Cursor, cause: ObservationCause },

        pub fn deinit(self: *Self) void {
            if (self.* == .source) outcomes.discardPayload(&self.source);
            self.* = undefined;
        }

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

pub fn AppendObservationError(comptime SourceError: type, comptime Success: type) type {
    const Result = local.AppendResult(Success);
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        source: SourceError,
        observation: struct { committed: Result, cause: ObservationCause },

        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .source => outcomes.discardPayload(&self.source),
                .observation => self.observation.committed.deinit(),
            }
            self.* = undefined;
        }

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
