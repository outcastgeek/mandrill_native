const fixture = @import("support/dsl_rpc_session.zig");
const fx = fixture.fx;
const Op = @TypeOf(fixture.operation);
const Stream = fx.Stream(Op.Event, i64, fixture.Env);
fn handle(_: anytype, _: *const i64, _: *const fixture.Env) Stream {
    return .{ .open_fn = struct {
        fn open(_: *fx.Scope(i64)) fx.Effect(fx.StreamReader(Op.Event, i64, fixture.Env), i64, fixture.Env) {
            return fx.Effect(fx.StreamReader(Op.Event, i64, fixture.Env), i64, fixture.Env).die(fx.Defect.init("fixture"));
        }
    }.open };
}
pub fn main() void {
    const env = fixture.environment();
    var described = fx.dsl.makeRpcHandler(&env, .{ .definition = fx.dsl.declareMetadata(.{ .operation = fixture.operation, .handler = handle }), .inputs = .{} });
    described.deinit();
}
