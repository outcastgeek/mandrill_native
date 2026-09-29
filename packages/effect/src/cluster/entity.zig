//! Typed logical entity definitions. An entity group names a handler family;
//! the physical owner and message store remain separate capabilities.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

/// Define a typed clustered entity operation and its handler contract.
/// Parameters:
/// - `RequestCodec: comptime type`: Compile-time codec type for the request payload.
/// - `SuccessCodec: comptime type`: Compile-time codec type for the successful result.
/// - `ErrorCodec: comptime type`: Compile-time codec type for the domain error.
/// Returns (`type`): typed entity operation with stable identity, codecs, and handler contract.
pub fn Entity(comptime RequestCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    comptime {
        for (.{ RequestCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    }
    return struct {
        const Self = @This();
        /// Typed request payload or request record for this operation.
        pub const Request = RequestCodec.Decoded;
        /// Value type carried by the successful channel.
        pub const Success = SuccessCodec.Decoded;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = ErrorCodec.Decoded;

        group: []const u8,
        version: u32,
        request: RequestCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

        /// Construct a statically checked definition.
        /// Parameters:
        /// - `group: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `request: comptime RequestCodec`: Compile-time typed input value; move or clone owning payloads according to
        ///   its type contract.
        /// - `success: comptime SuccessCodec`: Codec or value for the successful result channel.
        /// - `failure: comptime ErrorCodec`: Codec or value for the typed failure channel.
        /// Returns (`Self`): A constructed instance; its provider and allocation ownership follow the containing type.
        pub fn define(comptime group: []const u8, comptime version: u32, comptime request: RequestCodec, comptime success: SuccessCodec, comptime failure: ErrorCodec) Self {
            const definition: Self = .{ .group = group, .version = version, .request = request, .success = success, .failure = failure };
            definition.validate();
            return definition;
        }

        /// Validate the value against its declared contract.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`void`): no value.
        pub fn validate(comptime self: Self) void {
            if (comptime self.group.len == 0 or !std.unicode.utf8ValidateSlice(self.group))
                @compileError("entity group must be stable nonempty UTF-8");
            if (comptime self.version == 0) @compileError("entity definition version must be nonzero");
        }

        /// Attach a typed handler to this definition.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `body: comptime fn (*const Request, *const R) effects.Effect(Success, DomainError, R)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`Handler(Self, R)`): A completed typed handler binding its operation to the selected environment.
        pub fn handledBy(comptime self: Self, comptime R: type, comptime body: fn (*const Request, *const R) effects.Effect(Success, DomainError, R)) Handler(Self, R) {
            self.validate();
            return .{ .definition = self, .body = body };
        }
    };
}

/// Specialize the typed handler signature for cluster operations.
/// Parameters:
/// - `Definition: comptime type`: Compile-time typed definition used to derive this API.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): handler type whose request, result, error, and environment match its entity.
pub fn Handler(comptime Definition: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Completed typed entity definition carried by this handler.
        pub const EntityDefinition = Definition;
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;
        definition: Definition,
        body: *const fn (*const Definition.Request, *const R) effects.Effect(Definition.Success, Definition.DomainError, R),

        /// Invoke the selected typed operation.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select_request: comptime fn (*const R) *const Definition.Request`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(Definition.Success, Definition.DomainError, R)`): typed entity handler result under the supplied environment and scope.
        pub fn invoke(comptime self: Self, comptime select_request: fn (*const R) *const Definition.Request) effects.Effect(Definition.Success, Definition.DomainError, R) {
            self.definition.validate();
            const Program = effects.Effect(Definition.Success, Definition.DomainError, R);
            return Program.deferEffect(struct {
                fn run(env: *const R) Program {
                    return self.body(select_request(env), env);
                }
            }.run);
        }
    };
}

/// Build a closed registry of completed cluster definitions.
/// Parameters:
/// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// Returns (`type`): closed registry of completed entity definitions with checked identities.
pub fn Registry(comptime handlers: anytype) type {
    const Info = @typeInfo(@TypeOf(handlers));
    if (Info != .@"struct" or !Info.@"struct".is_tuple or handlers.len == 0)
        @compileError("entity registry requires a nonempty tuple of handlers");
    inline for (handlers, 0..) |handler, index| {
        const H = @TypeOf(handler);
        if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "EntityDefinition") or
            !@hasDecl(H, "Environment") or !@hasField(H, "definition") or
            H != Handler(H.EntityDefinition, H.Environment))
            @compileError("entity registry members must be completed handlers");
        handler.definition.validate();
        if (H.Environment != @TypeOf(handlers[0]).Environment)
            @compileError("entity registry needs one environment");
        inline for (handlers, 0..) |other, earlier| {
            if (earlier < index and std.mem.eql(u8, handler.definition.group, other.definition.group) and handler.definition.version == other.definition.version)
                @compileError("duplicate entity group and version");
        }
    }
    return struct {
        /// Exact caller environment type required when the description runs.
        pub const Environment = @TypeOf(handlers[0]).Environment;

        /// Resolve a request through the selected capability.
        /// Parameters:
        /// - `group: []const u8`: Stable, nonempty identity component validated at definition time.
        /// - `version: u32`: Stable version used to reject incompatible encoded or replayed data.
        /// Returns (`outcomes.Result(usize, ResolveError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn resolve(group: []const u8, version: u32) outcomes.Result(usize, ResolveError) {
            var supported: ?u32 = null;
            inline for (handlers, 0..) |handler, index| {
                if (std.mem.eql(u8, group, handler.definition.group)) {
                    if (version == handler.definition.version) return .{ .ok = index };
                    if (supported == null) supported = handler.definition.version;
                }
            }
            if (supported) |known| return .{ .err = .{ .incompatible_version = .{ .requested = version, .supported = known } } };
            return .{ .err = .unknown_group };
        }
    };
}

/// Typed failure while resolving an entity definition or operation.
pub const ResolveError = union(enum) {
    unknown_group,
    incompatible_version: struct { requested: u32, supported: u32 },
};
