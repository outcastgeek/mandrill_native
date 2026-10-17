const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io };
const Number = fx.Effect(u32, E, R);
const source = fx.dsl.streams.yieldEffect(Number.succeed(1));
pub fn main() void {
    const C = struct {
        fn open(_: *const R) Number {
            return Number.succeed(0);
        }
        fn pull(_: u32) fx.Effect(fx.Chunk(u32), E, void) {
            return fx.Effect(fx.Chunk(u32), E, void).fromOutcome(struct {
                fn run(_: *const void) fx.Exit(fx.Chunk(u32), E) {
                    return .{ .success = .end };
                }
            }.run);
        }
        fn release(_: u32) fx.Effect(void, E, R) {
            return fx.Effect(void, E, R).succeed({});
        }
    };
    _ = fx.dsl.stream(.{ .open = C.open, .pull = C.pull, .release = C.release });
}
