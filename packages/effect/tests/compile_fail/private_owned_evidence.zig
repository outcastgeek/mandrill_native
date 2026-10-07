const fx = @import("effect");
const fixture = @import("private_owned_fixture.zig");
pub fn main() void {
    _ = fx.requireOwnedPayload(fixture.Payload);
}
