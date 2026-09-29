//! Logical identities and ownership data, independent of physical topology.
const std = @import("std");
const keys = @import("../primary_key.zig");
const outcomes = @import("../outcome.zig");

/// Validation failure for entity, group, operation, or request identity.
pub const IdentityError = union(enum) {
    empty_group,
    invalid_group,
    invalid_entity_key: keys.Error,
    invalid_request_key: keys.Error,
};

/// Stable group and entity-key pair identifying one logical entity.
pub const EntityAddress = struct {
    group: []const u8,
    entity_key: []const u8,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: EntityAddress`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for a nonempty UTF-8 group and a valid encoded entity key.
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

    /// Validate the value against its declared contract.
    /// Parameters:
    /// - `self: Address`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`outcomes.Result(void, IdentityError)`): An ok/err Result; owned payloads must be released according to their type contract.
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

/// Generation and attempt token used to fence a message claim.
pub const ClaimToken = struct {
    address: EntityAddress,
    request_key: []const u8,
    owner_id: []const u8,
    generation: u64,
    delivery_attempt: u64,
};
