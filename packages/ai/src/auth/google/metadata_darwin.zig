//! Darwin's documented socket ioctl ABI, carried by explicit standard I/O.
//! Interface records use typed native-endian Reader operations. No libc
//! interface enumeration function, external library or credential read.
const std = @import("std");
const native = @import("builtin").cpu.arch.endian();
const Record = struct { name: [16]u8, address: bool = false, googleMac: bool = false };
const Configuration = extern struct { length: i32, pointer: usize align(4) };
const Link = extern struct { length: u8, family: u8, index: u16, kind: u8, nameLength: u8, addressLength: u8, selectorLength: u8 };
fn control(io: std.Io, socket: std.Io.net.Socket, code: u32, arg: *anyopaque) !void {
    const result = (try io.operate(.{ .device_io_control = .{ .file = .{ .handle = socket.handle, .flags = .{ .nonblocking = false } }, .code = code, .arg = arg } })).device_io_control;
    if (result < 0) return error.GoogleNetworkInterfaceQueryFailed;
}
pub fn googleMacAddress(a: std.mem.Allocator, io: std.Io) !bool {
    const address = std.Io.net.IpAddress{ .ip4 = .unspecified(0) };
    const socket = address.bind(io, .{ .mode = .dgram }) catch |err| socket: {
        switch (err) {
            error.AddressFamilyUnsupported, error.ProtocolUnsupportedByAddressFamily, error.ProtocolUnsupportedBySystem => {
                const ipv6 = std.Io.net.IpAddress{ .ip6 = .unspecified(0) };
                break :socket try ipv6.bind(io, .{ .mode = .dgram });
            },
            else => return err,
        }
    };
    defer socket.close(io);
    // Public Darwin _IOWR('i',36,ifconf), with its four-byte packed pointer;
    // _IOWR('i',17,ifreq) uses the fixed 32-byte ifreq flags request.
    const get_configuration: u32 = 0xc0006924 | (@as(u32, @sizeOf(Configuration)) << 16);
    const get_flags: u32 = 0xc0206911;
    var size: usize = 1024;
    while (true) {
        const buffer = try a.alloc(u8, size);
        defer a.free(buffer);
        var configuration = Configuration{ .length = std.math.cast(i32, size) orelse return error.GoogleNetworkInterfaceListTooLarge, .pointer = @intFromPtr(buffer.ptr) };
        try control(io, socket, get_configuration, &configuration);
        if (configuration.length < 0 or @as(usize, @intCast(configuration.length)) > size) return error.GoogleNetworkInterfaceQueryFailed;
        const used: usize = @intCast(configuration.length);
        // A Darwin sockaddr has an eight-bit length, plus the 16-byte name.
        // Leave space for the largest record before treating the list as full.
        if (size - used <= 16 + std.math.maxInt(u8)) {
            size = std.math.mul(usize, size, 2) catch return error.GoogleNetworkInterfaceListTooLarge;
            continue;
        }
        var records: std.ArrayList(Record) = .empty;
        defer records.deinit(a);
        var reader = std.Io.Reader.fixed(buffer[0..used]);
        while (reader.buffered().len > 0) {
            const name = (try reader.takeArray(16)).*;
            const header = try reader.peekArray(2);
            const record_size = @max(@as(usize, 16), header[0]);
            const bytes = try reader.take(record_size);
            const family = header[1];
            const record: *Record = for (records.items) |*item| {
                if (std.mem.eql(u8, &item.name, &name)) break item;
            } else record: {
                try records.append(a, .{ .name = name });
                break :record &records.items[records.items.len - 1];
            };
            if (family == std.c.AF.INET or family == std.c.AF.INET6) record.address = true;
            if (family == std.c.AF.LINK) {
                var link_reader = std.Io.Reader.fixed(bytes);
                const link = try link_reader.takeStruct(Link, native);
                if (link.length > bytes.len or link.length < @sizeOf(Link)) return error.GoogleNetworkInterfaceQueryFailed;
                link_reader = std.Io.Reader.fixed(bytes[0..link.length]);
                link_reader.toss(@sizeOf(Link));
                _ = try link_reader.take(link.nameLength);
                const mac = try link_reader.take(link.addressLength);
                record.googleMac = mac.len >= 2 and mac[0] == 0x42 and mac[1] == 0x01;
            }
        }
        for (records.items) |record| {
            if (!record.address or !record.googleMac) continue;
            var request: [32]u8 = @splat(0);
            @memcpy(request[0..16], &record.name);
            try control(io, socket, get_flags, &request);
            var flags = std.Io.Reader.fixed(request[16..]);
            const value = try flags.takeInt(u16, native);
            if (value & 0x1 != 0 and value & 0x40 != 0) return true;
        }
        return false;
    }
}
