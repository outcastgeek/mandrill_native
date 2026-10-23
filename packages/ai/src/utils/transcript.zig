const std = @import("std");
const t = @import("../types.zig");
const text = @import("text.zig");
const j = @import("../json/value.zig");
pub fn createInitialSystemMessage(prompt: ?[]const u8, tools: ?[]const t.Tool) ?t.SystemMessage {
    const has_prompt = if (prompt) |p| p.len > 0 else false;
    const has_tools = if (tools) |v| v.len > 0 else false;
    if (!has_prompt and !has_tools) return null;
    return .{ .content = .{ .string = prompt orelse "" }, .toolsAdded = if (has_tools) tools else null, .timestamp = 0 };
}
pub fn normalizeContext(a: std.mem.Allocator, context: t.Context) !t.TranscriptContext {
    if (createInitialSystemMessage(context.systemPrompt, context.tools)) |head| {
        const messages = try a.alloc(t.Message, context.messages.len + 1);
        messages[0] = .{ .system = head };
        @memcpy(messages[1..], context.messages);
        return .{ .messages = messages };
    }
    return .{ .messages = context.messages };
}
// Replay accepts an application's tagged message union when its `system`
// variant carries SystemMessage. Other roles do not participate in replay.
fn systemMessage(message: anytype) ?t.SystemMessage {
    return switch (@typeInfo(@TypeOf(message))) {
        .@"union" => |info| if (info.tag_type != null and @hasField(@TypeOf(message), "system")) switch (message) {
            .system => |system| system,
            else => null,
        } else null,
        else => null,
    };
}
pub fn getInitialSystemMessage(messages: anytype) ?t.SystemMessage {
    return if (messages.len > 0) systemMessage(messages[0]) else null;
}
pub fn withoutInitialSystemMessage(messages: []const t.Message) []const t.Message {
    return if (getInitialSystemMessage(messages) != null) messages[1..] else messages;
}
pub fn getCurrentTools(a: std.mem.Allocator, messages: anytype) ![]const t.Tool {
    var tools: std.StringArrayHashMapUnmanaged(t.Tool) = .{};
    defer tools.deinit(a);
    for (messages) |message| if (systemMessage(message)) |system| {
        for (system.toolsRemoved orelse @as([]const t.ToolReference, &.{})) |tool| _ = tools.orderedRemove(tool.name);
        for (system.toolsAdded orelse @as([]const t.Tool, &.{})) |tool| try tools.put(a, tool.name, tool);
    };
    return a.dupe(t.Tool, tools.values());
}
pub fn getCurrentSystemMessage(a: std.mem.Allocator, messages: anytype) !?t.SystemMessage {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    var parts: std.ArrayList([]const u8) = .empty;
    var sections = j.map(scratch);
    var timestamp: ?f64 = null;
    for (messages) |message| if (systemMessage(message)) |s| {
        if (timestamp == null) timestamp = s.timestamp;
        const value = try text.contentText(scratch, s.content, "\n");
        if (value.len > 0) try parts.append(scratch, value);
        if (s.sections) |patch| if (patch == .object) {
            const entries = try j.objectEntries(scratch, patch.object);
            for (entries) |entry| {
                if (entry.node.* == .null) {
                    _ = sections.object.orderedRemove(entry.key);
                } else try j.put(scratch, &sections, entry.key, entry.node.*);
            }
        };
    };
    const tools = try getCurrentTools(scratch, messages);
    if (timestamp == null and tools.len == 0) return null;
    return .{ .content = .{ .string = try std.mem.join(a, "\n\n", parts.items) }, .sections = if (sections.object.count() > 0) try j.clone(a, sections) else null, .toolsAdded = if (tools.len > 0) try a.dupe(t.Tool, tools) else null, .timestamp = timestamp orelse 0 };
}
pub fn getCurrentSystemPrompt(a: std.mem.Allocator, messages: anytype) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    return if (try getCurrentSystemMessage(temporary.allocator(), messages)) |s| text.getSystemMessageText(a, s) else "";
}
pub fn collapseSystemMessages(a: std.mem.Allocator, context: t.TranscriptContext) !t.TranscriptContext {
    var messages: std.ArrayList(t.Message) = .empty;
    if (try getCurrentSystemMessage(a, context.messages)) |head| try messages.append(a, .{ .system = head });
    for (context.messages) |message| if (message != .system) {
        try messages.append(a, message);
    };
    return .{ .messages = try messages.toOwnedSlice(a) };
}
pub fn resolveTranscript(a: std.mem.Allocator, context: t.TranscriptContext, supports: bool) !t.TranscriptContext {
    return if (supports) context else collapseSystemMessages(a, context);
}
pub fn toToolDeclaration(a: std.mem.Allocator, tool: t.Tool) !t.Tool {
    // Persisted declarations deliberately lose native construction metadata,
    // matching the source's JSON round-trip of the parameter schema.
    return .{ .name = try a.dupe(u8, tool.name), .description = try a.dupe(u8, tool.description), .parameters = .{ .value = try j.roundTrip(a, tool.parameters.value) }, .constrainedSampling = try t.cloneInto(?std.json.Value, a, try j.fieldValue(a, tool, "constrainedSampling")) };
}
pub fn declarationsEqual(a: std.mem.Allocator, left: t.Tool, right: t.Tool) !bool {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const l = try @import("../json/value.zig").stringify(a, try toToolDeclaration(scratch, left), .{ .emit_null_optional_fields = false });
    defer a.free(l);
    const r = try @import("../json/value.zig").stringify(a, try toToolDeclaration(scratch, right), .{ .emit_null_optional_fields = false });
    defer a.free(r);
    return std.mem.eql(u8, l, r);
}
pub const TranscriptMessages = []const t.Message;
pub const ToolStateChanges = struct { toolsAdded: []const t.Tool, toolsRemoved: []const t.ToolReference };
pub fn getToolStateChanges(a: std.mem.Allocator, previous: []const t.Tool, current: []const t.Tool) !ToolStateChanges {
    var previous_tools: std.StringHashMapUnmanaged(t.Tool) = .{};
    defer previous_tools.deinit(a);
    var current_tools: std.StringHashMapUnmanaged(t.Tool) = .{};
    defer current_tools.deinit(a);
    for (previous) |tool| try previous_tools.put(a, tool.name, tool);
    for (current) |tool| try current_tools.put(a, tool.name, tool);
    var added: std.ArrayList(t.Tool) = .empty;
    var removed: std.ArrayList(t.ToolReference) = .empty;
    for (current) |tool| {
        const same = if (previous_tools.get(tool.name)) |old| try declarationsEqual(a, old, tool) else false;
        if (!same) try added.append(a, try toToolDeclaration(a, tool));
    }
    for (previous) |tool| {
        const same = if (current_tools.get(tool.name)) |new| try declarationsEqual(a, tool, new) else false;
        if (!same) try removed.append(a, .{ .name = tool.name });
    }
    return .{ .toolsAdded = try added.toOwnedSlice(a), .toolsRemoved = try removed.toOwnedSlice(a) };
}
pub fn getDeclaredTools(a: std.mem.Allocator, messages: anytype) ![]const t.Tool {
    var tools: std.StringArrayHashMapUnmanaged(t.Tool) = .{};
    defer tools.deinit(a);
    for (messages) |message| if (systemMessage(message)) |system| {
        for (system.toolsAdded orelse @as([]const t.Tool, &.{})) |tool| try tools.put(a, tool.name, tool);
    };
    return a.dupe(t.Tool, tools.values());
}
pub fn hasToolRedefinitions(a: std.mem.Allocator, messages: anytype) !bool {
    var declared: std.StringHashMapUnmanaged(t.Tool) = .{};
    defer declared.deinit(a);
    for (messages) |message| if (systemMessage(message)) |system| {
        for (system.toolsAdded orelse @as([]const t.Tool, &.{})) |tool| {
            if (declared.get(tool.name)) |old| if (!try declarationsEqual(a, old, tool)) return true;
            try declared.put(a, tool.name, tool);
        }
    };
    return false;
}
pub fn hasNonAdditiveToolChanges(a: std.mem.Allocator, messages: anytype) !bool {
    var declared: std.StringHashMapUnmanaged(void) = .{};
    defer declared.deinit(a);
    for (messages) |message| if (systemMessage(message)) |system| {
        if ((system.toolsRemoved orelse @as([]const t.ToolReference, &.{})).len > 0) return true;
        for (system.toolsAdded orelse @as([]const t.Tool, &.{})) |tool| {
            if (declared.contains(tool.name)) return true;
            try declared.put(a, tool.name, {});
        }
    };
    return false;
}
pub const TranscriptTools = struct { requestTools: []const t.Tool, anchorsAdditions: bool };
pub fn resolveTranscriptTools(a: std.mem.Allocator, messages: anytype, supports: bool) !TranscriptTools {
    const anchors = supports and !try hasNonAdditiveToolChanges(a, messages);
    return .{ .requestTools = if (anchors) (if (getInitialSystemMessage(messages)) |s| s.toolsAdded orelse &.{} else &.{}) else try getCurrentTools(a, messages), .anchorsAdditions = anchors };
}
