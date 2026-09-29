//! Reentrant ownership is a caller-provided logical execution identity.
//! Native thread IDs are deliberately absent because execution can migrate.
const std = @import("std");
const tx = @import("transaction.zig");

pub fn ReentrantLock(comptime Owner: type) type {
    if (!tx.immutableValue(Owner)) @compileError("lock owners must be stable immutable values");
    return struct {
        const Self = @This();
        const State = struct { owner: ?Owner = null, depth: usize = 0 };
        pub const Acquire = enum { acquired, reentered, busy, depth_exhausted };
        pub const Release = enum { released, still_owned, not_owner };

        reference: tx.Ref(State),

        pub fn init(domain: *tx.Domain) Self {
            return .{ .reference = tx.Ref(State).init(domain, .{}) };
        }

        pub fn acquire(self: *Self, journal: *tx.Domain.Journal, owner: Owner) ?Acquire {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (state.owner) |current| {
                if (!std.meta.eql(current, owner)) return .busy;
                if (state.depth == std.math.maxInt(usize)) return .depth_exhausted;
                state.depth += 1;
                journal.write(State, &self.reference, state);
                return .reentered;
            }
            state.owner = owner;
            state.depth = 1;
            journal.write(State, &self.reference, state);
            return .acquired;
        }

        pub fn release(self: *Self, journal: *tx.Domain.Journal, owner: Owner) ?Release {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            const current = state.owner orelse return .not_owner;
            if (!std.meta.eql(current, owner)) return .not_owner;
            state.depth -= 1;
            if (state.depth == 0) {
                state.owner = null;
                journal.write(State, &self.reference, state);
                return .released;
            }
            journal.write(State, &self.reference, state);
            return .still_owned;
        }
    };
}
