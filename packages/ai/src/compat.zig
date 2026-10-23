//! Legacy dispatch with an explicit native owner in place of JavaScript globals.
const std = @import("std");
const t = @import("types.zig");
const m = @import("models.zig");
const events = @import("utils/event_stream.zig");
pub const core = @import("root.zig");
// The compatibility entrypoint re-exports the core API directly.
pub const types = core.types;
pub const Models = core.Models;
pub const Provider = core.Provider;
pub const createModels = core.createModels;
pub const createProvider = core.createProvider;
pub const auth = core.auth;
pub const credential_store = core.credential_store;
pub const auth_resolution = core.auth_resolution;
pub const AuthResolutionOwner = core.AuthResolutionOwner;
pub const auth_helpers = core.auth_helpers;
pub const model_catalog = core.model_catalog;
pub const models_store = core.models_store;
pub const event_stream = core.event_stream;
pub const transport = core.transport;
pub const transcript = core.transcript;
pub const text = core.text;
pub const json = core.json;
pub const thinking = core.thinking;
pub const cost = core.cost;
pub const uuid = core.uuid;
pub const UuidGenerator = core.UuidGenerator;
pub const uuidv7 = core.uuidv7;
pub const unicode = core.unicode;
pub const session_resources = core.session_resources;
pub const models = core.models;
pub const schema = core.schema;
pub const Type = core.Type;
pub const TSchema = core.TSchema;
pub const Static = core.Static;
pub const TypedSchema = core.TypedSchema;
pub const StringEnum = core.StringEnum;
pub const validation = core.validation;
pub const validateToolCall = core.validateToolCall;
pub const validateToolArguments = core.validateToolArguments;
pub const validateToolCallWithError = core.validateToolCallWithError;
pub const validateToolArgumentsWithError = core.validateToolArgumentsWithError;
pub const checkToolCall = core.checkToolCall;
pub const checkToolArguments = core.checkToolArguments;
pub const diagnostics = core.diagnostics;
pub const error_body = core.error_body;
pub const overflow = core.overflow;
pub const retry = core.retry;
pub const models_error = core.models_error;
pub const ModelsError = core.ModelsError;
pub const ModelsErrorCode = core.ModelsErrorCode;
pub const fauxProvider = core.fauxProvider;
pub const createFauxCore = core.createFauxCore;
pub const FauxCore = core.FauxCore;
pub const fauxText = core.fauxText;
pub const fauxThinking = core.fauxThinking;
pub const fauxToolCall = core.fauxToolCall;
pub const fauxAssistantMessage = core.fauxAssistantMessage;
pub const api = core.api;
pub const lazyStream = core.lazyStream;
pub const lazyApi = core.lazyApi;
pub const LazyApiCapabilities = core.LazyApiCapabilities;
pub const utils = core.utils;
pub const catalog_validation = core.catalog_validation;
pub const datetime = core.datetime;
pub const signing = core.signing;
pub const google_certificates = core.google_certificates;
pub const constrained_sampling = core.constrained_sampling;
pub const legacy_oauth = core.legacy_oauth;
pub const auth_context = core.auth_context;
pub const EnvironmentContext = core.EnvironmentContext;
pub const defaultProviderAuthContext = core.defaultProviderAuthContext;
pub const json_value = core.json_value;
pub const uri = core.uri;
pub const sse = core.sse;
pub const websocket = core.websocket;
pub const aws_event_stream = core.aws_event_stream;
pub const websocket_sessions = core.websocket_sessions;
pub const proxy = core.proxy;
pub const label_token_cache = core.label_token_cache;
pub const JsonValue = core.JsonValue;
pub const JsonObject = core.JsonObject;
pub const Api = core.Api;
pub const ProviderId = core.ProviderId;
pub const ThinkingLevel = core.ThinkingLevel;
pub const ModelThinkingLevel = core.ModelThinkingLevel;
pub const StopReason = core.StopReason;
pub const ModelType = core.ModelType;
pub const CacheRetention = core.CacheRetention;
pub const Transport = core.Transport;
pub const ToolChoice = core.ToolChoice;
pub const ProviderHeader = core.ProviderHeader;
pub const ProviderHeaders = core.ProviderHeaders;
pub const ProviderEnv = core.ProviderEnv;
pub const ErrorMessageStorage = core.ErrorMessageStorage;
pub const ModelCostRates = core.ModelCostRates;
pub const ModelCostTier = core.ModelCostTier;
pub const ModelCost = core.ModelCost;
pub const Usage = core.Usage;
pub const TextContent = core.TextContent;
pub const ThinkingContent = core.ThinkingContent;
pub const ImageContent = core.ImageContent;
pub const ToolCall = core.ToolCall;
pub const Content = core.Content;
pub const MessageContent = core.MessageContent;
pub const ConstrainedSamplingConfig = core.ConstrainedSamplingConfig;
pub const Tool = core.Tool;
pub const ToolReference = core.ToolReference;
pub const SystemMessage = core.SystemMessage;
pub const UserMessage = core.UserMessage;
pub const DeferredHandle = core.DeferredHandle;
pub const Diagnostic = core.Diagnostic;
pub const AssistantMessage = core.AssistantMessage;
pub const NestedToolCallRecord = core.NestedToolCallRecord;
pub const NestedToolCalls = core.NestedToolCalls;
pub const ToolResultMessage = core.ToolResultMessage;
pub const Message = core.Message;
pub const Context = core.Context;
pub const TranscriptContext = core.TranscriptContext;
pub const AssistantMessageEvent = core.AssistantMessageEvent;
pub const Model = core.Model;
pub const AnyModel = core.AnyModel;
pub const ImageModel = core.ImageModel;
pub const ClassifierModel = core.ClassifierModel;
pub const ImagesContext = core.ImagesContext;
pub const AssistantImages = core.AssistantImages;
pub const ClassifierQuestion = core.ClassifierQuestion;
pub const ClassifierContext = core.ClassifierContext;
pub const ClassifierAnswer = core.ClassifierAnswer;
pub const ClassifierResult = core.ClassifierResult;
pub const ThinkingBudgets = core.ThinkingBudgets;
pub const ProviderResponse = core.ProviderResponse;
pub const Callbacks = core.Callbacks;
pub const StreamOptions = core.StreamOptions;
pub const SimpleStreamOptions = core.SimpleStreamOptions;
pub const ProviderStreamOptions = core.ProviderStreamOptions;
pub const ImagesOptions = core.ImagesOptions;
pub const ClassifierOptions = core.ClassifierOptions;
pub const DeferredFetchOptions = core.DeferredFetchOptions;
pub const DeferredCancelOptions = core.DeferredCancelOptions;
pub const Owned = core.Owned;
pub const cloneInto = core.cloneInto;
pub const ModelsPublication = core.ModelsPublication;
pub const RefreshModelsContext = core.RefreshModelsContext;
pub const ModelsRefreshOptions = core.ModelsRefreshOptions;
pub const ModelsRefreshResult = core.ModelsRefreshResult;
pub const ModelsRequestTransforms = core.ModelsRequestTransforms;
pub const ApiImplementation = core.ApiImplementation;
pub const ProviderStreams = core.ProviderStreams;
pub const OperationContext = core.OperationContext;
pub const calculateCost = core.calculateCost;
pub const getSupportedThinkingLevels = core.getSupportedThinkingLevels;
pub const clampThinkingLevel = core.clampThinkingLevel;
pub const hasApi = core.hasApi;
pub const getModelType = core.getModelType;
pub const isModelType = core.isModelType;
pub const modelsAreEqual = core.modelsAreEqual;
pub const AuthContext = core.AuthContext;
pub const ModelAuth = core.ModelAuth;
pub const AuthResult = core.AuthResult;
pub const AuthCheck = core.AuthCheck;
pub const ApiKeyCredential = core.ApiKeyCredential;
pub const OAuthCredential = core.OAuthCredential;
pub const OAuthCredentials = core.OAuthCredentials;
pub const OAuthAuthInfo = core.OAuthAuthInfo;
pub const OAuthDeviceCodeInfo = core.OAuthDeviceCodeInfo;
pub const OAuthLoginCallbacks = core.OAuthLoginCallbacks;
pub const OAuthPrompt = core.OAuthPrompt;
pub const OAuthSelectOption = core.OAuthSelectOption;
pub const OAuthSelectPrompt = core.OAuthSelectPrompt;
pub const Credential = core.Credential;
pub const CredentialInfo = core.CredentialInfo;
pub const CredentialStore = core.CredentialStore;
pub const ApiKeyAuth = core.ApiKeyAuth;
pub const OAuthAuth = core.OAuthAuth;
pub const ProviderAuth = core.ProviderAuth;
pub const AuthPrompt = core.AuthPrompt;
pub const AuthEvent = core.AuthEvent;
pub const Interaction = core.Interaction;
pub const LoginOptions = core.LoginOptions;
pub const envApiKeyAuth = core.envApiKeyAuth;
pub const LazyOAuthInput = core.LazyOAuthInput;
pub const LazyOAuth = core.LazyOAuth;
pub const lazyOAuth = core.lazyOAuth;
pub const InMemoryCredentialStore = core.InMemoryCredentialStore;
pub const ModelsStore = core.ModelsStore;
pub const ModelsStoreEntry = core.ModelsStoreEntry;
pub const InMemoryModelsStore = core.InMemoryModelsStore;
pub const AssistantMessageEventStream = core.AssistantMessageEventStream;
pub const EventStream = core.EventStream;
pub const createAssistantMessageEventStream = core.createAssistantMessageEventStream;
pub const parseStreamingJson = core.parseStreamingJson;
pub const parseToolCallArguments = core.parseToolCallArguments;
pub const repairJson = core.repairJson;
pub const parseJsonWithRepair = core.parseJsonWithRepair;
pub const contentText = core.contentText;
pub const getSystemMessageText = core.getSystemMessageText;
pub const renderSystemMessageUpdate = core.renderSystemMessageUpdate;
pub const createInitialSystemMessage = core.createInitialSystemMessage;
pub const normalizeContext = core.normalizeContext;
pub const getInitialSystemMessage = core.getInitialSystemMessage;
pub const withoutInitialSystemMessage = core.withoutInitialSystemMessage;
pub const getCurrentTools = core.getCurrentTools;
pub const getCurrentSystemMessage = core.getCurrentSystemMessage;
pub const getCurrentSystemPrompt = core.getCurrentSystemPrompt;
pub const collapseSystemMessages = core.collapseSystemMessages;
pub const resolveTranscript = core.resolveTranscript;
pub const toToolDeclaration = core.toToolDeclaration;
pub const declarationsEqual = core.declarationsEqual;
pub const ToolStateChanges = core.ToolStateChanges;
pub const getToolStateChanges = core.getToolStateChanges;
pub const getDeclaredTools = core.getDeclaredTools;
pub const hasToolRedefinitions = core.hasToolRedefinitions;
pub const hasNonAdditiveToolChanges = core.hasNonAdditiveToolChanges;
pub const TranscriptTools = core.TranscriptTools;
pub const resolveTranscriptTools = core.resolveTranscriptTools;
pub const DiagnosticErrorInfo = core.DiagnosticErrorInfo;
pub const DiagnosticException = core.DiagnosticException;
pub const ThrownValue = core.ThrownValue;
pub const AssistantMessageDiagnostic = core.AssistantMessageDiagnostic;
pub const formatThrownValue = core.formatThrownValue;
pub const extractDiagnosticError = core.extractDiagnosticError;
pub const createAssistantMessageDiagnostic = core.createAssistantMessageDiagnostic;
pub const appendAssistantMessageDiagnostic = core.appendAssistantMessageDiagnostic;
pub const isContextOverflow = core.isContextOverflow;
pub const getOverflowPatterns = core.getOverflowPatterns;
pub const RetryPolicy = core.RetryPolicy;
pub const RetryCallbacks = core.RetryCallbacks;
pub const retryDelayMs = core.retryDelayMs;
pub const retryAssistantCall = core.retryAssistantCall;
pub const isRetryableAssistantError = core.isRetryableAssistantError;
pub const FauxModelDefinition = core.FauxModelDefinition;
pub const FauxProviderState = core.FauxProviderState;
pub const FauxResponseFactory = core.FauxResponseFactory;
pub const FauxResponseStep = core.FauxResponseStep;
pub const FauxAssistantOptions = core.FauxAssistantOptions;
pub const FauxProviderHandle = core.FauxProviderHandle;
pub const SessionResources = core.SessionResources;
pub const AnthropicOptions = core.AnthropicOptions;
pub const AnthropicEffort = core.AnthropicEffort;
pub const AnthropicThinkingDisplay = core.AnthropicThinkingDisplay;
pub const AzureOpenAIResponsesOptions = core.AzureOpenAIResponsesOptions;
pub const BedrockOptions = core.BedrockOptions;
pub const BedrockThinkingDisplay = core.BedrockThinkingDisplay;
pub const GoogleOptions = core.GoogleOptions;
pub const GoogleVertexOptions = core.GoogleVertexOptions;
pub const MistralOptions = core.MistralOptions;
pub const OpenAICodexResponsesOptions = core.OpenAICodexResponsesOptions;
pub const OpenAICodexWebSocketDebugStats = core.OpenAICodexWebSocketDebugStats;
pub const OpenAICompletionsOptions = core.OpenAICompletionsOptions;
pub const OpenAIResponsesOptions = core.OpenAIResponsesOptions;
pub const PiMessagesEvent = core.PiMessagesEvent;
pub const PiMessagesOptions = core.PiMessagesOptions;
pub const PiMessagesRewriteImpact = core.PiMessagesRewriteImpact;
pub const GoogleApiThinkingLevel = core.GoogleApiThinkingLevel;
pub const GoogleToolCallIds = core.GoogleToolCallIds;
pub const ResolvedGoogleThinkingLevel = core.ResolvedGoogleThinkingLevel;
pub const images = @import("images.zig");
pub const image_models = @import("image_models.zig");
pub const BuiltinImageProvider = image_models.BuiltinImageProvider;
pub const getImageModel = image_models.getImageModel;
pub const getImageModels = image_models.getImageModels;
pub const getImageProviders = image_models.getImageProviders;
pub const ImagesApiProvider = images.ImagesApiProvider;
pub const oauth = @import("oauth.zig");
pub const getModel = @import("providers/all.zig").getBuiltinModel;
pub const getModels = @import("providers/all.zig").getBuiltinModels;
pub const getProviders = @import("providers/all.zig").getBuiltinProviders;
pub const findEnvKeys = @import("env_api_keys.zig").findEnvKeys;
pub const getEnvApiKey = @import("env_api_keys.zig").getEnvApiKey;
pub const ApiProvider = m.ApiImplementation;
pub const anthropicMessagesApi = core.api.anthropic_messages.anthropicMessagesApi;
pub const azureOpenAIResponsesApi = core.api.azure_openai_responses.azureOpenAIResponsesApi;
pub const bedrockConverseStreamApi = core.api.bedrock_converse_stream.bedrockConverseStreamApi;
pub const googleGenerativeAIApi = core.api.google_generative_ai.googleGenerativeAIApi;
pub const googleVertexApi = core.api.google_vertex.googleVertexApi;
pub const mistralConversationsApi = core.api.mistral_conversations.mistralConversationsApi;
pub const openAICodexResponsesApi = core.api.openai_codex_responses.openAICodexResponsesApi;
pub const openAICompletionsApi = core.api.openai_completions.openAICompletionsApi;
pub const openAIResponsesApi = core.api.openai_responses.openAIResponsesApi;
pub const piMessagesApi = core.api.pi_messages.piMessagesApi;
const faux = @import("providers/faux.zig");
pub const RegisterFauxProviderOptions = faux.RegisterFauxProviderOptions;
/// Release before the Registry; the handle remains available after unregister.
pub const FauxProviderRegistration = struct {
    registry: *Registry,
    sourceId: []const u8,
    handle: *faux.FauxProviderHandle,
    api: []const u8,
    models: []const t.Model,
    state: *faux.FauxProviderState,
    pub fn getModel(self: @This(), id: ?[]const u8) ?t.Model {
        return self.handle.getModel(id);
    }
    pub fn setResponses(self: @This(), responses: []const faux.FauxResponseStep) !void {
        return self.handle.setResponses(responses);
    }
    pub fn appendResponses(self: @This(), responses: []const faux.FauxResponseStep) !void {
        return self.handle.appendResponses(responses);
    }
    pub fn getPendingResponseCount(self: @This()) !usize {
        return self.handle.getPendingResponseCount();
    }
    pub fn unregister(self: @This()) !void {
        return self.registry.unregisterApiProviders(self.sourceId);
    }
    pub fn deinit(self: *@This()) void {
        const io = self.registry.io;
        const protection = io.swapCancelProtection(.blocked);
        defer _ = io.swapCancelProtection(protection);
        self.unregister() catch |err| {
            if (err == error.Canceled) io.recancel();
        };
        self.handle.deinit();
        self.* = undefined;
    }
};
pub const builtinApis: []const []const u8 = &.{ "anthropic-messages", "openai-completions", "openai-responses", "openai-codex-responses", "azure-openai-responses", "google-generative-ai", "google-vertex", "mistral-conversations", "bedrock-converse-stream", "pi-messages" };
pub const Registry = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    arena: std.heap.ArenaAllocator,
    mutex: std.Io.Mutex = .init,
    entries: std.StringArrayHashMapUnmanaged(Entry) = .{},
    models: m.Models,
    imageRegistry: images.Registry,
    retainedFaux: std.ArrayList(*faux.FauxProviderHandle) = .empty,
    builtinSnapshots: [builtinApis.len]?*const Registered = @splat(null),
    const Entry = struct { provider: ApiProvider, sourceId: ?[]const u8, identity: *const Registered };
    const Selection = struct { provider: ApiProvider, matchesBuiltinSnapshot: bool };
    const Registered = struct {
        original: ApiProvider,
        fn stream(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
            const self: *@This() = @ptrCast(@alignCast(ctx.?));
            try @import("api/common.zig").requireApi(operation.allocator, model.api, self.original.api, operation.errorMessage orelse options.errorMessage, operation.errorStorage);
            const call = self.original.stream orelse return error.UnsupportedOperation;
            return call(self.original.context, operation, model, context, options);
        }
        fn streamSimple(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
            const self: *@This() = @ptrCast(@alignCast(ctx.?));
            try @import("api/common.zig").requireApi(operation.allocator, model.api, self.original.api, operation.errorMessage orelse options.errorMessage, operation.errorStorage);
            const call = self.original.streamSimple orelse return error.UnsupportedOperation;
            return call(self.original.context, operation, model, context, options);
        }
    };
    fn insertLocked(self: *@This(), provider: ApiProvider, source: ?[]const u8) !void {
        // Every registration owns a distinct wrapper, even when the original
        // functions and their context are identical to a previous registration.
        const registered = try self.arena.allocator().create(Registered);
        registered.* = .{ .original = provider };
        const wrapped = ApiProvider{ .api = provider.api, .context = registered, .stream = if (provider.stream != null) Registered.stream else null, .streamSimple = if (provider.streamSimple != null) Registered.streamSimple else null };
        try self.entries.put(self.allocator, provider.api, .{ .provider = wrapped, .sourceId = source, .identity = registered });
    }
    pub fn init(a: std.mem.Allocator, io: std.Io, options: m.Models.Options) !@This() {
        var model_collection = try @import("providers/all.zig").builtinModels(a, io, options);
        errdefer model_collection.deinit();
        var image_registry = try images.Registry.init(a, io);
        errdefer image_registry.deinit();
        var self = @This(){ .allocator = a, .io = io, .arena = .init(a), .models = model_collection, .imageRegistry = image_registry };
        errdefer {
            self.entries.deinit(a);
            self.arena.deinit();
        }
        try self.registerBuiltInApiProviders();
        return self;
    }
    pub fn deinit(self: *@This()) void {
        self.models.deinit();
        self.imageRegistry.deinit();
        self.entries.deinit(self.allocator);
        for (self.retainedFaux.items) |handle| handle.deinit();
        self.retainedFaux.deinit(self.allocator);
        self.arena.deinit();
        self.* = undefined;
    }
    pub fn registerFauxProvider(self: *@This(), options: RegisterFauxProviderOptions) !FauxProviderRegistration {
        const handle = try faux.fauxProvider(self.allocator, self.io, options);
        errdefer handle.deinit();
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const a = self.arena.allocator();
        const unique_id = try @import("utils/uuid.zig").randomV4(self.io);
        const source = try std.fmt.allocPrint(a, "faux-provider-{s}", .{unique_id});
        const api_id = try a.dupe(u8, handle.api);
        try self.retainedFaux.append(self.allocator, handle);
        errdefer _ = self.retainedFaux.pop();
        var implementation = handle.provider.apis[0];
        implementation.api = api_id;
        try self.insertLocked(implementation, source);
        handle.retainHandle();
        return .{ .registry = self, .sourceId = source, .handle = handle, .api = api_id, .models = handle.models, .state = &handle.state };
    }
    pub fn registerApiProvider(self: *@This(), provider: ApiProvider, source: ?[]const u8) !void {
        if (provider.stream == null) return error.MissingStreamImplementation;
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const a = self.arena.allocator();
        var owned = provider;
        owned.api = try a.dupe(u8, provider.api);
        try self.insertLocked(owned, if (source) |id| try a.dupe(u8, id) else null);
    }
    /// Implementations borrow this registry; original callback contexts remain
    /// caller-owned. Replacement does not invalidate a previously returned view.
    pub fn getApiProvider(self: *@This(), api_id: []const u8) !?ApiProvider {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        return if (self.entries.get(api_id)) |entry| entry.provider else null;
    }
    pub fn getApiProviders(self: *@This(), a: std.mem.Allocator) ![]const ApiProvider {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const out = try a.alloc(ApiProvider, self.entries.count());
        for (self.entries.values(), out) |entry, *value| value.* = entry.provider;
        return out;
    }
    pub fn unregisterApiProviders(self: *@This(), source: []const u8) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        var index: usize = 0;
        while (index < self.entries.count()) {
            const id = self.entries.values()[index].sourceId;
            if (id != null and std.mem.eql(u8, id.?, source)) {
                _ = self.entries.orderedRemove(self.entries.keys()[index]);
            } else index += 1;
        }
    }
    pub fn registerBuiltInApiProviders(self: *@This()) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        try self.registerBuiltInLocked();
    }
    fn registerBuiltInLocked(self: *@This()) !void {
        inline for (builtinApis, 0..) |api_id, index| {
            if (self.entries.get(api_id) == null) try self.insertLocked(@import("api/driver.zig").implementation(api_id), null);
            self.builtinSnapshots[index] = self.entries.get(api_id).?.identity;
        }
    }
    pub fn resetApiProviders(self: *@This()) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        self.entries.clearRetainingCapacity();
        self.builtinSnapshots = @splat(null);
        try self.registerBuiltInLocked();
    }
    fn lookup(self: *@This(), api_id: []const u8) !Selection {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const entry = self.entries.get(api_id) orelse return error.UnsupportedApi;
        const matches = for (builtinApis, self.builtinSnapshots) |builtin, snapshot| {
            if (std.mem.eql(u8, api_id, builtin)) break snapshot != null and snapshot.? == entry.identity;
        } else false;
        return .{ .provider = entry.provider, .matchesBuiltinSnapshot = matches };
    }
    fn withEnvApiKey(self: *@This(), a: std.mem.Allocator, model: t.Model, original: t.StreamOptions) !t.StreamOptions {
        if (original.apiKey) |key| if (!try @import("utils/unicode.zig").isBlank(key)) return original;
        var options = original;
        const key = (try getEnvApiKey(a, self.io, self.models.authContext, model.provider, options.env)) orelse return options;
        if (!std.mem.eql(u8, key, "<authenticated>")) options.apiKey = key;
        return options;
    }
    fn hasCloudflareAuth(options: t.StreamOptions) !bool {
        if (options.apiKey) |key| if (!try @import("utils/unicode.zig").isBlank(key)) return true;
        for (options.headers) |header| if (std.mem.eql(u8, header.name, "cf-aig-authorization") and header.value != null) return true;
        return false;
    }
    fn builtinProviderForModel(self: *@This(), entry: Selection, model: t.Model) !?m.Provider {
        if (!entry.matchesBuiltinSnapshot) return null;
        const provider = (try self.models.getProvider(model.provider)) orelse return null;
        var temporary = std.heap.ArenaAllocator.init(self.allocator);
        defer temporary.deinit();
        const candidates = if (provider.getModels) |get| try get(provider.context, temporary.allocator(), self.io) else provider.models;
        for (candidates) |candidate| if (candidate.modelType() == .chat and std.mem.eql(u8, candidate.api, model.api)) return provider;
        return null;
    }
    fn dispatch(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions, simple: bool) !*events.AssistantMessageEventStream {
        const entry = self.lookup(model.api) catch |err| {
            if (err == error.UnsupportedApi) if (options.errorMessage) |slot| {
                slot.* = try std.fmt.allocPrint(a, "No API provider registered for api: {s}", .{model.api});
            };
            return err;
        };
        const builtin = try self.builtinProviderForModel(entry, model);
        if (builtin != null and std.mem.startsWith(u8, model.provider, "cloudflare-") and !try hasCloudflareAuth(options)) return if (simple) self.models.streamSimple(a, model, context, options, .{}) else self.models.stream(a, model, context, options, .{});
        var arena = std.heap.ArenaAllocator.init(a);
        defer arena.deinit();
        const alloc = arena.allocator();
        const request = try self.withEnvApiKey(alloc, model, options);
        const implementation = if (builtin) |provider| provider.chatImplementation(model.api) orelse return error.UnsupportedApi else entry.provider;
        const call = (if (simple) implementation.streamSimple else implementation.stream) orelse return error.UnsupportedOperation;
        return call(implementation.context, .{ .allocator = a, .io = self.io, .transport = self.models.transport, .authContext = self.models.authContext, .awsState = &self.models.awsState, .awsBackgroundOwner = &self.models.awsState, .googleState = &self.models.googleState, .googleToolCallIds = &self.models.googleToolCallIds, .errorMessage = options.errorMessage }, model, try @import("utils/transcript.zig").normalizeContext(alloc, context), request);
    }
    pub fn stream(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions) !*events.AssistantMessageEventStream {
        return self.dispatch(a, model, context, options, false);
    }
    pub fn streamSimple(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions) !*events.AssistantMessageEventStream {
        return self.dispatch(a, model, context, options, true);
    }
    pub fn complete(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions) !t.Owned(t.AssistantMessage) {
        const handle = try self.stream(a, model, context, options);
        defer handle.destroy();
        return t.Owned(t.AssistantMessage).clone(a, try handle.result());
    }
    pub fn completeSimple(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions) !t.Owned(t.AssistantMessage) {
        const handle = try self.streamSimple(a, model, context, options);
        defer handle.destroy();
        return t.Owned(t.AssistantMessage).clone(a, try handle.result());
    }
    pub fn registerImagesApiProvider(self: *@This(), provider: ImagesApiProvider, source: ?[]const u8) !void {
        return self.imageRegistry.registerImagesApiProvider(provider, source);
    }
    pub fn getImagesApiProvider(self: *@This(), api_id: []const u8) !?ImagesApiProvider {
        return self.imageRegistry.getImagesApiProvider(api_id);
    }
    pub fn registerBuiltInImagesApiProviders(self: *@This()) !void {
        return self.imageRegistry.registerBuiltInImagesApiProviders();
    }
    pub fn generateImages(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.ImagesContext, options: t.ImagesOptions) !t.AssistantImages {
        return self.imageRegistry.generateImages(.{ .allocator = a, .io = self.io, .transport = self.models.transport, .authContext = self.models.authContext, .awsState = &self.models.awsState, .awsBackgroundOwner = &self.models.awsState, .googleState = &self.models.googleState }, model, context, options);
    }
};
pub const stream = Registry.stream;
pub const streamSimple = Registry.streamSimple;
pub const complete = Registry.complete;
pub const completeSimple = Registry.completeSimple;
pub const registerFauxProvider = Registry.registerFauxProvider;
pub const registerApiProvider = Registry.registerApiProvider;
pub const getApiProvider = Registry.getApiProvider;
pub const getApiProviders = Registry.getApiProviders;
pub const unregisterApiProviders = Registry.unregisterApiProviders;
pub const registerBuiltInApiProviders = Registry.registerBuiltInApiProviders;
pub const resetApiProviders = Registry.resetApiProviders;
pub const registerImagesApiProvider = Registry.registerImagesApiProvider;
pub const getImagesApiProvider = Registry.getImagesApiProvider;
pub const registerBuiltInImagesApiProviders = Registry.registerBuiltInImagesApiProviders;
pub const generateImages = Registry.generateImages;
pub fn generateImagesOpenRouter(operation: m.OperationContext, model: t.Model, context: t.ImagesContext, options: t.ImagesOptions) !t.AssistantImages {
    return core.api.openrouter_images.generateImages(null, operation, model, context, options);
}
fn Legacy(comptime api_id: []const u8, comptime simple: bool) type {
    return struct {
        pub fn call(operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
            try @import("api/common.zig").requireApi(operation.allocator, model.api, api_id, operation.errorMessage orelse options.errorMessage, operation.errorStorage);
            return if (simple) @import("api/driver.zig").streamSimple(null, operation, model, context, options) else @import("api/driver.zig").stream(null, operation, model, context, options);
        }
    };
}
pub const streamAnthropic = Legacy("anthropic-messages", false).call;
pub const streamSimpleAnthropic = Legacy("anthropic-messages", true).call;
pub const streamAzureOpenAIResponses = Legacy("azure-openai-responses", false).call;
pub const streamSimpleAzureOpenAIResponses = Legacy("azure-openai-responses", true).call;
pub const streamGoogle = Legacy("google-generative-ai", false).call;
pub const streamSimpleGoogle = Legacy("google-generative-ai", true).call;
pub const streamGoogleVertex = Legacy("google-vertex", false).call;
pub const streamSimpleGoogleVertex = Legacy("google-vertex", true).call;
pub const streamMistral = Legacy("mistral-conversations", false).call;
pub const streamSimpleMistral = Legacy("mistral-conversations", true).call;
pub const streamOpenAICodexResponses = Legacy("openai-codex-responses", false).call;
pub const streamSimpleOpenAICodexResponses = Legacy("openai-codex-responses", true).call;
pub const streamOpenAICompletions = Legacy("openai-completions", false).call;
pub const streamSimpleOpenAICompletions = Legacy("openai-completions", true).call;
pub const streamOpenAIResponses = Legacy("openai-responses", false).call;
pub const streamSimpleOpenAIResponses = Legacy("openai-responses", true).call;

pub const AiBinding = core.AiBinding;
pub const createAiBindingFetch = core.createAiBindingFetch;
pub const CLOUDFLARE_GATEWAY_BINDING_AUTH_SENTINEL = core.CLOUDFLARE_GATEWAY_BINDING_AUTH_SENTINEL;
pub const KnownApi = core.KnownApi;
pub const KnownImageApi = core.KnownImageApi;
pub const ImageApi = core.ImageApi;
pub const KnownClassifierApi = core.KnownClassifierApi;
pub const ClassifierApi = core.ClassifierApi;
pub const KnownProvider = core.KnownProvider;
pub const SamplingParams = core.SamplingParams;
pub const SamplingParamsByThinkingLevel = core.SamplingParamsByThinkingLevel;
pub const ThinkingLevelMap = core.ThinkingLevelMap;
pub const ModelInputModality = core.ModelInputModality;
pub const ModelPromptCache = core.ModelPromptCache;
pub const ModelImageResizeOptions = core.ModelImageResizeOptions;
pub const ModelImageInputLimits = core.ModelImageInputLimits;
pub const ModelInputLimits = core.ModelInputLimits;
pub const AnthropicAllowedFallbackModel = core.AnthropicAllowedFallbackModel;
pub const AnthropicMessagesCompat = core.AnthropicMessagesCompat;
pub const BedrockCompat = core.BedrockCompat;
pub const ChatTemplateKwargValue = core.ChatTemplateKwargValue;
pub const MistralConversationsCompat = core.MistralConversationsCompat;
pub const OpenAICompletionsCompat = core.OpenAICompletionsCompat;
pub const OpenAIResponsesCompat = core.OpenAIResponsesCompat;
pub const OpenRouterRouting = core.OpenRouterRouting;
pub const SessionAffinityFormat = core.SessionAffinityFormat;
pub const ThinkingTokenBudgetField = core.ThinkingTokenBudgetField;
pub const VercelGatewayRouting = core.VercelGatewayRouting;
pub const FetchFunction = core.FetchFunction;
pub const ProviderRequestOptions = core.ProviderRequestOptions;
pub const ApiOptionsMap = core.ApiOptionsMap;
pub const ApiStreamOptions = core.ApiStreamOptions;
pub const ProviderImages = core.ProviderImages;
pub const ProviderClassifier = core.ProviderClassifier;
pub const ProviderImagesOptions = core.ProviderImagesOptions;
pub const StreamFunction = core.StreamFunction;
pub const ImagesFunction = core.ImagesFunction;
pub const ClassifierFunction = core.ClassifierFunction;
pub const TextSignatureV1 = core.TextSignatureV1;
pub const JsonRepresentation = core.JsonRepresentation;
pub const ImagesInputContent = core.ImagesInputContent;
pub const ImagesOutputContent = core.ImagesOutputContent;
pub const ImagesStopReason = core.ImagesStopReason;
pub const ClassifierChoiceQuestion = core.ClassifierChoiceQuestion;
pub const ClassifierScoreQuestion = core.ClassifierScoreQuestion;
pub const ClassifierBoolQuestion = core.ClassifierBoolQuestion;
pub const ClassifierChoiceAnswer = core.ClassifierChoiceAnswer;
pub const ClassifierScoreAnswer = core.ClassifierScoreAnswer;
pub const ClassifierBoolAnswer = core.ClassifierBoolAnswer;
pub const ClassifierStopReason = core.ClassifierStopReason;
pub const GrammarFormat = core.GrammarFormat;
pub const GrammarVariants = core.GrammarVariants;
pub const BaseModel = core.BaseModel;
pub const ModelTypeMap = core.ModelTypeMap;
pub const MutableModels = core.MutableModels;
pub const CreateModelsOptions = core.CreateModelsOptions;
pub const CreateProviderOptions = core.CreateProviderOptions;
pub const ModelsApiStreamOptions = core.ModelsApiStreamOptions;
pub const ModelsSimpleStreamOptions = core.ModelsSimpleStreamOptions;
pub const ModelsDeferredFetchOptions = core.ModelsDeferredFetchOptions;
pub const ModelsDeferredCancelOptions = core.ModelsDeferredCancelOptions;
pub const ModelsImagesOptions = core.ModelsImagesOptions;
pub const ModelsClassifierOptions = core.ModelsClassifierOptions;
pub const AuthOperationOptions = core.AuthOperationOptions;
pub const AuthType = core.AuthType;
pub const AuthInfoLink = core.AuthInfoLink;
pub const AuthInteraction = core.AuthInteraction;
pub const ProviderAuthInteraction = core.ProviderAuthInteraction;
pub const CredentialInputs = core.CredentialInputs;
pub const CredentialCommand = core.CredentialCommand;
pub const ModelsStoreOperationOptions = core.ModelsStoreOperationOptions;
pub const SessionResourceCleanup = core.SessionResourceCleanup;
pub const SessionResourceRegistration = core.SessionResourceRegistration;
pub const registerSessionResourceCleanup = core.registerSessionResourceCleanup;
pub const cleanupSessionResources = core.cleanupSessionResources;
pub const FauxContentBlock = core.FauxContentBlock;
pub const isRecoverableLength = core.isRecoverableLength;
pub const DEFAULT_MAX_AGENT_RETRY_DELAY_MS = core.DEFAULT_MAX_AGENT_RETRY_DELAY_MS;
pub const TranscriptMessages = core.TranscriptMessages;

pub const BuiltinProvider = @import("providers/all.zig").BuiltinProvider;
pub const ApiStreamFunction = t.StreamFunction;
pub const ApiStreamSimpleFunction = t.StreamFunction;
pub const ImagesApiFunction = t.ImagesFunction;
pub const ANTHROPIC_AUTH_TOKEN_ENV = @import("env_api_keys.zig").ANTHROPIC_AUTH_TOKEN_ENV;
pub const ANTHROPIC_OAUTH_TOKEN_ENV = @import("env_api_keys.zig").ANTHROPIC_OAUTH_TOKEN_ENV;
pub const ANTHROPIC_API_KEY_ENV = @import("env_api_keys.zig").ANTHROPIC_API_KEY_ENV;
pub const ANTHROPIC_FEDERATION_RULE_ID_ENV = @import("env_api_keys.zig").ANTHROPIC_FEDERATION_RULE_ID_ENV;
pub const ANTHROPIC_ORGANIZATION_ID_ENV = @import("env_api_keys.zig").ANTHROPIC_ORGANIZATION_ID_ENV;
pub const ANTHROPIC_SERVICE_ACCOUNT_ID_ENV = @import("env_api_keys.zig").ANTHROPIC_SERVICE_ACCOUNT_ID_ENV;
pub const ANTHROPIC_IDENTITY_TOKEN_FILE_ENV = @import("env_api_keys.zig").ANTHROPIC_IDENTITY_TOKEN_FILE_ENV;
pub const ANTHROPIC_WORKSPACE_ID_ENV = @import("env_api_keys.zig").ANTHROPIC_WORKSPACE_ID_ENV;
