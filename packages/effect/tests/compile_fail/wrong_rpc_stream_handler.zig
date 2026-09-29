const std = @import("std");
const fx = @import("effect");

const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const Incomplete = fx.RpcProtocolBuilder(void, Env).streaming("stream", 1)
    .request(fx.JsonCodec.signed_integer)
    .chunk(fx.JsonCodec.signed_integer)
    .terminal(fx.JsonCodec.null_value)
    .failure(fx.JsonCodec.null_value);

fn unary(request: *const i64, _: *const Env) fx.Effect(i64, void, Env) {
    return fx.Effect(i64, void, Env).succeed(request.*);
}

pub fn main() void {
    _ = Incomplete.handledBy(unary);
}
