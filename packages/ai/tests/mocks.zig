const std = @import("std");
const ai = @import("ai");
const t = ai.types;
const http = ai.transport;
pub const Step = struct { status: u16 = 200, headers: t.ProviderHeaders = &.{}, body: []const u8 };
pub const Captured = struct { method: std.http.Method, url: []const u8, headers: t.ProviderHeaders, body: ?std.json.Value };
pub const Http = struct {
    allocator: std.mem.Allocator,
    arena: std.heap.ArenaAllocator,
    steps: []const Step,
    mutex: std.Io.Mutex = .init,
    index: usize = 0,
    released: usize = 0,
    captured: std.ArrayList(Captured) = .empty,
    pub fn init(a: std.mem.Allocator, steps: []const Step) @This() {
        return .{ .allocator = a, .arena = .init(a), .steps = steps };
    }
    pub fn deinit(self: *@This()) void {
        self.arena.deinit();
        self.* = undefined;
    }
    pub fn transport(self: *@This()) http.Transport {
        return .{ .context = self, .sendFn = send };
    }
    const Body = struct {
        owner: *Http,
        allocator: std.mem.Allocator,
        reader: std.Io.Reader,
        io: std.Io,
        fn release(ctx: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            self.owner.mutex.lockUncancelable(self.io);
            self.owner.released += 1;
            self.owner.mutex.unlock(self.io);
            self.allocator.destroy(self);
        }
    };
    fn send(ctx: *anyopaque, a: std.mem.Allocator, io: std.Io, request: http.Request) !http.Response {
        const self: *@This() = @ptrCast(@alignCast(ctx));
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        try io.checkCancel();
        if (self.index >= self.steps.len) return error.UnexpectedMockRequest;
        const step = self.steps[self.index];
        self.index += 1;
        const alloc = self.arena.allocator();
        const body = if (request.body) |bytes| try std.json.parseFromSliceLeaky(std.json.Value, alloc, bytes, .{ .allocate = .alloc_always }) else null;
        const captured_headers = try alloc.alloc(t.ProviderHeader, request.headers.len);
        for (request.headers, captured_headers) |header, *out| out.* = .{ .name = try alloc.dupe(u8, header.name), .value = if (header.value) |value| try alloc.dupe(u8, value) else null };
        try self.captured.append(alloc, .{ .method = request.method, .url = try ai.ai.uri.render(alloc, request.uri), .headers = captured_headers, .body = body });
        const active = try a.create(Body);
        active.* = .{ .owner = self, .allocator = a, .reader = .fixed(step.body), .io = io };
        return .{ .status = step.status, .headers = step.headers, .body = &active.reader, .owner = active, .releaseFn = Body.release };
    }
};
pub fn sse(a: std.mem.Allocator, values: []const std.json.Value) ![]const u8 {
    var writer = std.Io.Writer.Allocating.init(a);
    defer writer.deinit();
    for (values) |value| try ai.ai.sse.writeEvent(&writer.writer, .{ .data = try std.json.Stringify.valueAlloc(a, value, .{}) });
    return writer.toOwnedSlice();
}
pub fn model(api: []const u8) t.Model {
    return .{ .id = "fixture-model", .name = "Fixture model", .api = api, .provider = "fixture", .baseUrl = "http://fixture.invalid/v1", .maxTokens = 4096, .contextWindow = 32768, .cost = .{ .input = 1, .output = 2, .cacheRead = 0.1, .cacheWrite = 1.25 } };
}
pub fn emptyAuth(_: ?*anyopaque, _: std.mem.Allocator, io: std.Io, _: []const u8) !?[]const u8 {
    try io.checkCancel();
    return null;
}
pub fn noFile(_: ?*anyopaque, io: std.Io, _: []const u8) !bool {
    try io.checkCancel();
    return false;
}
pub const authContext = ai.auth.AuthContext{ .context = null, .envFn = emptyAuth, .fileExistsFn = noFile };
