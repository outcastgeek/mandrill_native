const std = @import("std");
const fx = @import("effect");
const Cfg = struct { offset: u16 };
const D = struct { allocator: std.mem.Allocator };
const W = struct { allocator: std.mem.Allocator, extra: bool };
const E = enum { expected };
const Child = struct {
    pub const effect_owned_payload = true;
    pub fn deinit(_: *@This()) void {}
};
const Value = struct {
    pub const effect_owned_payload = true;
    child: Child,
    pub fn deinit(self: *@This()) void {
        self.child.deinit();
    }
};

const Callbacks = struct {
    fn decode(_: *const Cfg, _: *const u16, _: *const D) fx.Effect(Value, E, D) {
        return fx.Effect(Value, E, D).die(fx.Defect.init("explicit codec test"));
    }
    fn encode(_: *const Cfg, _: *const Value, _: *const W) fx.Effect(u16, E, W) {
        return fx.Effect(u16, E, W).die(fx.Defect.init("explicit codec test"));
    }
};
pub fn main() void {
    const env: D = .{ .allocator = std.heap.page_allocator };
    var described = fx.dsl.makeCodec(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.contextual, .decode = Callbacks.decode, .encode = Callbacks.encode }),
        .inputs = Cfg{ .offset = 3 },
    });
    described.deinit();
}
