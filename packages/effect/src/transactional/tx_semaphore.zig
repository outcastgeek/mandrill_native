//! Transactional permit accounting. Blocked acquisitions return `waiting`;
//! their enclosing transaction chooses retry on the observed reference.
const tx = @import("transaction.zig");

/// `ConfigurationError` cases: `initial_exceeds_maximum`.
pub const ConfigurationError = enum { initial_exceeds_maximum };
/// `Acquire` cases: `acquired`, `waiting`, `exceeds_maximum`.
pub const Acquire = enum { acquired, waiting, exceeds_maximum };
/// `Release` cases: `released`, `exceeds_maximum`.
pub const Release = enum { released, exceeds_maximum };

/// Transaction-backed permit counter with explicit maximum capacity.
pub const Semaphore = struct {
    const Self = @This();
    const State = struct { available: usize, maximum: usize };
    reference: tx.Ref(State),

    /// Create a transactional semaphore with explicit initial and maximum permits.
    /// Parameters:
    /// - `domain: *tx.Domain`: Transaction domain containing the cells and journal for this operation.
    /// - `initial: usize`: Number of permits initially available; it must not exceed `maximum`.
    /// - `maximum: usize`: upper bound on permits; `initial` must not exceed it.
    /// Returns (`@import("../outcome.zig").Result(Self, ConfigurationError)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn init(domain: *tx.Domain, initial: usize, maximum: usize) @import("../outcome.zig").Result(Self, ConfigurationError) {
        if (initial > maximum) return .{ .err = .initial_exceeds_maximum };
        return .{ .ok = .{ .reference = tx.Ref(State).init(domain, .{
            .available = initial,
            .maximum = maximum,
        }) } };
    }

    /// Read available semaphore permits from the journal.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is selected
    ///   here.
    /// Returns (`?usize`): staged available permits; `null` means the journal read failed.
    pub fn available(self: *Self, journal: *tx.Domain.Journal) ?usize {
        return switch (journal.read(State, &self.reference)) {
            .ok => |state| state.available,
            .err => null,
        };
    }

    /// Acquire a resource from the selected capability.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is selected
    ///   here.
    /// - `permits: usize`: Number of permits requested from the semaphore.
    /// Returns (`?Acquire`): acquired/waiting/exceeds-maximum status; `null` means the journal read failed.
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

    /// Return or release the owned resource.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is selected
    ///   here.
    /// - `permits: usize`: Number of permits requested from the semaphore.
    /// Returns (`?Release`): released/exceeds-maximum status; `null` means the journal read failed.
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
