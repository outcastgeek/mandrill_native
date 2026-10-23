//! Linux interface metadata through caller-owned standard I/O. No credential
//! material is read and no packet is sent. IPv4 ioctl records and the kernel's
//! IPv6 address table select interfaces before querying their flags/MAC.
const std = @import("std");
const linux = std.os.linux;
fn control(io: std.Io, socket: std.Io.net.Socket, code: u32, arg: *anyopaque) !void {
    const result = (try io.operate(.{ .device_io_control = .{ .file = .{ .handle = socket.handle, .flags = .{ .nonblocking = false } }, .code = code, .arg = arg } })).device_io_control;
    if (result < 0) return error.GoogleNetworkInterfaceQueryFailed;
}
fn googleMac(io: std.Io, socket: std.Io.net.Socket, original: linux.ifreq) !bool {
    var request = original;
    try control(io, socket, linux.SIOCGIFFLAGS, &request);
    if (!request.ifru.flags.UP or !request.ifru.flags.RUNNING) return false;
    try control(io, socket, linux.SIOCGIFHWADDR, &request);
    return request.ifru.hwaddr.data[0] == 0x42 and request.ifru.hwaddr.data[1] == 0x01;
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
    const Configuration = extern struct { len: c_int, buffer: ?[*]u8 };
    var count: usize = 16;
    while (true) {
        const requests = try a.alloc(linux.ifreq, count);
        defer a.free(requests);
        const size = std.math.cast(c_int, std.mem.sliceAsBytes(requests).len) orelse return error.GoogleNetworkInterfaceListTooLarge;
        var configuration = Configuration{ .len = size, .buffer = std.mem.sliceAsBytes(requests).ptr };
        try control(io, socket, linux.SIOCGIFCONF, &configuration);
        if (configuration.len < 0 or configuration.len > size) return error.GoogleNetworkInterfaceQueryFailed;
        const used: usize = @intCast(configuration.len);
        // Leave one complete record spare: a full buffer may have silently
        // omitted interfaces, and must be retried with a larger allocation.
        if (used + @sizeOf(linux.ifreq) < @as(usize, @intCast(size))) {
            for (requests[0 .. used / @sizeOf(linux.ifreq)]) |request| if (try googleMac(io, socket, request)) return true;
            break;
        }
        count = std.math.mul(usize, count, 2) catch return error.GoogleNetworkInterfaceListTooLarge;
    }
    // SIOCGIFCONF reports IPv4 address records. Include interfaces which have
    // only IPv6 by their kernel interface index; raw names never need a text
    // encoding conversion. Kernel numeric fields use standard integer codecs.
    const table = std.Io.Dir.cwd().readFileAlloc(io, "/proc/net/if_inet6", a, .unlimited) catch |err| {
        if (err == error.FileNotFound) return false;
        return err;
    };
    defer a.free(table);
    var reader = std.Io.Reader.fixed(table);
    while (try reader.takeDelimiter('\n')) |line| {
        var fields = std.mem.tokenizeAny(u8, line, " \t");
        const ip = fields.next() orelse continue;
        if (ip.len != 32) continue;
        _ = std.fmt.parseInt(u128, ip, 16) catch continue;
        const index_text = fields.next() orelse continue;
        const index = std.fmt.parseInt(i32, index_text, 16) catch continue;
        if (index <= 0) continue;
        var request = std.mem.zeroes(linux.ifreq);
        request.ifru.ivalue = index;
        try control(io, socket, linux.SIOCGIFNAME, &request);
        if (try googleMac(io, socket, request)) return true;
    }
    return false;
}
