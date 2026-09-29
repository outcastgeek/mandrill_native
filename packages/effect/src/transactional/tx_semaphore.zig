//! Transactional permit accounting. Blocked acquisitions return `waiting`;
//! their enclosing transaction chooses retry on the observed reference.
const tx = @import("transaction.zig");

pub const ConfigurationError = enum { initial_exceeds_maximum };
pub const Acquire = enum { acquired, waiting, exceeds_maximum };
pub const Release = enum { released, exceeds_maximum };

pub const Semaphore = struct {
    const Self = @This();
    const State = struct { available: usize, maximum: usize };
    reference: tx.Ref(State),

    pub fn init(domain: *tx.Domain, initial: usize, maximum: usize) @import("../outcome.zig").Result(Self, ConfigurationError) {
        if (initial > maximum) return .{ .err = .initial_exceeds_maximum };
        return .{ .ok = .{ .reference = tx.Ref(State).init(domain, .{
            .available = initial,
            .maximum = maximum,
        }) } };
    }

    pub fn available(self: *Self, journal: *tx.Domain.Journal) ?usize {
        return switch (journal.read(State, &self.reference)) {
            .ok => |state| state.available,
            .err => null,
        };
    }

    pub fn acquire(self: *Self, journal: *tx.Domain.Journal, permits: usize) ?Acquire {
        var state = switch (journal.read(State, &self.reference)) {
            .ok => |current| current,
            .err => return null,
        };
        if (permits > state.maximum) return .exceeds_maximum;
        if (permits > state.available) return .waiting;
        state.available -= permits;
        journal.write(State, &self.reference, state);
        return .acquired;
    }

    pub fn release(self: *Self, journal: *tx.Domain.Journal, permits: usize) ?Release {
        var state = switch (journal.read(State, &self.reference)) {
            .ok => |current| current,
            .err => return null,
        };
        if (permits > state.maximum - state.available) return .exceeds_maximum;
        state.available += permits;
        journal.write(State, &self.reference, state);
        return .released;
    }
};
