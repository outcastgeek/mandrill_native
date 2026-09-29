//! Typed replay over authoritative journal pages. Pure folds are separate
//! from event handlers and never publish a checkpoint on partial failure.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const events = @import("event.zig");
const journal = @import("event_journal.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

/// Specialize the typed eventlog error channel from its compile-time error type.
/// Parameters:
/// - `FoldError: comptime type`: Compile-time typed projection-fold failure.
/// Returns (`type`): typed projection failure union retaining reducer and checkpoint errors.
pub fn Error(comptime FoldError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        fold: FoldError,
        schema: Issue,
        unknown_event,
        incompatible_version: struct { requested: u32, supported: u32 },
        history_gap,
        malformed_page,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .fold) outcomes.discardPayload(&self.fold);
            if (self.* == .schema) self.schema.deinit();
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .fold => |value| .{ .fold = try outcomes.clonePayload(value) },
                .schema => |value| .{ .schema = try value.clone() },
                else => self.*,
            };
        }
    };
}

/// Specialize projection state together with its replay cursor.
/// Parameters:
/// - `State: comptime type`: State value type retained between operations.
/// Returns (`type`): projection state paired with the cursor reached by replay.
pub fn Replayed(comptime State: type) type {
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        state: State,
        cursor: journal.Cursor,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(&self.state);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return .{ .state = try outcomes.clonePayload(self.state), .cursor = self.cursor };
        }
    };
}

/// `decode` receives a concrete handler and returns Exit(Payload, Issue).
/// `fold` receives that same handler, a private mutable state and a borrowed
/// payload, returning Result(void, FoldError). A failed fold discards the
/// private clone; the caller's prior projection state is untouched.
///
/// Parameters:
/// - `State: comptime type`: State value type retained between operations.
/// - `FoldError: comptime type`: Compile-time typed projection-fold failure.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `initial: *const State`: Borrowed initial projection state; it remains owned by the caller.
/// - `after: journal.Cursor`: Caller-supplied time or cursor boundary used for ordering and retention.
/// - `page: *const journal.ReadPage`: Borrowed journal page processed by this operation.
/// - `decode: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where static.
/// - `fold: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`outcomes.Exit(Replayed(State), Error(FoldError))`): Carries `Replayed(State)` on `.success` or `Error(FoldError)` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn replayPage(
    comptime State: type,
    comptime FoldError: type,
    comptime R: type,
    comptime handlers: anytype,
    env: *const R,
    initial: *const State,
    after: journal.Cursor,
    page: *const journal.ReadPage,
    comptime decode: anytype,
    comptime fold: anytype,
) outcomes.Exit(Replayed(State), Error(FoldError)) {
    const Failure = Error(FoldError);
    const Group = events.Group(handlers);
    if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("projection replay requires caller allocator in R");
    var state = outcomes.clonePayload(initial.*) catch return .{ .die = outcomes.Defect.out_of_memory };
    var keep_state = false;
    defer if (!keep_state) outcomes.discardPayload(&state);
    var cursor = after;
    for (page.entries.items) |*entry| {
        const expected = std.math.add(u64, cursor.sequence, 1) catch return .{ .fail = .history_gap };
        if (entry.cursor.sequence != expected) return .{ .fail = .history_gap };
        const selected = switch (Group.resolve(entry.tag.items, entry.schema_version)) {
            .ok => |index| index,
            .err => |problem| return switch (problem) {
                .unknown_event => .{ .fail = .unknown_event },
                .incompatible_version => |versions| .{ .fail = .{ .incompatible_version = .{
                    .requested = versions.requested,
                    .supported = versions.supported,
                } } },
            },
        };
        inline for (handlers, 0..) |handler, index| {
            if (selected == index) {
                var decoded = decode(handler, env, entry.encoded.items);
                switch (decoded) {
                    .success => {
                        defer decoded.deinit();
                        switch (fold(handler, &state, &decoded.success)) {
                            .ok => {},
                            .err => |problem| return .{ .fail = .{ .fold = problem } },
                        }
                    },
                    .fail => |problem| return .{ .fail = .{ .schema = problem } },
                    .die => |defect| return .{ .die = defect },
                    .interrupted => return .interrupted,
                    .finalization_failed => return outcomes.mapError(Failure, outcomes.castFailure(Replayed(State), decoded), env.allocator, struct {
                        fn map(problem: Issue) Failure {
                            return .{ .schema = problem };
                        }
                    }.map),
                }
            }
        }
        cursor = entry.cursor;
    }
    if (page.next.sequence != cursor.sequence) return .{ .fail = .malformed_page };
    keep_state = true;
    return .{ .success = .{ .state = state, .cursor = cursor } };
}
