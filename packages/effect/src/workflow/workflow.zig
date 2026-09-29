//! A workflow definition is a versioned description plus an Effect body.
//! The body is registered in process; only its name, versions and encoded
//! payload/outcome cross the engine boundary.
const std = @import("std");
const effects = @import("../effect.zig");
const types = @import("workflow_types.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

/// Define a typed, versioned workflow and its handler contract.
/// Parameters:
/// - `PayloadCodec: comptime type`: Compile-time codec type for the payload.
/// - `SuccessCodec: comptime type`: Compile-time codec type for the successful result.
/// - `ErrorCodec: comptime type`: Compile-time codec type for the domain error.
/// Returns (`type`): typed versioned workflow definition with codecs and handler contract.
pub fn Workflow(comptime PayloadCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    comptime {
        for (.{ PayloadCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    }
    return struct {
        const Self = @This();
        /// Typed payload carried by this definition.
        pub const Payload = PayloadCodec.Decoded;
        /// Value type carried by the successful channel.
        pub const Success = SuccessCodec.Decoded;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = ErrorCodec.Decoded;

        description: types.Definition,
        payload: PayloadCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

        /// Construct a statically checked definition.
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `payload_schema_version: comptime u32`: Compile-time stable version used to reject incompatible
        ///   encoded or replayed data.
        /// - `success_schema_version: comptime u32`: Compile-time stable version used to reject incompatible
        ///   encoded or replayed data.
        /// - `error_schema_version: comptime u32`: Compile-time stable version used to reject incompatible
        ///   encoded or replayed data.
        /// - `payload: comptime PayloadCodec`: Compile-time typed input value; move or clone owning payloads according to
        ///   its type contract.
        /// - `success: comptime SuccessCodec`: Codec or value for the successful result channel.
        /// - `failure: comptime ErrorCodec`: Codec or value for the typed failure channel.
        /// Returns (`Self`): A constructed instance; its provider and allocation ownership follow the containing type.
        pub fn define(
            comptime name: []const u8,
            comptime version: u32,
            comptime payload_schema_version: u32,
            comptime success_schema_version: u32,
            comptime error_schema_version: u32,
            comptime payload: PayloadCodec,
            comptime success: SuccessCodec,
            comptime failure: ErrorCodec,
        ) Self {
            const definition: Self = .{
                .description = .{
                    .name = name,
                    .version = version,
                    .payload_schema_version = payload_schema_version,
                    .success_schema_version = success_schema_version,
                    .error_schema_version = error_schema_version,
                },
                .payload = payload,
                .success = success,
                .failure = failure,
            };
            definition.validate();
            return definition;
        }

        /// Validate the value against its declared contract.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`void`): no value.
        pub fn validate(comptime self: Self) void {
            if (comptime self.description.name.len == 0 or !std.unicode.utf8ValidateSlice(self.description.name))
                @compileError("workflow name must be stable nonempty UTF-8");
            if (comptime self.description.version == 0 or self.description.payload_schema_version == 0 or
                self.description.success_schema_version == 0 or self.description.error_schema_version == 0)
                @compileError("workflow and schema versions must be nonzero");
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

        /// Create a typed handle for the selected definition or operation.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `execution: types.ExecutionId`: Workflow execution identity.
        /// Returns (`Handle(Self)`): A typed handle for the selected definition.
        pub fn handle(comptime self: Self, execution: types.ExecutionId) Handle(Self) {
            self.validate();
            return .{ .description = self.description, .execution = execution };
        }
    };
}

/// The type carries payload, success and domain-error identity through client
/// code; the process-local handler remains separate from the durable handle.
///
/// Parameters:
/// - `Definition: comptime type`: Compile-time typed definition used to derive this API.
/// Returns (`type`): workflow execution handle carrying its typed identity and result channels.
pub fn Handle(comptime Definition: type) type {
    return struct {
        /// Typed payload carried by this definition.
        pub const Payload = Definition.Payload;
        /// Value type carried by the successful channel.
        pub const Success = Definition.Success;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = Definition.DomainError;
        description: types.Definition,
        execution: types.ExecutionId,

        /// Construct the submission for this definition.
        /// Parameters:
        /// - `self: @This()`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `encoded_payload: []const u8`: Encoded payload owned by the caller until copied.
        /// Returns (`types.Submission`): Submission metadata for the selected workflow.
        pub fn submission(self: @This(), encoded_payload: []const u8) types.Submission {
            return .{ .definition = self.description, .execution = self.execution, .encoded_payload = encoded_payload };
        }
    };
}

/// Specialize the typed handler signature for workflow operations.
/// Parameters:
/// - `Definition: comptime type`: Compile-time typed definition used to derive this API.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): handler type whose payload, success, failure, and environment match the workflow.
pub fn Handler(comptime Definition: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Completed typed workflow definition.
        pub const WorkflowDefinition = Definition;
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
        /// Returns (`effects.Effect(Definition.Success, Definition.DomainError, R)`): typed result of invoking this workflow definition with its payload.
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
/// Returns (`type`): closed registry of completed workflow definitions.
pub fn Registry(comptime handlers: anytype) type {
    const Info = @typeInfo(@TypeOf(handlers));
    if (Info != .@"struct" or !Info.@"struct".is_tuple or handlers.len == 0)
        @compileError("workflow registry needs a nonempty tuple of completed handlers");
    inline for (handlers, 0..) |handler, index| {
        const H = @TypeOf(handler);
        if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "WorkflowDefinition") or
            !@hasDecl(H, "Environment") or !@hasField(H, "definition") or
            H != Handler(H.WorkflowDefinition, H.Environment))
            @compileError("workflow registry members must be completed handlers");
        handler.definition.validate();
        if (H.Environment != @TypeOf(handlers[0]).Environment)
            @compileError("workflow registry needs one environment");
        inline for (handlers, 0..) |other, earlier| {
            if (earlier < index and std.mem.eql(u8, handler.definition.description.name, other.definition.description.name) and
                handler.definition.description.version == other.definition.description.version)
                @compileError("duplicate workflow identity");
        }
    }
    return struct {
        /// Exact caller environment type required when the description runs.
        pub const Environment = @TypeOf(handlers[0]).Environment;

        /// Resolve a request through the selected capability.
        /// Parameters:
        /// - `name: []const u8`: Stable, nonempty identity component validated at definition time.
        /// - `version: u32`: Stable version used to reject incompatible encoded or replayed data.
        /// Returns (`?usize`): matching workflow definition, or `null` for an unknown identity.
        pub fn resolve(name: []const u8, version: u32) ?usize {
            inline for (handlers, 0..) |handler, index| {
                if (std.mem.eql(u8, name, handler.definition.description.name) and
                    version == handler.definition.description.version)
                    return index;
            }
            return null;
        }

        /// Return the completed typed definition at the selected index.
        /// Parameters:
        /// - `index: comptime usize`: Compile-time or runtime position in the selected collection.
        /// Returns (`types.Definition`): Definition metadata for the selected workflow.
        pub fn definition(comptime index: usize) types.Definition {
            if (index >= handlers.len) @compileError("workflow registry index is out of bounds");
            return handlers[index].definition.description;
        }
    };
}
