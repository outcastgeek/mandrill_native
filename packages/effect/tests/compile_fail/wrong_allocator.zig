const fx = @import("effect");
const Env = struct { allocator: usize };
pub fn main() void {
    const env: Env = .{ .allocator = 0 };
    _ = fx.run(&env, fx.Effect(u16, void, Env).succeed(3));
}
