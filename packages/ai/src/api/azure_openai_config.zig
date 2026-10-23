//! Azure config helpers share the adapter's request-scoped implementation.
const std = @import("std");
const t = @import("../types.zig");
const auth = @import("../auth/types.zig");
const adapter = @import("azure_openai_responses.zig");
const j = @import("../json/value.zig");
const common = @import("common.zig");
pub const AzureEndpointOptions = @import("options.zig").AzureOpenAIResponsesOptions;
pub const AzureConfig = struct { baseUrl: []const u8, apiVersion: []const u8 };
fn option(options: t.StreamOptions, name: []const u8) ?[]const u8 {
    return if (common.extra(options, name)) |value| j.text(value) else null;
}
fn nonempty(value: ?[]const u8) ?[]const u8 {
    const text = value orelse return null;
    return if (text.len > 0) text else null;
}
fn prepare(a: std.mem.Allocator, options: AzureEndpointOptions) !t.StreamOptions {
    var result = try options.streamOptions(a);
    result.env = .{};
    for (options.request.env.keys(), options.request.env.values()) |key, value| try result.env.put(a, key, value);
    return result;
}
fn ambient(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, options: *t.StreamOptions, name: []const u8) !void {
    if (try @import("../utils/provider_env.zig").getProviderEnvValue(a, io, context, name, options.env)) |value| try options.env.put(a, name, value);
}
pub fn resolveDeploymentName(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, model: t.Model, options: AzureEndpointOptions) ![]const u8 {
    var prepared = try prepare(a, options);
    if (nonempty(option(prepared, "azureDeploymentName")) == null) try ambient(a, io, context, &prepared, "AZURE_OPENAI_DEPLOYMENT_NAME_MAP");
    return adapter.resolveDeploymentName(a, model, prepared);
}
pub fn resolveAzureBaseUrl(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, model: t.Model, options: AzureEndpointOptions) ![]const u8 {
    var prepared = try prepare(a, options);
    const base = try @import("../utils/unicode.zig").trimWhitespace(a, option(prepared, "azureBaseUrl") orelse "");
    defer a.free(base);
    if (base.len == 0) try ambient(a, io, context, &prepared, "AZURE_OPENAI_BASE_URL");
    if (nonempty(option(prepared, "azureResourceName")) == null) try ambient(a, io, context, &prepared, "AZURE_OPENAI_RESOURCE_NAME");
    return adapter.resolveBaseUrl(a, model, prepared);
}
pub fn resolveAzureConfig(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, model: t.Model, options: AzureEndpointOptions) !AzureConfig {
    const base = try resolveAzureBaseUrl(a, io, context, model, options);
    var prepared = try prepare(a, options);
    const explicit = nonempty(option(prepared, "azureApiVersion"));
    if (explicit == null) try ambient(a, io, context, &prepared, "AZURE_OPENAI_API_VERSION");
    return .{ .baseUrl = base, .apiVersion = explicit orelse nonempty(prepared.env.get("AZURE_OPENAI_API_VERSION")) orelse "v1" };
}
