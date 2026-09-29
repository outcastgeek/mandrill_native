//! A typed, environment-aware consumer of a scoped Stream. Construction is
//! comptime, so running a sink introduces no dispatch layer or second runtime.
const effects = @import("effect.zig");

pub fn Sink(comptime A: type, comptime E: type, comptime R: type) type {
    return struct {
        const Self = @This();
        consume: *const fn (A, *const R) effects.Effect(void, E, R),

        pub fn init(comptime consume: fn (A, *const R) effects.Effect(void, E, R)) Self {
            return .{ .consume = consume };
        }

        pub fn run(comptime self: Self, comptime source: effects.Stream(A, E, R)) effects.Effect(void, E, R) {
            return source.runForEach(self.consume);
        }
    };
}
