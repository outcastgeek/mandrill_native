const std = @import("std");
const fx = @import("effect");
const State = struct { refs: std.atomic.Value(usize) = .init(1) };
const Retention = struct {
    fn retain(state: *State) *State {
        _ = state.refs.fetchAdd(1, .monotonic);
        return state;
    }
    fn release(state: **State) void {
        _ = state.*.refs.fetchSub(1, .acq_rel);
    }
};
const retention = .{ .retain = Retention.retain, .release = Retention.release, .synchronization = fx.dsl.ownership.Synchronization.thread_safe };
const Shared = @TypeOf(fx.dsl.ownership.retained(@as(*State, undefined), retention));
const Configuration = struct { state: Shared };
const Env = struct { allocator: std.mem.Allocator, parallel_engine: fx.Executor };
const Callbacks = struct {
    fn decode(_: *const Configuration, _: *const u16, _: *const Env) fx.Effect(u32, void, Env) {
        return fx.Effect(u32, void, Env).succeed(7);
    }
    fn encode(_: *const Configuration, _: *const u32, _: *const Env) fx.Effect(u16, void, Env) {
        return fx.Effect(u16, void, Env).succeed(7);
    }
};
pub fn main() void {
    var state: State = .{};
    const env: Env = .{ .allocator = std.heap.page_allocator, .parallel_engine = .{
        .userdata = null,
        .capabilities = .{ .independent = true, .cpu = true, .blocking = false },
        .submit_fn = struct {
            fn submit(_: ?*anyopaque, _: fx.ExecutionKind, _: fx.ExecutionJob) fx.Executor.Submission {
                return .{ .rejected = fx.Defect.init("fixture admission rejected") };
            }
        }.submit,
    } };
    var described = fx.dsl.makeCodec(&env, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.contextual, .decode = Callbacks.decode, .encode = Callbacks.encode }), .inputs = Configuration{ .state = fx.dsl.ownership.retained(&state, retention) } });
    defer described.deinit();
    var made = fx.run(&env, described);
    defer made.deinit();
    const Codec = @TypeOf(made).Success;
    const Forged = struct {
        pub const runtime_codec_shape = Codec.runtime_codec_shape;
    };
    comptime fx.dsl.ownership.requireConcurrent(Forged, "forged codec");
}
