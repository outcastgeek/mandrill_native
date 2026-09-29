const std = @import("std");
const fx = @import("effect");

const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const ValueCodec = @TypeOf(fx.JsonCodec.signed_integer);
const ErrorCodec = @TypeOf(fx.JsonCodec.null_value);
const Operation = fx.RpcUnary(ValueCodec, ValueCodec, ErrorCodec);
const forged: Operation = .{
    .name = "",
    .version = 1,
    .request = fx.JsonCodec.signed_integer,
    .success = fx.JsonCodec.signed_integer,
    .failure = fx.JsonCodec.null_value,
};

fn echo(value: *const i64, _: *const Env) fx.Effect(i64, void, Env) {
    return fx.Effect(i64, void, Env).succeed(value.*);
}

pub fn main() void {
    _ = forged.handledBy(Env, echo);
}
