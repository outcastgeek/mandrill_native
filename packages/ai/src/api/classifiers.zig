const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const common = @import("nonchat.zig");
const models = @import("../models.zig");
fn apiIs(model: t.Model, name: []const u8) bool {
    return std.mem.eql(u8, model.api, name);
}
fn classifierEndpoint(a: std.mem.Allocator, base: []const u8, name: []const u8) !std.Uri {
    const text = try std.fmt.allocPrint(a, "{s}/", .{std.mem.trimEnd(u8, base, "/")});
    const uri = @import("../transport/uri.zig");
    return uri.resolveHttpUrl(a, try uri.parseHttpUrl(a, text), name);
}
fn label(model: t.Model) []const u8 {
    return if (apiIs(model, "openai-decisions")) "OpenAI Decisions" else if (apiIs(model, "cloudflare-workers-ai-system-one")) "Cloudflare Workers AI" else "System One API";
}
fn failure(a: std.mem.Allocator, client: *common.Client, comptime format: []const u8, args: anytype) !void {
    client.failureMessage = try std.fmt.allocPrint(a, format, args);
}
fn required(a: std.mem.Allocator, client: *common.Client, value: std.json.Value, key: []const u8, field: []const u8) !f64 {
    return common.requiredNumber(value, key) catch |err| {
        try failure(a, client, "{s} returned an invalid {s}", .{ client.errorLabel, field });
        return err;
    };
}
fn question(a: std.mem.Allocator, raw: std.json.Value) !t.ClassifierQuestion {
    return std.json.parseFromValueLeaky(t.ClassifierQuestion, a, raw, .{});
}
fn probabilities(a: std.mem.Allocator, client: *common.Client, value: std.json.Value, decisions: bool, id: []const u8) !std.json.Value {
    var out = j.map(a);
    if (decisions) {
        if (value != .array) {
            try failure(a, client, "{s} returned invalid probabilities for {s}", .{ client.errorLabel, id });
            return error.InvalidClassifierProbabilities;
        }
        for (value.array.items) |entry| {
            const name = j.field(entry, "value") orelse {
                try failure(a, client, "{s} returned invalid probabilities for {s}", .{ client.errorLabel, id });
                return error.InvalidClassifierProbabilities;
            };
            try j.put(a, &out, name, j.num(try required(a, client, entry, "probability", try std.fmt.allocPrint(a, "probability for {s}.{s}", .{ id, name }))));
        }
    } else {
        if (value != .object) {
            try failure(a, client, "{s} returned invalid probabilities for {s}", .{ client.errorLabel, id });
            return error.InvalidClassifierProbabilities;
        }
        const entries = try j.objectEntries(a, value.object);
        defer a.free(entries);
        for (entries) |entry| {
            const number = common.number(entry.node.*) catch |err| {
                try failure(a, client, "{s} returned an invalid probability for {s}.{s}", .{ client.errorLabel, id, entry.key });
                return err;
            };
            try j.put(a, &out, entry.key, j.num(number));
        }
    }
    return out;
}
fn parseAnswer(a: std.mem.Allocator, client: *common.Client, q: std.meta.Tag(t.ClassifierQuestion), value: std.json.Value, decisions: bool, id: []const u8) !std.json.Value {
    if (value != .object) {
        try failure(a, client, "{s} did not return an answer for {s}", .{ client.errorLabel, id });
        return error.ClassifierAnswerMissing;
    }
    const kind = j.field(value, "type") orelse "";
    if (decisions and std.mem.eql(u8, kind, "refusal")) {
        try failure(a, client, "{s} refused to answer {s}", .{ client.errorLabel, id });
        return error.ClassifierRefusal;
    }
    const expected: []const u8 = switch (q) {
        .choice => "choice",
        .score => "score",
        .bool => if (decisions) "predicate" else "noul",
    };
    if (!std.mem.eql(u8, kind, expected) or (q == .choice and j.field(value, "choice") == null)) {
        try failure(a, client, "{s} did not return a {s} answer for {s}", .{ client.errorLabel, if (q == .bool and !decisions) "bool" else expected, id });
        return error.InvalidClassifierAnswer;
    }
    const answer: t.ClassifierAnswer = switch (q) {
        .choice => .{ .choice = .{ .choice = j.field(value, "choice").?, .probabilities = try probabilities(a, client, j.get(value, "probabilities") orelse .null, decisions, id), .confidence = try required(a, client, value, "confidence", try std.fmt.allocPrint(a, "confidence for {s}", .{id})) } },
        .score => .{ .score = .{ .score = try required(a, client, value, "score", try std.fmt.allocPrint(a, "score for {s}", .{id})), .confidence = try required(a, client, value, "confidence", try std.fmt.allocPrint(a, "confidence for {s}", .{id})) } },
        .bool => .{ .bool = .{ .probability = try required(a, client, value, if (decisions) "probability" else "noul", try std.fmt.allocPrint(a, "probability for {s}", .{id})) } },
    };
    return j.from(a, answer);
}
/// Shared native parser for custom System One transports.
pub fn parseSystemOneAnswers(a: std.mem.Allocator, client: *common.Client, value: std.json.Value, context: t.ClassifierContext) !std.json.Value {
    if (value != .object) {
        try failure(a, client, "{s} returned an unexpected response", .{client.errorLabel});
        return error.InvalidClassifierAnswers;
    }
    if (context.questions != .object) return error.InvalidQuestions;
    var answers = j.map(a);
    const entries = try j.objectEntries(a, context.questions.object);
    defer a.free(entries);
    for (entries) |entry| {
        const kind = j.field(entry.node.*, "type") orelse "";
        const tag: std.meta.Tag(t.ClassifierQuestion) = if (std.mem.eql(u8, kind, "choice")) .choice else if (std.mem.eql(u8, kind, "score")) .score else .bool;
        try j.put(a, &answers, entry.key, try parseAnswer(a, client, tag, j.get(value, entry.key) orelse .null, false, entry.key));
    }
    return answers;
}
fn decisionQuestions(a: std.mem.Allocator, questions: std.json.Value) !std.json.Value {
    var out = j.array(a);
    const entries = try j.objectEntries(a, questions.object);
    defer a.free(entries);
    for (entries) |entry| {
        const q = try question(a, entry.node.*);
        var item = j.map(a);
        try j.put(a, &item, "type", j.str(switch (q) {
            .choice => "choice",
            .score => "score",
            .bool => "predicate",
        }));
        try j.put(a, &item, "name", j.str(entry.key));
        switch (q) {
            .choice => |v| {
                try j.put(a, &item, "instructions", j.str(v.instructions));
                var choices = j.array(a);
                if (v.criteria != .object) return error.InvalidQuestion;
                const criteria = try j.objectEntries(a, v.criteria.object);
                defer a.free(criteria);
                for (criteria) |criterion| {
                    var choice = j.map(a);
                    try j.put(a, &choice, "value", j.str(criterion.key));
                    if (j.text(criterion.node.*)) |text| if (text.len > 0) {
                        try j.put(a, &choice, "description", j.str(text));
                    };
                    try choices.array.append(choice);
                }
                try j.put(a, &item, "choices", choices);
            },
            .score => |v| {
                try j.put(a, &item, "instructions", j.str(v.instructions));
                var levels = j.array(a);
                for (v.criteria) |criterion| try levels.array.append(try j.from(a, .{ .label = criterion }));
                try j.put(a, &item, "levels", levels);
            },
            .bool => |v| {
                var text = std.Io.Writer.Allocating.init(a);
                try text.writer.writeAll(v.instructions);
                if (v.criteria.true.len > 0 or v.criteria.false.len > 0) try text.writer.writeAll("\n\n");
                if (v.criteria.true.len > 0) try text.writer.print("True means: {s}", .{v.criteria.true});
                if (v.criteria.false.len > 0) {
                    if (v.criteria.true.len > 0) try text.writer.writeByte('\n');
                    try text.writer.print("False means: {s}", .{v.criteria.false});
                }
                try j.put(a, &item, "instructions", j.str(try text.toOwnedSlice()));
            },
        }
        try out.array.append(item);
    }
    return out;
}
fn execute(a: std.mem.Allocator, client: *common.Client, context: t.ClassifierContext, output: *t.ClassifierResult) !void {
    const model = client.model;
    const decisions = apiIs(model, "openai-decisions");
    const cloudflare = apiIs(model, "cloudflare-workers-ai-system-one");
    if (!decisions and !cloudflare and !apiIs(model, "typesafe-system-one")) {
        try failure(a, client, "Unsupported classifier API: {s}", .{model.api});
        return error.UnsupportedClassifierApi;
    }
    if (context.questions != .object) return error.InvalidQuestions;
    var payload = j.map(a);
    try j.put(a, &payload, "model", j.str(model.id));
    if (decisions) {
        const state = try @import("../json/value.zig").stringify(a, context.state, .{});
        var input = j.str(state);
        if (context.images) |images| if (images.len > 0) {
            if (images.len > 128) {
                try failure(a, client, "OpenAI Decisions accepts at most 128 images, got {d}", .{images.len});
                return error.TooManyClassifierImages;
            }
            var content = j.array(a);
            try content.array.append(try j.from(a, .{ .type = "input_text", .text = state }));
            for (images) |image| try content.array.append(try j.from(a, .{ .type = "input_image", .image_url = try @import("common.zig").imageDataUri(a, image) }));
            input = j.array(a);
            try input.array.append(try j.from(a, .{ .role = "user", .content = content }));
        };
        try j.put(a, &payload, "input", input);
        try j.put(a, &payload, "questions", try decisionQuestions(a, context.questions));
    } else {
        if (context.images) |images| if (images.len > 0) {
            try failure(a, client, "{s} does not support image input", .{label(model)});
            return error.ClassifierImagesUnsupported;
        };
        var questions = j.map(a);
        const entries = try j.objectEntries(a, context.questions.object);
        defer a.free(entries);
        for (entries) |entry| {
            var value = try j.clone(a, entry.node.*);
            if (std.mem.eql(u8, j.field(value, "type") orelse "", "bool")) try j.put(a, &value, "type", j.str("noul"));
            try j.put(a, &questions, entry.key, value);
        }
        const request = try j.from(a, .{ .state = context.state, .questions = questions });
        if (cloudflare) try j.put(a, &payload, "input", request) else {
            try j.put(a, &payload, "state", context.state);
            try j.put(a, &payload, "questions", questions);
        }
    }
    var body = try client.post(a, try classifierEndpoint(a, model.baseUrl, if (decisions) "decisions" else if (cloudflare) "run" else "systemone"), payload, true, true, 2, if (decisions) 504 else null);
    if (body != .object) {
        try failure(a, client, "{s} returned an unexpected response", .{label(model)});
        return error.InvalidClassifierResponse;
    }
    if (cloudflare) {
        if (j.get(body, "success")) |value| if (value == .bool and !value.bool) {
            var messages: std.ArrayList([]const u8) = .empty;
            if (j.get(body, "errors")) |errors| if (errors == .array) {
                for (errors.array.items) |entry| if (j.field(entry, "message")) |message| try messages.append(a, message);
            };
            client.failureMessage = if (messages.items.len > 0) try std.fmt.allocPrint(a, "Cloudflare Workers AI error: {s}", .{try std.mem.join(a, "; ", messages.items)}) else "Cloudflare Workers AI request failed";
            return error.CloudflareRequestFailed;
        };
        body = j.get(body, "result") orelse .null;
        if (body != .object) {
            try failure(a, client, "{s} returned an unexpected response", .{label(model)});
            return error.InvalidClassifierResponse;
        }
        if (j.get(body, "answers") == null) {
            if (!std.mem.eql(u8, j.field(body, "state") orelse "", "Completed")) {
                const state = try @import("../utils/diagnostics.zig").formatThrownValue(a, if (j.get(body, "state")) |value| .{ .value = value } else .undefined_value);
                try failure(a, client, "Cloudflare Workers AI run did not complete (state: {s})", .{state});
                return error.CloudflareRunIncomplete;
            }
            body = j.get(body, "result") orelse .null;
            if (body != .object) {
                try failure(a, client, "{s} returned an unexpected response", .{label(model)});
                return error.InvalidClassifierResponse;
            }
        }
    }
    output.usage = try common.parseUsage(a, model, j.get(body, "usage"));
    const raw = j.get(body, "answers") orelse .null;
    if ((decisions and raw != .array) or (!decisions and raw != .object)) {
        try failure(a, client, "{s} returned an unexpected response", .{label(model)});
        return error.InvalidClassifierAnswers;
    }
    if (!decisions) {
        output.answers = try parseSystemOneAnswers(a, client, raw, context);
        return;
    }
    var by_name = j.map(a);
    if (decisions) {
        if (raw != .array) return error.InvalidClassifierAnswers;
        for (raw.array.items) |answer| if (j.field(answer, "name")) |name| {
            try j.put(a, &by_name, name, answer);
        };
    } else by_name = raw;
    var answers = j.map(a);
    const entries = try j.objectEntries(a, context.questions.object);
    defer a.free(entries);
    for (entries) |entry| try j.put(a, &answers, entry.key, try parseAnswer(a, client, std.meta.activeTag(try question(a, entry.node.*)), j.get(by_name, entry.key) orelse .null, decisions, entry.key));
    output.answers = answers;
}
fn executeFor(comptime expected: ?[]const u8, comptime resolve_provider_model: bool, a: std.mem.Allocator, client: *common.Client, context: t.ClassifierContext, output: *t.ClassifierResult) !void {
    if (expected) |api| if (!apiIs(client.model, api)) {
        try failure(a, client, "Unsupported classifier API: {s}", .{client.model.api});
        return error.UnsupportedClassifierApi;
    };
    if (resolve_provider_model and std.mem.eql(u8, client.model.provider, "cloudflare-workers-ai"))
        client.model = try @import("../providers/cloudflare_stream.zig").resolveCloudflareModel(a, client.model, client.options.env);
    return execute(a, client, context, output);
}
fn classifyFor(comptime expected: ?[]const u8, comptime resolve_provider_model: bool, operation: models.OperationContext, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions) !t.ClassifierResult {
    if ((expected == null or std.mem.eql(u8, expected.?, "llama-cpp-classify")) and apiIs(model, "llama-cpp-classify")) return @import("llama_cpp_classify.zig").classify(null, operation, model, context, options);
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    var output = t.ClassifierResult{ .api = model.api, .provider = model.provider, .model = model.id, .answers = j.map(a), .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds())) };
    var client = common.Client{ .operation = operation, .model = model, .options = options, .errorLabel = label(model) };
    executeFor(expected, resolve_provider_model, a, &client, context, &output) catch |err| {
        if (err == error.OutOfMemory) return err;
        output.stopReason = if (err == error.Canceled) .aborted else .@"error";
        output.errorMessage = if (apiIs(model, "openai-decisions") and client.status == 504) "OpenAI Decisions error (504): the request timed out at the gateway. Very large inputs (above roughly 600K tokens) currently exceed its time limit." else try client.errorMessage(a, err, if (apiIs(model, "openai-decisions")) "OpenAI Decisions" else if (apiIs(model, "cloudflare-workers-ai-system-one")) "Cloudflare Workers AI" else "System One API");
        if (err == error.Canceled) operation.io.recancel();
    };
    return t.cloneInto(t.ClassifierResult, operation.allocator, output);
}
pub fn classify(_: ?*anyopaque, operation: models.OperationContext, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions) !t.ClassifierResult {
    return classifyFor(null, true, operation, model, context, options);
}
pub fn Api(comptime api: []const u8) type {
    return struct {
        pub fn classify(_: ?*anyopaque, operation: models.OperationContext, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions) !t.ClassifierResult {
            return classifyFor(api, false, operation, model, context, options);
        }
        fn providerClassify(_: ?*anyopaque, operation: models.OperationContext, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions) !t.ClassifierResult {
            return classifyFor(api, true, operation, model, context, options);
        }
        pub fn implementation() models.ApiImplementation {
            return .{ .api = api, .classify = @This().classify };
        }
        fn providerImplementation() models.ApiImplementation {
            return .{ .api = api, .classify = providerClassify };
        }
    };
}
pub fn implementation(comptime api: []const u8) models.ApiImplementation {
    return Api(api).providerImplementation();
}
