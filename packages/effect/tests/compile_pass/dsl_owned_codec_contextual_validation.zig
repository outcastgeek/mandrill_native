const std = @import("std");
const fx = @import("effect");
const Cfg = struct { offset: u8 };
const Env = struct { allocator: std.mem.Allocator };
const E = enum { invalid };
const Other = enum { invalid };
const Functions = struct {
    fn decodePure(cfg: *const Cfg, input: *const u8) fx.Result(u16, E) {
        return .{ .ok = @as(u16, input.*) + cfg.offset };
    }
    fn encodePure(cfg: *const Cfg, input: *const u16) fx.Result(u8, E) {
        return .{ .ok = @intCast(input.* - cfg.offset) };
    }
    fn decodeContextual(cfg: *const Cfg, input: *const u8, _: *const Env) fx.Effect(u16, E, Env) {
        return fx.Effect(u16, E, Env).succeed(@as(u16, input.*) + cfg.offset);
    }
    fn encodeContextual(cfg: *const Cfg, input: *const u16, _: *const Env) fx.Effect(u8, E, Env) {
        return fx.Effect(u8, E, Env).succeed(@intCast(input.* - cfg.offset));
    }
};
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator };
    var description = fx.dsl.makeCodec(&env, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = Functions.decodePure, .encode = Functions.encodePure }), .inputs = Cfg{ .offset = 2 } });
    defer description.deinit();
    var made = fx.run(&env, description);
    defer made.deinit();
    std.debug.assert(made == .success);
    var input = made.success.representation(7);
    defer input.deinit();
    var decoded = made.success.decodeDirect(&input);
    defer decoded.deinit();
    var encoded = made.success.validateDirect(&decoded.ok);
    defer encoded.deinit();
}
