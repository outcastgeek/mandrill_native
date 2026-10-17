const std = @import("std");
const fx = @import("effect");
const C = struct { offset: u16 };
const R = struct { allocator: std.mem.Allocator };
const E = enum { failure };
const Callbacks = struct {
    fn decode(_: *const C, input: anytype, _: *const R) fx.Effect(@TypeOf(input.*), E, R) {
        return fx.Effect(@TypeOf(input.*), E, R).succeed(input.*);
    }
    fn encode(_: *const C, input: anytype, _: *const R) fx.Effect(@TypeOf(input.*), E, R) {
        return fx.Effect(@TypeOf(input.*), E, R).succeed(input.*);
    }
};
pub fn main() void {
    const env: R = .{ .allocator = std.heap.page_allocator };
    var described = fx.dsl.makeCodec(&env, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.contextual, .decode = Callbacks.decode, .encode = Callbacks.encode }), .inputs = C{ .offset = 1 } });
    described.deinit();
}
