//! Durable activity definitions pair payload, result and error codecs with
//! one typed Effect body. Replay records the entire Exit for a stable attempt.
const std = @import("std");
const effects = @import("../effect.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

/// Define a typed, versioned workflow activity and handler contract.
/// Parameters:
/// - `PayloadCodec: comptime type`: Compile-time codec type for the payload.
/// - `SuccessCodec: comptime type`: Compile-time codec type for the successful result.
/// - `ErrorCodec: comptime type`: Compile-time codec type for the domain error.
/// Returns (`type`): typed versioned activity definition with stable identity and codecs.
pub fn Activity(comptime PayloadCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    inline for (.{ PayloadCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    return struct {
        const Self = @This();
        /// Typed payload carried by this definition.
        pub const Payload = PayloadCodec.Decoded;
        /// Value type carried by the successful channel.
        pub const Success = SuccessCodec.Decoded;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = ErrorCodec.Decoded;

        name: []const u8,
        version: u32,
        payload: PayloadCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

        /// Construct a statically checked definition.
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `payload: comptime PayloadCodec`: Compile-time typed input value; move or clone owning payloads according to
        ///   its type contract.
        /// - `success: comptime SuccessCodec`: Codec or value for the successful result channel.
        /// - `failure: comptime ErrorCodec`: Codec or value for the typed failure channel.
        /// Returns (`Self`): A constructed instance; its provider and allocation ownership follow the containing type.
        pub fn define(
            comptime name: []const u8,
            comptime version: u32,
            comptime payload: PayloadCodec,
            comptime success: SuccessCodec,
            comptime failure: ErrorCodec,
        ) Self {
            const definition: Self = .{ .name = name, .version = version, .payload = payload, .success = success, .failure = failure };
            definition.validate();
            return definition;
        }

        /// Validate the value against its declared contract.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`void`): no value.
        pub fn validate(comptime self: Self) void {
            if (comptime self.name.len == 0 or !std.unicode.utf8ValidateSlice(self.name))
                @compileError("activity names must be stable nonempty UTF-8");
            if (comptime self.version == 0) @compileError("activity version zero is reserved for absent metadata");
        }

        /// Attach a typed handler to this definition.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `body: comptime fn (*const Payload, *const R) effects.Effect(Success, DomainError, R)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`Handler(Self, R)`): A completed typed handler binding its operation to the selected environment.
        pub fn handledBy(
            comptime self: Self,
            comptime R: type,
            comptime body: fn (*const Payload, *const R) effects.Effect(Success, DomainError, R),
        ) Handler(Self, R) {
            self.validate();
            return .{ .definition = self, .body = body };
        }
    };
}

/// Specialize the typed handler signature for workflow operations.
/// Parameters:
/// - `Definition: comptime type`: Compile-time typed definition used to derive this API.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): handler type matching the activity input, result, error, and environment.
pub fn Handler(comptime Definition: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Completed typed activity definition.
        pub const ActivityDefinition = Definition;
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;
        definition: Definition,
        body: *const fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, Definition.DomainError, R),

        /// Execute the description using its environment.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select_payload: comptime fn (*const R) *const Definition.Payload`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(Definition.Success, Definition.DomainError, R)`): typed activity result from its completed definition and handler.
        pub fn run(comptime self: Self, comptime select_payload: fn (*const R) *const Definition.Payload) effects.Effect(Definition.Success, Definition.DomainError, R) {
            self.definition.validate();
            const Program = effects.Effect(Definition.Success, Definition.DomainError, R);
            return Program.deferEffect(struct {
                fn start(env: *const R) Program {
                    return self.body(select_payload(env), env);
                }
            }.start);
        }
    };
}

/// Build a closed registry of completed workflow definitions.
/// Parameters:
/// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// Returns (`type`): closed registry of completed activity definitions.
pub fn Registry(comptime handlers: anytype) type {
    const Info = @typeInfo(@TypeOf(handlers));
    if (Info != .@"struct" or !Info.@"struct".is_tuple or handlers.len == 0)
        @compileError("activity registry needs a nonempty tuple of completed handlers");
    inline for (handlers, 0..) |handler, index| {
        const H = @TypeOf(handler);
        if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "ActivityDefinition") or
            !@hasDecl(H, "Environment") or !@hasField(H, "definition") or
            H != Handler(H.ActivityDefinition, H.Environment))
            @compileError("activity registry members must be completed handlers");
        handler.definition.validate();
        if (H.Environment != @TypeOf(handlers[0]).Environment)
            @compileError("activity registry needs one environment");
        inline for (handlers, 0..) |other, earlier| {
            if (earlier < index and std.mem.eql(u8, handler.definition.name, other.definition.name) and
                handler.definition.version == other.definition.version)
                @compileError("duplicate activity identity");
        }
    }
    return struct {
        /// Exact caller environment type required when the description runs.
        pub const Environment = @TypeOf(handlers[0]).Environment;

        /// Resolve a request through the selected capability.
        /// Parameters:
        /// - `name: []const u8`: Stable, nonempty identity component validated at definition time.
        /// - `version: u32`: Stable version used to reject incompatible encoded or replayed data.
        /// Returns (`?usize`): matching activity definition, or `null` for an unknown identity.
        pub fn resolve(name: []const u8, version: u32) ?usize {
            inline for (handlers, 0..) |handler, index| {
                if (std.mem.eql(u8, name, handler.definition.name) and version == handler.definition.version)
                    return index;
            }
            return null;
        }
    };
}
