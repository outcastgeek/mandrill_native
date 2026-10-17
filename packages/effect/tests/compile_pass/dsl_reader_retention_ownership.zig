const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator, io: std.Io };
const Reader = fx.StreamReader(i64, void, R);
const Capture = struct {
    pub const effect_owned_payload = true;
    bytes: fx.OwnedSlice(u8),
    pub fn deinit(self: *@This()) void {
        self.bytes.deinit();
    }
};
pub fn main() void {
    const reader: Reader = undefined;
    var retained = Reader.retainCaptureAllocated(std.heap.page_allocator, reader, Capture{ .bytes = undefined }) catch return;
    retained.deinit();
}
