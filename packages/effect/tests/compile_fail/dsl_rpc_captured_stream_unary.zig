const unary = fx.dsl.rpcUnaryDefinition(.{ .name = "unary.fixture", .version = 1, .request = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.signed_integer });
fn unaryHandler(_: anytype, _: *const i64, _: *const fixture.Env) fx.Effect(i64, i64, fixture.Env) {
    return fx.Effect(i64, i64, fixture.Env).succeed(1);
}
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

fn use(registry: anytype, inputs: anytype, _: *const fixture.Env) @TypeOf(registry.open(inputs.scope, unary, inputs.request)) {
    return registry.open(inputs.scope, unary, inputs.request);
}
pub fn main() void {
    const env = fixture.environment();
    var scope: fx.Scope(i64) = .init(&env);
    var described = fx.dsl.withRpcRegistry(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .use = use }),
        .inputs = .{ .bindings = .{.{ .definition = fx.dsl.declareMetadata(.{ .operation = unary, .handler = unaryHandler }), .inputs = .{} }}, .scope = &scope, .request = @as(i64, 1) },
    });
    described.deinit();
}
