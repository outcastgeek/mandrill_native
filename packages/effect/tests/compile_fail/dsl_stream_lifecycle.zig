const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io };
const Number = fx.Effect(u32, E, R);
const source = fx.dsl.streams.yieldEffect(Number.succeed(1));
pub fn main() void {
    const C = struct {
        fn open(_: *const R) fx.Effect(u32, E, R) {
            return Number.succeed(0);
        }
        fn pull(_: u32) fx.Effect(fx.Chunk(u32), E, R) {
            return fx.Effect(fx.Chunk(u32), E, R).fromOutcome(struct {
                fn run(_: *const R) fx.Exit(fx.Chunk(u32), E) {
                    return .{ .success = .end };
                }
            }.run);
        }
    };
    _ = fx.dsl.stream(.{ .open = C.open, .pull = C.pull });
}
