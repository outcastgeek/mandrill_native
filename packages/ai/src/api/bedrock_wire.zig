const std = @import("std");
const j = @import("../json/value.zig");
const Value = std.json.Value;
const Mode = enum { write, read };
const Shape = enum {
    document,
    scalar,
    number,
    blob,
    request,
    messages,
    message,
    contents,
    content,
    systems,
    system,
    inference,
    tools_config,
    tools,
    tool,
    tool_spec,
    input_schema,
    tool_choice,
    specific_tool,
    empty,
    tool_use,
    tool_result,
    results,
    result,
    image,
    image_source,
    s3,
    cache,
    reasoning,
    reasoning_text,
    strings,
    string_map,
    prompt_map,
    prompt_value,
    guardrail,
    performance,
    tier,
    output,
    format,
    format_structure,
    json_schema,
    error_block,
    message_start,
    block_start,
    block_start_value,
    block_delta,
    block_delta_value,
    block_stop,
    message_stop,
    metadata,
    usage,
    cache_details,
    cache_detail,
    metrics,
    reasoning_delta,
    tool_start,
    tool_delta,
    tool_result_start,
    image_start,
    image_delta,
    stream_error,
    trace,
    prompt_router,
    document_block,
    document_source,
    text_unions,
    text_union,
    text_structs,
    text_struct,
    video,
    guard_content,
    guard_image,
    bytes_only,
    guard_text,
    citations_config,
    citations_content,
    citations,
    citation,
    source_content_delta,
    citation_location,
    document_location,
    search_location,
    web_location,
    search,
    tool_change,
    tool_reference,
    tool_result_deltas,
    tool_result_delta,
    guard_trace,
    assessment_map,
    assessment_list_map,
    assessments,
    assessment,
    topic_policy,
    topics,
    topic,
    content_policy,
    content_filters,
    content_filter,
    word_policy,
    custom_words,
    custom_word,
    typed_words,
    typed_word,
    sensitive_policy,
    regexes,
    regex_filter,
    grounding_policy,
    grounding_filters,
    grounding_filter,
    invocation,
    coverage,
    coverage_counter,
    guard_usage,
    applied_guard,
    auto_policy,
    findings,
    finding,
    valid_finding,
    invalid_finding,
    satisfiable_finding,
    ambiguous_finding,
    translation,
    translations,
    statements,
    statement,
    input_refs,
    warning,
    scenario,
    scenarios,
    rules,
    rule,
    translation_options,
    translation_option,
};
const Field = struct { name: []const u8, shape: Shape = .scalar };
// Only ConverseStream-reachable shapes are represented. Native callback
// blobs are JSON byte arrays; standard codecs perform the wire conversion.
fn fields(shape: Shape) ?[]const Field {
    return switch (shape) {
        .request => &.{ .{ .name = "messages", .shape = .messages }, .{ .name = "system", .shape = .systems }, .{ .name = "inferenceConfig", .shape = .inference }, .{ .name = "toolConfig", .shape = .tools_config }, .{ .name = "guardrailConfig", .shape = .guardrail }, .{ .name = "additionalModelRequestFields", .shape = .document }, .{ .name = "promptVariables", .shape = .prompt_map }, .{ .name = "additionalModelResponseFieldPaths", .shape = .strings }, .{ .name = "requestMetadata", .shape = .string_map }, .{ .name = "performanceConfig", .shape = .performance }, .{ .name = "serviceTier", .shape = .tier }, .{ .name = "outputConfig", .shape = .output } },
        .message => &.{ .{ .name = "role" }, .{ .name = "content", .shape = .contents } },
        .content => &.{ .{ .name = "text" }, .{ .name = "image", .shape = .image }, .{ .name = "document", .shape = .document_block }, .{ .name = "video", .shape = .video }, .{ .name = "audio", .shape = .image }, .{ .name = "toolUse", .shape = .tool_use }, .{ .name = "toolResult", .shape = .tool_result }, .{ .name = "guardContent", .shape = .guard_content }, .{ .name = "cachePoint", .shape = .cache }, .{ .name = "reasoningContent", .shape = .reasoning }, .{ .name = "citationsContent", .shape = .citations_content }, .{ .name = "searchResult", .shape = .search }, .{ .name = "toolAddition", .shape = .tool_change }, .{ .name = "toolRemoval", .shape = .tool_change } },
        .system => &.{ .{ .name = "text" }, .{ .name = "guardContent", .shape = .guard_content }, .{ .name = "cachePoint", .shape = .cache } },
        .inference => &.{ .{ .name = "maxTokens" }, .{ .name = "temperature" }, .{ .name = "topP" }, .{ .name = "stopSequences", .shape = .strings } },
        .tools_config => &.{ .{ .name = "tools", .shape = .tools }, .{ .name = "toolChoice", .shape = .tool_choice } },
        .tool => &.{ .{ .name = "toolSpec", .shape = .tool_spec }, .{ .name = "systemTool", .shape = .specific_tool }, .{ .name = "cachePoint", .shape = .cache } },
        .tool_spec => &.{ .{ .name = "name" }, .{ .name = "inputSchema", .shape = .input_schema }, .{ .name = "description" }, .{ .name = "strict" } },
        .input_schema => &.{.{ .name = "json", .shape = .document }},
        .tool_choice => &.{ .{ .name = "auto", .shape = .empty }, .{ .name = "any", .shape = .empty }, .{ .name = "tool", .shape = .specific_tool } },
        .specific_tool => &.{.{ .name = "name" }},
        .empty => &.{},
        .tool_use => &.{ .{ .name = "toolUseId" }, .{ .name = "name" }, .{ .name = "input", .shape = .document }, .{ .name = "type" } },
        .tool_result => &.{ .{ .name = "toolUseId" }, .{ .name = "content", .shape = .results }, .{ .name = "status" }, .{ .name = "type" } },
        .result => &.{ .{ .name = "json", .shape = .document }, .{ .name = "text" }, .{ .name = "image", .shape = .image }, .{ .name = "document", .shape = .document_block }, .{ .name = "video", .shape = .video }, .{ .name = "searchResult", .shape = .search } },
        .image => &.{ .{ .name = "format" }, .{ .name = "source", .shape = .image_source }, .{ .name = "error", .shape = .error_block } },
        .image_source => &.{ .{ .name = "bytes", .shape = .blob }, .{ .name = "s3Location", .shape = .s3 } },
        .s3 => &.{ .{ .name = "uri" }, .{ .name = "bucketOwner" } },
        .cache => &.{ .{ .name = "type" }, .{ .name = "ttl" } },
        .reasoning => &.{ .{ .name = "reasoningText", .shape = .reasoning_text }, .{ .name = "redactedContent", .shape = .blob } },
        .reasoning_text => &.{ .{ .name = "text" }, .{ .name = "signature" } },
        .prompt_value => &.{.{ .name = "text" }},
        .guardrail => &.{ .{ .name = "guardrailIdentifier" }, .{ .name = "guardrailVersion" }, .{ .name = "trace" }, .{ .name = "streamProcessingMode" } },
        .performance => &.{.{ .name = "latency" }},
        .tier => &.{.{ .name = "type" }},
        .output => &.{ .{ .name = "textFormat", .shape = .format }, .{ .name = "effort" } },
        .format => &.{ .{ .name = "type" }, .{ .name = "structure", .shape = .format_structure } },
        .format_structure => &.{.{ .name = "jsonSchema", .shape = .json_schema }},
        .json_schema => &.{ .{ .name = "schema" }, .{ .name = "name" }, .{ .name = "description" } },
        .error_block => &.{.{ .name = "message" }},
        .message_start => &.{.{ .name = "role" }},
        .block_start => &.{ .{ .name = "start", .shape = .block_start_value }, .{ .name = "contentBlockIndex", .shape = .number } },
        .block_start_value => &.{ .{ .name = "toolUse", .shape = .tool_start }, .{ .name = "toolResult", .shape = .tool_result_start }, .{ .name = "image", .shape = .image_start } },
        .block_delta => &.{ .{ .name = "delta", .shape = .block_delta_value }, .{ .name = "contentBlockIndex", .shape = .number } },
        .block_delta_value => &.{ .{ .name = "text" }, .{ .name = "toolUse", .shape = .tool_delta }, .{ .name = "toolResult", .shape = .tool_result_deltas }, .{ .name = "reasoningContent", .shape = .reasoning_delta }, .{ .name = "citation", .shape = .source_content_delta }, .{ .name = "image", .shape = .image_delta } },
        .block_stop => &.{.{ .name = "contentBlockIndex", .shape = .number }},
        .message_stop => &.{ .{ .name = "stopReason" }, .{ .name = "additionalModelResponseFields", .shape = .document } },
        .metadata => &.{ .{ .name = "usage", .shape = .usage }, .{ .name = "metrics", .shape = .metrics }, .{ .name = "trace", .shape = .trace }, .{ .name = "performanceConfig", .shape = .performance }, .{ .name = "serviceTier", .shape = .tier } },
        .usage => &.{ .{ .name = "inputTokens", .shape = .number }, .{ .name = "outputTokens", .shape = .number }, .{ .name = "totalTokens", .shape = .number }, .{ .name = "cacheReadInputTokens", .shape = .number }, .{ .name = "cacheWriteInputTokens", .shape = .number }, .{ .name = "cacheDetails", .shape = .cache_details } },
        .cache_detail => &.{ .{ .name = "ttl" }, .{ .name = "inputTokens", .shape = .number } },
        .metrics => &.{.{ .name = "latencyMs", .shape = .number }},
        .reasoning_delta => &.{ .{ .name = "text" }, .{ .name = "redactedContent", .shape = .blob }, .{ .name = "signature" } },
        .tool_start => &.{ .{ .name = "toolUseId" }, .{ .name = "name" }, .{ .name = "type" } },
        .tool_delta => &.{.{ .name = "input" }},
        .tool_result_start => &.{ .{ .name = "toolUseId" }, .{ .name = "type" }, .{ .name = "status" } },
        .image_start => &.{.{ .name = "format" }},
        .image_delta => &.{ .{ .name = "source", .shape = .image_source }, .{ .name = "error", .shape = .error_block } },
        .stream_error => &.{ .{ .name = "message" }, .{ .name = "originalStatusCode", .shape = .number }, .{ .name = "originalMessage" } },
        .trace => &.{ .{ .name = "guardrail", .shape = .guard_trace }, .{ .name = "promptRouter", .shape = .prompt_router } },
        .prompt_router => &.{.{ .name = "invokedModelId" }},
        .document_block => &.{ .{ .name = "name" }, .{ .name = "source", .shape = .document_source }, .{ .name = "format" }, .{ .name = "context" }, .{ .name = "citations", .shape = .citations_config } },
        .document_source => &.{ .{ .name = "bytes", .shape = .blob }, .{ .name = "s3Location", .shape = .s3 }, .{ .name = "text" }, .{ .name = "content", .shape = .text_unions } },
        .text_union, .text_struct => &.{.{ .name = "text" }},
        .video => &.{ .{ .name = "format" }, .{ .name = "source", .shape = .image_source } },
        .guard_content => &.{ .{ .name = "text", .shape = .guard_text }, .{ .name = "image", .shape = .guard_image } },
        .guard_image => &.{ .{ .name = "format" }, .{ .name = "source", .shape = .bytes_only } },
        .bytes_only => &.{.{ .name = "bytes", .shape = .blob }},
        .guard_text => &.{ .{ .name = "text" }, .{ .name = "qualifiers", .shape = .strings } },
        .citations_config => &.{.{ .name = "enabled" }},
        .citations_content => &.{ .{ .name = "content", .shape = .text_unions }, .{ .name = "citations", .shape = .citations } },
        .citation => &.{ .{ .name = "title" }, .{ .name = "source" }, .{ .name = "sourceContent", .shape = .text_unions }, .{ .name = "location", .shape = .citation_location } },
        .source_content_delta => &.{ .{ .name = "title" }, .{ .name = "source" }, .{ .name = "sourceContent", .shape = .text_structs }, .{ .name = "location", .shape = .citation_location } },
        .citation_location => &.{ .{ .name = "web", .shape = .web_location }, .{ .name = "documentChar", .shape = .document_location }, .{ .name = "documentPage", .shape = .document_location }, .{ .name = "documentChunk", .shape = .document_location }, .{ .name = "searchResultLocation", .shape = .search_location } },
        .document_location => &.{ .{ .name = "documentIndex", .shape = .number }, .{ .name = "start", .shape = .number }, .{ .name = "end", .shape = .number } },
        .search_location => &.{ .{ .name = "searchResultIndex", .shape = .number }, .{ .name = "start", .shape = .number }, .{ .name = "end", .shape = .number } },
        .web_location => &.{ .{ .name = "url" }, .{ .name = "domain" } },
        .search => &.{ .{ .name = "source" }, .{ .name = "title" }, .{ .name = "content", .shape = .text_structs }, .{ .name = "citations", .shape = .citations_config } },
        .tool_change => &.{.{ .name = "tool", .shape = .tool_reference }},
        .tool_reference => &.{ .{ .name = "type" }, .{ .name = "name" }, .{ .name = "serverName" } },
        .tool_result_delta => &.{ .{ .name = "text" }, .{ .name = "json", .shape = .document } },
        .guard_trace => &.{ .{ .name = "modelOutput", .shape = .strings }, .{ .name = "inputAssessment", .shape = .assessment_map }, .{ .name = "outputAssessments", .shape = .assessment_list_map }, .{ .name = "actionReason" } },
        .assessment => &.{ .{ .name = "topicPolicy", .shape = .topic_policy }, .{ .name = "contentPolicy", .shape = .content_policy }, .{ .name = "wordPolicy", .shape = .word_policy }, .{ .name = "sensitiveInformationPolicy", .shape = .sensitive_policy }, .{ .name = "contextualGroundingPolicy", .shape = .grounding_policy }, .{ .name = "automatedReasoningPolicy", .shape = .auto_policy }, .{ .name = "invocationMetrics", .shape = .invocation }, .{ .name = "appliedGuardrailDetails", .shape = .applied_guard } },
        .topic_policy => &.{.{ .name = "topics", .shape = .topics }},
        .topic => &.{ .{ .name = "name" }, .{ .name = "type" }, .{ .name = "action" }, .{ .name = "detected" } },
        .content_policy => &.{.{ .name = "filters", .shape = .content_filters }},
        .content_filter => &.{ .{ .name = "type" }, .{ .name = "confidence" }, .{ .name = "action" }, .{ .name = "filterStrength" }, .{ .name = "detected" } },
        .word_policy => &.{ .{ .name = "customWords", .shape = .custom_words }, .{ .name = "managedWordLists", .shape = .typed_words } },
        .custom_word => &.{ .{ .name = "match" }, .{ .name = "action" }, .{ .name = "detected" } },
        .typed_word => &.{ .{ .name = "match" }, .{ .name = "type" }, .{ .name = "action" }, .{ .name = "detected" } },
        .sensitive_policy => &.{ .{ .name = "piiEntities", .shape = .typed_words }, .{ .name = "regexes", .shape = .regexes } },
        .regex_filter => &.{ .{ .name = "action" }, .{ .name = "name" }, .{ .name = "match" }, .{ .name = "regex" }, .{ .name = "detected" } },
        .grounding_policy => &.{.{ .name = "filters", .shape = .grounding_filters }},
        .grounding_filter => &.{ .{ .name = "type" }, .{ .name = "threshold", .shape = .number }, .{ .name = "score", .shape = .number }, .{ .name = "action" }, .{ .name = "detected" } },
        .invocation => &.{ .{ .name = "guardrailProcessingLatency", .shape = .number }, .{ .name = "usage", .shape = .guard_usage }, .{ .name = "guardrailCoverage", .shape = .coverage } },
        .coverage => &.{ .{ .name = "textCharacters", .shape = .coverage_counter }, .{ .name = "images", .shape = .coverage_counter } },
        .coverage_counter => &.{ .{ .name = "guarded", .shape = .number }, .{ .name = "total", .shape = .number } },
        .guard_usage => &.{ .{ .name = "topicPolicyUnits", .shape = .number }, .{ .name = "contentPolicyUnits", .shape = .number }, .{ .name = "wordPolicyUnits", .shape = .number }, .{ .name = "sensitiveInformationPolicyUnits", .shape = .number }, .{ .name = "sensitiveInformationPolicyFreeUnits", .shape = .number }, .{ .name = "contextualGroundingPolicyUnits", .shape = .number }, .{ .name = "contentPolicyImageUnits", .shape = .number }, .{ .name = "automatedReasoningPolicyUnits", .shape = .number }, .{ .name = "automatedReasoningPolicies", .shape = .number } },
        .applied_guard => &.{ .{ .name = "guardrailId" }, .{ .name = "guardrailVersion" }, .{ .name = "guardrailArn" }, .{ .name = "guardrailOrigin", .shape = .strings }, .{ .name = "guardrailOwnership" } },
        .auto_policy => &.{.{ .name = "findings", .shape = .findings }},
        .finding => &.{ .{ .name = "valid", .shape = .valid_finding }, .{ .name = "invalid", .shape = .invalid_finding }, .{ .name = "satisfiable", .shape = .satisfiable_finding }, .{ .name = "impossible", .shape = .invalid_finding }, .{ .name = "translationAmbiguous", .shape = .ambiguous_finding }, .{ .name = "tooComplex", .shape = .empty }, .{ .name = "noTranslations", .shape = .empty } },
        .valid_finding => &.{ .{ .name = "translation", .shape = .translation }, .{ .name = "claimsTrueScenario", .shape = .scenario }, .{ .name = "supportingRules", .shape = .rules }, .{ .name = "logicWarning", .shape = .warning } },
        .invalid_finding => &.{ .{ .name = "translation", .shape = .translation }, .{ .name = "contradictingRules", .shape = .rules }, .{ .name = "logicWarning", .shape = .warning } },
        .satisfiable_finding => &.{ .{ .name = "translation", .shape = .translation }, .{ .name = "claimsTrueScenario", .shape = .scenario }, .{ .name = "claimsFalseScenario", .shape = .scenario }, .{ .name = "logicWarning", .shape = .warning } },
        .ambiguous_finding => &.{ .{ .name = "options", .shape = .translation_options }, .{ .name = "differenceScenarios", .shape = .scenarios } },
        .translation => &.{ .{ .name = "premises", .shape = .statements }, .{ .name = "claims", .shape = .statements }, .{ .name = "untranslatedPremises", .shape = .input_refs }, .{ .name = "untranslatedClaims", .shape = .input_refs }, .{ .name = "confidence", .shape = .number } },
        .statement => &.{ .{ .name = "logic" }, .{ .name = "naturalLanguage" } },
        .warning => &.{ .{ .name = "type" }, .{ .name = "premises", .shape = .statements }, .{ .name = "claims", .shape = .statements } },
        .scenario => &.{.{ .name = "statements", .shape = .statements }},
        .rule => &.{ .{ .name = "identifier" }, .{ .name = "policyVersionArn" } },
        .translation_option => &.{.{ .name = "translations", .shape = .translations }},
        else => null,
    };
}
fn unionShape(shape: Shape) bool {
    return switch (shape) {
        .content, .system, .tool, .input_schema, .tool_choice, .result, .image_source, .reasoning, .prompt_value, .format_structure, .block_start_value, .block_delta_value, .reasoning_delta, .document_source, .text_union, .guard_content, .bytes_only, .citation_location, .tool_result_delta, .finding => true,
        else => false,
    };
}
fn listMember(shape: Shape) ?Shape {
    return switch (shape) {
        .messages => .message,
        .contents => .content,
        .systems => .system,
        .tools => .tool,
        .results => .result,
        .strings => .scalar,
        .cache_details => .cache_detail,
        .text_unions => .text_union,
        .text_structs => .text_struct,
        .citations => .citation,
        .tool_result_deltas => .tool_result_delta,
        .assessments => .assessment,
        .topics => .topic,
        .content_filters => .content_filter,
        .custom_words => .custom_word,
        .typed_words => .typed_word,
        .regexes => .regex_filter,
        .grounding_filters => .grounding_filter,
        .findings => .finding,
        .translations => .translation,
        .statements => .statement,
        .input_refs => .text_struct,
        .scenarios => .scenario,
        .rules => .rule,
        .translation_options => .translation_option,
        else => null,
    };
}
fn mapMember(shape: Shape) ?Shape {
    return switch (shape) {
        .string_map => .scalar,
        .prompt_map => .prompt_value,
        .assessment_map => .assessment,
        .assessment_list_map => .assessments,
        else => null,
    };
}
fn put(a: std.mem.Allocator, out: *Value, key: []const u8, input: Value, shape: Shape, mode: Mode) !void {
    const owned_key = try a.dupe(u8, key);
    errdefer a.free(owned_key);
    var child = try project(a, input, shape, mode);
    errdefer j.deinitCloned(a, &child);
    try j.put(a, out, owned_key, child);
}
fn append(a: std.mem.Allocator, out: *Value, input: Value, shape: Shape, mode: Mode) !void {
    var child = try project(a, input, shape, mode);
    errdefer j.deinitCloned(a, &child);
    try out.array.append(child);
}
fn project(a: std.mem.Allocator, input: Value, shape: Shape, mode: Mode) anyerror!Value {
    if (shape == .blob) {
        if (mode == .read and input == .string) {
            const bytes = try @import("../utils/base64.zig").decodeSmithy(a, input.string);
            defer a.free(bytes);
            return blobValue(a, bytes);
        }
        if (mode == .write and (input == .string or input == .array)) {
            const bytes = if (input == .array) try blobBytes(a, input) else text: {
                const units = try std.unicode.wtf8ToWtf16LeAlloc(a, input.string);
                defer a.free(units);
                break :text try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf16Le(units)});
            };
            defer a.free(bytes);
            const encoded = try a.alloc(u8, std.base64.standard.Encoder.calcSize(bytes.len));
            return j.str(std.base64.standard.Encoder.encode(encoded, bytes));
        }
    }
    if (mode == .read and shape == .number and input == .string) {
        if (std.mem.eql(u8, input.string, "NaN")) return j.num(std.math.nan(f64));
        if (std.mem.eql(u8, input.string, "Infinity")) return j.num(std.math.inf(f64));
        if (std.mem.eql(u8, input.string, "-Infinity")) return j.num(-std.math.inf(f64));
    }
    if (mode == .write and input == .float and !std.math.isFinite(input.float)) return j.str(try a.dupe(u8, if (std.math.isNan(input.float)) "NaN" else if (input.float > 0) "Infinity" else "-Infinity"));
    if (shape == .document) {
        if (input == .object) {
            var out = j.map(a);
            errdefer j.deinitCloned(a, &out);
            const entries = try j.objectEntries(a, input.object);
            defer a.free(entries);
            for (entries) |entry| try put(a, &out, entry.key, entry.node.*, .document, mode);
            return out;
        }
        if (input == .array) {
            var out = j.array(a);
            errdefer j.deinitCloned(a, &out);
            for (input.array.items) |item| try append(a, &out, item, .document, mode);
            return out;
        }
    }
    if (listMember(shape)) |member| if (input == .array) {
        var out = j.array(a);
        errdefer j.deinitCloned(a, &out);
        for (input.array.items) |item| if (mode == .read or item != .null) {
            try append(a, &out, item, member, mode);
        };
        return out;
    };
    if (mapMember(shape)) |member| if (input == .object or input == .array) {
        var members = if (input == .array) try j.objectSpread(a, input) else input;
        defer if (input == .array) j.deinitCloned(a, &members);
        var out = j.map(a);
        errdefer j.deinitCloned(a, &out);
        const entries = try j.objectEntries(a, members.object);
        defer a.free(entries);
        for (entries) |entry| if (mode == .read or entry.node.* != .null) {
            try put(a, &out, entry.key, entry.node.*, member, mode);
        };
        return out;
    };
    if (fields(shape)) |members| if (input == .object or input == .array) {
        var out = j.map(a);
        errdefer j.deinitCloned(a, &out);
        for (members) |member| if (j.getNonNull(input, member.name)) |value| try put(a, &out, member.name, value, member.shape, mode);
        if (out.object.count() == 0 and unionShape(shape)) {
            if (mode == .write) {
                if (j.get(input, "$unknown")) |unknown| if (unknown == .array and unknown.array.items.len >= 2) {
                    if (j.text(unknown.array.items[0])) |key| try put(a, &out, key, unknown.array.items[1], .document, mode);
                };
            } else if (input == .object) {
                var unknown_key: ?[]const u8 = null;
                var unknown_count: usize = 0;
                var entries = input.object.iterator();
                while (entries.next()) |entry| {
                    if (std.mem.eql(u8, entry.key_ptr.*, "__type")) continue;
                    const known = for (members) |member| {
                        if (std.mem.eql(u8, entry.key_ptr.*, member.name)) break true;
                    } else false;
                    if (!known) {
                        unknown_key = entry.key_ptr.*;
                        unknown_count += 1;
                    }
                }
                if (unknown_count == 1) {
                    var unknown = j.array(a);
                    defer j.deinitCloned(a, &unknown);
                    try append(a, &unknown, j.str(unknown_key.?), .scalar, mode);
                    try append(a, &unknown, input.object.get(unknown_key.?).?, .document, mode);
                    try put(a, &out, "$unknown", unknown, .document, mode);
                }
            }
        } else if (shape != .request and (mode == .write or !unionShape(shape)) and j.field(input, "__type") != null) if (input == .object) {
            const entries = try j.objectEntries(a, input.object);
            defer a.free(entries);
            for (entries) |entry| if (!out.object.contains(entry.key)) {
                try put(a, &out, entry.key, entry.node.*, .document, mode);
            };
        };
        return out;
    };
    return j.clone(a, input);
}
pub fn request(a: std.mem.Allocator, payload: Value) !Value {
    return project(a, payload, .request, .write);
}
/// Native JSON view of an SDK Uint8Array, owned by a.
pub fn blobValue(a: std.mem.Allocator, bytes: []const u8) !Value {
    var value = j.array(a);
    errdefer j.deinitCloned(a, &value);
    for (bytes) |byte| try value.array.append(j.int(byte));
    return value;
}
pub fn blobBytes(a: std.mem.Allocator, value: Value) ![]u8 {
    const parsed = try std.json.parseFromValue([]u8, a, value, .{ .allocate = .alloc_always });
    defer parsed.deinit();
    return a.dupe(u8, parsed.value);
}
pub fn event(a: std.mem.Allocator, member: []const u8, payload: Value) !Value {
    const shape: Shape = if (std.mem.eql(u8, member, "messageStart")) .message_start else if (std.mem.eql(u8, member, "contentBlockStart")) .block_start else if (std.mem.eql(u8, member, "contentBlockDelta")) .block_delta else if (std.mem.eql(u8, member, "contentBlockStop")) .block_stop else if (std.mem.eql(u8, member, "messageStop")) .message_stop else if (std.mem.eql(u8, member, "metadata")) .metadata else if (std.mem.eql(u8, member, "modelStreamErrorException")) .stream_error else .error_block;
    return project(a, payload, shape, .read);
}
