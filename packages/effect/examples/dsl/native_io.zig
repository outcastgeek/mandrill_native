//! EX-01: native file I/O, CPU parsing, bounded mixed-provider lookups, and save.
const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const Error = enum { io, malformed };
const R = struct { allocator: std.mem.Allocator, io: std.Io, dir: std.Io.Dir, async_engine: fx.Executor, parallel_engine: fx.Executor, blocking_engine: fx.Executor };
const Bytes = fx.OwnedSlice(u8);
const Row = struct { value: u32, kind: enum { io, cpu, blocking } };
const Number = fx.Effect(u32, Error, R);
const Roles = struct {
    fn fetch(input: anytype, _: *const R) fx.Effect(Bytes, Error, R) {
        return fx.Effect(Bytes, Error, R).fromNativeBorrowed(@typeInfo(@TypeOf(input)).pointer.child, input, struct {
            fn read(value: @TypeOf(input), env: *const R) fx.Exit(Bytes, Error) {
                var buffer: [256]u8 = undefined;
                const data = env.dir.readFile(env.io, value.path, &buffer) catch return .{ .fail = .io };
                const owned = env.allocator.dupe(u8, data) catch return .{ .die = fx.Defect.out_of_memory };
                return .{ .success = .{ .items = owned, .allocator = env.allocator } };
            }
        }.read);
    }
    fn parse(bytes: *const Bytes, _: *const R) fx.Effect([3]Row, Error, R) {
        return fx.Effect([3]Row, Error, R).onCpuBorrowed(Bytes, bytes, struct {
            fn decode(input: *const Bytes, _: *const R, cancellation: *const fx.Cancellation) fx.Exit([3]Row, Error) {
                if (cancellation.isRequested()) return .interrupted;
                var tokens = std.mem.splitScalar(u8, input.items, ',');
                var rows: [3]Row = undefined;
                for (&rows, 0..) |*row, index| row.* = .{ .value = std.fmt.parseInt(u32, tokens.next() orelse return .{ .fail = .malformed }, 10) catch return .{ .fail = .malformed }, .kind = switch (index) {
                    0 => .io,
                    1 => .cpu,
                    else => .blocking,
                } };
                if (tokens.next() != null) return .{ .fail = .malformed };
                return .{ .success = rows };
            }
        }.decode);
    }
    fn ioLookup(row: *const Row, env: *const R) fx.Exit(u32, Error) {
        var buffer: [32]u8 = undefined;
        const bytes = env.dir.readFile(env.io, "lookup", &buffer) catch return .{ .fail = .io };
        return .{ .success = row.value + (std.fmt.parseInt(u32, bytes, 10) catch return .{ .fail = .malformed }) };
    }
    fn cpuLookup(row: *const Row, _: *const R, cancellation: *const fx.Cancellation) fx.Exit(u32, Error) {
        return if (cancellation.isRequested()) .interrupted else .{ .success = row.value * 2 };
    }
    fn blockingLookup(row: *const Row, env: *const R, cancellation: *const fx.Cancellation) fx.Exit(u32, Error) {
        if (cancellation.isRequested()) return .interrupted;
        var file = env.dir.openFile(env.io, "lookup", .{}) catch return .{ .fail = .io };
        defer file.close(env.io);
        var buffer: [32]u8 = undefined;
        const len = file.readPositionalAll(env.io, &buffer, 0) catch return .{ .fail = .io };
        return .{ .success = row.value + (std.fmt.parseInt(u32, buffer[0..len], 10) catch return .{ .fail = .malformed }) };
    }
    fn lookup(row: *const Row, _: *const R) Number {
        return switch (row.kind) {
            .io => Number.fromNativeBorrowed(Row, row, ioLookup),
            .cpu => Number.onCpuBorrowed(Row, row, cpuLookup),
            .blocking => Number.onBlockingBorrowed(Row, row, blockingLookup),
        };
    }
    fn lookups(input: anytype, rows: *const [3]Row, env: *const R) fx.Effect(fx.OwnedSlice(u32), Error, R) {
        return fx.dsl.collections.forEachConcurrent(env, &rows.*, input.concurrency, .{ .body = lookup });
    }
    fn save(input: anytype, values: *const fx.OwnedSlice(u32), env: *const R) Number {
        return fx.dsl.owned(env, .{ .path = input.output, .values = values }, struct {
            fn start(config: anytype, _: *const R) Number {
                return Number.fromNativeBorrowed(@typeInfo(@TypeOf(config)).pointer.child, config, struct {
                    fn write(data: @TypeOf(config), current: *const R) fx.Exit(u32, Error) {
                        var total: u32 = 0;
                        for (data.values.items) |value| total += value;
                        var buffer: [32]u8 = undefined;
                        const encoded = std.fmt.bufPrint(&buffer, "{d}", .{total}) catch return .{ .fail = .malformed };
                        current.dir.writeFile(current.io, .{ .sub_path = data.path, .data = encoded }) catch return .{ .fail = .io };
                        return .{ .success = total };
                    }
                }.write);
            }
        }.start);
    }
};
pub fn process(env: *const R, input: anytype) Number {
    return p.captured(env, input, .{ p.binding("fetched", .{"inputs"}, Roles.fetch), p.binding("parsed", .{"fetched"}, Roles.parse), p.binding("lookups", .{ "inputs", "parsed" }, Roles.lookups), p.binding("saved", .{ "inputs", "lookups" }, Roles.save) });
}
fn check(allocator: std.mem.Allocator, io: std.Io, dir: std.Io.Dir) !void {
    var async_engine: fx.NativeIoExecutor = .{ .allocator = allocator, .io = io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 4 };
    defer async_engine.deinit();
    var cpu: fx.NativeIoExecutor = .{ .allocator = allocator, .io = io, .capabilities = .{ .independent = true, .cpu = true, .blocking = false }, .maximum_owned = 2 };
    defer cpu.deinit();
    var blocking: fx.NativeIoExecutor = .{ .allocator = allocator, .io = io, .capabilities = .{ .independent = true, .cpu = false, .blocking = true }, .maximum_owned = 2 };
    defer blocking.deinit();
    const env: R = .{ .allocator = allocator, .io = io, .dir = dir, .async_engine = async_engine.executor(), .parallel_engine = cpu.executor(), .blocking_engine = blocking.executor() };
    try dir.writeFile(io, .{ .sub_path = "request", .data = "3,4,5" });
    try dir.writeFile(io, .{ .sub_path = "lookup", .data = "10" });
    var description = process(&env, .{ .path = @as([]const u8, "request"), .output = @as([]const u8, "saved"), .concurrency = @as(usize, 2) });
    var result = fx.run(&env, description);
    description.deinit();
    defer result.deinit();
    switch (result) {
        .success => |value| try std.testing.expectEqual(@as(u32, 36), value),
        .fail => return error.BusinessFailure,
        .die => return error.Defect,
        .interrupted => return error.Interrupted,
        .finalization_failed => return error.CleanupFailure,
    }
    var buffer: [32]u8 = undefined;
    try std.testing.expectEqualStrings("36", try dir.readFile(io, "saved", &buffer));
}
pub fn main(init: std.process.Init) !void {
    const dir = try std.Io.Dir.cwd().createDirPathOpen(init.io, "zig-out/dsl-example-io", .{});
    defer dir.close(init.io);
    try check(init.gpa, init.io, dir);
}
test "EX-01 native I/O CPU parse bounded mixed-provider lookups and native save use inferred captured composition" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    try check(std.testing.allocator, std.testing.io, temporary.dir);
}
