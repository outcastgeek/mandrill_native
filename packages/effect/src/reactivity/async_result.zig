//! Reactive state retains typed failure classification and previous success.
const outcomes = @import("../outcome.zig");

pub fn Failure(comptime E: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        domain: E,
        defect: outcomes.Defect,
        interrupted,
        finalization_failed: outcomes.Finalization(E),

        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .domain => outcomes.discardPayload(&self.domain),
                .finalization_failed => {
                    var owned: outcomes.Exit(void, E) = .{ .finalization_failed = self.finalization_failed };
                    owned.deinit();
                },
                else => {},
            }
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .domain => |value| .{ .domain = try outcomes.clonePayload(value) },
                .finalization_failed => |value| blk: {
                    const wrapped: outcomes.Exit(void, E) = .{ .finalization_failed = value };
                    const copy = wrapped.clone();
                    if (copy == .die) return error.OutOfMemory;
                    break :blk .{ .finalization_failed = copy.finalization_failed };
                },
                else => self.*,
            };
        }
    };
}

pub fn AsyncResult(comptime A: type, comptime E: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;

        initial,
        waiting: ?A,
        success: A,
        refreshing: A,
        failure: struct { cause: Failure(E), previous: ?A },

        /// Moves the completed Exit into this state. Previous success is
        /// retained only for failures; a new success releases the old owner.
        pub fn fromExit(result: outcomes.Exit(A, E), previous: ?A) Self {
            return switch (result) {
                .success => |value| blk: {
                    var old = previous;
                    outcomes.discardPayload(&old);
                    break :blk .{ .success = value };
                },
                .fail => |err| .{ .failure = .{ .cause = .{ .domain = err }, .previous = previous } },
                .die => |defect| .{ .failure = .{ .cause = .{ .defect = defect }, .previous = previous } },
                .interrupted => .{ .failure = .{ .cause = .interrupted, .previous = previous } },
                .finalization_failed => |details| .{ .failure = .{ .cause = .{ .finalization_failed = details }, .previous = previous } },
            };
        }

        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .waiting => outcomes.discardPayload(&self.waiting),
                .success => outcomes.discardPayload(&self.success),
                .refreshing => outcomes.discardPayload(&self.refreshing),
                .failure => {
                    self.failure.cause.deinit();
                    outcomes.discardPayload(&self.failure.previous);
                },
                .initial => {},
            }
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .initial => .initial,
                .waiting => |value| .{ .waiting = try outcomes.clonePayload(value) },
                .success => |value| .{ .success = try outcomes.clonePayload(value) },
                .refreshing => |value| .{ .refreshing = try outcomes.clonePayload(value) },
                .failure => |value| blk: {
                    var cause = try value.cause.clone();
                    errdefer cause.deinit();
                    break :blk .{ .failure = .{ .cause = cause, .previous = try outcomes.clonePayload(value.previous) } };
                },
            };
        }
    };
}
