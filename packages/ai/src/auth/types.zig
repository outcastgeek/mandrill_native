const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
pub const AwsState = @import("aws/state.zig").State;
pub const GoogleState = @import("google/state.zig").State;
pub const ModelAuth = struct { apiKey: ?[]const u8 = null, headers: t.ProviderHeaders = &.{}, baseUrl: ?[]const u8 = null };
pub const AuthResult = struct { auth: ModelAuth, source: ?[]const u8 = null, env: t.ProviderEnv = .{} };
pub const ApiKeyCredential = struct {
    key: ?[]const u8 = null,
    env: ?std.json.Value = null,
    jsonFields: std.json.Value = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const OAuthCredential = struct {
    refresh: []const u8,
    access: []const u8,
    expires: f64,
    metadata: ?std.json.Value = null,
    jsonFields: std.json.Value = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !@This() {
        var configured = options;
        configured.duplicate_field_behavior = .use_last;
        configured.parse_numbers = false;
        var value = try std.json.Value.jsonParse(a, source, configured);
        try j.normalizeParsed(&value);
        return jsonParseFromValue(a, value, options);
    }
    pub fn jsonParseFromValue(a: std.mem.Allocator, value: std.json.Value, options: std.json.ParseOptions) !@This() {
        var result = try j.Record(@This()).jsonParseFromValue(a, value, options);
        result.metadata = try j.clone(a, value);
        return result;
    }
    pub const jsonRecordMetadata = .{"metadata"};
    pub fn jsonExtra(self: @This()) std.json.Value {
        return self.metadata orelse .null;
    }
};
pub const Credential = union(enum) {
    api_key: ApiKeyCredential,
    oauth: OAuthCredential,
    pub const jsonDiscriminator = "type";
    pub fn jsonToValue(self: @This(), a: std.mem.Allocator, options: std.json.Stringify.Options) !std.json.Value {
        return j.taggedToValue(self, "type", a, options);
    }
    pub fn jsonStringify(self: @This(), writer: *std.json.Stringify) !void {
        try j.taggedStringify(self, "type", writer);
    }
    pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !@This() {
        var configured = options;
        configured.duplicate_field_behavior = .use_last;
        configured.parse_numbers = false;
        var value = try std.json.Value.jsonParse(a, source, configured);
        try j.normalizeParsed(&value);
        return jsonParseFromValue(a, value, options);
    }
    pub fn jsonParseFromValue(a: std.mem.Allocator, value: std.json.Value, options: std.json.ParseOptions) !@This() {
        const kind = j.field(value, "type") orelse return error.UnexpectedToken;
        if (std.mem.eql(u8, kind, "api_key")) return .{ .api_key = try std.json.innerParseFromValue(ApiKeyCredential, a, value, options) };
        if (std.mem.eql(u8, kind, "oauth")) {
            var result = try std.json.innerParseFromValue(OAuthCredential, a, value, options);
            result.metadata = try j.clone(a, value);
            return .{ .oauth = result };
        }
        return error.UnknownField;
    }
};
pub const CredentialInfo = struct { providerId: []const u8, type: std.meta.Tag(Credential) };
/// The host retains this interface/context while Models uses it. Results are
/// owned by the supplied allocator; transaction snapshots borrow the callback
/// lifetime. Implementations must not retrieve credentials from filesystem.
pub const CredentialStore = struct {
    context: *anyopaque,
    readFn: *const fn (*anyopaque, std.mem.Allocator, std.Io, []const u8) anyerror!?Credential,
    listFn: *const fn (*anyopaque, std.mem.Allocator, std.Io) anyerror![]const CredentialInfo,
    modifyFn: *const fn (*anyopaque, std.mem.Allocator, std.Io, []const u8, Mutation) anyerror!?Credential,
    deleteFn: *const fn (*anyopaque, std.Io, []const u8) anyerror!void,
    /// completeOnCancel leaves lock acquisition cancelable, then protects
    /// callback execution and persistence together. Failed/no-op mutations
    /// retain the previous record. Implementations release the provider lock
    /// only after the new credential is committed or failure is settled.
    pub const Mutation = struct { context: ?*anyopaque, call: *const fn (?*anyopaque, std.mem.Allocator, std.Io, ?Credential) anyerror!?Credential, completeOnCancel: bool = false };
    pub fn read(self: @This(), a: std.mem.Allocator, io: std.Io, id: []const u8) !?Credential {
        return self.readFn(self.context, a, io, id);
    }
    pub fn list(self: @This(), a: std.mem.Allocator, io: std.Io) ![]const CredentialInfo {
        return self.listFn(self.context, a, io);
    }
    pub fn modify(self: @This(), a: std.mem.Allocator, io: std.Io, id: []const u8, mutation: Mutation) !?Credential {
        return self.modifyFn(self.context, a, io, id, mutation);
    }
    pub fn delete(self: @This(), io: std.Io, id: []const u8) !void {
        return self.deleteFn(self.context, io, id);
    }
};
/// Borrowed, decoded in-memory inputs. The owner retains them for the context
/// lifetime; operations clone retained values. These are documents and tokens,
/// not file paths or callbacks that retrieve credential files. Network exchange,
/// metadata, helper invocation and local signing remain library operations.
pub const CredentialInputs = struct {
    /// Objects keyed by profile/session/service name, respectively.
    awsConfig: ?std.json.Value = null,
    awsCredentials: ?std.json.Value = null,
    awsSsoSessions: ?std.json.Value = null,
    awsServices: ?std.json.Value = null,
    awsSsoTokens: ?std.json.Value = null,
    awsLoginTokens: ?std.json.Value = null,
    awsWebIdentityToken: ?[]const u8 = null,
    awsContainerAuthorizationToken: ?[]const u8 = null,
    googleCredentials: ?std.json.Value = null,
    googleSubjectToken: ?[]const u8 = null,
    googleCertificate: ?[]const u8 = null,
    googleCertificateChain: ?[]const u8 = null,
    googleCertificatePrivateKey: ?[]const u8 = null,
    anthropicIdentityToken: ?[]const u8 = null,
};
pub const AuthContext = struct {
    inputs: CredentialInputs = .{},
    context: ?*anyopaque,
    /// A present environment value is owned by the supplied allocator. The
    /// caller may free it; implementations must clone borrowed snapshots.
    envFn: *const fn (?*anyopaque, std.mem.Allocator, std.Io, []const u8) anyerror!?[]const u8,
    /// Raw process-snapshot values for SDK selectors. Omission falls back to
    /// envFn for existing custom hosts; ownership/cancellation rules are equal.
    rawEnvFn: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, []const u8) anyerror!?[]const u8 = null,
    fileExistsFn: *const fn (?*anyopaque, std.Io, []const u8) anyerror!bool,
    /// Optional context-owned Vertex discovery cache: 0 unknown, 1 absent,
    /// 2 present. Explicit provider-overlay credential paths bypass this cache.
    vertexAdcExistsCache: ?*std.atomic.Value(u8) = null,
    /// Context-owned wall-clock deadline for failed IMDS region discovery.
    /// Zero permits discovery; a failure suppresses new attempts for 60s.
    awsImdsRegionNegativeCache: ?*std.atomic.Value(i64) = null,
    /// CredentialCommandInvalidJson denotes successful execution whose output
    /// failed JSON decoding. Execution failures retain their own errors and
    /// may provide a diagnostic through CredentialCommand.errorMessage.
    credentialCommandFn: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, CredentialCommand) anyerror!std.json.Value = null,
    homeDirFn: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror!?[]const u8 = null,
    pub fn env(self: @This(), a: std.mem.Allocator, io: std.Io, name: []const u8) !?[]const u8 {
        try io.checkCancel();
        const value = try self.envFn(self.context, a, io, name);
        errdefer if (value) |text| a.free(text);
        try io.checkCancel();
        return value;
    }
    pub fn rawEnv(self: @This(), a: std.mem.Allocator, io: std.Io, name: []const u8) !?[]const u8 {
        try io.checkCancel();
        const get = self.rawEnvFn orelse self.envFn;
        const value = try get(self.context, a, io, name);
        errdefer if (value) |text| a.free(text);
        try io.checkCancel();
        return value;
    }
};
/// The host owns executable invocation and its argument/quoting semantics.
/// Credentials cross this boundary as a decoded std.json tree.
pub const CredentialCommand = struct {
    command: []const u8,
    timeoutMs: u64,
    env: t.ProviderEnv = .{},
    maxOutputBytes: ?usize = null,
    /// Remove inherited entries before applying env, matching child-process
    /// entries whose source value was explicitly undefined.
    unsetEnv: []const []const u8 = &.{},
    kind: enum { shell, google_executable } = .shell,
    errorMessage: ?*?[]const u8 = null,
};
pub const AuthType = std.meta.Tag(Credential);
/// Cancellation travels through the operation's explicit std.Io argument.
pub const AuthOperationOptions = std.Io;
pub const AuthInfoLink = struct { url: []const u8, label: ?[]const u8 = null };
pub const AuthInteraction = Interaction;
pub const ProviderAuthInteraction = Interaction;
pub const AuthCheck = struct { type: AuthType, source: ?[]const u8 = null };
pub const ApiKeyAuth = struct {
    name: []const u8,
    context: ?*anyopaque = null,
    login: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, Interaction) anyerror!ApiKeyCredential = null,
    resolve: *const fn (?*anyopaque, std.mem.Allocator, std.Io, AuthContext, ?ApiKeyCredential) anyerror!?AuthResult,
    check: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, AuthContext, ?ApiKeyCredential) anyerror!?AuthCheck = null,
};
pub const PromptOperation = @import("prompt.zig").PromptOperation;
pub const PromptOwner = @import("prompt.zig").PromptOwner;
pub const Interaction = struct {
    context: ?*anyopaque,
    prompt: *const fn (?*anyopaque, std.mem.Allocator, std.Io, std.json.Value) anyerror![]const u8,
    notify: *const fn (?*anyopaque, std.Io, std.json.Value) anyerror!void,
    /// Optional owned operation. The returned waiting reference owns its
    /// lifetime independently of this call's allocator/context/error slot.
    /// Callback races cancel only its waiter, then release this reference.
    startPrompt: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, std.json.Value) anyerror!PromptOperation = null,
    /// Optional caller-owned error slot. Borrowed text follows the operation
    /// allocator's lifetime; error unions still carry the native error code.
    errorMessage: ?*?[]const u8 = null,
};
pub const AuthPrompt = union(enum) {
    text: struct { message: []const u8, placeholder: ?[]const u8 = null },
    secret: struct { message: []const u8, placeholder: ?[]const u8 = null },
    select: struct { message: []const u8, options: []const struct { id: []const u8, label: []const u8, description: ?[]const u8 = null } },
    manual_code: struct { message: []const u8, placeholder: ?[]const u8 = null },
    pub const jsonDiscriminator = "type";
    pub const jsonStringify = j.Tagged(@This(), "type").jsonStringify;
    pub const jsonParse = j.Tagged(@This(), "type").jsonParse;
    pub const jsonParseFromValue = j.Tagged(@This(), "type").jsonParseFromValue;
};
pub const AuthEvent = union(enum) {
    info: struct { message: []const u8, links: ?[]const AuthInfoLink = null },
    auth_url: struct { url: []const u8, instructions: ?[]const u8 = null },
    device_code: struct { userCode: []const u8, verificationUri: []const u8, intervalSeconds: ?f64 = null, expiresInSeconds: ?f64 = null },
    progress: struct { message: []const u8 },
    pub const jsonDiscriminator = "type";
    pub const jsonStringify = j.Tagged(@This(), "type").jsonStringify;
    pub const jsonParse = j.Tagged(@This(), "type").jsonParse;
    pub const jsonParseFromValue = j.Tagged(@This(), "type").jsonParseFromValue;
};
pub const LoginOptions = struct { deviceContext: ?*anyopaque = null, getDeviceId: ?*const fn (?*anyopaque) []const u8 = null, agentName: ?[]const u8 = null, env: t.ProviderEnv = .{} };
pub const OAuthAuth = struct {
    name: []const u8,
    isSubscription: bool = false,
    loginLabel: ?[]const u8 = null,
    context: ?*anyopaque = null,
    login: *const fn (?*anyopaque, std.mem.Allocator, std.Io, @import("../transport/http.zig").Transport, Interaction, LoginOptions) anyerror!OAuthCredential,
    refresh: *const fn (?*anyopaque, std.mem.Allocator, std.Io, @import("../transport/http.zig").Transport, OAuthCredential) anyerror!OAuthCredential,
    refreshWithError: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, @import("../transport/http.zig").Transport, OAuthCredential, ?*?[]const u8) anyerror!OAuthCredential = null,
    /// Native replacement for ambient environment reads during token refresh.
    refreshWithContext: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, @import("../transport/http.zig").Transport, OAuthCredential, ?AuthContext, ?*?[]const u8) anyerror!OAuthCredential = null,
    toAuth: *const fn (?*anyopaque, std.mem.Allocator, std.Io, OAuthCredential) anyerror!ModelAuth,
    pub fn refreshCredential(self: @This(), a: std.mem.Allocator, io: std.Io, network: @import("../transport/http.zig").Transport, credential: OAuthCredential, error_message: ?*?[]const u8) !OAuthCredential {
        return self.refreshCredentialWithContext(a, io, network, credential, null, error_message);
    }
    pub fn refreshCredentialWithContext(self: @This(), a: std.mem.Allocator, io: std.Io, network: @import("../transport/http.zig").Transport, credential: OAuthCredential, context: ?AuthContext, error_message: ?*?[]const u8) !OAuthCredential {
        if (error_message) |slot| slot.* = null;
        if (self.refreshWithContext) |call| return call(self.context, a, io, network, credential, context, error_message);
        if (self.refreshWithError) |call| return call(self.context, a, io, network, credential, error_message);
        return self.refresh(self.context, a, io, network, credential);
    }
};
pub const ProviderAuth = struct { apiKey: ?ApiKeyAuth = null, oauth: ?OAuthAuth = null };
