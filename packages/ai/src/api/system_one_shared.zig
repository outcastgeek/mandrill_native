//! Public System One transport injection with the shared classifier parser.
const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const m = @import("../models.zig");
const shared = @import("classifier_shared.zig");
pub const SystemOneWireRequest = struct { state: std.json.Value, questions: std.json.Value };
/// Callbacks return allocator-owned values and use error_message for native
/// exception detail. Their opaque context remains borrowed for the call.
pub const SystemOneTransport = struct {
    api: []const u8,
    label: []const u8,
    context: ?*anyopaque = null,
    url: *const fn (?*anyopaque, std.mem.Allocator, t.Model, ?*?[]const u8) anyerror!std.Uri,
    payload: *const fn (?*anyopaque, std.mem.Allocator, t.Model, SystemOneWireRequest, ?*?[]const u8) anyerror!std.json.Value,
    output: *const fn (?*anyopaque, std.mem.Allocator, std.json.Value, ?*?[]const u8) anyerror!std.json.Value,
};
fn execute(a: std.mem.Allocator, operation: m.OperationContext, transport: SystemOneTransport, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions, result: *t.ClassifierResult, message: *?[]const u8, detail: *?shared.ClassifierHttpError) !void {
    if (!std.mem.eql(u8, model.api, transport.api)) {
        message.* = try std.fmt.allocPrint(a, "Unsupported classifier API: {s}", .{model.api});
        return error.UnsupportedClassifierApi;
    }
    if (context.images) |images| if (images.len > 0) {
        message.* = try std.fmt.allocPrint(a, "{s} does not support image input", .{transport.label});
        return error.ClassifierImagesUnsupported;
    };
    if (context.questions != .object) return error.InvalidQuestions;
    var questions = j.map(a);
    const entries = try j.objectEntries(a, context.questions.object);
    defer a.free(entries);
    for (entries) |entry| {
        var value = try j.clone(a, entry.node.*);
        if (std.mem.eql(u8, j.field(value, "type") orelse "", "bool")) {
            value = try j.objectSpread(a, value);
            try j.put(a, &value, "type", j.str("noul"));
        }
        try j.put(a, &questions, entry.key, value);
    }
    const url = try transport.url(transport.context, a, model, message);
    const payload = try transport.payload(transport.context, a, model, .{ .state = context.state, .questions = questions }, message);
    var request = options;
    request.errorMessage = message;
    var scoped = operation;
    scoped.allocator = a;
    const body = try shared.postClassifierRequest(scoped, transport.label, url, model, payload, request, &.{}, detail);
    const output = try transport.output(transport.context, a, body, message);
    result.usage = try shared.parseClassifierUsage(a, j.get(output, "usage") orelse .null, model);
    var parser = @import("nonchat.zig").Client{ .operation = operation, .model = model, .options = options, .errorLabel = transport.label };
    result.answers = @import("classifiers.zig").parseSystemOneAnswers(a, &parser, j.get(output, "answers") orelse .null, context) catch |err| {
        message.* = parser.failureMessage;
        return err;
    };
}
pub fn classifySystemOne(operation: m.OperationContext, transport: SystemOneTransport, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions) !t.ClassifierResult {
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    var result = t.ClassifierResult{ .api = model.api, .provider = model.provider, .model = model.id, .answers = j.map(a), .timestamp = @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds()) };
    var message: ?[]const u8 = null;
    var detail: ?shared.ClassifierHttpError = null;
    execute(a, operation, transport, model, context, options, &result, &message, &detail) catch |err| {
        if (err == error.OutOfMemory) return err;
        result.stopReason = if (err == error.Canceled) .aborted else .@"error";
        const errors = @import("../utils/error_body.zig");
        const normalized = try errors.normalizeProviderError(a, .{ .sdk = .{ .message = message orelse @errorName(err), .status = if (detail) |http| (if (http.status) |status| @as(f64, @floatFromInt(status)) else null) else null, .body = if (detail) |http| http.body else null } });
        result.errorMessage = try errors.formatProviderError(a, normalized, try std.fmt.allocPrint(a, "{s} error", .{transport.label}));
        if (err == error.Canceled) operation.io.recancel();
    };
    return t.cloneInto(t.ClassifierResult, operation.allocator, result);
}
