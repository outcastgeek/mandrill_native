const fixture = @import("support/dsl_rpc_session.zig");
const fx = fixture.fx;
const Op = @TypeOf(fixture.operation);
const Stream = fx.Stream(Op.Event, i64, fixture.Env);
fn handle(_: anytype, _: *const i64, _: *const fixture.Env) fx.Effect(i64, i64, fixture.Env) {
    return fx.Effect(i64, i64, fixture.Env).succeed(1);
}
pub fn main() void {
    const env = fixture.environment();
    var described = fx.dsl.makeRpcHandler(&env, .{ .definition = fx.dsl.declareMetadata(.{ .operation = fixture.operation, .handler = handle }), .inputs = .{} });
    described.deinit();
}
