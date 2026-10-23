const std = @import("std");
const ai = @import("ai");
pub fn main(init: std.process.Init) !void {
    var operation_arena = std.heap.ArenaAllocator.init(init.gpa);
    defer operation_arena.deinit();
    const a = operation_arena.allocator();
    const io = init.io;
    const schema = try ai.Type.Object(a, try ai.json_value.from(a, .{ .city = try ai.Type.String(a, .{}) }), &.{"city"}, .{});
    const Arguments = struct { city: []const u8 };
    const typed = ai.Type.Typed(Arguments, schema);
    const tool = ai.types.Tool{ .name = "weather", .description = "Read the weather for a city", .parameters = typed.schema };
    // Provider schema entrypoints can also describe configuration tools.
    const model_schema = ai.providers.model_schema;
    const compat_schema = ai.providers.compat_schema;
    const configuration = ai.types.Tool{ .name = "configure_provider_model", .description = "Describe a provider model and its compatibility overrides", .parameters = try ai.Type.Object(a, .{
        .compat = try compat_schema.ProviderCompatSchema(a),
        .cost = try model_schema.ModelCostSchema(a),
        .thinkingLevelMap = try model_schema.ThinkingLevelMapSchema(a),
        .promptCache = try model_schema.ModelPromptCacheSchema(a),
        .inputLimits = try model_schema.ModelInputLimitsSchema(a),
        .thinkingLevel = try model_schema.ThinkingLevelSchema(a),
        .defaultThinkingLevel = try model_schema.ModelThinkingLevelSchema(a),
        .input = try ai.Type.Array(a, try model_schema.ModelInputModalitySchema(a), .{}),
        .cacheRetention = try model_schema.CacheRetentionSchema(a),
        .sessionAffinityFormat = try compat_schema.SessionAffinityFormatSchema(a),
        .thinkingTokenBudgetField = try compat_schema.ThinkingTokenBudgetFieldSchema(a),
    }, &.{ "compat", "cost", "input" }, .{}) };
    const call = try ai.fauxToolCall(a, io, "weather", try ai.json_value.from(a, .{ .city = "Toronto" }), null);
    const faux = try ai.fauxProvider(init.gpa, io, .{ .api = "example-tools" });
    defer faux.deinit();
    try faux.setResponses(&.{.{ .message = try ai.fauxAssistantMessage(a, io, .{ .blocks = &.{call} }, .{ .stopReason = .toolUse }) }});
    var environment = ai.EnvironmentContext{ .allocator = a, .environ = init.environ_map };
    var models = try ai.createModels(init.gpa, io, .{ .authContext = environment.context(), .providers = &.{faux.provider} });
    defer models.deinit();
    var result = try models.complete(a, faux.getModel(null).?, .{ .messages = &.{}, .tools = &.{ tool, configuration } }, .{}, .{});
    defer result.deinit();
    const validated = try ai.validateToolCall(a, &.{ tool, configuration }, result.value().content[0].toolCall, .{});
    const arguments = try std.json.parseFromValueLeaky(ai.Static(@TypeOf(typed)), a, validated, .{});
    _ = arguments.city; // Native typing follows explicit schema validation.
    // The application chooses whether and how to execute the returned call.
    var buffer: [4096]u8 = undefined;
    var output = std.Io.File.stdout().writerStreaming(io, &buffer);
    try std.json.Stringify.value(validated, .{}, &output.interface);
    try output.interface.writeByte('\n');
    try output.interface.flush();
}
