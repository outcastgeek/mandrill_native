const std = @import("std");
const fx = @import("effect");

const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const Operation = fx.RpcProtocolBuilder(void, Env).unary("unpaired", 1)
    .request(fx.JsonCodec.signed_integer)
    .success(fx.JsonCodec.signed_integer)
    .failure(fx.JsonCodec.null_value);

pub fn main() void {
    _ = fx.RpcProtocolBuilder(void, Env).registry(.{Operation.definition});
}
