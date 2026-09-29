//! Option is an explicit tagged representation, distinct from JSON null and
//! from an absent record field.
const outcomes = @import("../outcome.zig");
const Issue = @import("schema_issue.zig").Issue;
const PureCodec = @import("codec.zig").PureCodec;

pub fn Representation(comptime Encoded: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(Encoded);
        none,
        some: Encoded,

        pub fn deinit(self: *Self) void {
            if (self.* == .some) outcomes.discardPayload(&self.some);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .none => .none,
                .some => |value| .{ .some = try outcomes.clonePayload(value) },
            };
        }
    };
}

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
