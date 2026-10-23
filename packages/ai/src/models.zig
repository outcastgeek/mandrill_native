const std = @import("std");
const t = @import("types.zig");
const auth = @import("auth/types.zig");
const resolve = @import("auth/resolve.zig");
const ms = @import("models_store.zig");
const http = @import("transport/http.zig");
const events = @import("utils/event_stream.zig");
const headers = @import("utils/headers.zig");
pub const calculateCost = @import("utils/cost.zig").calculateCost;
pub const getSupportedThinkingLevels = @import("utils/thinking.zig").getSupportedThinkingLevels;
pub const clampThinkingLevel = @import("utils/thinking.zig").clampThinkingLevel;
pub const ModelsError = @import("utils/models_error.zig").ModelsError;
pub const ModelsErrorCode = @import("utils/models_error.zig").ModelsErrorCode;
pub const OperationContext = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    transport: http.Transport,
    authContext: ?auth.AuthContext = null,
    awsState: ?*@import("auth/aws/state.zig").State = null,
    googleState: ?*@import("auth/google/state.zig").State = null,
    awsRegion: ?[]const u8 = null,
    /// Optional owner for credential refresh that may outlive this request.
    /// Io, operation state and opaque auth/transport contexts remain valid
    /// through its deinit.
    /// Models supplies this only with its own context/transport lifetime.
    awsBackgroundOwner: ?*@import("auth/aws/state.zig").State = null,
    googleToolCallIds: ?*@import("api/google_shared.zig").GoogleToolCallIds = null,
    /// A borrowed diagnostic view, with no allocation ownership transfer.
    /// Before destroying a private owner, retain its text through errorStorage.
    errorMessage: ?*?[]const u8 = null,
    /// Caller-owned storage for explicit copies. Its lifetime includes joining
    /// any returned stream; static or otherwise borrowed diagnostics need none.
    errorStorage: ?*t.ErrorMessageStorage = null,
};
pub const ApiImplementation = struct {
    api: []const u8 = "",
    context: ?*anyopaque = null,
    stream: ?*const fn (?*anyopaque, OperationContext, t.Model, t.TranscriptContext, t.StreamOptions) anyerror!*events.AssistantMessageEventStream = null,
    streamSimple: ?*const fn (?*anyopaque, OperationContext, t.Model, t.TranscriptContext, t.StreamOptions) anyerror!*events.AssistantMessageEventStream = null,
    complete: ?*const fn (?*anyopaque, OperationContext, t.Model, t.TranscriptContext, t.StreamOptions) anyerror!t.Owned(t.AssistantMessage) = null,
    completeSimple: ?*const fn (?*anyopaque, OperationContext, t.Model, t.TranscriptContext, t.StreamOptions) anyerror!t.Owned(t.AssistantMessage) = null,
    generateImages: ?*const fn (?*anyopaque, OperationContext, t.Model, t.ImagesContext, t.ImagesOptions) anyerror!t.AssistantImages = null,
    classify: ?*const fn (?*anyopaque, OperationContext, t.Model, t.ClassifierContext, t.ClassifierOptions) anyerror!t.ClassifierResult = null,
    fetchDeferred: ?*const fn (?*anyopaque, OperationContext, t.Model, t.DeferredHandle, t.DeferredFetchOptions) anyerror!*events.AssistantMessageEventStream = null,
    cancelDeferred: ?*const fn (?*anyopaque, OperationContext, t.Model, t.DeferredHandle, t.DeferredCancelOptions) anyerror!void = null,
};
pub const ProviderStreams = ApiImplementation;
pub const ProviderImages = ApiImplementation;
pub const ProviderClassifier = ApiImplementation;
pub const MutableModels = Models;
pub const CreateModelsOptions = Models.Options;
// Native calls pass transforms separately from these request-option values.
pub const ModelsApiStreamOptions = t.ApiStreamOptions;
pub const ModelsSimpleStreamOptions = t.SimpleStreamOptions;
pub const ModelsDeferredFetchOptions = t.DeferredFetchOptions;
pub const ModelsDeferredCancelOptions = t.DeferredCancelOptions;
pub const ModelsImagesOptions = t.ImagesOptions;
pub const ModelsClassifierOptions = t.ClassifierOptions;
pub const ModelsPublication = struct { models: ?[]const t.Model = null, persist: union(enum) { unchanged, delete, write: ms.ModelsStoreEntry } = .unchanged, updateContext: ?*anyopaque = null, update: ?*const fn (?*anyopaque) void = null };
pub const RefreshModelsContext = struct {
    operation: OperationContext,
    stored: ?ms.ModelsStoreEntry,
    credential: ?auth.Credential,
    auth: ?auth.AuthResult,
    allowNetwork: bool,
    force: ?bool,
    baseUrl: ?[]const u8,
    owner: *Models,
    providerId: []const u8,
    generation: u64,
    pub fn publish(self: @This(), publication: ModelsPublication) !bool {
        return self.owner.publish(self.providerId, self.generation, publication);
    }
};
pub const Provider = struct {
    id: []const u8,
    name: []const u8,
    baseUrl: ?[]const u8 = null,
    headers: t.ProviderHeaders = &.{},
    auth: auth.ProviderAuth,
    models: []const t.Model = &.{},
    apis: []const ApiImplementation = &.{},
    context: ?*anyopaque = null,
    /// A single chat implementation handles every chat API ID. Image and
    /// classifier implementations continue to dispatch through `apis`.
    streams: ?ProviderStreams = null,
    cloneContext: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror!?*anyopaque = null,
    deinitContext: ?*const fn (?*anyopaque, std.mem.Allocator) void = null,
    getModels: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror![]const t.Model = null,
    getAllModels: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror![]const t.Model = null,
    refreshModels: ?*const fn (?*anyopaque, RefreshModelsContext) anyerror!void = null,
    fetchModels: ?*const fn (?*anyopaque, RefreshModelsContext) anyerror![]const t.Model = null,
    filterAllModels: ?*const fn (?*anyopaque, std.mem.Allocator, []const t.Model, ?auth.Credential) anyerror![]const t.Model = null,
    filterModels: ?*const fn (?*anyopaque, std.mem.Allocator, []const t.Model, ?auth.Credential) anyerror![]const t.Model = null,
    pub fn implementation(self: @This(), api: []const u8) ?ApiImplementation {
        for (self.apis) |impl| if (std.mem.eql(u8, impl.api, api)) return impl;
        return null;
    }
    pub fn chatImplementation(self: @This(), api: []const u8) ?ApiImplementation {
        return self.streams orelse self.implementation(api);
    }
    /// Direct provider calls receive normalized input and already selected
    /// request authentication, just as the pinned Provider methods do.
    pub fn stream(self: @This(), operation: OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
        const impl = self.chatImplementation(model.api) orelse return self.missingStream(operation, model, false);
        const call = impl.stream orelse return self.missingStream(operation, model, false);
        return call(impl.context, operation, model, context, options);
    }
    pub fn streamSimple(self: @This(), operation: OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*events.AssistantMessageEventStream {
        const impl = self.chatImplementation(model.api) orelse return self.missingStream(operation, model, false);
        const call = impl.streamSimple orelse return self.missingStream(operation, model, false);
        return call(impl.context, operation, model, context, options);
    }
    pub fn fetchDeferred(self: @This(), operation: OperationContext, model: t.Model, handle: t.DeferredHandle, options: t.DeferredFetchOptions) !*events.AssistantMessageEventStream {
        const impl = self.chatImplementation(model.api) orelse return self.missingStream(operation, model, true);
        const call = impl.fetchDeferred orelse return self.missingStream(operation, model, true);
        return call(impl.context, operation, model, handle, options);
    }
    pub fn cancelDeferred(self: @This(), operation: OperationContext, model: t.Model, handle: t.DeferredHandle, options: t.DeferredCancelOptions) !void {
        const impl = self.chatImplementation(model.api);
        const call = if (impl) |value| value.cancelDeferred else null;
        if (call) |cancel| return cancel(impl.?.context, operation, model, handle, options);
        try @import("api/common.zig").rememberError(operation.allocator, operation.errorMessage orelse options.errorMessage, operation.errorStorage, "Provider {s} cannot cancel deferred responses for \"{s}\"", .{ self.id, model.api });
        return error.UnsupportedDeferredCancelApi;
    }
    fn missingStream(self: @This(), operation: OperationContext, model: t.Model, deferred: bool) !*events.AssistantMessageEventStream {
        const result = try events.AssistantMessageEventStream.create(operation.allocator, operation.io);
        errdefer result.destroy();
        try result.prepareModel(model);
        const a = result.arena.allocator();
        const message = if (deferred) try std.fmt.allocPrint(a, "Provider {s} does not support deferred responses for \"{s}\"", .{ self.id, model.api }) else try std.fmt.allocPrint(a, "Provider {s} has no API implementation for \"{s}\"", .{ self.id, model.api });
        const failure = t.AssistantMessage{ .api = model.api, .provider = model.provider, .model = model.id, .timestamp = @floatFromInt(result.started_wall), .stopReason = .@"error", .errorMessage = message };
        try result.push(.{ .@"error" = .{ .reason = .@"error", .@"error" = failure } });
        return result;
    }
    pub fn generateImages(self: @This(), operation: OperationContext, model: t.Model, context: t.ImagesContext, options: t.ImagesOptions) !t.AssistantImages {
        if (self.implementation(model.api)) |impl| if (impl.generateImages) |call| return call(impl.context, operation, model, context, options);
        var temporary = std.heap.ArenaAllocator.init(operation.allocator);
        defer temporary.deinit();
        const message = try std.fmt.allocPrint(temporary.allocator(), "Provider {s} has no image generation implementation for \"{s}\"", .{ self.id, model.api });
        return @import("utils/model_operations.zig").imageErrorResult(operation.allocator, operation.io, model, message, false);
    }
    pub fn classify(self: @This(), operation: OperationContext, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions) !t.ClassifierResult {
        if (self.implementation(model.api)) |impl| if (impl.classify) |call| return call(impl.context, operation, model, context, options);
        var temporary = std.heap.ArenaAllocator.init(operation.allocator);
        defer temporary.deinit();
        const message = try std.fmt.allocPrint(temporary.allocator(), "Provider {s} has no classifier implementation for \"{s}\"", .{ self.id, model.api });
        return @import("utils/model_operations.zig").classifierErrorResult(operation.allocator, operation.io, model, message, false);
    }
    /// Native accessor names distinguish calls from optional getModels /
    /// getAllModels callback fields. Results follow the callback's allocation
    /// contract; an operation arena accommodates static and allocated lists.
    pub const listModels = chatModels;
    pub const listAllModels = allModels;
    fn supportsOperation(self: @This(), comptime operation_name: []const u8) bool {
        if (comptime !std.mem.eql(u8, operation_name, "generateImages") and !std.mem.eql(u8, operation_name, "classify")) {
            if (self.streams) |impl| return @field(impl, operation_name) != null;
        }
        for (self.apis) |impl| if (@field(impl, operation_name) != null) return true;
        return false;
    }
    fn chatModels(self: @This(), a: std.mem.Allocator, io: std.Io) ![]const t.Model {
        if (self.getModels) |get| return get(self.context, a, io);
        var chats: std.ArrayList(t.Model) = .empty;
        errdefer chats.deinit(a);
        for (self.models) |model| if (model.modelType() == .chat) {
            try chats.append(a, model);
        };
        return chats.toOwnedSlice(a);
    }
    fn allModels(self: @This(), a: std.mem.Allocator, io: std.Io) ![]const t.Model {
        if (self.getAllModels) |get| return get(self.context, a, io);
        if (self.getModels) |get| return get(self.context, a, io);
        return self.models;
    }
};
/// Typed construction input; streams handles a single chat implementation,
/// while apis contains implementations keyed by API for all model types.
pub const CreateProviderOptions = struct {
    id: []const u8,
    name: ?[]const u8 = null,
    baseUrl: ?[]const u8 = null,
    headers: t.ProviderHeaders = &.{},
    auth: auth.ProviderAuth,
    models: []const t.Model = &.{},
    apis: []const ApiImplementation = &.{},
    context: ?*anyopaque = null,
    streams: ?ProviderStreams = null,
    cloneContext: @FieldType(Provider, "cloneContext") = null,
    deinitContext: @FieldType(Provider, "deinitContext") = null,
    getModels: @FieldType(Provider, "getModels") = null,
    getAllModels: @FieldType(Provider, "getAllModels") = null,
    refreshModels: @FieldType(Provider, "refreshModels") = null,
    fetchModels: @FieldType(Provider, "fetchModels") = null,
    filterAllModels: @FieldType(Provider, "filterAllModels") = null,
    filterModels: @FieldType(Provider, "filterModels") = null,
};
pub fn createProvider(input: anytype) !Provider {
    const Input = @TypeOf(input);
    const supplied_name = if (@hasField(Input, "name")) input.name else null;
    const name: []const u8 = switch (@typeInfo(@TypeOf(supplied_name))) {
        .null => input.id,
        .optional => supplied_name orelse input.id,
        else => supplied_name,
    };
    var provider = Provider{ .id = input.id, .name = name, .auth = input.auth };
    inline for (@typeInfo(Input).@"struct".field_names) |field| {
        if (comptime std.mem.eql(u8, field, "id") or std.mem.eql(u8, field, "name") or std.mem.eql(u8, field, "auth")) continue;
        if (!@hasField(Provider, field)) @compileError("Unknown provider field: " ++ field);
        @field(provider, field) = @field(input, field);
    }
    if (!provider.supportsOperation("stream") and !provider.supportsOperation("streamSimple") and !provider.supportsOperation("complete") and !provider.supportsOperation("completeSimple") and !provider.supportsOperation("generateImages") and !provider.supportsOperation("classify")) return error.ProviderImplementationRequired;
    return provider;
}
pub const ModelsRequestTransforms = struct { context: ?*anyopaque = null, transformHeaders: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, t.ProviderHeaders) anyerror!t.ProviderHeaders = null };
pub const ModelsRefreshOptions = struct { allowNetwork: bool = true, force: ?bool = null, providers: ?[]const []const u8 = null };
pub const RefreshError = struct { provider: []const u8, err: anyerror, message: ?[]const u8 = null };
pub const ModelsRefreshResult = struct { aborted: bool = false, errors: []const RefreshError };
/// Retain supplied Io/allocator, contexts and external stores/transports until
/// all operations have joined. Destroy request handles before deinit. Lookup
/// values borrow retained collection snapshots; use Owned(T).clone to detach.
/// The backing allocator must support concurrent allocation when operations
/// overlap: store transactions, HTTP pools and separate refresh arenas share it.
/// Collection snapshot mutations remain protected by the collection mutex.
pub const Models = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    arena: std.heap.ArenaAllocator,
    mutex: std.Io.Mutex = .init,
    providers: std.StringArrayHashMapUnmanaged(Registration) = .{},
    generation: u64 = 0,
    publicationLocks: std.StringHashMapUnmanaged(*std.Io.Mutex) = .{},
    credentials: auth.CredentialStore,
    store: ms.ModelsStore,
    authContext: auth.AuthContext,
    transport: http.Transport,
    authResolutions: resolve.AuthResolutionOwner,
    sessionResources: @import("session_resources.zig").SessionResources,
    awsState: @import("auth/aws/state.zig").State,
    googleState: @import("auth/google/state.zig").State = .{},
    googleToolCallIds: @import("api/google_shared.zig").GoogleToolCallIds = .{},
    ownedCredentials: ?*@import("auth/credential_store.zig").InMemoryCredentialStore = null,
    ownedStore: ?*ms.InMemoryModelsStore = null,
    ownedHttp: ?*http.Http = null,
    retainedContexts: std.ArrayList(struct { value: ?*anyopaque, destroy: *const fn (?*anyopaque, std.mem.Allocator) void }) = .empty,
    refreshJobs: std.ArrayList(*RefreshJob) = .empty,
    refreshErrorSequence: std.atomic.Value(u64) = .init(0),
    const Registration = struct { provider: Provider, generation: u64, baseline: []const t.Model, refreshJob: ?*RefreshJob = null };
    const RefreshJob = struct {
        owner: *Models,
        arena: std.heap.ArenaAllocator,
        registration: Registration,
        options: ModelsRefreshOptions,
        superseded: std.Io.Event = .unset,
        canceled: std.Io.Event = .unset,
        done: std.Io.Event = .unset,
        observed: std.Io.Event = .unset,
        callerGone: std.atomic.Value(bool) = .init(false),
        future: std.Io.Future(void) = undefined,
        failure: ?anyerror = null,
        errorMessage: ?[]const u8 = null,
        failureOrder: u64 = std.math.maxInt(u64),
        fn work(self: *RefreshJob) !void {
            if (self.superseded.isSet() or self.canceled.isSet()) return;
            return self.owner.refreshOne(self.arena.allocator(), self.registration, self.options, &self.errorMessage);
        }
        fn run(self: *RefreshJob) void {
            const io = self.owner.io;
            defer {
                if (self.failure != null and !self.superseded.isSet() and !self.canceled.isSet()) self.failureOrder = self.owner.refreshErrorSequence.fetchAdd(1, .monotonic);
                self.done.set(io);
                self.observed.set(io);
            }
            const Outcome = union(enum) { work: anyerror!void, superseded: std.Io.Cancelable!void, canceled: std.Io.Cancelable!void };
            var buffer: [3]Outcome = undefined;
            var select = std.Io.Select(Outcome).init(io, &buffer);
            defer select.cancelDiscard();
            select.concurrent(.work, RefreshJob.work, .{self}) catch |err| {
                self.failure = err;
                return;
            };
            select.concurrent(.superseded, std.Io.Event.wait, .{ &self.superseded, io }) catch |err| {
                self.failure = err;
                return;
            };
            select.concurrent(.canceled, std.Io.Event.wait, .{ &self.canceled, io }) catch |err| {
                self.failure = err;
                return;
            };
            const outcome = select.await() catch |err| {
                self.failure = err;
                return;
            };
            switch (outcome) {
                .work => |result| result catch |err| {
                    self.failure = err;
                },
                .superseded, .canceled => |result| {
                    result catch {};
                    self.failure = error.Canceled;
                },
            }
        }
        fn wait(self: *RefreshJob) std.Io.Cancelable!void {
            return self.observed.wait(self.owner.io);
        }
        fn supersede(self: *RefreshJob) void {
            self.superseded.set(self.owner.io);
            self.observed.set(self.owner.io);
        }
        fn destroy(self: *RefreshJob) void {
            const a = self.owner.allocator;
            self.arena.deinit();
            a.destroy(self);
        }
    };
    pub const Options = struct { credentials: ?auth.CredentialStore = null, store: ?ms.ModelsStore = null, authContext: auth.AuthContext, transport: ?http.Transport = null, providers: []const Provider = &.{} };
    pub fn init(a: std.mem.Allocator, io: std.Io, options: Options) !@This() {
        var transferred = false;
        const owned_credentials = if (options.credentials == null) try a.create(@import("auth/credential_store.zig").InMemoryCredentialStore) else null;
        errdefer if (!transferred) {
            if (owned_credentials) |value| a.destroy(value);
        };
        if (owned_credentials) |value| value.* = .init(a);
        const owned_store = if (options.store == null) try a.create(ms.InMemoryModelsStore) else null;
        errdefer if (!transferred) {
            if (owned_store) |value| a.destroy(value);
        };
        if (owned_store) |value| value.* = .init(a);
        const owned_http = if (options.transport == null) try a.create(http.Http) else null;
        errdefer if (!transferred) {
            if (owned_http) |value| a.destroy(value);
        };
        if (owned_http) |value| {
            value.* = .init(a, io);
            value.authContext = options.authContext;
        }
        var self = @This(){ .allocator = a, .io = io, .arena = .init(a), .credentials = options.credentials orelse owned_credentials.?.store(), .store = options.store orelse owned_store.?.store(), .authContext = options.authContext, .transport = options.transport orelse owned_http.?.transport(), .ownedCredentials = owned_credentials, .ownedStore = owned_store, .ownedHttp = owned_http, .authResolutions = .init(a, io), .sessionResources = .init(a), .awsState = .init(a) };
        transferred = true;
        errdefer self.deinit();
        if (self.transport.sessions) |cache| try self.sessionResources.register(cache.sessionResource());
        for (options.providers) |provider| try self.register(provider);
        return self;
    }
    pub fn deinit(self: *@This()) void {
        for (self.refreshJobs.items) |job| job.canceled.set(self.io);
        for (self.refreshJobs.items) |job| {
            job.future.await(self.io);
            job.destroy();
        }
        self.refreshJobs.deinit(self.allocator);
        self.authResolutions.deinit();
        self.awsState.deinit();
        for (self.retainedContexts.items) |context| context.destroy(context.value, self.allocator);
        self.retainedContexts.deinit(self.allocator);
        if (self.ownedCredentials) |value| {
            value.deinit();
            self.allocator.destroy(value);
        }
        if (self.ownedStore) |value| {
            value.deinit();
            self.allocator.destroy(value);
        }
        if (self.ownedHttp) |value| {
            value.deinit();
            self.allocator.destroy(value);
        }
        self.sessionResources.deinit();
        self.providers.deinit(self.allocator);
        self.publicationLocks.deinit(self.allocator);
        self.arena.deinit();
        self.* = undefined;
    }
    fn makeOperation(self: *@This(), a: std.mem.Allocator) OperationContext {
        return .{ .allocator = a, .io = self.io, .transport = self.transport, .authContext = self.authContext, .awsState = &self.awsState, .awsBackgroundOwner = &self.awsState, .googleState = &self.googleState, .googleToolCallIds = &self.googleToolCallIds };
    }
    pub fn cleanupSessionResources(self: *@This(), session: ?[]const u8) ![]const anyerror {
        return self.sessionResources.close(self.io, session);
    }
    pub fn register(self: *@This(), original: Provider) !void {
        const provider = try createProvider(original);
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const a = self.arena.allocator();
        var owned = provider;
        var context_retained = false;
        if (provider.cloneContext) |clone| {
            owned.context = try clone(provider.context, self.allocator, self.io);
            errdefer if (!context_retained) {
                if (provider.deinitContext) |destroy| destroy(owned.context, self.allocator);
            };
            if (provider.deinitContext) |destroy| {
                try self.retainedContexts.append(self.allocator, .{ .value = owned.context, .destroy = destroy });
                context_retained = true;
            }
            if (owned.auth.apiKey) |*method| if (method.context == provider.context) {
                method.context = owned.context;
            };
            if (owned.auth.oauth) |*method| if (method.context == provider.context) {
                method.context = owned.context;
            };
        }
        owned.id = try a.dupe(u8, provider.id);
        owned.name = try a.dupe(u8, provider.name);
        owned.baseUrl = if (provider.baseUrl) |url| try a.dupe(u8, url) else null;
        const provider_headers = try a.alloc(t.ProviderHeader, provider.headers.len);
        for (provider.headers, provider_headers) |header, *out| out.* = .{ .name = try a.dupe(u8, header.name), .value = if (header.value) |value| try a.dupe(u8, value) else null };
        owned.headers = provider_headers;
        if (owned.auth.apiKey) |*method| method.name = try a.dupe(u8, method.name);
        if (owned.auth.oauth) |*method| {
            method.name = try a.dupe(u8, method.name);
            method.loginLabel = if (method.loginLabel) |label| try a.dupe(u8, label) else null;
        }
        owned.models = (try t.Owned([]const t.Model).clone(a, provider.models)).value();
        const implementations = try a.dupe(ApiImplementation, provider.apis);
        for (implementations) |*implementation| {
            implementation.api = try a.dupe(u8, implementation.api);
            if (provider.context != null and implementation.context == provider.context) implementation.context = owned.context;
        }
        owned.apis = implementations;
        if (owned.streams) |*implementation| {
            implementation.api = try a.dupe(u8, implementation.api);
            if (provider.context != null and implementation.context == provider.context) implementation.context = owned.context;
        }
        if (self.providers.get(owned.id)) |previous| if (previous.refreshJob) |job| job.supersede();
        self.generation += 1;
        try self.providers.put(self.allocator, owned.id, .{ .provider = owned, .generation = self.generation, .baseline = owned.models });
    }
    pub fn unregister(self: *@This(), id: []const u8) !bool {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        if (self.providers.get(id)) |registration| if (registration.refreshJob) |job| job.supersede();
        return self.providers.orderedRemove(id);
    }
    pub const setProvider = register;
    pub const deleteProvider = unregister;
    pub fn clearProviders(self: *@This()) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        self.generation += 1;
        for (self.providers.values()) |registration| if (registration.refreshJob) |job| job.supersede();
        self.providers.clearRetainingCapacity();
    }
    /// The caller owns the outer slice; provider fields borrow this collection.
    pub fn getProviders(self: *@This(), a: std.mem.Allocator) ![]const Provider {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const result = try a.alloc(Provider, self.providers.count());
        for (self.providers.values(), result) |registration, *provider| provider.* = registration.provider;
        return result;
    }
    pub fn getProvider(self: *@This(), id: []const u8) !?Provider {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        return if (self.providers.get(id)) |r| r.provider else null;
    }
    pub fn getModel(self: *@This(), provider_id: []const u8, id: []const u8) !?t.Model {
        return self.findModel(provider_id, .chat, id, true);
    }
    pub fn getImageModel(self: *@This(), provider_id: []const u8, id: []const u8) !?t.Model {
        return self.getModelByType(provider_id, .image, id);
    }
    pub fn getClassifierModel(self: *@This(), provider_id: []const u8, id: []const u8) !?t.Model {
        return self.getModelByType(provider_id, .classifier, id);
    }
    pub fn getModelByType(self: *@This(), provider_id: []const u8, kind: t.ModelType, id: []const u8) !?t.Model {
        return self.findModel(provider_id, kind, id, false);
    }
    fn findModel(self: *@This(), provider_id: []const u8, kind: t.ModelType, id: []const u8, chats_only: bool) !?t.Model {
        const provider = (try self.getProvider(provider_id)) orelse return null;
        if ((chats_only and provider.getModels != null) or (!chats_only and (provider.getAllModels != null or provider.getModels != null))) {
            // Provider code may call back into Models. Run it outside the
            // collection lock, then retain only the selected snapshot.
            var temporary = std.heap.ArenaAllocator.init(self.allocator);
            defer temporary.deinit();
            const all = (if (chats_only) provider.chatModels(temporary.allocator(), self.io) else provider.allModels(temporary.allocator(), self.io)) catch |err| switch (err) {
                error.OutOfMemory, error.Canceled => return err,
                else => return null,
            };
            for (all) |model| if (model.modelType() == kind and std.mem.eql(u8, model.id, id)) {
                try self.mutex.lock(self.io);
                defer self.mutex.unlock(self.io);
                return try t.cloneInto(t.Model, self.arena.allocator(), model);
            };
            return null;
        }
        for (provider.models) |model| if (model.modelType() == kind and std.mem.eql(u8, model.id, id)) return model;
        return null;
    }
    pub fn getAll(self: *@This(), a: std.mem.Allocator, provider_id: ?[]const u8) ![]const t.Model {
        const providers = try self.getProviders(a);
        defer a.free(providers);
        var out: std.ArrayList(t.Model) = .empty;
        errdefer out.deinit(a);
        for (providers) |provider| {
            if (provider_id) |id| if (!std.mem.eql(u8, id, provider.id)) continue;
            if (provider.getAllModels == null and provider.getModels == null) {
                try out.appendSlice(a, provider.models);
                continue;
            }
            var temporary = std.heap.ArenaAllocator.init(a);
            defer temporary.deinit();
            const all = provider.allModels(temporary.allocator(), self.io) catch |err| switch (err) {
                error.OutOfMemory, error.Canceled => return err,
                else => continue,
            };
            for (all) |model| try out.append(a, try t.cloneInto(t.Model, a, model));
        }
        return out.toOwnedSlice(a);
    }
    pub fn getModels(self: *@This(), a: std.mem.Allocator, provider_id: ?[]const u8) ![]const t.Model {
        const providers = try self.getProviders(a);
        defer a.free(providers);
        var out: std.ArrayList(t.Model) = .empty;
        errdefer out.deinit(a);
        for (providers) |provider| {
            if (provider_id) |id| if (!std.mem.eql(u8, id, provider.id)) continue;
            if (provider.getModels == null) {
                for (provider.models) |model| if (model.modelType() == .chat) {
                    try out.append(a, model);
                };
                continue;
            }
            var temporary = std.heap.ArenaAllocator.init(a);
            defer temporary.deinit();
            const chats = provider.chatModels(temporary.allocator(), self.io) catch |err| switch (err) {
                error.OutOfMemory, error.Canceled => return err,
                else => continue,
            };
            for (chats) |model| try out.append(a, try t.cloneInto(t.Model, a, model));
        }
        return out.toOwnedSlice(a);
    }
    pub fn getModelsOfType(self: *@This(), a: std.mem.Allocator, kind: t.ModelType, provider_id: ?[]const u8) ![]const t.Model {
        const all = try self.getAll(a, provider_id);
        defer a.free(all);
        var out: std.ArrayList(t.Model) = .empty;
        errdefer out.deinit(a);
        for (all) |model| if (model.modelType() == kind) {
            try out.append(a, model);
        };
        return out.toOwnedSlice(a);
    }
    pub fn getModelOfType(self: *@This(), kind: t.ModelType, provider_id: []const u8, id: []const u8) !?t.Model {
        return self.getModelByType(provider_id, kind, id);
    }
    /// Accept a provider id or a model. Model headers override the resolved
    /// auth headers with the same case-insensitive tombstones as request setup.
    pub fn getAuth(self: *@This(), a: std.mem.Allocator, provider_or_model: anytype, overrides: resolve.AuthResolutionOverrides) !?auth.AuthResult {
        const id: []const u8 = if (comptime @TypeOf(provider_or_model) == t.Model) provider_or_model.provider else provider_or_model;
        const provider = (try self.getProvider(id)) orelse return null;
        const result = (try resolve.resolveProviderAuth(a, self.io, &self.authResolutions, id, provider.auth, self.credentials, self.authContext, overrides, self.transport)) orelse return null;
        if (comptime @TypeOf(provider_or_model) == t.Model) if (provider_or_model.headers != null) {
            var with_headers = result;
            with_headers.auth.headers = try headers.overlay(a, &.{ result.auth.headers, try headers.fromValue(a, provider_or_model.headers) });
            return with_headers;
        };
        return result;
    }
    fn readCredential(self: *@This(), a: std.mem.Allocator, id: []const u8, error_message: ?*?[]const u8) !?auth.Credential {
        const credential = self.credentials.read(a, self.io, id) catch |err| {
            try resolve.remember(a, error_message, err, "Credential store read failed for {s}", .{id});
            return err;
        };
        try self.io.checkCancel();
        return credential;
    }
    fn checkProviderAuth(self: *@This(), a: std.mem.Allocator, provider: Provider, credential: ?auth.Credential, error_message: ?*?[]const u8) !?auth.AuthCheck {
        try self.io.checkCancel();
        if (credential) |value| if (value == .oauth) return if (provider.auth.oauth != null) .{ .type = .oauth, .source = "OAuth" } else null;
        const method = provider.auth.apiKey orelse return null;
        const key = if (credential) |value| if (value == .api_key) value.api_key else null else null;
        if (method.check) |check| {
            const result = check(method.context, a, self.io, self.authContext, key) catch |err| {
                try resolve.remember(a, error_message, err, "API key auth check failed for provider {s}", .{provider.id});
                return err;
            };
            try self.io.checkCancel();
            return result;
        }
        const resolved = try resolve.resolveProviderAuth(a, self.io, &self.authResolutions, provider.id, provider.auth, self.credentials, self.authContext, .{ .errorMessage = error_message }, self.transport);
        try self.io.checkCancel();
        const result = resolved orelse return null;
        return .{ .type = .api_key, .source = result.source };
    }
    pub fn checkAuth(self: *@This(), a: std.mem.Allocator, id: []const u8) !?auth.AuthCheck {
        return self.checkAuthWithError(a, id, null);
    }
    /// A failure detail is allocated with `a`; the original native error is
    /// returned unchanged. Cancellation and allocation failures stay unwrapped.
    pub fn checkAuthWithError(self: *@This(), a: std.mem.Allocator, id: []const u8, error_message: ?*?[]const u8) !?auth.AuthCheck {
        if (error_message) |slot| slot.* = null;
        try self.io.checkCancel();
        const provider = (try self.getProvider(id)) orelse return null;
        return self.checkProviderAuth(a, provider, try self.readCredential(a, id, error_message), error_message);
    }
    const AvailabilityCheck = struct {
        owner: *Models,
        provider: Provider,
        arena: std.heap.ArenaAllocator,
        index: usize,
        credential: ?auth.Credential = null,
        authenticated: bool = false,
        errorMessage: ?[]const u8 = null,
        const Outcome = struct { index: usize, result: anyerror!?auth.AuthCheck };
        fn check(self: *@This()) !?auth.AuthCheck {
            self.credential = try self.owner.readCredential(self.arena.allocator(), self.provider.id, &self.errorMessage);
            try self.owner.io.checkCancel();
            return self.owner.checkProviderAuth(self.arena.allocator(), self.provider, self.credential, &self.errorMessage);
        }
        fn run(self: *@This()) Outcome {
            return .{ .index = self.index, .result = self.check() };
        }
    };
    fn available(self: *@This(), a: std.mem.Allocator, provider_id: ?[]const u8, chats_only: bool, error_message: ?*?[]const u8) ![]const t.Model {
        if (error_message) |slot| slot.* = null;
        try self.io.checkCancel();
        var backing = @import("utils/synchronized_allocator.zig").SynchronizedAllocator{ .child = a, .io = self.io };
        var temporary = std.heap.ArenaAllocator.init(backing.allocator());
        defer temporary.deinit();
        const scratch = temporary.allocator();
        const providers = try self.getProviders(scratch);
        var out: std.ArrayList(t.Model) = .empty;
        errdefer out.deinit(a);
        var checks: std.ArrayList(AvailabilityCheck) = .empty;
        defer for (checks.items) |*check| check.arena.deinit();
        for (providers) |provider| {
            if (provider_id) |id| if (id.len > 0 and !std.mem.eql(u8, id, provider.id)) continue;
            try checks.append(scratch, .{ .owner = self, .provider = provider, .arena = .init(backing.allocator()), .index = checks.items.len });
        }
        // The pinned implementation starts all provider checks together. Each
        // task has its own arena; cancellation joins before any arena is freed.
        const Outcome = union(enum) { checked: AvailabilityCheck.Outcome };
        const buffer = try scratch.alloc(Outcome, checks.items.len);
        var select = std.Io.Select(Outcome).init(self.io, buffer);
        defer select.cancelDiscard();
        for (checks.items) |*check| try select.concurrent(.checked, AvailabilityCheck.run, .{check});
        for (0..checks.items.len) |_| switch (try select.await()) {
            .checked => |finished| {
                const result = finished.result catch |err| {
                    // Stop all allocator users before copying a task-owned
                    // diagnostic into the caller's surviving allocation region.
                    select.cancelDiscard();
                    if (err != error.Canceled and err != error.OutOfMemory) if (error_message) |slot| if (checks.items[finished.index].errorMessage) |message| {
                        slot.* = try a.dupe(u8, message);
                    };
                    return err;
                };
                checks.items[finished.index].authenticated = result != null;
            },
        };
        for (checks.items) |*check| {
            if (!check.authenticated) continue;
            const provider = check.provider;
            const credential = check.credential;
            if (chats_only) {
                const chats = try provider.chatModels(scratch, self.io);
                const selected = if (provider.filterModels) |filter| try filter(provider.context, scratch, chats, credential) else chats;
                for (selected) |model| try out.append(a, try t.cloneInto(t.Model, a, model));
                continue;
            }
            const all = try provider.allModels(scratch, self.io);
            if (provider.filterAllModels) |filter| {
                for (try filter(provider.context, scratch, all, credential)) |model| try out.append(a, try t.cloneInto(t.Model, a, model));
            } else if (provider.filterModels) |filter| {
                const chats = try provider.chatModels(scratch, self.io);
                const selected = try filter(provider.context, scratch, chats, credential);
                for (all) |model| {
                    if (model.modelType() != .chat) {
                        try out.append(a, try t.cloneInto(t.Model, a, model));
                        continue;
                    }
                    for (selected) |chat| if (std.mem.eql(u8, model.id, chat.id)) {
                        try out.append(a, try t.cloneInto(t.Model, a, model));
                        break;
                    };
                }
            } else for (all) |model| try out.append(a, try t.cloneInto(t.Model, a, model));
        }
        return out.toOwnedSlice(a);
    }
    pub fn getAvailable(self: *@This(), a: std.mem.Allocator, provider_id: ?[]const u8) ![]const t.Model {
        return self.getAvailableWithError(a, provider_id, null);
    }
    /// Auth failure details are copied into `a`, after concurrent checks join.
    pub fn getAvailableWithError(self: *@This(), a: std.mem.Allocator, provider_id: ?[]const u8, error_message: ?*?[]const u8) ![]const t.Model {
        return self.available(a, provider_id, true, error_message);
    }
    pub fn getAllAvailable(self: *@This(), a: std.mem.Allocator, provider_id: ?[]const u8) ![]const t.Model {
        return self.getAllAvailableWithError(a, provider_id, null);
    }
    pub fn getAllAvailableWithError(self: *@This(), a: std.mem.Allocator, provider_id: ?[]const u8, error_message: ?*?[]const u8) ![]const t.Model {
        return self.available(a, provider_id, false, error_message);
    }
    pub const getAllModels = getAll;
    pub fn getAvailableOfType(self: *@This(), a: std.mem.Allocator, kind: t.ModelType, provider_id: ?[]const u8) ![]const t.Model {
        return self.getAvailableOfTypeWithError(a, kind, provider_id, null);
    }
    pub fn getAvailableOfTypeWithError(self: *@This(), a: std.mem.Allocator, kind: t.ModelType, provider_id: ?[]const u8, error_message: ?*?[]const u8) ![]const t.Model {
        const all = try self.getAllAvailableWithError(a, provider_id, error_message);
        defer a.free(all);
        var out: std.ArrayList(t.Model) = .empty;
        errdefer out.deinit(a);
        for (all) |model| if (model.modelType() == kind) {
            try out.append(a, model);
        };
        return out.toOwnedSlice(a);
    }
    pub fn login(self: *@This(), a: std.mem.Allocator, id: []const u8, kind: auth.AuthType, interaction: auth.Interaction, options: auth.LoginOptions) !auth.Credential {
        if (interaction.errorMessage) |slot| slot.* = null;
        try self.io.checkCancel();
        const provider = (try self.getProvider(id)) orelse {
            if (interaction.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "Unknown provider: {s}", .{id});
            return error.UnknownProvider;
        };
        const supported = switch (kind) {
            .oauth => provider.auth.oauth != null,
            .api_key => if (provider.auth.apiKey) |method| method.login != null else false,
        };
        if (!supported) {
            if (interaction.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "{s} does not support {s} login", .{ provider.name, @tagName(kind) });
            return error.UnsupportedLogin;
        }
        var login_options = options;
        if (kind == .oauth) {
            var env: t.ProviderEnv = .{};
            for (options.env.keys(), options.env.values()) |key, text| try env.put(a, key, text);
            for ([_][]const u8{ "PI_OAUTH_CALLBACK_HOST", "KIMI_CODE_OAUTH_HOST", "KIMI_OAUTH_HOST" }) |name| {
                if (try @import("utils/provider_env.zig").getProviderEnvValue(a, self.io, self.authContext, name, options.env)) |text| try env.put(a, name, text);
            }
            login_options.env = env;
        }
        const credential: auth.Credential = switch (kind) {
            .api_key => blk: {
                const method = provider.auth.apiKey orelse return error.UnsupportedLogin;
                const call = method.login orelse return error.UnsupportedLogin;
                break :blk .{ .api_key = try call(method.context, a, self.io, interaction) };
            },
            .oauth => blk: {
                const method = provider.auth.oauth orelse return error.UnsupportedLogin;
                break :blk .{ .oauth = try method.login(method.context, a, self.io, self.transport, interaction, login_options) };
            },
        };
        if (interaction.errorMessage) |slot| slot.* = null;
        var retained = credential;
        _ = self.credentials.modify(a, self.io, id, .{ .context = &retained, .completeOnCancel = true, .call = struct {
            fn save(context: ?*anyopaque, _: std.mem.Allocator, _: std.Io, _: ?auth.Credential) !?auth.Credential {
                return @as(*auth.Credential, @ptrCast(@alignCast(context.?))).*;
            }
        }.save }) catch |err| {
            try self.io.checkCancel();
            try resolve.remember(a, interaction.errorMessage, err, "Credential store modify failed for {s}", .{id});
            return err;
        };
        return credential;
    }
    pub fn logout(self: *@This(), id: []const u8) !void {
        return self.logoutWithError(self.allocator, id, null);
    }
    pub fn logoutWithError(self: *@This(), a: std.mem.Allocator, id: []const u8, error_message: ?*?[]const u8) !void {
        if (error_message) |slot| slot.* = null;
        try self.io.checkCancel();
        self.credentials.delete(self.io, id) catch |err| {
            try self.io.checkCancel();
            try resolve.remember(a, error_message, err, "Credential store delete failed for {s}", .{id});
            return err;
        };
    }
    const Prepared = struct { model: t.Model, options: t.StreamOptions, provider: Provider };
    fn prepare(self: *@This(), a: std.mem.Allocator, original: t.Model, original_options: t.StreamOptions, transforms: ModelsRequestTransforms) !Prepared {
        return self.prepareWithError(a, original, original_options, transforms, null);
    }
    fn prepareWithError(self: *@This(), a: std.mem.Allocator, original: t.Model, original_options: t.StreamOptions, transforms: ModelsRequestTransforms, error_message: ?*?[]const u8) !Prepared {
        const provider = (try self.getProvider(original.provider)) orelse return error.UnknownProvider;
        var auth_overrides = resolve.AuthResolutionOverrides{ .apiKey = original_options.apiKey, .env = original_options.env, .errorMessage = error_message };
        if (@import("json/value.zig").fieldPresent(original_options, "apiKey")) @import("json/value.zig").markPresent(&auth_overrides, "apiKey");
        const resolved = (try self.getAuth(a, original, auth_overrides)) orelse return error.MissingAuthentication;
        var model = original;
        var options = original_options;
        if (resolved.auth.baseUrl) |url| if (url.len > 0) {
            model.baseUrl = url;
        };
        options.apiKey = original_options.apiKey orelse resolved.auth.apiKey;
        options.env = .{};
        for (resolved.env.keys(), resolved.env.values()) |key, value| try options.env.put(a, key, value);
        for (original_options.env.keys(), original_options.env.values()) |key, value| try options.env.put(a, key, value);
        options.headers = try headers.overlay(a, &.{ resolved.auth.headers, original_options.headers });
        if (transforms.transformHeaders) |transform| options.headers = try transform(transforms.context, a, self.io, options.headers);
        return .{ .model = model, .options = options, .provider = provider };
    }
    fn prepareForResult(self: *@This(), a: std.mem.Allocator, result_allocator: std.mem.Allocator, model: t.Model, options: t.StreamOptions, transforms: ModelsRequestTransforms, error_message: *?[]const u8) !Prepared {
        var detail: ?[]const u8 = null;
        return self.prepareWithError(a, model, options, transforms, &detail) catch |err| {
            if (detail) |message| error_message.* = try result_allocator.dupe(u8, message);
            return err;
        };
    }
    const StreamRequest = struct { owner: *Models, handle: *events.AssistantMessageEventStream, model: t.Model, context: t.Context, options: t.StreamOptions, transforms: ModelsRequestTransforms, simple: bool, deferred: ?t.DeferredHandle = null, wait: ?f64 = null, errorMessage: ?[]const u8 = null };
    fn runStream(request: *StreamRequest) void {
        streamSetup(request) catch |err| {
            const io = request.owner.io;
            const protection = io.swapCancelProtection(.blocked);
            defer _ = io.swapCancelProtection(protection);
            var message = request.handle.latestPartial() orelse t.AssistantMessage{ .api = request.model.api, .provider = request.model.provider, .model = request.model.id, .timestamp = @floatFromInt(request.handle.started_wall) };
            message.stopReason = if (err == error.Canceled) .aborted else .@"error";
            message.errorMessage = request.errorMessage orelse (setupErrorMessage(request.handle.arena.allocator(), request.model, err) catch @errorName(err));
            request.handle.push(.{ .@"error" = .{ .reason = message.stopReason, .@"error" = message } }) catch request.handle.fail(err);
            if (err == error.Canceled) io.recancel();
        };
    }
    fn streamSetup(request: *StreamRequest) !void {
        if (request.model.modelType() != .chat) return error.ExpectedChatModel;
        const self = request.owner;
        const a = request.handle.arena.allocator();
        if (request.deferred != null) {
            const provider = (try self.getProvider(request.model.provider)) orelse return error.UnknownProvider;
            if (!provider.supportsOperation("fetchDeferred")) return error.UnsupportedDeferredOperation;
        }
        var prepared = try self.prepareWithError(a, request.model, request.options, request.transforms, &request.errorMessage);
        const implementation = prepared.provider.chatImplementation(request.model.api) orelse return if (request.deferred != null) error.UnsupportedDeferredApi else error.UnsupportedApi;
        var provider_detail: ?[]const u8 = null;
        var provider_storage = t.ErrorMessageStorage{ .allocator = request.handle.allocator };
        defer provider_storage.deinit();
        var operation = self.makeOperation(request.handle.allocator);
        operation.errorMessage = &provider_detail;
        operation.errorStorage = &provider_storage;
        prepared.options.errorMessage = &provider_detail;
        const inner = (if (request.deferred) |handle| blk: {
            const call = implementation.fetchDeferred orelse return error.UnsupportedDeferredApi;
            break :blk call(implementation.context, operation, prepared.model, handle, .{ .request = prepared.options, .wait = request.wait });
        } else blk: {
            const call = (if (request.simple) implementation.streamSimple else implementation.stream) orelse return error.UnsupportedOperation;
            break :blk call(implementation.context, operation, prepared.model, try @import("utils/transcript.zig").normalizeContext(a, request.context), prepared.options);
        }) catch |err| {
            if (provider_detail) |message| request.errorMessage = try a.dupe(u8, message);
            return err;
        };
        defer inner.destroy();
        // Preserve a borrowed diagnostic before destroying a stream that may
        // own it. Only provider_storage's explicit copies are freed here.
        errdefer {
            inner.cancel();
            if (provider_detail) |message| request.errorMessage = a.dupe(u8, message) catch "OutOfMemory";
        }
        while (try inner.next()) |event| try request.handle.push(event);
        try request.handle.end(try inner.result());
    }
    fn startStream(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions, transforms: ModelsRequestTransforms, simple: bool, deferred: ?t.DeferredHandle, wait: ?f64) !*events.AssistantMessageEventStream {
        const handle = try events.AssistantMessageEventStream.create(a, self.io);
        errdefer handle.destroy();
        try handle.prepareModel(model);
        const alloc = handle.arena.allocator();
        const request = try alloc.create(StreamRequest);
        request.* = .{ .owner = self, .handle = handle, .model = try t.cloneInto(t.Model, alloc, model), .context = try t.cloneInto(t.Context, alloc, context), .options = try @import("api/driver.zig").cloneOptions(alloc, options), .transforms = transforms, .simple = simple, .deferred = if (deferred) |handle_value| try t.cloneInto(t.DeferredHandle, alloc, handle_value) else null, .wait = wait };
        handle.producer = try self.io.concurrent(runStream, .{request});
        return handle;
    }
    pub fn stream(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions, transforms: ModelsRequestTransforms) !*events.AssistantMessageEventStream {
        return self.startStream(a, model, context, options, transforms, false, null, null);
    }
    pub fn streamSimple(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions, transforms: ModelsRequestTransforms) !*events.AssistantMessageEventStream {
        return self.startStream(a, model, context, options, transforms, true, null, null);
    }
    fn completeResolved(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions, transforms: ModelsRequestTransforms, simple: bool, error_message: *?[]const u8) !t.Owned(t.AssistantMessage) {
        if (model.modelType() != .chat) return error.ExpectedChatModel;
        var arena = std.heap.ArenaAllocator.init(a);
        defer arena.deinit();
        const alloc = arena.allocator();
        var prepared = try self.prepareForResult(alloc, a, model, options, transforms, error_message);
        const impl = prepared.provider.chatImplementation(model.api) orelse return error.UnsupportedApi;
        var provider_detail: ?[]const u8 = null;
        var provider_storage = t.ErrorMessageStorage{ .allocator = a };
        defer provider_storage.deinit();
        var operation = self.makeOperation(a);
        operation.errorMessage = &provider_detail;
        operation.errorStorage = &provider_storage;
        prepared.options.errorMessage = &provider_detail;
        const normalized = try @import("utils/transcript.zig").normalizeContext(alloc, context);
        const direct = if (simple) impl.completeSimple else impl.complete;
        if (direct) |call| return call(impl.context, operation, prepared.model, normalized, prepared.options) catch |err| {
            if (provider_detail) |message| error_message.* = try a.dupe(u8, message);
            return err;
        };
        const call = (if (simple) impl.streamSimple else impl.stream) orelse return error.UnsupportedOperation;
        const handle = call(impl.context, operation, prepared.model, normalized, prepared.options) catch |err| {
            if (provider_detail) |message| error_message.* = try a.dupe(u8, message);
            return err;
        };
        defer handle.destroy();
        const final = handle.result() catch |err| {
            handle.cancel();
            if (provider_detail) |message| error_message.* = try a.dupe(u8, message);
            return err;
        };
        return t.Owned(t.AssistantMessage).clone(a, final);
    }
    fn completeResult(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions, transforms: ModelsRequestTransforms, simple: bool) !t.Owned(t.AssistantMessage) {
        const timestamp = std.Io.Clock.real.now(self.io).toMilliseconds();
        // completeResolved copies borrowed provider/auth details into `a`.
        var detail: ?[]const u8 = null;
        defer if (detail) |message| a.free(message);
        return self.completeResolved(a, model, context, options, transforms, simple, &detail) catch |err| {
            if (err == error.OutOfMemory) return err;
            var arena = std.heap.ArenaAllocator.init(a);
            defer arena.deinit();
            const message = t.AssistantMessage{ .api = model.api, .provider = model.provider, .model = model.id, .timestamp = @floatFromInt(timestamp), .stopReason = if (err == error.Canceled) .aborted else .@"error", .errorMessage = detail orelse try setupErrorMessage(arena.allocator(), model, err) };
            if (err == error.Canceled) self.io.recancel();
            return t.Owned(t.AssistantMessage).clone(a, message);
        };
    }
    pub fn complete(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions, transforms: ModelsRequestTransforms) !t.Owned(t.AssistantMessage) {
        return self.completeResult(a, model, context, options, transforms, false);
    }
    pub fn completeSimple(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.Context, options: t.StreamOptions, transforms: ModelsRequestTransforms) !t.Owned(t.AssistantMessage) {
        return self.completeResult(a, model, context, options, transforms, true);
    }
    fn generateImagesResolved(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.ImagesContext, options: t.ImagesOptions, transforms: ModelsRequestTransforms, error_message: *?[]const u8) !t.AssistantImages {
        if (model.modelType() != .image) return error.ExpectedImageModel;
        const provider = (try self.getProvider(model.provider)) orelse return error.UnknownProvider;
        if (!provider.supportsOperation("generateImages")) return error.UnsupportedImageOperation;
        var arena = std.heap.ArenaAllocator.init(a);
        defer arena.deinit();
        var prepared = try self.prepareForResult(arena.allocator(), a, model, options, transforms, error_message);
        const impl = prepared.provider.implementation(model.api) orelse return error.UnsupportedImageApi;
        const call = impl.generateImages orelse return error.UnsupportedImageApi;
        var provider_detail: ?[]const u8 = null;
        var provider_storage = t.ErrorMessageStorage{ .allocator = arena.allocator() };
        defer provider_storage.deinit();
        var operation = self.makeOperation(arena.allocator());
        operation.errorMessage = &provider_detail;
        operation.errorStorage = &provider_storage;
        prepared.options.errorMessage = &provider_detail;
        const result = call(impl.context, operation, prepared.model, context, prepared.options) catch |err| {
            if (provider_detail) |message| error_message.* = try a.dupe(u8, message);
            return err;
        };
        return t.cloneInto(t.AssistantImages, a, result);
    }
    pub fn generateImages(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.ImagesContext, options: t.ImagesOptions, transforms: ModelsRequestTransforms) !t.AssistantImages {
        var detail: ?[]const u8 = null;
        return self.generateImagesResolved(a, model, context, options, transforms, &detail) catch |err| {
            if (err == error.OutOfMemory) return err;
            const result = t.AssistantImages{ .api = try a.dupe(u8, model.api), .provider = try a.dupe(u8, model.provider), .model = try a.dupe(u8, model.id), .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds())), .stopReason = if (err == error.Canceled) .aborted else .@"error", .errorMessage = detail orelse try setupErrorMessage(a, model, err) };
            if (err == error.Canceled) self.io.recancel();
            return result;
        };
    }
    fn classifyResolved(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions, transforms: ModelsRequestTransforms, error_message: *?[]const u8) !t.ClassifierResult {
        if (model.modelType() != .classifier) return error.ExpectedClassifierModel;
        try @import("utils/model_operations.zig").assertClassifierInputSupported(model, context);
        const provider = (try self.getProvider(model.provider)) orelse return error.UnknownProvider;
        if (!provider.supportsOperation("classify")) return error.UnsupportedClassifierOperation;
        var arena = std.heap.ArenaAllocator.init(a);
        defer arena.deinit();
        var prepared = try self.prepareForResult(arena.allocator(), a, model, options, transforms, error_message);
        const impl = prepared.provider.implementation(model.api) orelse return error.UnsupportedClassifierApi;
        const call = impl.classify orelse return error.UnsupportedClassifierApi;
        var provider_detail: ?[]const u8 = null;
        var provider_storage = t.ErrorMessageStorage{ .allocator = arena.allocator() };
        defer provider_storage.deinit();
        var operation = self.makeOperation(arena.allocator());
        operation.errorMessage = &provider_detail;
        operation.errorStorage = &provider_storage;
        prepared.options.errorMessage = &provider_detail;
        const result = call(impl.context, operation, prepared.model, context, prepared.options) catch |err| {
            if (provider_detail) |message| error_message.* = try a.dupe(u8, message);
            return err;
        };
        return t.cloneInto(t.ClassifierResult, a, result);
    }
    pub fn classify(self: *@This(), a: std.mem.Allocator, model: t.Model, context: t.ClassifierContext, options: t.ClassifierOptions, transforms: ModelsRequestTransforms) !t.ClassifierResult {
        var detail: ?[]const u8 = null;
        return self.classifyResolved(a, model, context, options, transforms, &detail) catch |err| {
            if (err == error.OutOfMemory) return err;
            const result = t.ClassifierResult{ .api = try a.dupe(u8, model.api), .provider = try a.dupe(u8, model.provider), .model = try a.dupe(u8, model.id), .answers = @import("json/value.zig").map(a), .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds())), .stopReason = if (err == error.Canceled) .aborted else .@"error", .errorMessage = detail orelse try setupErrorMessage(a, model, err) };
            if (err == error.Canceled) self.io.recancel();
            return result;
        };
    }
    pub fn streamDeferred(self: *@This(), a: std.mem.Allocator, model: t.Model, handle: t.DeferredHandle, options: t.DeferredFetchOptions, transforms: ModelsRequestTransforms) !*events.AssistantMessageEventStream {
        return self.startStream(a, model, .{ .messages = &.{} }, options.request, transforms, false, handle, options.wait);
    }
    pub fn fetchDeferred(self: *@This(), a: std.mem.Allocator, model: t.Model, handle: t.DeferredHandle, options: t.DeferredFetchOptions, transforms: ModelsRequestTransforms) !t.Owned(t.AssistantMessage) {
        const stream_handle = try self.streamDeferred(a, model, handle, options, transforms);
        defer stream_handle.destroy();
        return t.Owned(t.AssistantMessage).clone(a, try stream_handle.result());
    }
    pub fn cancelDeferred(self: *@This(), a: std.mem.Allocator, model: t.Model, handle: t.DeferredHandle, options: t.DeferredCancelOptions, transforms: ModelsRequestTransforms) !void {
        if (options.errorMessage) |slot| slot.* = null;
        return self.cancelDeferredInner(a, model, handle, options, transforms) catch |err| {
            if (err != error.Canceled and err != error.OutOfMemory) {
                if (options.errorMessage) |slot| if (slot.* == null) {
                    slot.* = setupErrorMessage(a, model, err) catch @errorName(err);
                };
            }
            return err;
        };
    }
    fn cancelDeferredInner(self: *@This(), a: std.mem.Allocator, model: t.Model, handle: t.DeferredHandle, options: t.DeferredCancelOptions, transforms: ModelsRequestTransforms) !void {
        if (model.modelType() != .chat) return error.ExpectedChatModel;
        const provider = (try self.getProvider(model.provider)) orelse return error.UnknownProvider;
        if (!provider.supportsOperation("cancelDeferred")) return error.UnsupportedDeferredOperation;
        const prepared = try self.prepareWithError(a, model, options, transforms, options.errorMessage);
        const impl = prepared.provider.chatImplementation(model.api) orelse return error.UnsupportedDeferredCancelApi;
        const call = impl.cancelDeferred orelse return error.UnsupportedDeferredCancelApi;
        var operation = self.makeOperation(a);
        operation.errorMessage = options.errorMessage;
        return call(impl.context, operation, prepared.model, handle, prepared.options);
    }
    fn publish(self: *@This(), id: []const u8, generation: u64, publication: ModelsPublication) !bool {
        try self.io.checkCancel();
        const publication_lock = blk: {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            const current = self.providers.get(id) orelse return false;
            if (current.generation != generation) return false;
            if (self.publicationLocks.get(id)) |value| break :blk value;
            const a = self.arena.allocator();
            const value = try a.create(std.Io.Mutex);
            value.* = .init;
            try self.publicationLocks.put(self.allocator, try a.dupe(u8, id), value);
            break :blk value;
        };
        try publication_lock.lock(self.io);
        defer publication_lock.unlock(self.io);
        const models = blk: {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            const current = self.providers.get(id) orelse return false;
            if (current.generation != generation) return false;
            break :blk if (publication.models) |values| try t.cloneInto([]const t.Model, self.arena.allocator(), values) else null;
        };
        var temporary = std.heap.ArenaAllocator.init(self.allocator);
        defer temporary.deinit();
        switch (publication.persist) {
            .unchanged => {},
            .delete => try self.store.delete(self.io, id),
            .write => |value| try self.store.write(self.io, id, try t.cloneInto(ms.ModelsStoreEntry, temporary.allocator(), value)),
        }
        try self.io.checkCancel();
        {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            const current = self.providers.getPtr(id) orelse return false;
            if (current.generation != generation) return false;
            if (models) |values| current.provider.models = values;
        }
        // The generation-checked snapshot commit above is the publication
        // point. Keep publications for this provider ordered, but release the
        // collection mutex before entering caller code so lookups can reenter.
        // Old provider contexts/snapshots remain retained through teardown.
        if (publication.update) |update| update(publication.updateContext);
        return true;
    }
    fn refreshPhase(r: Registration, context: RefreshModelsContext) !void {
        if (r.provider.refreshModels) |call| return call(r.provider.context, context);
        const fetch = r.provider.fetchModels orelse return;
        const a = context.operation.allocator;
        if (context.stored) |stored| {
            var restored: std.ArrayList(t.Model) = .empty;
            for (stored.models) |model| if (std.mem.eql(u8, model.provider, r.provider.id)) {
                try restored.append(a, model);
            };
            if (!try context.publish(.{ .models = try mergeCatalog(a, r.baseline, restored.items) })) return;
        }
        if (!context.allowNetwork) return;
        try context.operation.io.checkCancel();
        const raw_fetched = try fetch(r.provider.context, context);
        try context.operation.io.checkCancel();
        const fetched = try knownModels(a, raw_fetched);
        _ = try context.publish(.{ .models = try mergeCatalog(a, r.baseline, fetched), .persist = .{ .write = .{ .models = fetched, .checkedAt = @as(f64, @floatFromInt(std.Io.Clock.real.now(context.operation.io).toMilliseconds())) } } });
    }
    fn readKnownStore(self: *@This(), a: std.mem.Allocator, id: []const u8) !?ms.ModelsStoreEntry {
        const stored = try self.store.read(a, self.io, id);
        try self.io.checkCancel();
        const entry = stored orelse return null;
        var owned = try t.cloneInto(ms.ModelsStoreEntry, a, entry);
        owned.models = try knownModels(a, owned.models);
        try self.io.checkCancel();
        return owned;
    }
    fn refreshOne(self: *@This(), a: std.mem.Allocator, r: Registration, options: ModelsRefreshOptions, error_message: ?*?[]const u8) !void {
        var credential_error: ?anyerror = null;
        var credential_message: ?[]const u8 = null;
        const initial = self.readCredential(a, r.provider.id, &credential_message) catch |err| blk: {
            credential_error = err;
            break :blk null;
        };
        var context = RefreshModelsContext{ .operation = self.makeOperation(a), .stored = try self.readKnownStore(a, r.provider.id), .credential = initial, .auth = null, .allowNetwork = false, .force = null, .baseUrl = r.provider.baseUrl, .owner = self, .providerId = r.provider.id, .generation = r.generation };
        context.operation.errorMessage = error_message;
        try refreshPhase(r, context);
        if (credential_error) |err| {
            if (error_message) |slot| slot.* = credential_message;
            return err;
        }
        if (!options.allowNetwork) return;
        try self.io.checkCancel();
        if (initial) |value| {
            if (value == .oauth) {
                const oauth = r.provider.auth.oauth orelse return;
                context.credential = if (std.Io.Clock.real.now(self.io).toMilliseconds() >= value.oauth.expires) try resolve.refreshStoredOAuthCredentialWithError(a, self.io, r.provider.id, oauth, self.credentials, self.transport, 0, self.authContext, error_message) else value;
            } else {
                const handler = r.provider.auth.apiKey orelse return;
                const result = (try handler.resolve(handler.context, a, self.io, self.authContext, value.api_key)) orelse return;
                context.credential = .{ .api_key = .{ .key = result.auth.apiKey, .env = try @import("auth/helpers.zig").envValue(a, result.env) } };
            }
        } else {
            const handler = r.provider.auth.apiKey orelse return;
            const result = (try handler.resolve(handler.context, a, self.io, self.authContext, null)) orelse return;
            var env = @import("json/value.zig").map(a);
            for (result.env.keys(), result.env.values()) |key, value| try @import("json/value.zig").put(a, &env, key, .{ .string = value });
            context.credential = .{ .api_key = .{ .key = result.auth.apiKey, .env = env } };
        }
        if (context.credential == null) return;
        context.stored = try self.readKnownStore(a, r.provider.id);
        context.allowNetwork = true;
        context.force = options.force;
        try refreshPhase(r, context);
    }
    fn reapRefreshJobs(self: *@This()) void {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        var index: usize = 0;
        while (index < self.refreshJobs.items.len) {
            const job = self.refreshJobs.items[index];
            if (job.done.isSet() and job.callerGone.load(.acquire)) {
                _ = self.refreshJobs.swapRemove(index);
                job.future.await(self.io);
                job.destroy();
            } else index += 1;
        }
    }
    pub fn refresh(self: *@This(), a: std.mem.Allocator, options: ModelsRefreshOptions) !ModelsRefreshResult {
        self.io.checkCancel() catch |err| {
            if (err == error.Canceled) {
                self.io.recancel();
                return .{ .aborted = true, .errors = &.{} };
            }
            return err;
        };
        self.reapRefreshJobs();
        var jobs: std.ArrayList(*RefreshJob) = .empty;
        defer {
            self.mutex.lockUncancelable(self.io);
            for (jobs.items) |job| {
                job.canceled.set(self.io);
                if (self.providers.getPtr(job.registration.provider.id)) |registration| if (registration.refreshJob == job) {
                    if (!job.done.isSet()) {
                        self.generation += 1;
                        registration.generation = self.generation;
                    }
                    registration.refreshJob = null;
                };
                job.callerGone.store(true, .release);
            }
            self.mutex.unlock(self.io);
            jobs.deinit(a);
            self.reapRefreshJobs();
        }
        {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            for (self.providers.values()) |*registration| {
                if (registration.provider.refreshModels == null and registration.provider.fetchModels == null) continue;
                if (options.providers) |ids| {
                    const selected = for (ids) |id| {
                        if (std.mem.eql(u8, id, registration.provider.id)) break true;
                    } else false;
                    if (!selected) continue;
                }
                try jobs.ensureUnusedCapacity(a, 1);
                try self.refreshJobs.ensureUnusedCapacity(self.allocator, 1);
                const job = try self.allocator.create(RefreshJob);
                job.* = .{ .owner = self, .arena = .init(self.allocator), .registration = registration.*, .options = .{ .allowNetwork = options.allowNetwork, .force = options.force } };
                errdefer job.destroy();
                const previous = registration.refreshJob;
                const previous_generation = registration.generation;
                self.generation += 1;
                registration.generation = self.generation;
                errdefer {
                    registration.generation = previous_generation;
                    registration.refreshJob = previous;
                }
                job.registration = registration.*;
                registration.refreshJob = job;
                job.future = try self.io.concurrent(RefreshJob.run, .{job});
                self.refreshJobs.appendAssumeCapacity(job);
                jobs.appendAssumeCapacity(job);
                if (previous) |old| old.supersede();
            }
        }
        var group: std.Io.Group = .init;
        defer group.cancel(self.io);
        for (jobs.items) |job| try group.concurrent(self.io, RefreshJob.wait, .{job});
        var aborted = false;
        group.await(self.io) catch |err| {
            if (err == error.Canceled) {
                aborted = true;
                self.io.recancel();
            } else return err;
        };
        self.io.checkCancel() catch |err| {
            if (err == error.Canceled) {
                aborted = true;
                self.io.recancel();
            } else return err;
        };
        if (aborted) for (jobs.items) |job| {
            job.canceled.set(self.io);
        };
        var completed: std.ArrayList(*RefreshJob) = .empty;
        defer completed.deinit(a);
        for (jobs.items) |job| if (job.done.isSet() and job.failureOrder != std.math.maxInt(u64)) {
            try completed.append(a, job);
        };
        std.mem.sort(*RefreshJob, completed.items, {}, struct {
            fn less(_: void, left: *RefreshJob, right: *RefreshJob) bool {
                return left.failureOrder < right.failureOrder;
            }
        }.less);
        var errors: std.ArrayList(RefreshError) = .empty;
        for (completed.items) |job| if (job.failure) |err| {
            if (aborted and err == error.Canceled) continue;
            try errors.append(a, .{ .provider = job.registration.provider.id, .err = err, .message = if (job.errorMessage) |message| try a.dupe(u8, message) else null });
        };
        return .{ .aborted = aborted, .errors = try errors.toOwnedSlice(a) };
    }
};
pub const createModels = Models.init;
pub fn hasApi(model: t.Model, api: []const u8) bool {
    return model.modelType() == .chat and std.mem.eql(u8, model.api, api);
}
pub fn getModelType(model: t.Model) t.ModelType {
    return model.modelType();
}
pub fn isModelType(model: t.Model, kind: t.ModelType) bool {
    return model.modelType() == kind;
}
pub fn modelsAreEqual(left: ?t.Model, right: ?t.Model) bool {
    const a = left orelse return false;
    const b = right orelse return false;
    return a.modelType() == b.modelType() and std.mem.eql(u8, a.id, b.id) and std.mem.eql(u8, a.provider, b.provider);
}
fn setupErrorMessage(a: std.mem.Allocator, model: t.Model, err: anyerror) ![]const u8 {
    return switch (err) {
        error.UnknownProvider => std.fmt.allocPrint(a, "Unknown provider: {s}", .{model.provider}),
        error.MissingAuthentication => std.fmt.allocPrint(a, "Provider is not configured: {s}", .{model.provider}),
        error.ExpectedChatModel => std.fmt.allocPrint(a, "Model {s}/{s} is not a chat model", .{ model.provider, model.id }),
        error.ExpectedImageModel => std.fmt.allocPrint(a, "Model {s}/{s} is not an image model", .{ model.provider, model.id }),
        error.ExpectedClassifierModel => std.fmt.allocPrint(a, "Model {s}/{s} is not a classifier model", .{ model.provider, model.id }),
        error.ClassifierImagesUnsupported => std.fmt.allocPrint(a, "Model {s}/{s} does not accept image input", .{ model.provider, model.id }),
        error.UnsupportedApi => std.fmt.allocPrint(a, "Provider {s} has no API implementation for \"{s}\"", .{ model.provider, model.api }),
        error.UnsupportedDeferredOperation => std.fmt.allocPrint(a, "Provider {s} does not support deferred responses", .{model.provider}),
        error.UnsupportedDeferredApi => std.fmt.allocPrint(a, "Provider {s} does not support deferred responses for \"{s}\"", .{ model.provider, model.api }),
        error.UnsupportedDeferredCancelApi => std.fmt.allocPrint(a, "Provider {s} cannot cancel deferred responses for \"{s}\"", .{ model.provider, model.api }),
        error.UnsupportedImageOperation => std.fmt.allocPrint(a, "Provider {s} does not support image generation", .{model.provider}),
        error.UnsupportedImageApi => std.fmt.allocPrint(a, "Provider {s} has no image generation implementation for \"{s}\"", .{ model.provider, model.api }),
        error.UnsupportedClassifierOperation => std.fmt.allocPrint(a, "Provider {s} does not support classification", .{model.provider}),
        error.UnsupportedClassifierApi => std.fmt.allocPrint(a, "Provider {s} has no classifier implementation for \"{s}\"", .{ model.provider, model.api }),
        else => a.dupe(u8, @errorName(err)),
    };
}
fn mergeCatalog(a: std.mem.Allocator, baseline: []const t.Model, dynamic: []const t.Model) ![]const t.Model {
    var merged: std.ArrayList(t.Model) = .empty;
    try merged.appendSlice(a, baseline);
    for (dynamic) |model| {
        const index = for (merged.items, 0..) |entry, position| {
            if (entry.modelType() == model.modelType() and std.mem.eql(u8, entry.id, model.id)) break position;
        } else null;
        if (index) |position| merged.items[position] = model else try merged.append(a, model);
    }
    return merged.toOwnedSlice(a);
}
fn knownModels(a: std.mem.Allocator, models: []const t.Model) ![]const t.Model {
    var known: std.ArrayList(t.Model) = .empty;
    errdefer known.deinit(a);
    for (models) |model| if (model.hasKnownType()) {
        try known.append(a, model);
    };
    return known.toOwnedSlice(a);
}
