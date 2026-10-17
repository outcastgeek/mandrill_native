const fixture = @import("support/dsl_rpc_session.zig");
const fx = fixture.fx;
fn wrong(_: anytype, _: *fx.Scope(i64), _: *const fixture.Env) u32 {
    return 1;
}
pub fn main() void {
    const env = fixture.environment();
    var described = fx.dsl.withRpcSession(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .server = fixture.Server, .acquire = fixture.acquire, .project = fixture.project, .use = wrong }),
        .inputs = .{},
    });
    described.deinit();
}
