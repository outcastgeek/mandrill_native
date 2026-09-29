//! Option is an explicit tagged representation, distinct from JSON null and
//! from an absent record field.
const outcomes = @import("../outcome.zig");
const Issue = @import("schema_issue.zig").Issue;
const PureCodec = @import("codec.zig").PureCodec;

/// Compute the optional encoded representation of a child codec.
/// Parameters:
/// - `Encoded: comptime type`: Compile-time encoded representation type.
/// Returns (`type`): optional wire representation determined by the child codec.
pub fn Representation(comptime Encoded: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(Encoded);
        none,
        some: Encoded,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .some) outcomes.discardPayload(&self.some);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .none => .none,
                .some => |value| .{ .some = try outcomes.clonePayload(value) },
            };
        }
    };
}

/// Lift the child codec to an optional-value codec.
/// Parameters:
/// - `child: comptime anytype`: Compile-time statically inferred input checked when this operation is instantiated.
/// Returns (`PureCodec(?@TypeOf(child).Decoded, Representation(@TypeOf(child).Representation))`): A typed codec or schema preserving the declared decoded value, representation, and environments.
pub fn option(comptime child: anytype) PureCodec(?@TypeOf(child).Decoded, Representation(@TypeOf(child).Representation)) {
    const C = @TypeOf(child);
    if (!@hasDecl(C, "Decoded") or !@hasDecl(C, "Representation") or
        !@hasDecl(C, "decodeDirect") or !@hasDecl(C, "encodeDirect") or
        !@hasDecl(C, "DecodingEnvironment") or !@hasDecl(C, "EncodingEnvironment") or
        C.DecodingEnvironment != void or C.EncodingEnvironment != void)
        @compileError("option requires a pure child codec");
    const Encoded = Representation(C.Representation);
    return PureCodec(?C.Decoded, Encoded).init(struct {
        fn decode(input: *const Encoded) outcomes.Result(?C.Decoded, Issue) {
            return switch (input.*) {
                .none => .{ .ok = null },
                .some => |*value| switch (child.decodeDirect(value)) {
                    .ok => |decoded| .{ .ok = decoded },
                    .err => |issue| .{ .err = issue },
                },
            };
        }
    }.decode, struct {
        fn encode(value: *const ?C.Decoded) outcomes.Result(Encoded, Issue) {
            if (value.*) |*present| {
                return switch (child.encodeDirect(present)) {
                    .ok => |encoded| .{ .ok = .{ .some = encoded } },
                    .err => |issue| .{ .err = issue },
                };
            }
            return .{ .ok = .none };
        }
    }.encode);
}
