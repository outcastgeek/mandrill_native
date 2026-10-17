const fx = @import("effect");
const R = struct {};
const P = fx.Effect(u32, void, R);
pub fn main() !void {
    const env: R = .{};
    var result = fx.run(&env, P.succeed(1));
    defer result.deinit();
    switch (result) {
        .success => {},
        else => return error.Failed,
    }
}
