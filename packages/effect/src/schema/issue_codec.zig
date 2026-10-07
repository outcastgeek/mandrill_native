//! Version-one owned SchemaIssue JSON envelopes. Earlier envelopes without
//! optional detail remain readable; retained runtime strings and paths are copied.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const issues = @import("schema_issue.zig");
const Issue = issues.Issue;
const json_writer = @import("json_writer.zig");
/// Independent budgets for a version-one owned Issue envelope, including related issues and diagnostic text.
pub const Limits = struct {
    /// Inclusive UTF-8 envelope byte budget before DOM parsing.
    input_bytes: usize,
    /// Inclusive related-issue nesting budget.
    issue_depth: usize,
    /// Inclusive total root/related Issue count.
    issue_count: usize,
    /// Independent JSON output/value/depth/integer-digit budgets.
    json: json_writer.Limits,
};
const Error = error{ OutOfMemory, Invalid, Limit, MissingAllocator };
fn string(text: []const u8) std.json.Value {
    return .{ .string = text };
}
fn integer(value: usize) Error!std.json.Value {
    return .{ .integer = std.math.cast(i64, value) orelse return error.Invalid };
}
fn object() std.json.Value {
    return .{ .object = .{} };
}
fn put(allocator: std.mem.Allocator, value: *std.json.Value, key: []const u8, field: std.json.Value) error{OutOfMemory}!void {
    try value.object.put(allocator, key, field);
}
fn encodeIssue(allocator: std.mem.Allocator, issue: *const Issue, limits: Limits, depth: usize, count: *usize) Error!std.json.Value {
    if (depth >= limits.issue_depth or count.* >= limits.issue_count) return error.Limit;
    count.* += 1;
    var result = object();
    try put(allocator, &result, "direction", string(@tagName(issue.direction)));
    try put(allocator, &result, "kind", string(@tagName(issue.kind)));
    if (issue.kind == .expected) try put(allocator, &result, "expected", string(@tagName(issue.kind.expected)));
    if (issue.kind == .custom) try put(allocator, &result, "custom", string(issue.kind.custom));
    var path = std.json.Array.init(allocator);
    if (issue.path) |elements| for (elements) |element| {
        var entry = object();
        switch (element) {
            .field => |value| try put(allocator, &entry, "field", string(value.items)),
            .property_name => |value| try put(allocator, &entry, "property_name", string(value.items)),
            .index => |value| try put(allocator, &entry, "index", try integer(value)),
            .alternative => |value| try put(allocator, &entry, "alternative", try integer(value)),
        }
        try path.append(entry);
    };
    try put(allocator, &result, "path", .{ .array = path });
    try put(allocator, &result, "limit_reached", .{ .bool = issue.limit_reached });
    if (issue.detail) |detail| {
        var encoded = object();
        try put(allocator, &encoded, "type", string(@tagName(detail)));
        switch (detail) {
            .constraint => |value| {
                try put(allocator, &encoded, "name", string(value.name.items));
                try put(allocator, &encoded, "expected", string(value.expected.items));
                try put(allocator, &encoded, "actual", string(value.actual.items));
            },
            .alternatives => |value| {
                var array = std.json.Array.init(allocator);
                for (value.items) |index| try array.append(try integer(index));
                try put(allocator, &encoded, "matches", .{ .array = array });
            },
            .resource => |value| {
                try put(allocator, &encoded, "resource", string(@tagName(value.resource)));
                try put(allocator, &encoded, "limit", try integer(value.limit));
                try put(allocator, &encoded, "actual", try integer(value.actual));
            },
            .integer_digits => |value| {
                try put(allocator, &encoded, "limit", try integer(value.limit));
                try put(allocator, &encoded, "required", if (value.required) |required| try integer(required) else .null);
            },
            .pattern => |value| {
                try put(allocator, &encoded, "dialect", string(value.dialect.items));
                try put(allocator, &encoded, "source", string(value.source.items));
                const text = try allocator.print("{d}", .{value.timeout_nanoseconds});
                try put(allocator, &encoded, "timeout_nanoseconds", .{ .number_string = text });
            },
        }
        try put(allocator, &result, "detail", encoded);
    }
    if (issue.related) |related| {
        var array = std.json.Array.init(allocator);
        for (related) |*child| try array.append(try encodeIssue(allocator, child, limits, depth + 1, count));
        try put(allocator, &result, "related", .{ .array = array });
    }
    return result;
}
fn memberText(value: *const std.json.Value, name: []const u8) Error![]const u8 {
    if (value.* != .object) return error.Invalid;
    const field = value.object.get(name) orelse return error.Invalid;
    if (field != .string or !std.unicode.utf8ValidateSlice(field.string)) return error.Invalid;
    return field.string;
}
fn number(value: *const std.json.Value, name: []const u8) Error!usize {
    if (value.* != .object) return error.Invalid;
    const field = value.object.get(name) orelse return error.Invalid;
    return switch (field) {
        .integer => |n| std.math.cast(usize, n) orelse error.Invalid,
        .number_string => |n| std.fmt.parseInt(usize, n, 10) catch error.Invalid,
        else => error.Invalid,
    };
}
fn copied(allocator: std.mem.Allocator, source: []const u8) error{OutOfMemory}!outcomes.OwnedSlice(u8) {
    return .{ .items = try allocator.dupe(u8, source), .allocator = allocator };
}
fn decodeIssue(env: anytype, value: *const std.json.Value, limits: Limits, depth: usize, count: *usize) Error!Issue {
    if (depth >= limits.issue_depth or count.* >= limits.issue_count) return error.Limit;
    count.* += 1;
    const direction = std.meta.stringToEnum(issues.Direction, try memberText(value, "direction")) orelse return error.Invalid;
    const kind = try memberText(value, "kind");
    var result = Issue.init(env, direction, .invalid_value);
    errdefer result.deinit();
    if (std.mem.eql(u8, kind, "expected")) result.kind = .{ .expected = std.meta.stringToEnum(issues.Expected, try memberText(value, "expected")) orelse return error.Invalid } else if (std.mem.eql(u8, kind, "custom")) {
        result.owned_custom = try copied(env.allocator, try memberText(value, "custom"));
        result.kind = .{ .custom = result.owned_custom.?.items };
    } else {
        const shape = @typeInfo(issues.Kind).@"union";
        var found = false;
        inline for (shape.field_names, shape.field_types) |name, T| if (T == void and std.mem.eql(u8, name, kind)) {
            result.kind = @unionInit(issues.Kind, name, {});
            found = true;
        };
        if (!found) return error.Invalid;
    }
    if (value.object.get("limit_reached")) |field| {
        if (field != .bool) return error.Invalid;
        result.limit_reached = field.bool;
    }
    if (value.object.get("path")) |path| {
        if (path != .array) return error.Invalid;
        var index = path.array.items.len;
        while (index != 0) {
            index -= 1;
            const entry = &path.array.items[index];
            if (entry.* != .object or entry.object.count() != 1) return error.Invalid;
            if (entry.object.contains("field")) try result.prependField(try memberText(entry, "field")) else if (entry.object.contains("property_name")) try result.prependPropertyName(try memberText(entry, "property_name")) else if (entry.object.contains("index")) try result.prependIndex(try number(entry, "index")) else if (entry.object.contains("alternative")) try result.prependAlternative(try number(entry, "alternative")) else return error.Invalid;
        }
    }
    if (value.object.get("detail")) |detail_| {
        const detail = &detail_;
        const tag = try memberText(detail, "type");
        if (std.mem.eql(u8, tag, "constraint")) {
            var name = try copied(env.allocator, try memberText(detail, "name"));
            errdefer name.deinit();
            var expected = try copied(env.allocator, try memberText(detail, "expected"));
            errdefer expected.deinit();
            result.detail = .{ .constraint = .{ .name = name, .expected = expected, .actual = try copied(env.allocator, try memberText(detail, "actual")) } };
        } else if (std.mem.eql(u8, tag, "alternatives")) {
            const matches = detail.object.get("matches") orelse return error.Invalid;
            if (matches != .array) return error.Invalid;
            const items = try env.allocator.alloc(usize, matches.array.items.len);
            errdefer env.allocator.free(items);
            for (matches.array.items, items) |item, *slot| {
                slot.* = switch (item) {
                    .integer => |n| std.math.cast(usize, n) orelse return error.Invalid,
                    .number_string => |n| std.fmt.parseInt(usize, n, 10) catch return error.Invalid,
                    else => return error.Invalid,
                };
            }
            result.detail = .{ .alternatives = .{ .items = items, .allocator = env.allocator } };
        } else if (std.mem.eql(u8, tag, "resource")) result.detail = .{ .resource = .{ .resource = std.meta.stringToEnum(issues.Detail.Resource, try memberText(detail, "resource")) orelse return error.Invalid, .limit = try number(detail, "limit"), .actual = try number(detail, "actual") } } else if (std.mem.eql(u8, tag, "integer_digits")) {
            const required = detail.object.get("required") orelse .null;
            result.detail = .{ .integer_digits = .{ .limit = try number(detail, "limit"), .required = if (required == .null) null else try number(detail, "required") } };
        } else if (std.mem.eql(u8, tag, "pattern")) {
            var dialect = try copied(env.allocator, try memberText(detail, "dialect"));
            errdefer dialect.deinit();
            var source = try copied(env.allocator, try memberText(detail, "source"));
            errdefer source.deinit();
            const timeout = detail.object.get("timeout_nanoseconds") orelse return error.Invalid;
            const nanos: i96 = switch (timeout) {
                .integer => |n| n,
                .number_string => |n| std.fmt.parseInt(i96, n, 10) catch return error.Invalid,
                else => return error.Invalid,
            };
            result.detail = .{ .pattern = .{ .dialect = dialect, .source = source, .timeout_nanoseconds = nanos } };
        } else return error.Invalid;
    }
    if (value.object.get("related")) |children| {
        if (children != .array) return error.Invalid;
        const owned = try env.allocator.alloc(Issue, children.array.items.len);
        var completed: usize = 0;
        errdefer {
            for (owned[0..completed]) |*child| child.deinit();
            env.allocator.free(owned);
        }
        for (children.array.items, owned) |*child, *slot| {
            slot.* = try decodeIssue(env, child, limits, depth + 1, count);
            completed += 1;
        }
        result.related = owned;
    }
    return result;
}
/// Borrow an Issue and encode its versioned envelope into caller-owned bytes.
/// R supplies allocator; temporary JSON representation is released before return. OOM is .die, validation/limits are .fail.
pub fn encodeDirect(env: anytype, input: *const Issue, limits: Limits) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
    var arena = std.heap.ArenaAllocator.init(env.allocator);
    defer arena.deinit();
    const allocator = arena.allocator();
    var count: usize = 0;
    var root = object();
    put(allocator, &root, "version", .{ .integer = 1 }) catch return .{ .die = outcomes.Defect.out_of_memory };
    const encoded = encodeIssue(allocator, input, limits, 0, &count) catch |problem| return if (problem == error.OutOfMemory) .{ .die = outcomes.Defect.out_of_memory } else .{ .fail = Issue.init(env, .encode, .size_limit) };
    put(allocator, &root, "issue", encoded) catch return .{ .die = outcomes.Defect.out_of_memory };
    var writer = json_writer.Writer.init(env, limits.json);
    defer writer.deinit();
    writer.json(&root) catch |problem| return json_writer.failureWith(env, .encode, problem, &writer);
    return .{ .success = writer.finish() catch return .{ .die = outcomes.Defect.out_of_memory } };
}
/// Read a bounded version-one envelope through std.json and copy every retained path/detail string.
/// Returned Issue owns its diagnostics and outlives input; envelopes omitting optional detail remain accepted.
pub fn decodeDirect(env: anytype, input: []const u8, limits: Limits) outcomes.Exit(Issue, Issue) {
    if (input.len > limits.input_bytes) return .{ .fail = Issue.init(env, .decode, .size_limit) };
    var scanner = std.json.Scanner.initCompleteInput(env.allocator, input);
    defer scanner.deinit();
    while (true) {
        const token = scanner.next() catch |problem| return if (problem == error.OutOfMemory) .{ .die = outcomes.Defect.out_of_memory } else .{ .fail = Issue.init(env, .decode, .invalid_value) };
        if (scanner.stackHeight() > limits.json.max_json_depth) return .{ .fail = Issue.init(env, .decode, .depth_limit) };
        if (token == .end_of_document) break;
    }
    var document = std.json.parseFromSlice(std.json.Value, env.allocator, input, .{ .allocate = .alloc_always, .parse_numbers = false, .max_value_len = limits.json.max_value_bytes, .duplicate_field_behavior = .@"error" }) catch |problem| return if (problem == error.OutOfMemory) .{ .die = outcomes.Defect.out_of_memory } else .{ .fail = Issue.init(env, .decode, .invalid_value) };
    defer document.deinit();
    if (document.value != .object) return .{ .fail = Issue.init(env, .decode, .invalid_value) };
    const version = number(&document.value, "version") catch return .{ .fail = Issue.init(env, .decode, .incompatible_version) };
    if (version != 1) return .{ .fail = Issue.init(env, .decode, .incompatible_version) };
    const issue = document.value.object.get("issue") orelse return .{ .fail = Issue.init(env, .decode, .invalid_value) };
    var count: usize = 0;
    return .{ .success = decodeIssue(env, &issue, limits, 0, &count) catch |problem| return if (problem == error.OutOfMemory) .{ .die = outcomes.Defect.out_of_memory } else .{ .fail = Issue.init(env, .decode, if (problem == error.Limit) .size_limit else .invalid_value) } };
}
