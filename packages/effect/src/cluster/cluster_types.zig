//! Logical identities and ownership data, independent of physical topology.
const std = @import("std");
const keys = @import("../primary_key.zig");
const outcomes = @import("../outcome.zig");

pub const IdentityError = union(enum) {
    empty_group,
    invalid_group,
    invalid_entity_key: keys.Error,
    invalid_request_key: keys.Error,
};

pub const EntityAddress = struct {
    group: []const u8,
    entity_key: []const u8,

    pub fn valid(self: EntityAddress) bool {
        return self.group.len > 0 and std.unicode.utf8ValidateSlice(self.group) and
            keys.inspectEncoded(self.entity_key) == .ok;
    }
};

/// Group identifies a registered logical entity definition; the two keys are
/// canonical, length-delimited PrimaryKey encodings, not process-local hashes.
/// An address borrows all three slices until the provider Effect settles.
pub const Address = struct {
    group: []const u8,
    entity_key: []const u8,
    request_key: []const u8,

    pub fn validate(self: Address) outcomes.Result(void, IdentityError) {
        if (self.group.len == 0) return .{ .err = .empty_group };
        if (!std.unicode.utf8ValidateSlice(self.group)) return .{ .err = .invalid_group };
        switch (keys.inspectEncoded(self.entity_key)) {
            .ok => {},
            .err => |problem| return .{ .err = .{ .invalid_entity_key = problem } },
        }
        switch (keys.inspectEncoded(self.request_key)) {
            .ok => {},
            .err => |problem| return .{ .err = .{ .invalid_request_key = problem } },
        }
        return .{ .ok = {} };
    }
};

/// A new generation fences all claims/completions issued to older owners.
/// Providers choose the opaque owner ID; no thread, machine or shard algorithm
/// is prescribed by this contract.
pub const Ownership = struct {
    owner_id: []const u8,
    generation: u64,
};

pub const ClaimToken = struct {
    address: EntityAddress,
    request_key: []const u8,
    owner_id: []const u8,
    generation: u64,
    delivery_attempt: u64,
};
