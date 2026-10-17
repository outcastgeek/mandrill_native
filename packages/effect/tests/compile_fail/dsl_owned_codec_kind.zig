const std = @import("std");
const fx = @import("effect");
const Cfg = struct { offset: u16 };
const D = struct { allocator: std.mem.Allocator };
const W = struct { allocator: std.mem.Allocator, extra: bool };
const E = enum { expected };

const Callbacks = struct {
    fn decode(_: *const Cfg, _: *const u16, _: *const D) fx.Effect(u32, E, D) {
        return fx.Effect(u32, E, D).die(fx.Defect.init("explicit codec test"));
    }
    fn encode(_: *const Cfg, _: *const u32, _: *const W) fx.Effect(u16, E, W) {
        return fx.Effect(u16, E, W).die(fx.Defect.init("explicit codec test"));
    }
};
pub fn main() void {
    const env: D = .{ .allocator = std.heap.page_allocator };
    var described = fx.dsl.makeCodec(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = Callbacks.decode, .encode = Callbacks.encode }),
        .inputs = Cfg{ .offset = 3 },
    });
    described.deinit();
}
