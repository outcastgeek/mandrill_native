const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const Child = struct {
    pub const effect_owned_payload = true;
    pub fn deinit(_: *@This()) void {}
};
const Value = struct { child: Child };
const Issue = fx.SchemaIssue.Issue;
const C = struct {
    fn decode(_: *const std.json.Value) fx.Result(Value, Issue) {
        return .{ .err = Issue.pure(.decode, .invalid_value) };
    }
    fn encode(_: *const Value) fx.Result(std.json.Value, Issue) {
        return .{ .ok = .null };
    }
};
const codec = fx.PureCodec(Value, std.json.Value).init(C.decode, C.encode);
const operation = fx.dsl.rpcUnaryDefinition(.{ .name = "owned.contract", .version = 1, .request = fx.JsonCodec.signed_integer, .success = codec, .failure = fx.JsonCodec.null_value });
fn handle(_: anytype, _: *const i64, _: *const Env) fx.Effect(Value, void, Env) {
    return fx.Effect(Value, void, Env).die(fx.Defect.init("fixture"));
}
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator };
    var described = fx.dsl.makeRpcHandler(&env, .{ .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = handle }), .inputs = .{} });
    described.deinit();
}
