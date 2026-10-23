//! Explicit convenience entrypoint. Individual provider modules remain selective.
const std = @import("std");
const t = @import("../types.zig");
const m = @import("../models.zig");
pub const model_schema = @import("model_schema.zig");
pub const compat_schema = @import("compat_schema.zig");
pub const radius_config = @import("radius_config.zig");
pub const openrouter_api = @import("openrouter_api.zig");
pub const cloudflare_auth = @import("cloudflare_auth.zig");
pub const cloudflare_stream = @import("cloudflare_stream.zig");
pub const opencode_headers = @import("opencode_headers.zig");
pub const faux = @import("faux.zig");
pub const images = struct {
    pub const register_builtins = @import("images/register_builtins.zig");
};
pub const amazonBedrockProvider = @import("amazon_bedrock.zig").amazonBedrockProvider;
pub const antLingProvider = @import("ant_ling.zig").antLingProvider;
pub const anthropicProvider = @import("anthropic.zig").anthropicProvider;
pub const azureProvider = @import("azure.zig").azureProvider;
pub const basetenProvider = @import("baseten.zig").basetenProvider;
pub const cerebrasProvider = @import("cerebras.zig").cerebrasProvider;
pub const cloudflareAIGatewayProvider = @import("cloudflare_ai_gateway.zig").cloudflareAIGatewayProvider;
pub const cloudflareWorkersAIProvider = @import("cloudflare_workers_ai.zig").cloudflareWorkersAIProvider;
pub const deepseekProvider = @import("deepseek.zig").deepseekProvider;
pub const fireworksProvider = @import("fireworks.zig").fireworksProvider;
pub const githubCopilotProvider = @import("github_copilot.zig").githubCopilotProvider;
pub const googleProvider = @import("google.zig").googleProvider;
pub const googleVertexProvider = @import("google_vertex.zig").googleVertexProvider;
pub const groqProvider = @import("groq.zig").groqProvider;
pub const huggingfaceProvider = @import("huggingface.zig").huggingfaceProvider;
pub const kimiCodingProvider = @import("kimi_coding.zig").kimiCodingProvider;
pub const metaProvider = @import("meta.zig").metaProvider;
pub const minimaxProvider = @import("minimax.zig").minimaxProvider;
pub const minimaxCnProvider = @import("minimax_cn.zig").minimaxCnProvider;
pub const mistralProvider = @import("mistral.zig").mistralProvider;
pub const moonshotaiProvider = @import("moonshotai.zig").moonshotaiProvider;
pub const moonshotaiCnProvider = @import("moonshotai_cn.zig").moonshotaiCnProvider;
pub const nvidiaProvider = @import("nvidia.zig").nvidiaProvider;
pub const openaiProvider = @import("openai.zig").openaiProvider;
pub const openaiCodexProvider = @import("openai_codex.zig").openaiCodexProvider;
pub const opencodeProvider = @import("opencode.zig").opencodeProvider;
pub const opencodeGoProvider = @import("opencode_go.zig").opencodeGoProvider;
pub const openrouterProvider = @import("openrouter.zig").openrouterProvider;
pub const personalizeOpenRouterCatalog = @import("openrouter.zig").personalizeOpenRouterCatalog;
pub const qwenTokenPlanProvider = @import("qwen_token_plan.zig").qwenTokenPlanProvider;
pub const qwenTokenPlanCnProvider = @import("qwen_token_plan_cn.zig").qwenTokenPlanCnProvider;
pub const qwenTokenPlanIndividualProvider = @import("qwen_token_plan_individual.zig").qwenTokenPlanIndividualProvider;
pub const radiusProvider = @import("radius.zig").radiusProvider;
pub const togetherProvider = @import("together.zig").togetherProvider;
pub const typesafeProvider = @import("typesafe.zig").typesafeProvider;
pub const vercelAIGatewayProvider = @import("vercel_ai_gateway.zig").vercelAIGatewayProvider;
pub const xaiProvider = @import("xai.zig").xaiProvider;
pub const xiaomiProvider = @import("xiaomi.zig").xiaomiProvider;
pub const xiaomiTokenPlanAmsProvider = @import("xiaomi_token_plan_ams.zig").xiaomiTokenPlanAmsProvider;
pub const xiaomiTokenPlanCnProvider = @import("xiaomi_token_plan_cn.zig").xiaomiTokenPlanCnProvider;
pub const xiaomiTokenPlanSgpProvider = @import("xiaomi_token_plan_sgp.zig").xiaomiTokenPlanSgpProvider;
pub const zaiProvider = @import("zai.zig").zaiProvider;
pub const zaiCodingCnProvider = @import("zai_coding_cn.zig").zaiCodingCnProvider;
pub const BuiltinProvider = enum { @"amazon-bedrock", @"ant-ling", anthropic, azure, baseten, cerebras, @"cloudflare-ai-gateway", @"cloudflare-workers-ai", deepseek, fireworks, @"github-copilot", google, @"google-vertex", groq, huggingface, @"kimi-coding", meta, minimax, @"minimax-cn", mistral, moonshotai, @"moonshotai-cn", nvidia, openai, @"openai-codex", opencode, @"opencode-go", openrouter, @"qwen-token-plan", @"qwen-token-plan-cn", @"qwen-token-plan-individual", radius, together, typesafe, @"vercel-ai-gateway", xai, xiaomi, @"xiaomi-token-plan-ams", @"xiaomi-token-plan-cn", @"xiaomi-token-plan-sgp", zai, @"zai-coding-cn" };
pub const providerIds: []const []const u8 = &.{ "amazon-bedrock", "ant-ling", "anthropic", "azure", "baseten", "cerebras", "cloudflare-ai-gateway", "cloudflare-workers-ai", "deepseek", "fireworks", "github-copilot", "google", "google-vertex", "groq", "huggingface", "kimi-coding", "meta", "minimax", "minimax-cn", "mistral", "moonshotai", "moonshotai-cn", "nvidia", "openai", "openai-codex", "opencode", "opencode-go", "openrouter", "qwen-token-plan", "qwen-token-plan-cn", "qwen-token-plan-individual", "radius", "together", "typesafe", "vercel-ai-gateway", "xai", "xiaomi", "xiaomi-token-plan-ams", "xiaomi-token-plan-cn", "xiaomi-token-plan-sgp", "zai", "zai-coding-cn" };
pub fn getBuiltinProviders() []const []const u8 {
    return providerIds;
}
pub fn catalogBytes(provider: []const u8) ?[]const u8 {
    if (std.mem.eql(u8, provider, "amazon-bedrock")) return @embedFile("../../data/amazon-bedrock.json");
    if (std.mem.eql(u8, provider, "ant-ling")) return @embedFile("../../data/ant-ling.json");
    if (std.mem.eql(u8, provider, "anthropic")) return @embedFile("../../data/anthropic.json");
    if (std.mem.eql(u8, provider, "azure")) return @embedFile("../../data/azure.json");
    if (std.mem.eql(u8, provider, "baseten")) return @embedFile("../../data/baseten.json");
    if (std.mem.eql(u8, provider, "cerebras")) return @embedFile("../../data/cerebras.json");
    if (std.mem.eql(u8, provider, "cloudflare-ai-gateway")) return @embedFile("../../data/cloudflare-ai-gateway.json");
    if (std.mem.eql(u8, provider, "cloudflare-workers-ai")) return @embedFile("../../data/cloudflare-workers-ai.json");
    if (std.mem.eql(u8, provider, "deepseek")) return @embedFile("../../data/deepseek.json");
    if (std.mem.eql(u8, provider, "fireworks")) return @embedFile("../../data/fireworks.json");
    if (std.mem.eql(u8, provider, "github-copilot")) return @embedFile("../../data/github-copilot.json");
    if (std.mem.eql(u8, provider, "google")) return @embedFile("../../data/google.json");
    if (std.mem.eql(u8, provider, "google-vertex")) return @embedFile("../../data/google-vertex.json");
    if (std.mem.eql(u8, provider, "groq")) return @embedFile("../../data/groq.json");
    if (std.mem.eql(u8, provider, "huggingface")) return @embedFile("../../data/huggingface.json");
    if (std.mem.eql(u8, provider, "kimi-coding")) return @embedFile("../../data/kimi-coding.json");
    if (std.mem.eql(u8, provider, "meta")) return @embedFile("../../data/meta.json");
    if (std.mem.eql(u8, provider, "minimax")) return @embedFile("../../data/minimax.json");
    if (std.mem.eql(u8, provider, "minimax-cn")) return @embedFile("../../data/minimax-cn.json");
    if (std.mem.eql(u8, provider, "mistral")) return @embedFile("../../data/mistral.json");
    if (std.mem.eql(u8, provider, "moonshotai")) return @embedFile("../../data/moonshotai.json");
    if (std.mem.eql(u8, provider, "moonshotai-cn")) return @embedFile("../../data/moonshotai-cn.json");
    if (std.mem.eql(u8, provider, "nvidia")) return @embedFile("../../data/nvidia.json");
    if (std.mem.eql(u8, provider, "openai")) return @embedFile("../../data/openai.json");
    if (std.mem.eql(u8, provider, "openai-codex")) return @embedFile("../../data/openai-codex.json");
    if (std.mem.eql(u8, provider, "opencode")) return @embedFile("../../data/opencode.json");
    if (std.mem.eql(u8, provider, "opencode-go")) return @embedFile("../../data/opencode-go.json");
    if (std.mem.eql(u8, provider, "openrouter")) return @embedFile("../../data/openrouter.json");
    if (std.mem.eql(u8, provider, "qwen-token-plan")) return @embedFile("../../data/qwen-token-plan.json");
    if (std.mem.eql(u8, provider, "qwen-token-plan-cn")) return @embedFile("../../data/qwen-token-plan-cn.json");
    if (std.mem.eql(u8, provider, "qwen-token-plan-individual")) return @embedFile("../../data/qwen-token-plan-individual.json");
    if (std.mem.eql(u8, provider, "radius")) return @embedFile("../../data/radius.json");
    if (std.mem.eql(u8, provider, "together")) return @embedFile("../../data/together.json");
    if (std.mem.eql(u8, provider, "typesafe")) return @embedFile("../../data/typesafe.json");
    if (std.mem.eql(u8, provider, "vercel-ai-gateway")) return @embedFile("../../data/vercel-ai-gateway.json");
    if (std.mem.eql(u8, provider, "xai")) return @embedFile("../../data/xai.json");
    if (std.mem.eql(u8, provider, "xiaomi")) return @embedFile("../../data/xiaomi.json");
    if (std.mem.eql(u8, provider, "xiaomi-token-plan-ams")) return @embedFile("../../data/xiaomi-token-plan-ams.json");
    if (std.mem.eql(u8, provider, "xiaomi-token-plan-cn")) return @embedFile("../../data/xiaomi-token-plan-cn.json");
    if (std.mem.eql(u8, provider, "xiaomi-token-plan-sgp")) return @embedFile("../../data/xiaomi-token-plan-sgp.json");
    if (std.mem.eql(u8, provider, "zai")) return @embedFile("../../data/zai.json");
    if (std.mem.eql(u8, provider, "zai-coding-cn")) return @embedFile("../../data/zai-coding-cn.json");
    return null;
}
pub fn getAllBuiltinModels(a: std.mem.Allocator, provider: []const u8) ![]const t.Model {
    const bytes = catalogBytes(provider) orelse return &.{};
    var loaded = try @import("../model_catalog.zig").Catalog.decode(a, bytes);
    defer loaded.deinit();
    return t.cloneInto([]const t.Model, a, loaded.models);
}
pub fn getBuiltinModelsOfType(a: std.mem.Allocator, kind: t.ModelType, provider: []const u8) ![]const t.Model {
    const all = try getAllBuiltinModels(a, provider);
    var out: std.ArrayList(t.Model) = .empty;
    for (all) |model| if (model.modelType() == kind) {
        try out.append(a, model);
    };
    return out.toOwnedSlice(a);
}
pub fn getBuiltinModels(a: std.mem.Allocator, provider: []const u8) ![]const t.Model {
    return getBuiltinModelsOfType(a, .chat, provider);
}
pub fn getBuiltinImageModels(a: std.mem.Allocator, provider: []const u8) ![]const t.Model {
    return getBuiltinModelsOfType(a, .image, provider);
}
pub fn getBuiltinClassifierModels(a: std.mem.Allocator, provider: []const u8) ![]const t.Model {
    return getBuiltinModelsOfType(a, .classifier, provider);
}
pub fn getBuiltinModelOfType(a: std.mem.Allocator, kind: t.ModelType, provider: []const u8, id: []const u8) !?t.Model {
    const bytes = catalogBytes(provider) orelse return null;
    var loaded = try @import("../model_catalog.zig").Catalog.decode(a, bytes);
    defer loaded.deinit();
    return if (loaded.get(kind, id)) |model| try t.cloneInto(t.Model, a, model) else null;
}
pub fn getBuiltinModel(a: std.mem.Allocator, provider: []const u8, id: []const u8) !?t.Model {
    return getBuiltinModelOfType(a, .chat, provider, id);
}
pub fn getBuiltinImageModel(a: std.mem.Allocator, provider: []const u8, id: []const u8) !?t.Model {
    return getBuiltinModelOfType(a, .image, provider, id);
}
pub fn getBuiltinClassifierModel(a: std.mem.Allocator, provider: []const u8, id: []const u8) !?t.Model {
    return getBuiltinModelOfType(a, .classifier, provider, id);
}
pub fn getBuiltinModelDataGeneratedAt(a: std.mem.Allocator) !?i64 {
    const value = try @import("../json/value.zig").parseFromSlice(std.json.Value, a, @embedFile("../../data/.manifest.json"), .{ .duplicate_field_behavior = .use_last });
    defer value.deinit();
    const timestamp = @import("../json/value.zig").field(value.value, "generatedAt") orelse return null;
    return @import("../utils/datetime.zig").parseRfc3339(a, timestamp) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        return null;
    };
}
pub fn builtinProviders(a: std.mem.Allocator) ![]const m.Provider {
    const providers = try a.alloc(m.Provider, providerIds.len);
    providers[0] = try amazonBedrockProvider(a);
    providers[1] = try antLingProvider(a);
    providers[2] = try anthropicProvider(a);
    providers[3] = try azureProvider(a);
    providers[4] = try basetenProvider(a);
    providers[5] = try cerebrasProvider(a);
    providers[6] = try cloudflareAIGatewayProvider(a);
    providers[7] = try cloudflareWorkersAIProvider(a);
    providers[8] = try deepseekProvider(a);
    providers[9] = try fireworksProvider(a);
    providers[10] = try githubCopilotProvider(a);
    providers[11] = try googleProvider(a);
    providers[12] = try googleVertexProvider(a);
    providers[13] = try groqProvider(a);
    providers[14] = try huggingfaceProvider(a);
    providers[15] = try kimiCodingProvider(a);
    providers[16] = try metaProvider(a);
    providers[17] = try minimaxProvider(a);
    providers[18] = try minimaxCnProvider(a);
    providers[19] = try mistralProvider(a);
    providers[20] = try moonshotaiProvider(a);
    providers[21] = try moonshotaiCnProvider(a);
    providers[22] = try nvidiaProvider(a);
    providers[23] = try openaiProvider(a);
    providers[24] = try openaiCodexProvider(a);
    providers[25] = try opencodeProvider(a);
    providers[26] = try opencodeGoProvider(a);
    providers[27] = try openrouterProvider(a);
    providers[28] = try qwenTokenPlanProvider(a);
    providers[29] = try qwenTokenPlanCnProvider(a);
    providers[30] = try qwenTokenPlanIndividualProvider(a);
    providers[31] = try radiusProvider(a, .{});
    providers[32] = try togetherProvider(a);
    providers[33] = try typesafeProvider(a);
    providers[34] = try vercelAIGatewayProvider(a);
    providers[35] = try xaiProvider(a);
    providers[36] = try xiaomiProvider(a);
    providers[37] = try xiaomiTokenPlanAmsProvider(a);
    providers[38] = try xiaomiTokenPlanCnProvider(a);
    providers[39] = try xiaomiTokenPlanSgpProvider(a);
    providers[40] = try zaiProvider(a);
    providers[41] = try zaiCodingCnProvider(a);
    return providers;
}
pub fn builtinModels(a: std.mem.Allocator, io: std.Io, options: m.Models.Options) !m.Models {
    var result = try m.Models.init(a, io, options);
    errdefer result.deinit();
    // Wrapper callback contexts allocated by factories must outlive every
    // registered request, including streams retained beyond a lookup.
    for (try builtinProviders(result.arena.allocator())) |provider| try result.register(provider);
    return result;
}
