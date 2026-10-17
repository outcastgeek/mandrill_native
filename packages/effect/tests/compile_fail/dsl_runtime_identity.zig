const std = @import("std");
const fx = @import("effect");
pub fn main() void {
    var name: [4]u8 = "call".*;
    std.mem.doNotOptimizeAway(&name);
    _ = fx.dsl.rpcUnaryDefinition(.{ .name = &name, .version = 1, .request = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value });
}
