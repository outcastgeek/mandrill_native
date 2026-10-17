const std = @import("std");
const fx = @import("effect");
const A = fx.ClusterEntityResourcePlan;
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io };
const Number = fx.Effect(u32, E, R);
const Unit = fx.Effect(void, E, R);
const Service = fx.ClusterEntityResource(u32, E, R);
const Helpers = struct {
    fn value(input: *const u32, _: A.Token, _: *const R) Number {
        return Number.succeed(input.*);
    }
    fn provider(_: *const R) *const Service {
        return &service;
    }
    fn acquire(_: ?*anyopaque, _: *const R, _: A.Token) Number {
        return Number.succeed(1);
    }
    fn release(_: ?*anyopaque, _: *const R, _: u32, _: fx.PrimaryOutcome(E)) Unit {
        return Unit.succeed({});
    }
    fn token(_: *const void, _: *const R) A.Token {
        return .{ .address = .{ .group = "g", .entity_key = "" }, .owner_id = "owner", .generation = 1 };
    }
    fn enter(_: ?*anyopaque, _: A.Token, _: fx.WallInstant) A.Fence.Enter {
        return .{ .held = .{ .userdata = null, .until = .never, .release_fn = unlock } };
    }
    fn unlock(_: ?*anyopaque) void {}
    fn fence(_: *const void, _: *const R) A.Fence {
        return .{ .userdata = null, .enter = enter };
    }
    fn now(_: *const R) fx.WallInstant {
        return .{ .unix_nanoseconds = 0 };
    }
    fn mapError(_: A.Error) E {
        return .expected;
    }
    fn finish(resource: fx.ResourceValue(u32), _: *const R) @TypeOf(A.use(@as(u32, 0), value)) {
        return A.use(resource.value, value);
    }
};
const service = Service.fromProvider(null, &.{ .acquire = Helpers.acquire, .release = Helpers.release });
const source = A.use(@as(u32, 1), Helpers.value);
pub fn main(init: std.process.Init) void {
    const env: R = .{ .allocator = std.heap.page_allocator, .io = init.io };
    var effect = A.run(&env, @as(void, {}), A.acquire(Helpers.provider), .{ .token = Helpers.token, .fence = Helpers.fence, .now = Helpers.now, .map_error = Helpers.mapError });
    effect.deinit();
}
