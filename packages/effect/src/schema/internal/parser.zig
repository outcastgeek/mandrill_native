//! Caller-bounded JSON parsing shared by typed structural schemas.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const legacy = @import("../../schema.zig");
const ast = @import("../schema_ast.zig");
const Issue = @import("../schema_issue.zig").Issue;

pub const Limits = struct {
    max_document_bytes: usize,
    max_depth: usize,
    max_value_bytes: usize,
};

/// Duplicate policy is applied before field construction. The scanner pass
/// bounds nesting before a document tree can allocate at an excessive depth.
pub fn parseWith(
    comptime T: type,
    env: anytype,
    bytes: []const u8,
    comptime duplicates: ast.DuplicateFields,
    limits: Limits,
    comptime decode_value: anytype,
) outcomes.Exit(legacy.Parsed(T), Issue) {
    if (bytes.len > limits.max_document_bytes) return .{ .fail = Issue.init(env, .decode, .size_limit) };
    var scanner = std.json.Scanner.initCompleteInput(env.allocator, bytes);
    defer scanner.deinit();
    while (true) {
        const token = scanner.next() catch |err| {
            if (err == error.OutOfMemory) return .{ .die = outcomes.Defect.out_of_memory };
            return .{ .fail = Issue.init(env, .decode, .invalid_value) };
        };
        if (scanner.stackHeight() > limits.max_depth)
            return .{ .fail = Issue.init(env, .decode, .depth_limit) };
        if (token == .end_of_document) break;
    }
    var document = std.json.parseFromSlice(std.json.Value, env.allocator, bytes, .{
        .allocate = .alloc_always,
        .parse_numbers = false,
        .max_value_len = limits.max_value_bytes,
        .duplicate_field_behavior = switch (duplicates) {
            .reject => .@"error",
            .first_wins => .use_first,
            .last_wins => .use_last,
        },
    }) catch |err| {
        if (err == error.OutOfMemory) return .{ .die = outcomes.Defect.out_of_memory };
        return .{ .fail = Issue.init(env, .decode, if (err == error.DuplicateField) .duplicate_field else .invalid_value) };
    };
    const decoded = decode_value(env, &document.value);
    if (decoded != .success) {
        document.deinit();
        return outcomes.castFailure(legacy.Parsed(T), decoded);
    }
    const result = legacy.Parsed(T).init(env.allocator, document, decoded.success) catch {
        var discarded = decoded.success;
        outcomes.discardPayload(&discarded);
        document.deinit();
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    return .{ .success = result };
}
