const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const operation = fx.dsl.rpcUnaryDefinition(.{ .name = "runtime.contract", .version = 1, .request = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.borrowed_string, .failure = fx.JsonCodec.borrowed_string });
const foreign = fx.dsl.rpcUnaryDefinition(.{ .name = "unpaired", .version = 1, .request = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.borrowed_string, .failure = fx.JsonCodec.borrowed_string });
fn handle(_: anytype, _: *const i64, _: *const Env) fx.Effect([]const u8, []const u8, Env) {
    return fx.Effect([]const u8, []const u8, Env).succeed("reply");
}
const Use = struct {
    fn run(registry: anytype, inputs: anytype, _: *const Env) @TypeOf(registry.invoke(operation, inputs.request)) {
        return registry.invoke(operation, inputs.request);
    }
};
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator };
    var described = fx.dsl.withRpcRegistry(&env, .{ .definition = fx.dsl.declareMetadata(.{ .use = Use.run }), .inputs = .{ .bindings = .{}, .request = @as(i64, 11) } });
    described.deinit();
}
