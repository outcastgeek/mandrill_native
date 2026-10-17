const std = @import("std");
pub const fx = @import("effect");
pub const R = struct { allocator: std.mem.Allocator, io: std.Io };
pub fn handle(value: *const i64, _: *const R) fx.Effect(i64, void, R) {
    return fx.Effect(i64, void, R).succeed(value.*);
}
pub const event = fx.dsl.event(.{ .tag = "amount", .version = 1, .payload = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value, .handler = handle });
pub fn fold(state: *i64, value: *const i64) void {
    state.* += value.*;
}
pub const item = fx.dsl.eventCase(event, fold);
pub const limits = .{ .max_document_bytes = 256, .max_depth = 8, .max_value_bytes = 64 };
