const std = @import("std");
pub const fx = @import("effect");
pub const Env = struct { allocator: std.mem.Allocator, io: std.Io };
pub const operation = fx.dsl.rpcStreamDefinition(.{ .name = "session.fixture", .version = 1, .request = fx.JsonCodec.signed_integer, .chunk = fx.JsonCodec.signed_integer, .terminal = fx.JsonCodec.null_value, .failure = fx.JsonCodec.signed_integer });
const Event = @TypeOf(operation).Event;
fn stream(_: *const i64, _: *const Env) fx.Stream(Event, i64, Env) {
    return fx.Stream(Event, i64, Env).fromPull(fx.Effect(void, i64, Env).succeed({}), struct {
        fn next(_: void) fx.Effect(fx.Chunk(Event), i64, Env) {
            return fx.Effect(fx.Chunk(Event), i64, Env).fromOutcome(struct {
                fn done(_: *const Env) fx.Exit(fx.Chunk(Event), i64) {
                    return .{ .success = .end };
                }
            }.done);
        }
    }.next, struct {
        fn release(_: void) fx.Effect(void, i64, Env) {
            return fx.Effect(void, i64, Env).succeed({});
        }
    }.release);
}
fn Wire(comptime A: type) type {
    return struct {
        fn decode(_: *const fx.OwnedSlice(u8)) fx.Effect(A, fx.SchemaIssue.Issue, Env) {
            return fx.Effect(A, fx.SchemaIssue.Issue, Env).die(fx.Defect.init("fixture"));
        }
        fn encode(_: *const A) fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, Env) {
            return fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, Env).die(fx.Defect.init("fixture"));
        }
        const value = fx.RpcWireCodec(A, Env).init(decode, encode);
    };
}
pub const Server = fx.RpcStreamServer(operation.handledBy(Env, stream), Wire(i64).value, Wire(i64).value, Wire(void).value, Wire(i64).value, u8, struct {
    fn session(_: *const Env) *const fx.RpcSession(Env, u8) {
        unreachable;
    }
}.session, struct {
    fn policy(_: *const Env) fx.RpcSessionPolicy {
        unreachable;
    }
}.policy);
pub fn acquire(_: anytype, _: *const Env) fx.Effect(u32, u8, Env) {
    return fx.Effect(u32, u8, Env).succeed(1);
}
pub fn project(_: anytype, _: *const u32, env: *const Env) Env {
    return env.*;
}
pub fn use(_: anytype, _: *fx.Scope(i64), _: *const Env) fx.Effect(void, Server.Failure, Env) {
    return fx.Effect(void, Server.Failure, Env).succeed({});
}
pub fn environment() Env {
    return .{ .allocator = std.heap.page_allocator, .io = undefined };
}
