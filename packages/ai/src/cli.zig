const std = @import("std");
const ai = @import("ai");
const j = ai.ai.api.driver;
const value = ai.json_value;
const t = ai.types;
fn selection(a: std.mem.Allocator, answer: []const u8, count: usize) !?usize {
    const text = try ai.unicode.trimWhitespace(a, answer);
    defer a.free(text);
    var parser = std.fmt.Parser{ .bytes = text, .i = 0 };
    if (parser.maybe('-')) return null;
    _ = parser.maybe('+');
    const digits = std.mem.trimStart(u8, text[parser.i..], "0");
    // Parser.number owns decimal-prefix parsing, but its accumulator does not
    // report overflow. Bound it to a safe number of significant digits; a
    // longer decimal prefix cannot index either resident menu's item array.
    const maximum_digits = comptime std.math.log10_int(@as(usize, std.math.maxInt(usize)));
    parser = .{ .bytes = digits[0..@min(digits.len, maximum_digits)], .i = 0 };
    const number = parser.number() orelse return null;
    if (parser.i == parser.bytes.len and parser.i < digits.len) {
        var continuation = std.fmt.Parser{ .bytes = digits[parser.i..][0..1], .i = 0 };
        if (continuation.number() != null) return null;
    }
    return if (number == 0 or number > count) null else number - 1;
}
const Interaction = struct {
    mutex: std.Io.Mutex = .init,
    output: *std.Io.File.Writer,
    inputBuffer: [4096]u8 = undefined,
    input: ?std.Io.File.Reader = null,
    fn line(self: *@This(), a: std.mem.Allocator, io: std.Io, message: []const u8) ![]const u8 {
        try self.output.interface.writeAll(message);
        try self.output.interface.flush();
        if (self.input == null) self.input = std.Io.File.stdin().readerStreaming(io, &self.inputBuffer);
        var captured = std.Io.Writer.Allocating.init(a);
        defer captured.deinit();
        const ended = if (self.input.?.interface.streamDelimiter(&captured.writer, '\n')) |_| false else |err| switch (err) {
            error.EndOfStream => true,
            error.ReadFailed => return self.input.?.err orelse err,
            else => return err,
        };
        if (ended and captured.written().len == 0) return error.EndOfStream;
        if (!ended) self.input.?.interface.toss(1);
        return a.dupe(u8, std.mem.trimEnd(u8, captured.written(), "\r"));
    }
    fn prompt(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, payload: std.json.Value) ![]const u8 {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        const message = value.field(payload, "message") orelse "";
        if (std.mem.eql(u8, value.field(payload, "type") orelse "", "select")) {
            const options = value.get(payload, "options") orelse return error.InvalidPrompt;
            if (options != .array) return error.InvalidPrompt;
            var writer = std.Io.Writer.Allocating.init(a);
            defer writer.deinit();
            try writer.writer.print("\n{s}\n", .{message});
            for (options.array.items, 0..) |item, index| try writer.writer.print("  {d}. {s}\n", .{ index + 1, value.field(item, "label") orelse "" });
            try writer.writer.print("Enter number (1-{d}): ", .{options.array.items.len});
            const answer = try self.line(a, io, writer.written());
            const selected = (try selection(a, answer, options.array.items.len)) orelse return error.InvalidSelection;
            return a.dupe(u8, value.field(options.array.items[selected], "id") orelse return error.InvalidSelection);
        }
        const placeholder = if (value.field(payload, "placeholder")) |text| (if (text.len > 0) text else null) else null;
        return self.line(a, io, if (placeholder) |text| try std.fmt.allocPrint(a, "{s} ({s}): ", .{ message, text }) else try std.fmt.allocPrint(a, "{s}: ", .{message}));
    }
    fn notify(ctx: ?*anyopaque, io: std.Io, payload: std.json.Value) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        const output = self.output;
        const kind = value.field(payload, "type") orelse "";
        if (std.mem.eql(u8, kind, "auth_url")) {
            try output.interface.print("\nOpen this URL in your browser:\n{s}\n", .{value.field(payload, "url") orelse ""});
            if (value.field(payload, "instructions")) |text| if (text.len > 0) try output.interface.print("{s}\n", .{text});
        } else if (std.mem.eql(u8, kind, "device_code")) try output.interface.print("\nOpen this URL in your browser:\n{s}\nEnter code: {s}\n", .{ value.field(payload, "verificationUri") orelse "", value.field(payload, "userCode") orelse "" }) else try output.interface.print("{s}\n", .{value.field(payload, "message") orelse ""});
        try output.interface.flush();
    }
};
fn device(ctx: ?*anyopaque) []const u8 {
    return @as(*const [36]u8, @ptrCast(@alignCast(ctx.?)));
}
pub fn main(init: std.process.Init) !u8 {
    var output_buffer: [4096]u8 = undefined;
    var output = std.Io.File.stdout().writerStreaming(init.io, &output_buffer);
    var message: ?[]const u8 = null;
    run(init, &message, &output) catch |failure| {
        const err = if (failure == error.WriteFailed) output.err orelse failure else failure;
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        var error_buffer: [4096]u8 = undefined;
        var error_output = std.Io.File.stderr().writerStreaming(init.io, &error_buffer);
        const text = message orelse switch (err) {
            error.InvalidSelection => "Invalid selection",
            else => @errorName(err),
        };
        error_output.interface.print("Error: {s}\n", .{text}) catch |write_err| return error_output.err orelse write_err;
        error_output.interface.flush() catch |write_err| return error_output.err orelse write_err;
        return 1;
    };
    return 0;
}
fn run(init: std.process.Init, error_message: *?[]const u8, output: *std.Io.File.Writer) !void {
    const a = init.arena.allocator();
    const io = init.io;
    const args = try init.minimal.args.toSlice(a);
    var auth_context = ai.auth_context.EnvironmentContext{ .allocator = a, .environ = init.environ_map };
    var models = try ai.providers.builtinModels(a, io, .{ .authContext = auth_context.context() });
    defer models.deinit();
    const command = if (args.len > 1 and args[1].len > 0) args[1] else "help";
    var providers: std.ArrayList(ai.Provider) = .empty;
    for (try models.getProviders(a)) |provider| if (provider.auth.oauth != null) {
        try providers.append(a, provider);
    };
    if (std.mem.eql(u8, command, "list")) {
        for (providers.items) |provider| try output.interface.print("{s: <20} {s}\n", .{ provider.id, provider.name });
        try output.interface.flush();
        return;
    }
    if (std.mem.eql(u8, command, "help") or std.mem.eql(u8, command, "--help") or std.mem.eql(u8, command, "-h")) {
        try output.interface.writeAll("Usage: pi-ai <command> [provider]\n\nCommands:\n  login [provider]  Login to an OAuth provider\n  list              List available providers\n\nProviders:\n");
        for (providers.items) |provider| try output.interface.print("  {s: <20} {s}\n", .{ provider.id, provider.name });
        try output.interface.flush();
        return;
    }
    if (!std.mem.eql(u8, command, "login")) {
        error_message.* = try std.fmt.allocPrint(a, "Unknown command: {s}", .{command});
        return error.UnknownCommand;
    }
    var interaction = Interaction{ .output = output };
    const id = if (args.len > 2 and args[2].len > 0) args[2] else blk: {
        var choices = std.Io.Writer.Allocating.init(a);
        defer choices.deinit();
        for (providers.items, 0..) |provider, index| try choices.writer.print("  {d}. {s}\n", .{ index + 1, provider.name });
        try choices.writer.print("Enter number (1-{d}): ", .{providers.items.len});
        const answer = try interaction.line(a, io, choices.written());
        const selected = (try selection(a, answer, providers.items.len)) orelse {
            error_message.* = "Unknown provider: ";
            return error.UnknownProvider;
        };
        break :blk providers.items[selected].id;
    };
    const selected = (try models.getProvider(id)) orelse {
        error_message.* = try std.fmt.allocPrint(a, "Unknown provider: {s}", .{id});
        return error.UnknownProvider;
    };
    if (selected.auth.oauth == null) {
        error_message.* = try std.fmt.allocPrint(a, "Unknown provider: {s}", .{id});
        return error.UnknownProvider;
    }
    var env = t.ProviderEnv{};
    for (init.environ_map.keys(), init.environ_map.values()) |key, text| try env.put(a, key, text);
    var device_id = try ai.ai.uuid.randomV4(io);
    const credential = try models.login(a, id, .oauth, .{ .context = &interaction, .prompt = Interaction.prompt, .notify = Interaction.notify, .errorMessage = error_message }, .{ .deviceContext = &device_id, .getDeviceId = device, .env = env });
    // Login produces a new in-memory record. Never retrieve an existing
    // credential file to merge it, and never truncate another provider's data.
    var stored = value.map(a);
    try value.put(a, &stored, id, try value.from(a, credential));
    const encoded = try ai.json_value.stringify(a, stored, .{ .whitespace = .indent_2 });
    const file = std.Io.Dir.cwd().createFile(io, "auth.json", .{ .exclusive = true }) catch |err| {
        if (err == error.PathAlreadyExists) error_message.* = "auth.json already exists. Merging requires reading the existing credentials, which is prohibited; the existing file has been preserved.";
        return err;
    };
    defer file.close(io);
    try file.writeStreamingAll(io, encoded);
    try output.interface.writeAll("\nCredentials saved to auth.json\n");
    try output.interface.flush();
}
