const std = @import("std");
const fx = @import("effect");
pub fn main() void {
    _ = fx.dsl.rpcUnaryDefinition(.{ .name = "call", .version = 1, .request = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value });
}
