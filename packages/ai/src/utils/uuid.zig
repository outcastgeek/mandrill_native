const std = @import("std");
pub fn randomV4(io: std.Io) ![36]u8 {
    var bytes: [16]u8 = undefined;
    try io.randomSecure(&bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    var out: [36]u8 = undefined;
    var writer = std.Io.Writer.fixed(&out);
    for (bytes, 0..) |byte, i| {
        if (i == 4 or i == 6 or i == 8 or i == 10) try writer.writeByte('-');
        try writer.print("{x:0>2}", .{byte});
    }
    return out;
}
pub const Generator = struct {
    mutex: std.Io.Mutex = .init,
    last_timestamp: i64 = -1,
    sequence: ?u64 = null,
    pub fn uuidv7(self: *@This(), io: std.Io, timestamp: ?i64) ![36]u8 {
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        const requested = timestamp orelse std.Io.Clock.real.now(io).toMilliseconds();
        if (requested < 0 or requested > 0xffffffffffff) return error.InvalidTimestamp;
        const effective = if (timestamp == null) @max(requested, self.last_timestamp) else requested;
        if (timestamp == null) self.last_timestamp = effective;
        var bytes: [16]u8 = undefined;
        try io.randomSecure(&bytes);
        var sequence = self.sequence orelse ((@as(u64, bytes[1]) << 32) | (@as(u64, bytes[2]) << 24) | (@as(u64, bytes[3]) << 16) | (@as(u64, bytes[4]) << 8) | bytes[5]);
        if (self.sequence != null) {
            if (sequence == (@as(u64, 1) << 41) - 1) return error.SequenceExhausted;
            sequence += 1;
        }
        self.sequence = sequence;
        std.mem.writeInt(u48, bytes[0..6], @intCast(effective), .big);
        bytes[6] = 0x70 | @as(u8, @intCast((sequence >> 37) & 0xf));
        bytes[7] = @truncate(sequence >> 29);
        bytes[8] = 0x80 | @as(u8, @intCast((sequence >> 23) & 0x3f));
        bytes[9] = @truncate(sequence >> 15);
        bytes[10] = @truncate(sequence >> 7);
        bytes[11] = @as(u8, @intCast((sequence & 0x7f) << 1)) | (bytes[11] & 1);
        var out: [36]u8 = undefined;
        var fixed = std.Io.Writer.fixed(&out);
        for (bytes, 0..) |byte, i| {
            if (i == 4 or i == 6 or i == 8 or i == 10) try fixed.writeByte('-');
            try fixed.print("{x:0>2}", .{byte});
        }
        return out;
    }
};

/// Native direct-module entrypoint retains an explicit Generator owner.
pub const uuidv7 = Generator.uuidv7;
