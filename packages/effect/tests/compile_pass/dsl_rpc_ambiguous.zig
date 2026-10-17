const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, offset: i64 };
const Other = struct { allocator: std.mem.Allocator, extra: bool };
const integer = fx.JsonCodec.signed_integer;
const nothing = fx.JsonCodec.null_value;
fn handle(input: anytype, env: anytype) fx.Effect(@TypeOf(input.*), void, @typeInfo(@TypeOf(env)).pointer.child) {
    return fx.Effect(@TypeOf(input.*), void, @typeInfo(@TypeOf(env)).pointer.child).succeed(input.* + env.offset);
}
pub fn main() void {
    _ = fx.dsl.rpcUnary(.{ .name = "ambiguous", .version = 1, .request = integer, .success = integer, .failure = nothing, .handler = handle, .environment = Env });
}
