const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const models = @import("../models.zig");
const common = @import("nonchat.zig");
fn property(a: std.mem.Allocator, client: *common.Client, target: ?std.json.Value, key: []const u8) !?std.json.Value {
    const object = target orelse {
        client.failureMessage = try std.fmt.allocPrint(a, "Cannot read properties of undefined (reading '{s}')", .{key});
        return error.InvalidOpenRouterImageResponse;
    };
    if (object == .null) {
        client.failureMessage = try std.fmt.allocPrint(a, "Cannot read properties of null (reading '{s}')", .{key});
        return error.InvalidOpenRouterImageResponse;
    }
    return j.get(object, key);
}
fn firstChoice(a: std.mem.Allocator, client: *common.Client, choices: ?std.json.Value) !?std.json.Value {
    const value = choices orelse return property(a, client, null, "0");
    return switch (value) {
        .array => |items| if (items.items.len > 0) items.items[0] else null,
        .string => |text| string: {
            if (text.len == 0) break :string null;
            const units = try std.unicode.wtf8ToWtf16LeAlloc(a, text);
            defer a.free(units);
            break :string j.str(try std.unicode.wtf16LeToWtf8Alloc(a, units[0..1]));
        },
        else => try property(a, client, value, "0"),
    };
}
fn generatedImage(a: std.mem.Allocator, raw: []const u8) !?t.Content {
    var regex = try @import("../regex/regexp.zig").Regex.init(a, null, "^data:([^;]+);base64,(.+)$", .{});
    defer regex.deinit();
    var match = (try regex.exec(a, raw, 0)) orelse return null;
    defer match.deinit();
    return .{ .image = .{ .mimeType = (try match.text(a, 1)).?, .data = (try match.text(a, 2)).? } };
}
fn execute(a: std.mem.Allocator, client: *common.Client, context: t.ImagesContext, output: *t.AssistantImages) !void {
    if (client.options.apiKey == null or client.options.apiKey.?.len == 0) return error.MissingAuthentication;
    const sdk_headers = @import("sdk_request_headers.zig");
    const headers = @import("../utils/headers.zig");
    client.sdkDefaultHeaders = try sdk_headers.openAI(a, client.operation.io, client.operation.authContext, client.options.timeoutMs);
    const custom = try sdk_headers.openAICustomHeaders(a, client.operation.io, client.operation.authContext, &client.failureMessage);
    const caller = try headers.merge(a, &.{try headers.spread(a, &.{ try headers.fromValue(a, client.model.headers), client.options.headers })});
    client.sdkCustomHeaders = try headers.merge(a, &.{ custom, caller });
    var content = j.array(a);
    for (context.input) |item| switch (item) {
        .text => |text| try content.array.append(try j.from(a, .{ .type = "text", .text = try @import("../utils/unicode.zig").sanitizeSurrogates(a, text.text) })),
        .image => |image| try content.array.append(try j.from(a, .{ .type = "image_url", .image_url = .{ .url = try @import("common.zig").imageDataUri(a, image) } })),
        else => return error.UnsupportedImageInput,
    };
    const has_text = for (client.model.output orelse &.{}) |modality| {
        if (modality == .text) break true;
    } else false;
    const payload = try j.from(a, .{ .model = client.model.id, .messages = .{.{ .role = "user", .content = content }}, .stream = false, .modalities = if (has_text) @as([]const []const u8, &.{ "image", "text" }) else @as([]const []const u8, &.{"image"}) });
    const response = try client.postOpenAIImages(a, client.model.baseUrl, payload);
    if (response == .null) {
        client.failureMessage = if (client.responseUndefined) "Cannot read properties of undefined (reading 'id')" else "Cannot read properties of null (reading 'id')";
        return error.InvalidOpenRouterImageResponse;
    }
    if (j.get(response, "id")) |id| {
        if (j.text(id)) |text| {
            output.responseId = text;
            j.markPresent(output, "responseId");
        } else try j.retainField(a, output, "responseId", id);
    } else j.markAbsent(output, "responseId");
    const usage = j.get(response, "usage") orelse .null;
    if (j.truthy(usage)) {
        const accounting = @import("../utils/usage.zig");
        const numbers = @import("../utils/numbers.zig");
        const details = j.get(usage, "prompt_tokens_details") orelse .null;
        const writes = accounting.orZero(details, "cache_write_tokens");
        const cached = accounting.orZero(details, "cached_tokens");
        const write_number = try accounting.number(a, writes);
        const reads = if (write_number > 0) j.num(numbers.maximum(0, try accounting.number(a, cached) - write_number)) else cached;
        var result = t.Usage{ .input = numbers.maximum(0, try accounting.number(a, accounting.orZero(usage, "prompt_tokens")) - try accounting.number(a, reads) - write_number) };
        const output_tokens = accounting.orZero(usage, "completion_tokens");
        try accounting.set(a, &result, "output", output_tokens);
        try accounting.set(a, &result, "cacheRead", reads);
        try accounting.set(a, &result, "cacheWrite", writes);
        try accounting.total(a, &result);
        const rates = (try j.fieldValue(a, client.model, "cost")) orelse return error.InvalidModelCost;
        if (rates == .null) return error.InvalidModelCost;
        result.cost.input = try accounting.number(a, j.get(rates, "input")) / 1e6 * result.input;
        result.cost.output = try accounting.number(a, j.get(rates, "output")) / 1e6 * try accounting.number(a, output_tokens);
        result.cost.cacheRead = try accounting.number(a, j.get(rates, "cacheRead")) / 1e6 * try accounting.number(a, reads);
        result.cost.cacheWrite = try accounting.number(a, j.get(rates, "cacheWrite")) / 1e6 * write_number;
        result.cost.total = result.cost.input + result.cost.output + result.cost.cacheRead + result.cost.cacheWrite;
        output.usage = result;
    }
    const choice = (try firstChoice(a, client, j.get(response, "choices"))) orelse return;
    if (!j.truthy(choice)) return;
    const message = try property(a, client, choice, "message");
    const text = try property(a, client, message, "content");
    var blocks: std.ArrayList(t.Content) = .empty;
    if (text) |value| if (j.text(value)) |bytes| if (bytes.len > 0) {
        try blocks.append(a, .{ .text = .{ .text = bytes } });
        output.output = blocks.items;
    };
    const images = (try property(a, client, message, "images")) orelse return;
    // A string's iterable elements are strings too and have no image_url.
    if (images == .null or images == .string) return;
    if (images != .array) {
        client.failureMessage = "Image response images value is not iterable";
        return error.InvalidOpenRouterImageResponse;
    }
    for (images.array.items) |image| {
        const raw = (try property(a, client, image, "image_url")) orelse continue;
        const address = (if (raw == .string) raw else j.get(raw, "url")) orelse continue;
        if (address == .null) continue;
        const url = j.text(address) orelse {
            client.failureMessage = "imageUrl?.startsWith is not a function";
            return error.InvalidOpenRouterImageResponse;
        };
        if (!std.mem.startsWith(u8, url, "data:")) continue;
        if (try generatedImage(a, url)) |value| {
            try blocks.append(a, value);
            output.output = blocks.items;
        }
    }
}
pub fn generateImages(_: ?*anyopaque, operation: models.OperationContext, model: t.Model, context: t.ImagesContext, options: t.ImagesOptions) !t.AssistantImages {
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    var output = t.AssistantImages{ .api = model.api, .provider = model.provider, .model = model.id, .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds())) };
    var client = common.Client{ .operation = operation, .model = model, .options = options, .errorLabel = "OpenRouter", .sdkImages = true };
    execute(a, &client, context, &output) catch |err| {
        if (err == error.OutOfMemory) return err;
        var aborted = err == error.Canceled;
        if (!aborted) operation.io.checkCancel() catch {
            aborted = true;
        };
        output.stopReason = if (aborted) .aborted else .@"error";
        output.errorMessage = try client.errorMessage(a, err, "OpenRouter");
        if (aborted) operation.io.recancel();
    };
    return t.cloneInto(t.AssistantImages, operation.allocator, output);
}
pub fn implementation() models.ApiImplementation {
    return .{ .api = "openrouter-images", .generateImages = generateImages };
}
