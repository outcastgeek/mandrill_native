const std = @import("std");
pub const Payload = struct {
    pub const effect_owned_payload = true;
    bytes: []u8,
    allocator: std.mem.Allocator,
    pub fn init(allocator: std.mem.Allocator) error{OutOfMemory}!@This() {
        return .{ .bytes = try allocator.dupe(u8, "imported owner"), .allocator = allocator };
    }
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .bytes = try self.allocator.dupe(u8, self.bytes), .allocator = self.allocator };
    }
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        self.* = undefined;
    }
};
comptime {
    if (!@hasDecl(Payload, "effect_owned_payload")) @compileError("defining file ownership evidence missing");
}
