//! Caller-bounded JSON parsing shared by typed structural schemas.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const legacy = @import("../../schema.zig");
const ast = @import("../schema_ast.zig");
const Issue = @import("../schema_issue.zig").Issue;

/// Explicit parser bounds used before JSON input is allocated or decoded.
pub const Limits = struct {
    max_document_bytes: usize,
    max_depth: usize,
    max_value_bytes: usize,
    max_integer_digits: ?usize = null,
};

/// Duplicate policy is applied before field construction. The scanner pass
/// bounds nesting before a document tree can allocate at an excessive depth.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `bytes: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined it.
/// - `duplicates: comptime ast.DuplicateFields`: Compile-time policy for duplicate record fields.
/// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
///   installed.
/// - `decode_value: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// Returns (`outcomes.Exit(legacy.Parsed(T), Issue)`): Carries `legacy.Parsed(T)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn parseWith(
    comptime T: type,
    env: anytype,
    bytes: []const u8,
    comptime duplicates: ast.DuplicateFields,
    limits: Limits,
    comptime decode_value: anytype,
) outcomes.Exit(legacy.Parsed(T), Issue) {
    if (limits.max_document_bytes == 0 or limits.max_depth == 0 or limits.max_value_bytes == 0 or (limits.max_integer_digits != null and limits.max_integer_digits.? == 0)) return .{ .fail = Issue.init(env, .decode, .invalid_value) };
    if (!std.unicode.utf8ValidateSlice(bytes)) return .{ .fail = Issue.init(env, .decode, .invalid_utf8) };
    if (bytes.len > limits.max_document_bytes) {
        var issue = Issue.init(env, .decode, .size_limit);
        issue.detail = .{ .resource = .{ .resource = .input_bytes, .limit = limits.max_document_bytes, .actual = bytes.len } };
        return .{ .fail = issue };
    }
    var scanner = std.json.Scanner.initCompleteInput(env.allocator, bytes);
    defer scanner.deinit();
    while (true) {
        const token = scanner.next() catch |err| {
            if (err == error.OutOfMemory) return .{ .die = outcomes.Defect.out_of_memory };
            return .{ .fail = Issue.init(env, .decode, .invalid_value) };
        };
        const scalar = switch (token) {
            .number, .string, .true, .false, .null => true,
            else => false,
        };
        const depth = scanner.stackHeight() + @as(usize, if (scalar) 1 else 0);
        if (depth > limits.max_depth) {
            var issue = Issue.init(env, .decode, .depth_limit);
            issue.detail = .{ .resource = .{ .resource = .json_depth, .limit = limits.max_depth, .actual = depth } };
            return .{ .fail = issue };
        }
        switch (token) {
            .number => |text| {
                if (text.len > limits.max_value_bytes) {
                    var issue = Issue.init(env, .decode, .size_limit);
                    issue.detail = .{ .resource = .{ .resource = .value_bytes, .limit = limits.max_value_bytes, .actual = text.len } };
                    return .{ .fail = issue };
                }
                if (limits.max_integer_digits) |limit| {
                    const number = @import("../json_number_codec.zig").Number.parse(text) catch return .{ .fail = Issue.init(env, .decode, .invalid_number) };
                    const digits: ?usize = number.integerDigits() catch null;
                    if (digits == null or digits.? > limit) {
                        var issue = Issue.init(env, .decode, .integer_digits);
                        issue.detail = .{ .integer_digits = .{ .limit = limit, .required = digits } };
                        return .{ .fail = issue };
                    }
                }
            },
            else => {},
        }
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
