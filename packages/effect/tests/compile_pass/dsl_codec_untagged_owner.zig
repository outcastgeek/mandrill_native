const fx = @import("effect");
const E = enum { invalid };
const Owner = struct {
    pub const effect_owned_payload = true;
    live: *usize,
    pub fn deinit(self: *@This()) void {
        self.live.* -= 1;
    }
};
const Value = union(enum) { owner: Owner, code: u8 };
const Rep = struct { live: *usize };
fn decode(input: *const Rep) fx.Result(Value, E) {
    input.live.* += 1;
    return .{ .ok = .{ .owner = .{ .live = input.live } } };
}
fn encode(value: *const Value) fx.Result(Rep, E) {
    return switch (value.*) {
        .owner => |owner| .{ .ok = .{ .live = owner.live } },
        .code => .{ .err = .invalid },
    };
}
pub fn main() void {
    _ = fx.dsl.codec(.{ .kind = .pure, .decode = decode, .encode = encode });
}
