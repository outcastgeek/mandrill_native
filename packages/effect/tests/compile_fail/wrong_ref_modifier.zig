const fx = @import("effect");
const std = @import("std");
fn wrong(value: u32) struct { result: bool, value: u64 } {
    return .{ .result = true, .value = value };
}
pub fn main(init: std.process.Init) void {
    var ref = fx.Ref(u32).init(&.{ .io = init.io }, 1);
    _ = ref.modify(wrong);
}
