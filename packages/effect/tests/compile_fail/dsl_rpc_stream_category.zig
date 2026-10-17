const fx = @import("effect");
const Env = struct {};
const definition = fx.dsl.rpcStreamDefinition(.{ .name = "static.stream", .version = 1, .request = fx.JsonCodec.signed_integer, .chunk = fx.JsonCodec.signed_integer, .terminal = fx.JsonCodec.null_value, .failure = fx.JsonCodec.signed_integer });
fn handle(_: *const i64, _: *const Env) fx.Effect(i64, i64, Env) {
    return fx.Effect(i64, i64, Env).succeed(1);
}
pub fn main() void {
    _ = fx.dsl.rpcHandled(.{ .definition = definition, .handler = handle });
}
