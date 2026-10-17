const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, offset: i64 };
const Other = struct { allocator: std.mem.Allocator, distinct: bool };
const policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first };
const Request = fx.dsl.schemaRecord(Env, .{ .fields = .{ .value = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required } }, .policy = policy });
const Success = fx.dsl.schemaRecord(Other, .{ .fields = .{ .value = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required } }, .policy = policy });
fn handle(input: anytype, env: anytype) fx.Effect(Success.Decoded, void, @typeInfo(@TypeOf(env)).pointer.child) {
    return fx.Effect(Success.Decoded, void, @typeInfo(@TypeOf(env)).pointer.child).succeed(.{ .value = input.value + env.offset });
}
pub fn main() void {
    _ = fx.dsl.rpcUnary(.{ .name = "ambiguous.codecs", .version = 1, .request = Request, .success = Success, .failure = fx.JsonCodec.null_value, .handler = handle });
}
