const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const diagnostics = @import("diagnostics.zig");
pub fn getModelType(model: t.Model) t.ModelType {
    return model.modelType();
}
pub fn isModelType(model: t.Model, kind: t.ModelType) bool {
    return model.modelType() == kind;
}
pub fn assertChatModel(model: t.Model) !void {
    if (!isModelType(model, .chat)) return error.ExpectedChatModel;
}
pub fn assertImageModel(model: t.Model) !void {
    if (!isModelType(model, .image)) return error.ExpectedImageModel;
}
pub fn assertClassifierModel(model: t.Model) !void {
    if (!isModelType(model, .classifier)) return error.ExpectedClassifierModel;
}
pub fn assertClassifierInputSupported(model: t.Model, context: t.ClassifierContext) !void {
    if (context.images) |images| if (images.len > 0) {
        for (model.input) |input| if (input == .image) return;
        return error.ClassifierImagesUnsupported;
    };
}
pub fn imageErrorResult(a: std.mem.Allocator, io: std.Io, model: t.Model, message: []const u8, aborted: bool) !t.AssistantImages {
    return imageErrorResultWithError(a, io, model, .{ .text = message }, aborted);
}
pub fn classifierErrorResult(a: std.mem.Allocator, io: std.Io, model: t.Model, message: []const u8, aborted: bool) !t.ClassifierResult {
    return classifierErrorResultWithError(a, io, model, .{ .text = message }, aborted);
}
fn errorResultMessage(a: std.mem.Allocator, value: diagnostics.ThrownValue) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    // These result helpers read Error.message directly, including an empty
    // message. Diagnostic formatting has a different fallback to Error.name.
    const message = if (value == .exception) value.exception.message else try diagnostics.formatThrownValue(temporary.allocator(), value);
    return a.dupe(u8, message);
}
/// Native counterpart of the source helper's unknown thrown value. The text
/// entrypoint above remains convenient for callers that already have a message.
pub fn imageErrorResultWithError(a: std.mem.Allocator, io: std.Io, model: t.Model, value: diagnostics.ThrownValue, aborted: bool) !t.AssistantImages {
    const api = try a.dupe(u8, model.api);
    errdefer a.free(api);
    const provider = try a.dupe(u8, model.provider);
    errdefer a.free(provider);
    const id = try a.dupe(u8, model.id);
    errdefer a.free(id);
    const message = try errorResultMessage(a, value);
    errdefer a.free(message);
    const timestamp: f64 = @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds());
    return .{ .api = api, .provider = provider, .model = id, .output = &.{}, .stopReason = if (aborted) .aborted else .@"error", .errorMessage = message, .timestamp = timestamp };
}
pub fn classifierErrorResultWithError(a: std.mem.Allocator, io: std.Io, model: t.Model, value: diagnostics.ThrownValue, aborted: bool) !t.ClassifierResult {
    const api = try a.dupe(u8, model.api);
    errdefer a.free(api);
    const provider = try a.dupe(u8, model.provider);
    errdefer a.free(provider);
    const id = try a.dupe(u8, model.id);
    errdefer a.free(id);
    const message = try errorResultMessage(a, value);
    errdefer a.free(message);
    const timestamp: f64 = @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds());
    return .{ .api = api, .provider = provider, .model = id, .answers = j.map(a), .stopReason = if (aborted) .aborted else .@"error", .errorMessage = message, .timestamp = timestamp };
}
