//! Typed replay over authoritative journal pages. Pure folds are separate
//! from event handlers and never publish a checkpoint on partial failure.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const events = @import("event.zig");
const journal = @import("event_journal.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

pub fn Error(comptime FoldError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        fold: FoldError,
        schema: Issue,
        unknown_event,
        incompatible_version: struct { requested: u32, supported: u32 },
        history_gap,
        malformed_page,

        pub fn deinit(self: *Self) void {
            if (self.* == .fold) outcomes.discardPayload(&self.fold);
            if (self.* == .schema) self.schema.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .fold => |value| .{ .fold = try outcomes.clonePayload(value) },
                .schema => |value| .{ .schema = try value.clone() },
                else => self.*,
            };
        }
    };
}

pub fn Replayed(comptime State: type) type {
    return struct {
        const Self = @This();
        pub const effect_owned_payload = true;
        state: State,
        cursor: journal.Cursor,

        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(&self.state);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return .{ .state = try outcomes.clonePayload(self.state), .cursor = self.cursor };
        }
    };
}

/// `decode` receives a concrete handler and returns Exit(Payload, Issue).
/// `fold` receives that same handler, a private mutable state and a borrowed
/// payload, returning Result(void, FoldError). A failed fold discards the
/// private clone; the caller's prior projection state is untouched.
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
