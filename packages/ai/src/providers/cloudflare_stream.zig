const std = @import("std");
const t = @import("../types.zig");
const m = @import("../models.zig");
const events = @import("../utils/event_stream.zig");
const driver = @import("../api/driver.zig");
const Replacement = union(enum) { literal: []const u8, match, prefix, suffix };
/// The source passes a string replacement to replaceAll, so its four dollar
/// substitutions apply before the selected API constructs any URI. Regex
/// captures and standard Unicode conversion retain JavaScript string units.
fn replaceTemplate(a: std.mem.Allocator, input: []const u8, needle: []const u8, replacement: []const u8) ![]const u8 {
    if (std.mem.findScalar(u8, replacement, '$') == null) return std.mem.replaceOwned(u8, a, input, needle, replacement);
    var regex = try @import("../regex/regexp.zig").Regex.init(a, null, "\\$([$&`'])", .{});
    defer regex.deinit();
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, replacement);
    defer a.free(units);
    var pieces: std.ArrayList(Replacement) = .empty;
    defer pieces.deinit(a);
    var offset: usize = 0;
    while (try regex.exec(a, replacement, offset)) |found| {
        var token = found;
        defer token.deinit();
        const span = token.captures[0].?;
        try pieces.append(a, .{ .literal = try std.unicode.wtf16LeToWtf8Alloc(a, units[offset..span.start]) });
        try pieces.append(a, switch (token.input[token.captures[1].?.start]) {
            '$' => .{ .literal = "$" },
            '&' => .match,
            '`' => .prefix,
            '\'' => .suffix,
            else => unreachable,
        });
        offset = span.end;
    }
    try pieces.append(a, .{ .literal = try std.unicode.wtf16LeToWtf8Alloc(a, units[offset..]) });
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    var cursor: usize = 0;
    while (std.mem.findPos(u8, input, cursor, needle)) |start| {
        try output.writer.writeAll(input[cursor..start]);
        const end = start + needle.len;
        for (pieces.items) |piece| try output.writer.writeAll(switch (piece) {
            .literal => |text| text,
            .match => needle,
            .prefix => input[0..start],
            .suffix => input[end..],
        });
        cursor = end;
    }
    try output.writer.writeAll(input[cursor..]);
    return output.toOwnedSlice();
}
/// Expand the source's literal endpoint template before URI construction.
/// Empty values/slashes and replaceAll's dollar substitutions are retained;
/// the selected API owns subsequent URL parsing/encoding.
pub fn resolveCloudflareModel(a: std.mem.Allocator, original: t.Model, env: t.ProviderEnv) !t.Model {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    var base = original.baseUrl;
    inline for (.{ "CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_GATEWAY_ID" }) |name| {
        if (env.get(name)) |value| base = try replaceTemplate(temporary.allocator(), base, "{" ++ name ++ "}", value);
    }
    if (std.mem.eql(u8, base, original.baseUrl)) return original;
    var model = original;
    model.baseUrl = try a.dupe(u8, base);
    return model;
}
const Wrapped = struct {
    original: m.ApiImplementation,
    fn owner(ctx: ?*anyopaque) *@This() {
        return @ptrCast(@alignCast(ctx.?));
    }
    fn StreamCall(comptime simple: bool) type {
        return struct {
            fn call(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
                const original = owner(ctx).original;
                const inner = (if (simple) original.streamSimple else original.stream) orelse return error.UnsupportedStreamApi;
                var temporary = std.heap.ArenaAllocator.init(operation.allocator);
                defer temporary.deinit();
                return inner(original.context, operation, try resolveCloudflareModel(temporary.allocator(), model, options.env), context, options);
            }
        };
    }
    fn CompleteCall(comptime simple: bool) type {
        return struct {
            fn call(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.Owned(t.AssistantMessage) {
                const original = owner(ctx).original;
                const inner = (if (simple) original.completeSimple else original.complete) orelse return error.UnsupportedCompleteApi;
                var temporary = std.heap.ArenaAllocator.init(operation.allocator);
                defer temporary.deinit();
                return inner(original.context, operation, try resolveCloudflareModel(temporary.allocator(), model, options.env), context, options);
            }
        };
    }
    fn classify(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions) !t.ClassifierResult {
        const original = owner(ctx).original;
        const inner = original.classify orelse return error.UnsupportedClassifierApi;
        var temporary = std.heap.ArenaAllocator.init(operation.allocator);
        defer temporary.deinit();
        return inner(original.context, operation, try resolveCloudflareModel(temporary.allocator(), model, options.env), context, options);
    }
};
/// Wrapper storage belongs to a; retain it and the original opaque context
/// until all returned streams have joined. Inner streams copy retained inputs.
pub fn cloudflareStreams(a: std.mem.Allocator, original: m.ProviderStreams) !m.ProviderStreams {
    const state = try a.create(Wrapped);
    errdefer a.destroy(state);
    state.* = .{ .original = original };
    return .{ .api = try a.dupe(u8, original.api), .context = state, .stream = if (original.stream != null) Wrapped.StreamCall(false).call else null, .streamSimple = if (original.streamSimple != null) Wrapped.StreamCall(true).call else null, .complete = if (original.complete != null) Wrapped.CompleteCall(false).call else null, .completeSimple = if (original.completeSimple != null) Wrapped.CompleteCall(true).call else null };
}
pub fn cloudflareClassifier(a: std.mem.Allocator, original: m.ProviderClassifier) !m.ProviderClassifier {
    const state = try a.create(Wrapped);
    errdefer a.destroy(state);
    state.* = .{ .original = original };
    return .{ .api = try a.dupe(u8, original.api), .context = state, .classify = if (original.classify != null) Wrapped.classify else null };
}
fn Calls(comptime simple: bool) type {
    return struct {
        fn stream(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
            var arena = std.heap.ArenaAllocator.init(operation.allocator);
            defer arena.deinit();
            const resolved = try resolveCloudflareModel(arena.allocator(), model, options.env);
            return if (simple) driver.streamSimple(null, operation, resolved, context, options) else driver.stream(null, operation, resolved, context, options);
        }
        fn complete(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.Owned(t.AssistantMessage) {
            var arena = std.heap.ArenaAllocator.init(operation.allocator);
            defer arena.deinit();
            const resolved = try resolveCloudflareModel(arena.allocator(), model, options.env);
            return if (simple) driver.directCompleteSimple(null, operation, resolved, context, options) else driver.complete(operation, resolved, context, options);
        }
    };
}
pub fn implementation(comptime api: []const u8) m.ApiImplementation {
    if (comptime std.mem.eql(u8, api, "cloudflare-workers-ai-system-one")) return .{ .api = api, .classify = struct {
        fn classify(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions) !t.ClassifierResult {
            var arena = std.heap.ArenaAllocator.init(operation.allocator);
            defer arena.deinit();
            const resolved = try resolveCloudflareModel(arena.allocator(), model, options.env);
            return @import("../api/classifiers.zig").Api(api).classify(null, operation, resolved, context, options);
        }
    }.classify };
    return .{ .api = api, .stream = Calls(false).stream, .streamSimple = Calls(true).stream, .complete = Calls(false).complete, .completeSimple = Calls(true).complete };
}
