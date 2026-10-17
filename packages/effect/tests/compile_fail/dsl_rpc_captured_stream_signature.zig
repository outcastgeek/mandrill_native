const fixture = @import("support/dsl_rpc_session.zig");
const fx = fixture.fx;
const Op = @TypeOf(fixture.operation);
const Stream = fx.Stream(Op.Event, i64, fixture.Env);
fn handle(_: *const fixture.Env) Stream {
    return undefined;
}
pub fn main() void {
    const env = fixture.environment();
    var described = fx.dsl.makeRpcHandler(&env, .{ .definition = fx.dsl.declareMetadata(.{ .operation = fixture.operation, .handler = handle }), .inputs = .{} });
    described.deinit();
}
