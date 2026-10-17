const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const Other = struct { allocator: std.mem.Allocator, extra: bool };
const Domain = enum { failure };
const integer = fx.JsonCodec.signed_integer;
const nothing = fx.JsonCodec.null_value;
fn handle(_: *const i64, _: *const Env) fx.Effect(i64, void, Env) {
    return fx.Effect(i64, void, Env).succeed(11);
}
pub fn main() void {
    const handler = comptime fx.dsl.rpcUnary(.{ .name = "dsl.contract", .version = 1, .request = integer, .success = integer, .failure = nothing, .handler = handle });
    const Registry = fx.dsl.rpcRegistry(.{handler});
    _ = Registry.resolve("dsl.contract", 1);
    const changed = comptime blk: {
        var codec = integer;
        codec.writer = null;
        break :blk codec;
    };
    const alternate = comptime fx.dsl.rpcUnaryDefinition(.{ .name = "dsl.contract", .version = 1, .request = integer, .success = changed, .failure = nothing });
    const input: i64 = 11;
    _ = Registry.invoke(alternate, &input);
}
