const std = @import("std");
const t = @import("types.zig");
/// Native applications supply their environment snapshot and retain its owner;
/// constructing the default interface performs no ambient reads or I/O.
pub fn defaultProviderAuthContext(environment: *EnvironmentContext) t.AuthContext {
    return environment.context();
}
/// Environment access is explicit and app-owned; importing the module reads none.
pub const EnvironmentContext = struct {
    allocator: std.mem.Allocator,
    environ: *const std.process.Environ.Map,
    /// Decodes the pinned Google command grammar into owned argv. The runner
    /// remains standard-I/O based; a shell/shlex tokenizer is not equivalent.
    inputs: t.CredentialInputs = .{},
    vertexAdcExistsCache: std.atomic.Value(u8) = .init(0),
    awsImdsRegionNegativeCache: std.atomic.Value(i64) = .init(0),
    googleCommandCodec: ?GoogleCommandCodec = null,
    pub fn context(self: *@This()) t.AuthContext {
        return .{ .inputs = self.inputs, .context = self, .envFn = env, .rawEnvFn = rawEnv, .fileExistsFn = exists, .vertexAdcExistsCache = &self.vertexAdcExistsCache, .awsImdsRegionNegativeCache = &self.awsImdsRegionNegativeCache, .credentialCommandFn = credentialCommand };
    }
    fn credentialCommand(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, command: t.CredentialCommand) !std.json.Value {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        var environment = try self.environ.clone(a);
        defer environment.deinit();
        for (command.unsetEnv) |name| _ = environment.orderedRemove(name);
        for (command.env.keys(), command.env.values()) |name, value| try environment.put(name, value);
        if (command.kind == .google_executable) {
            const codec = self.googleCommandCodec orelse GoogleCommandCodec{ .decodeFn = decodeGoogleCommand };
            var arguments = std.heap.ArenaAllocator.init(a);
            defer arguments.deinit();
            const argv = try codec.decodeFn(codec.context, arguments.allocator(), io, command.command);
            return runGoogleExecutable(a, io, argv, &environment, command);
        }
        const windows = @import("builtin").os.tag == .windows;
        const argv: []const []const u8 = if (windows) &.{ self.environ.get("ComSpec") orelse "cmd.exe", "/d", "/s", "/c", command.command } else &.{ "/bin/sh", "-c", command.command };
        const timeout: std.Io.Timeout = if (command.timeoutMs == 0) .none else .{ .duration = .{ .clock = .awake, .raw = .fromNanoseconds(@as(i96, command.timeoutMs) * std.time.ns_per_ms) } };
        const limit = command.maxOutputBytes orelse 1024 * 1024;
        const result = try std.process.run(a, io, .{ .argv = argv, .environ_map = &environment, .stdout_limit = .limited(limit), .stderr_limit = .limited(limit), .timeout = timeout.toDeadline(io) });
        defer a.free(result.stdout);
        defer a.free(result.stderr);
        if (!result.term.success()) return error.CredentialCommandFailed;
        const decoded = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(result.stdout)});
        defer a.free(decoded);
        const text = try @import("../utils/unicode.zig").trimWhitespace(a, decoded);
        defer a.free(text);
        return @import("../json/value.zig").parseFromSliceLeaky(std.json.Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
            if (err == error.OutOfMemory or err == error.Canceled) return err;
            return error.CredentialCommandInvalidJson;
        };
    }
    fn env(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, name: []const u8) !?[]const u8 {
        try io.checkCancel();
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const value = self.environ.get(name) orelse return null;
        // The default context uses trim only to decide whether a value exists;
        // it returns the original text, including surrounding whitespace.
        if (try @import("../utils/unicode.zig").isBlank(value)) return null;
        return try a.dupe(u8, value);
    }
    fn rawEnv(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, name: []const u8) !?[]const u8 {
        try io.checkCancel();
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return if (self.environ.get(name)) |value| try a.dupe(u8, value) else null;
    }
    fn exists(ctx: ?*anyopaque, io: std.Io, path: []const u8) !bool {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const expanded = if (std.mem.startsWith(u8, path, "~")) blk: {
            const home = self.environ.get(if (@import("builtin").os.tag == .windows) "USERPROFILE" else "HOME") orelse return false;
            break :blk try std.fmt.allocPrint(self.allocator, "{s}{s}", .{ home, path[1..] });
        } else null;
        defer if (expanded) |owned| self.allocator.free(owned);
        // fs.access checks existence without opening for reading, and accepts
        // directories. Permission and other filesystem errors mean false.
        std.Io.Dir.cwd().access(io, expanded orelse path, .{}) catch |err| switch (err) {
            error.Canceled => return err,
            else => return false,
        };
        return true;
    }
};

pub const GoogleCommandCodec = struct {
    context: ?*anyopaque = null,
    decodeFn: *const fn (?*anyopaque, std.mem.Allocator, std.Io, []const u8) anyerror![]const []const u8,
};
fn commandFailure(a: std.mem.Allocator, command: t.CredentialCommand, err: anyerror, comptime format: []const u8, args: anytype) anyerror {
    if (command.errorMessage) |slot| slot.* = std.fmt.allocPrint(a, format, args) catch return error.OutOfMemory;
    return err;
}
/// Invokes already-decoded Google argv through caller-owned Io. Both pipes
/// contribute bytes in completion order, matching the source's two data
/// listeners. A deadline cancels/joins reads, kills/reaps the child, and only
/// then releases pipe buffers and the borrowed environment/argv.
pub fn runGoogleExecutable(a: std.mem.Allocator, io: std.Io, argv: []const []const u8, environment: *const std.process.Environ.Map, command: t.CredentialCommand) !std.json.Value {
    if (argv.len == 0) return error.InvalidGoogleCredentialCommand;
    try io.checkCancel();
    if (command.timeoutMs == 0) return runGoogleExecutableAttempt(a, io, argv, environment, command);
    const Outcome = union(enum) { result: anyerror!std.json.Value, timeout: std.Io.Cancelable!void };
    var storage: [2]Outcome = undefined;
    var select = std.Io.Select(Outcome).init(io, &storage);
    defer while (select.cancel()) |_| {};
    try select.concurrent(.result, runGoogleExecutableAttempt, .{ a, io, argv, environment, command });
    try select.concurrent(.timeout, commandTimeout, .{ io, command.timeoutMs });
    return switch (try select.await()) {
        .result => |result| result,
        .timeout => |result| {
            try result;
            while (select.cancel()) |_| {}
            return commandFailure(a, command, error.GoogleCredentialExecutableTimeout, "The executable failed to finish within the timeout specified.", .{});
        },
    };
}
fn commandTimeout(io: std.Io, milliseconds: u64) std.Io.Cancelable!void {
    try io.sleep(.fromNanoseconds(@as(i96, milliseconds) * std.time.ns_per_ms), .awake);
}
fn runGoogleExecutableAttempt(a: std.mem.Allocator, io: std.Io, argv: []const []const u8, environment: *const std.process.Environ.Map, command: t.CredentialCommand) !std.json.Value {
    var child = try std.process.spawn(io, .{ .argv = argv, .environ_map = environment, .stdin = .pipe, .stdout = .pipe, .stderr = .pipe });
    defer child.kill(io);
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    var read_storage: [2]std.Io.Operation.Storage = undefined;
    var batch = std.Io.Batch.init(&read_storage);
    defer batch.cancel(io);
    const files = [_]std.Io.File{ child.stdout.?, child.stderr.? };
    var buffers: [2][8192]u8 = undefined;
    var vectors = [2][1][]u8{ .{&buffers[0]}, .{&buffers[1]} };
    for (files, 0..) |file, index| batch.addAt(@intCast(index), .{ .file_read_streaming = .{ .file = file, .data = &vectors[index] } });
    var remaining: usize = 2;
    var received_bytes: usize = 0;
    while (remaining != 0) {
        try batch.awaitConcurrent(io, .none);
        while (batch.next()) |completion| {
            const n = completion.result.file_read_streaming catch |err| {
                if (err == error.EndOfStream) {
                    remaining -= 1;
                    continue;
                }
                return err;
            };
            if (command.maxOutputBytes) |limit| {
                if (n > limit - received_bytes) return error.StreamTooLong;
                received_bytes += n;
            }
            // Node appends each Buffer chunk as a separately decoded string.
            try output.writer.print("{f}", .{std.unicode.fmtUtf8(buffers[completion.index][0..n])});
            batch.addAt(completion.index, .{ .file_read_streaming = .{ .file = files[completion.index], .data = &vectors[completion.index] } });
        }
    }
    const term = try child.wait(io);
    if (!term.success()) {
        const code = if (term == .exited) try std.fmt.allocPrint(a, "{d}", .{term.exited}) else try std.fmt.allocPrint(a, "{f}", .{term});
        return commandFailure(a, command, error.GoogleCredentialExecutableFailed, "The executable failed with exit code: {s} and error message: {s}.", .{ code, output.written() });
    }
    const value = @import("../json/value.zig").parseFromSliceLeaky(std.json.Value, a, output.written(), .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        return commandFailure(a, command, error.InvalidGoogleExecutableResponse, "The executable returned an invalid response: {s}", .{output.written()});
    };
    if (value == .null) return commandFailure(a, command, error.InvalidGoogleExecutableResponse, "The executable returned an invalid response: {s}", .{output.written()});
    return value;
}

fn decodeGoogleCommand(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, command: []const u8) ![]const []const u8 {
    var regex = try @import("../regex/regexp.zig").Regex.init(a, io,
        \\(?:[^\s"]+|"[^"]*")+
    , .{});
    defer regex.deinit();
    var arguments: std.ArrayList([]const u8) = .empty;
    var offset: usize = 0;
    while (try regex.exec(a, command, offset)) |found| {
        var match = found;
        defer match.deinit();
        const span = match.captures[0].?;
        offset = span.end;
        const value = (try match.text(a, 0)).?;
        const unquoted = if (std.mem.startsWith(u8, value, "\"") and std.mem.endsWith(u8, value, "\"")) value[1 .. value.len - 1] else value;
        try arguments.append(a, try a.dupe(u8, unquoted));
        a.free(value);
    }
    if (arguments.items.len == 0) return error.InvalidGoogleCredentialCommand;
    return arguments.toOwnedSlice(a);
}
