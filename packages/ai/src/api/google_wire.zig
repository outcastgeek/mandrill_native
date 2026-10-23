//! Provider request-tree conversion.
const std = @import("std");
const j = @import("../json/value.zig");
const t = @import("../types.zig");
const Value = std.json.Value;
const Allocator = std.mem.Allocator;
pub const Resources = struct { project: ?[]const u8 = null, location: ?[]const u8 = null };
pub fn cachedContentName(a: Allocator, vertex: bool, input: Value, resources: Resources) !Value {
    const name = j.text(input) orelse return error.InvalidGoogleCachedContentName;
    const qualified = std.mem.findScalar(u8, name, '/') != null;
    if (!vertex) return if (qualified) input else j.str(try std.fmt.allocPrint(a, "cachedContents/{s}", .{name}));
    if (std.mem.startsWith(u8, name, "projects/")) return input;
    if (std.mem.startsWith(u8, name, "locations/")) return j.str(try std.fmt.allocPrint(a, "projects/{s}/{s}", .{ resources.project orelse "undefined", name }));
    if (std.mem.startsWith(u8, name, "cachedContents/") or !qualified) return j.str(try std.fmt.allocPrint(a, "projects/{s}/locations/{s}/{s}{s}", .{ resources.project orelse "undefined", resources.location orelse "undefined", if (qualified) "" else "cachedContents/", name }));
    return input;
}
pub fn mergeExtraBody(a: Allocator, target: Value, source: Value) anyerror!Value {
    if (source != .object or target != .object) return error.InvalidGoogleExtraBody;
    var out = try j.clone(a, target);
    var iterator = source.object.iterator();
    while (iterator.next()) |entry| {
        const previous = j.get(out, entry.key_ptr.*) orelse .null;
        const value = if (previous == .object and entry.value_ptr.* == .object) try mergeExtraBody(a, previous, entry.value_ptr.*) else entry.value_ptr.*;
        try j.put(a, &out, entry.key_ptr.*, value);
    }
    return out;
}

pub fn fields(a: Allocator, source: Value, names: []const []const u8) !Value {
    var out = j.map(a);
    for (names) |name| if (j.get(source, name)) |value| if (value != .null) try j.put(a, &out, name, value);
    return out;
}
const UnsupportedField = enum {
    toolCall,
    toolResponse,
    partMetadata,
    partialArgs,
    willContinue,
    displayName,
    fileSearch,
    retrieval,
    enterpriseWebSearch,
    exaAiSearch,
    parallelAiSearch,
    disabledSafetyPolicies,
    name,
    streamableHttpTransport,
    blockingConfidence,
    excludeDomains,
    groundingTypes,
    apiKeyConfig,
    authType,
    googleServiceAccountConfig,
    httpBasicAuthConfig,
    oauthConfig,
    oidcConfig,
    includeServerSideToolInvocations,
    streamFunctionCallArguments,
    consentAudio,
    voiceConsentSignature,
    personGeneration,
    outputMimeType,
    outputCompressionQuality,
    imageOutputOptions,
    prominentPeople,
};
fn reject(source: Value, comptime names: []const []const u8) !void {
    inline for (names) |name| if (j.get(source, name) != null) {
        const field: UnsupportedField = @field(UnsupportedField, name);
        return @field(anyerror, "GoogleSdkUnsupported_" ++ @tagName(field));
    };
}
pub fn unsupportedField(err: anyerror) ?[]const u8 {
    inline for (comptime std.meta.fieldNames(UnsupportedField)) |name| {
        if (err == @field(anyerror, "GoogleSdkUnsupported_" ++ name)) return name;
    }
    return null;
}
fn schemaType(value: Value) !Value {
    const name = j.text(value) orelse return error.InvalidGoogleSchemaType;
    for ([_][]const u8{ "TYPE_UNSPECIFIED", "STRING", "NUMBER", "INTEGER", "BOOLEAN", "ARRAY", "OBJECT", "NULL" }) |known| if (std.ascii.eqlIgnoreCase(name, known)) return j.str(known);
    return j.str("TYPE_UNSPECIFIED");
}
pub fn schema(a: Allocator, input: Value) anyerror!Value {
    if (input == .null) return error.InvalidGoogleSchema;
    if (j.truthy(j.get(input, "type") orelse .null) and j.truthy(j.get(input, "anyOf") orelse .null)) return error.GoogleSchemaTypeAndAnyOf;
    var out = j.map(a);
    var source = if (input == .object) input else try j.objectSpread(a, input);
    if (j.get(input, "anyOf")) |variants| if (variants == .array and variants.array.items.len == 2) {
        for (variants.array.items, 0..) |variant, index| if (std.mem.eql(u8, j.field(variant, "type") orelse "", "null")) {
            try j.put(a, &out, "nullable", .{ .bool = true });
            source = variants.array.items[1 - index];
            break;
        };
    };
    if (source == .null) return error.InvalidGoogleSchema;
    if (source != .object) source = try j.objectSpread(a, source);
    if (j.get(source, "type")) |types| if (types == .array) {
        var alternatives = j.array(a);
        for (types.array.items) |item| {
            if (std.mem.eql(u8, j.text(item) orelse "", "null")) {
                try j.put(a, &out, "nullable", .{ .bool = true });
                continue;
            }
            var variant = j.map(a);
            try j.put(a, &variant, "type", try schemaType(item));
            try alternatives.array.append(variant);
        }
        if (alternatives.array.items.len == 1) try j.put(a, &out, "type", j.get(alternatives.array.items[0], "type").?) else try j.put(a, &out, "anyOf", alternatives);
    };
    var iterator = source.object.iterator();
    while (iterator.next()) |entry| {
        const name = entry.key_ptr.*;
        const value = entry.value_ptr.*;
        if (value == .null) continue;
        if (std.mem.eql(u8, name, "type")) {
            if (value == .array) continue;
            if (std.mem.eql(u8, j.text(value) orelse "", "null")) return error.GoogleSchemaOnlyNull;
            try j.put(a, &out, name, try schemaType(value));
        } else if (std.mem.eql(u8, name, "items")) try j.put(a, &out, name, try schema(a, value)) else if (std.mem.eql(u8, name, "anyOf")) {
            if (value != .array) return error.InvalidGoogleSchema;
            var variants = j.array(a);
            for (value.array.items) |variant| {
                if (std.mem.eql(u8, j.field(variant, "type") orelse "", "null")) {
                    try j.put(a, &out, "nullable", .{ .bool = true });
                    continue;
                }
                try variants.array.append(try schema(a, variant));
            }
            try j.put(a, &out, name, variants);
        } else if (std.mem.eql(u8, name, "properties")) {
            if (value != .object) return error.InvalidGoogleSchema;
            var properties = j.map(a);
            var property_iterator = value.object.iterator();
            while (property_iterator.next()) |property| try j.put(a, &properties, property.key_ptr.*, try schema(a, property.value_ptr.*));
            try j.put(a, &out, name, properties);
        } else if (!std.mem.eql(u8, name, "additionalProperties")) try j.put(a, &out, name, value);
    }
    return out;
}
fn isContent(value: Value) bool {
    return (j.get(value, "parts") orelse .null) == .array;
}
fn hasFunction(value: Value) bool {
    return j.get(value, "functionCall") != null or j.get(value, "functionResponse") != null;
}
fn partInput(a: Allocator, input: Value) !Value {
    return switch (input) {
        .null => error.MissingGooglePart,
        .object, .array => input,
        .string => try j.from(a, .{ .text = input.string }),
        .bool => error.UnsupportedGoogleBooleanPart,
        .integer, .float, .number_string => error.UnsupportedGoogleNumberPart,
    };
}
fn part(a: Allocator, vertex: bool, input: Value) !Value {
    if (vertex) try reject(input, &.{ "toolCall", "toolResponse" });
    var out = try fields(a, input, &.{ "mediaResolution", "audioTranscription", "codeExecutionResult", "executableCode", "functionResponse", "text", "thought", "thoughtSignature", "videoMetadata", "mediaProcessing" });
    if (!vertex) for ([_][]const u8{ "toolCall", "toolResponse", "partMetadata" }) |key| if (j.get(input, key)) |value| if (value != .null) try j.put(a, &out, key, value);
    for ([_][]const u8{ "fileData", "functionCall", "inlineData" }) |key| if (j.get(input, key)) |value| if (value != .null) {
        var converted = value;
        if (!vertex) {
            if (std.mem.eql(u8, key, "functionCall")) {
                try reject(value, &.{ "partialArgs", "willContinue" });
                converted = try fields(a, value, &.{ "args", "id", "name" });
            } else {
                try reject(value, &.{"displayName"});
                converted = try fields(a, value, if (std.mem.eql(u8, key, "fileData")) &.{ "fileUri", "mimeType" } else &.{ "data", "mimeType" });
            }
        }
        try j.put(a, &out, key, converted);
    };
    if (vertex) try reject(input, &.{"partMetadata"});
    return out;
}
pub fn content(a: Allocator, vertex: bool, input: Value) !Value {
    if (input == .null) return error.MissingGoogleContent;
    const is_content = isContent(input);
    var source_parts = if (is_content) j.get(input, "parts").? else j.array(a);
    if (!is_content) {
        if (input == .array) {
            if (input.array.items.len == 0) return error.MissingGoogleParts;
            for (input.array.items) |item| try source_parts.array.append(try partInput(a, item));
        } else try source_parts.array.append(try partInput(a, input));
    }
    var out = j.map(a);
    var parts = j.array(a);
    for (source_parts.array.items) |item| try parts.array.append(try part(a, vertex, item));
    try j.put(a, &out, "parts", parts);
    if (is_content) {
        if (j.get(input, "role")) |role| if (role != .null) try j.put(a, &out, "role", role);
    } else {
        try j.put(a, &out, "role", j.str("user"));
    }
    return out;
}
pub fn contents(a: Allocator, vertex: bool, input: Value) !Value {
    if (input == .null or (input == .array and input.array.items.len == 0)) return error.MissingGoogleContents;
    var out = j.array(a);
    if (input != .array) {
        if (hasFunction(input)) return error.GoogleFunctionPartRequiresContent;
        try out.array.append(try content(a, vertex, input));
        return out;
    }
    const content_array = isContent(input.array.items[0]);
    for (input.array.items) |item| {
        if (isContent(item) != content_array) return error.MixedGoogleContentAndParts;
        if (!content_array and hasFunction(item)) return error.GoogleFunctionPartListRequiresContent;
    }
    if (content_array) {
        for (input.array.items) |item| try out.array.append(try content(a, vertex, item));
    } else try out.array.append(try content(a, vertex, input));
    return out;
}
fn declaration(a: Allocator, input: Value) !Value {
    if (input == .null) return error.InvalidGoogleFunctionDeclaration;
    var out = try j.clone(a, input);
    for ([_][]const u8{ "parameters", "response" }, [_][]const u8{ "parametersJsonSchema", "responseJsonSchema" }) |name, json_name| if (j.get(out, name)) |value| if (j.truthy(value)) {
        if (j.get(value, "$schema") == null) try j.put(a, &out, name, try schema(a, value)) else if (!j.truthy(j.get(out, json_name) orelse .null)) {
            try j.put(a, &out, json_name, value);
            _ = out.object.swapRemove(name);
        }
    };
    return out;
}
pub fn tools(a: Allocator, vertex: bool, input: Value) !Value {
    if (input != .array) return error.InvalidGoogleTools;
    var out = j.array(a);
    for (input.array.items) |original| {
        if (original == .null) return error.InvalidGoogleTool;
        var tool = try j.clone(a, original);
        // tTool transforms declarations before either backend converter runs.
        if (j.get(tool, "functionDeclarations")) |declarations| if (j.truthy(declarations)) {
            if (declarations != .array) return error.InvalidGoogleFunctionDeclarations;
            var values = j.array(a);
            for (declarations.array.items) |item| try values.array.append(try declaration(a, item));
            try j.put(a, &tool, "functionDeclarations", values);
        };
        var converted = j.map(a);
        if (vertex) {
            if (j.get(tool, "retrieval")) |value| if (value != .null) try j.put(a, &converted, "retrieval", value);
        } else try reject(tool, &.{"retrieval"});
        if (j.get(tool, "googleMaps")) |maps| if (maps != .null) {
            var result = maps;
            if (!vertex) {
                result = j.map(a);
                if (j.get(maps, "authConfig")) |auth| if (auth != .null) {
                    try reject(auth, &.{ "apiKeyConfig", "authType", "googleServiceAccountConfig", "httpBasicAuthConfig", "oauthConfig", "oidcConfig" });
                    try j.put(a, &result, "authConfig", try fields(a, auth, &.{"apiKey"}));
                };
                if (j.get(maps, "enableWidget")) |value| if (value != .null) try j.put(a, &result, "enableWidget", value);
                try reject(maps, &.{"groundingTypes"});
            }
            try j.put(a, &converted, "googleMaps", result);
        };
        if (j.get(tool, "mcpServers")) |servers| if (servers != .null) {
            var values = servers;
            if (vertex and servers == .array) {
                values = j.array(a);
                for (servers.array.items) |server| {
                    try reject(server, &.{ "name", "streamableHttpTransport" });
                    try values.array.append(j.map(a));
                }
            }
            try j.put(a, &converted, "mcpServers", values);
        };
        if (j.get(tool, "codeExecution")) |value| if (value != .null) try j.put(a, &converted, "codeExecution", value);
        if (j.get(tool, "computerUse")) |computer| if (computer != .null) {
            var value = computer;
            if (vertex) {
                value = try fields(a, computer, &.{ "enablePromptInjectionDetection", "environment", "excludedPredefinedFunctions" });
                try reject(computer, &.{"disabledSafetyPolicies"});
            }
            try j.put(a, &converted, "computerUse", value);
        };
        if (vertex) {
            for ([_][]const u8{ "enterpriseWebSearch", "exaAiSearch" }) |key| if (j.get(tool, key)) |value| if (value != .null) try j.put(a, &converted, key, value);
        } else try reject(tool, &.{ "enterpriseWebSearch", "exaAiSearch" });
        if (j.get(tool, "functionDeclarations")) |value| if (value != .null) try j.put(a, &converted, "functionDeclarations", value);
        if (j.get(tool, "googleSearch")) |search| if (search != .null) {
            var value = search;
            if (!vertex) {
                try reject(search, &.{ "blockingConfidence", "excludeDomains" });
                value = try fields(a, search, &.{ "searchTypes", "timeRangeFilter" });
            }
            try j.put(a, &converted, "googleSearch", value);
        };
        if (j.get(tool, "googleSearchRetrieval")) |value| if (value != .null) try j.put(a, &converted, "googleSearchRetrieval", value);
        if (vertex) {
            if (j.get(tool, "parallelAiSearch")) |value| if (value != .null) try j.put(a, &converted, "parallelAiSearch", value);
        } else try reject(tool, &.{"parallelAiSearch"});
        if (j.get(tool, "urlContext")) |value| if (value != .null) try j.put(a, &converted, "urlContext", value);
        if (vertex) try reject(tool, &.{"fileSearch"}) else if (j.get(tool, "fileSearch")) |value| if (value != .null) try j.put(a, &converted, "fileSearch", value);
        try out.array.append(converted);
    }
    return out;
}
pub fn toolConfig(a: Allocator, vertex: bool, input: Value) !Value {
    if (vertex) try reject(input, &.{"includeServerSideToolInvocations"});
    var out = try fields(a, input, &.{ "functionCallingConfig", "retrievalConfig" });
    if (!vertex) {
        if (j.get(input, "includeServerSideToolInvocations")) |value| if (value != .null) try j.put(a, &out, "includeServerSideToolInvocations", value);
        if (j.get(input, "functionCallingConfig")) |config| if (config != .null) {
            try reject(config, &.{"streamFunctionCallArguments"});
            try j.put(a, &out, "functionCallingConfig", try fields(a, config, &.{ "allowedFunctionNames", "mode" }));
        };
    }
    return out;
}
pub fn speechConfig(a: Allocator, input: Value) !Value {
    return switch (input) {
        .string => j.from(a, .{ .voiceConfig = .{ .prebuiltVoiceConfig = .{ .voiceName = input.string } } }),
        .object, .array, .null => input,
        .bool => error.UnsupportedGoogleBooleanSpeechConfig,
        .integer, .float, .number_string => error.UnsupportedGoogleNumberSpeechConfig,
    };
}
fn vertexVoice(a: Allocator, input: Value) !Value {
    var out = try fields(a, input, &.{"prebuiltVoiceConfig"});
    if (j.get(input, "replicatedVoiceConfig")) |value| if (value != .null) {
        try reject(value, &.{ "consentAudio", "voiceConsentSignature" });
        try j.put(a, &out, "replicatedVoiceConfig", try fields(a, value, &.{ "mimeType", "voiceSampleAudio" }));
    };
    return out;
}
pub fn speechConfigForApi(a: Allocator, vertex: bool, input: Value) !Value {
    const source = try speechConfig(a, input);
    if (!vertex) return source;
    var out = try fields(a, source, &.{"languageCode"});
    if (j.get(source, "voiceConfig")) |voice| if (voice != .null) try j.put(a, &out, "voiceConfig", try vertexVoice(a, voice));
    if (j.get(source, "multiSpeakerVoiceConfig")) |multi| if (multi != .null) {
        var result = j.map(a);
        if (j.get(multi, "speakerVoiceConfigs")) |speakers| if (speakers != .null) {
            var converted = speakers;
            if (speakers == .array) {
                converted = j.array(a);
                for (speakers.array.items) |speaker| {
                    var config = try fields(a, speaker, &.{"speaker"});
                    if (j.get(speaker, "voiceConfig")) |voice| if (voice != .null) try j.put(a, &config, "voiceConfig", try vertexVoice(a, voice));
                    try converted.array.append(config);
                }
            }
            try j.put(a, &result, "speakerVoiceConfigs", converted);
        };
        try j.put(a, &out, "multiSpeakerVoiceConfig", result);
    };
    return out;
}
pub fn imageConfig(a: Allocator, vertex: bool, input: Value) !Value {
    var out = try fields(a, input, &.{ "aspectRatio", "imageSize" });
    if (!vertex) {
        try reject(input, &.{ "personGeneration", "outputMimeType", "outputCompressionQuality", "imageOutputOptions", "prominentPeople" });
        return out;
    }
    for ([_][]const u8{ "personGeneration", "prominentPeople" }) |key| if (j.get(input, key)) |value| if (value != .null) try j.put(a, &out, key, value);
    var options = j.map(a);
    for ([_][]const u8{ "outputMimeType", "outputCompressionQuality" }, [_][]const u8{ "mimeType", "compressionQuality" }) |name, wire| if (j.get(input, name)) |value| if (value != .null) try j.put(a, &options, wire, value);
    if (options.object.count() > 0) try j.put(a, &out, "imageOutputOptions", options);
    if (j.get(input, "imageOutputOptions")) |value| if (value != .null) try j.put(a, &out, "imageOutputOptions", value);
    return out;
}
pub fn event(a: Allocator, vertex: bool, input: Value, response_headers: t.ProviderHeaders) !Value {
    var out = try fields(a, input, if (vertex) &.{ "sdkHttpResponse", "candidates", "createTime", "modelVersion", "promptFeedback", "responseId", "usageMetadata" } else &.{ "sdkHttpResponse", "candidates", "modelVersion", "promptFeedback", "responseId", "usageMetadata", "modelStatus" });
    if (!vertex) if (j.get(input, "candidates")) |candidates| if (candidates == .array) {
        var values = j.array(a);
        for (candidates.array.items) |candidate| {
            var converted = try fields(a, candidate, &.{ "content", "citationMetadata", "tokenCount", "finishReason", "groundingMetadata", "avgLogprobs", "index", "logprobsResult", "safetyRatings", "urlContextMetadata" });
            if (j.get(candidate, "citationMetadata")) |metadata| if (metadata != .null) {
                var citations = j.map(a);
                if (j.get(metadata, "citationSources")) |sources| if (sources != .null) try j.put(a, &citations, "citations", sources);
                try j.put(a, &converted, "citationMetadata", citations);
            };
            try values.array.append(converted);
        }
        try j.put(a, &out, "candidates", values);
    };
    const header_values = try @import("../utils/headers.zig").headersToRecord(a, response_headers);
    var response = j.map(a);
    try j.put(a, &response, "headers", header_values);
    try j.put(a, &out, "sdkHttpResponse", response);
    return out;
}
