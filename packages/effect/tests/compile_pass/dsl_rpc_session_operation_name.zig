const fixture = @import("support/dsl_rpc_session.zig");
const fx = fixture.fx;
const Definition = fixture.Server.server_definition;
const selected_handler = blk: {
    var selected = Definition.Handler;
    selected.operation.name = "session.valid-mutated";
    break :blk selected;
};
const Selected = fx.RpcStreamServer(selected_handler, Definition.RequestWire, Definition.ChunkWire, Definition.TerminalWire, Definition.FailureWire, Definition.Provider, Definition.SelectSession, Definition.SelectPolicy);
pub fn main() void {
    const env = fixture.environment();
    var described = fx.dsl.withRpcSession(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .server = Selected, .acquire = fixture.acquire, .project = fixture.project, .use = fixture.use }),
        .inputs = .{},
    });
    described.deinit();
}
