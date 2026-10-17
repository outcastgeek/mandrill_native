const fixture = @import("support/dsl_rpc_session.zig");
const fx = fixture.fx;

pub fn main() void {
    const env = fixture.environment();
    var described = fx.dsl.withRpcSession(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .acquire = fixture.acquire, .project = fixture.project, .use = fixture.use }),
        .inputs = .{},
    });
    described.deinit();
}
