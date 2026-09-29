//! Typed RPC-facing Workflow call descriptor. It retains the original
//! request/payload type and operation descriptor while deriving a durable
//! execution identity; transport and hosting remain caller capabilities.
const outcomes = @import("../outcome.zig");
const keys = @import("../primary_key.zig");
const effects = @import("../effect.zig");
const types = @import("workflow_types.zig");
const workflows = @import("workflow.zig");

/// Specialize a typed workflow client from its definition.
/// Parameters:
/// - `Definition: comptime type`: Compile-time typed definition used to derive this API.
/// - `Operation: comptime type`: Compile-time typed operation descriptor defining request, success, and error channels.
/// Returns (`type`): typed workflow client derived from a definition and its operation.
pub fn Proxy(comptime Definition: type, comptime Operation: type) type {
    comptime {
        if (!@hasDecl(Definition, "Payload") or !@hasDecl(Definition, "Success") or
            !@hasDecl(Definition, "DomainError") or
            !@hasDecl(Operation, "Request") or !@hasDecl(Operation, "Success") or
            !@hasDecl(Operation, "DomainError"))
            @compileError("workflow proxy requires typed workflow and RPC descriptors");
        if (Definition.Payload != Operation.Request or
            Definition.Success != Operation.Success or
            Definition.DomainError != Operation.DomainError)
            @compileError("workflow proxy must preserve the original RPC request, success and error types");
    }
    return struct {
        const Self = @This();
        /// Typed payload carried by this definition.
        pub const Payload = Definition.Payload;
        /// Value type carried by the successful channel.
        pub const Success = Definition.Success;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = Definition.DomainError;

        definition: Definition,
        operation: Operation,
        execution_key: *const fn (*const Payload) []const u8,

        /// Derive a typed proxy from the completed definition.
        /// Parameters:
        /// - `definition: comptime Definition`: Typed definition whose codec, error, and environment members are preserved.
        /// - `operation: comptime Operation`: Compile-time RPC or workflow operation descriptor selected by this API.
        /// - `execution_key: comptime fn (*const Payload) []const u8`: Compile-time typed function selected at comptime; no runtime
        ///   callback lookup is required.
        /// Returns (`Self`): proxy bound to this workflow definition, operation, and execution-key selector.
        pub fn derive(comptime definition: Definition, comptime operation: Operation, comptime execution_key: fn (*const Payload) []const u8) Self {
            return .{ .definition = definition, .operation = operation, .execution_key = execution_key };
        }

        /// Create a typed handle for the selected definition or operation.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `payload: *const Payload`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(workflows.Handle(Definition), keys.Error)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn handle(comptime self: Self, payload: *const Payload) outcomes.Result(workflows.Handle(Definition), keys.Error) {
            const encoded = self.execution_key(payload);
            return switch (keys.inspectEncoded(encoded)) {
                .ok => .{ .ok = self.definition.handle(.{ .encoded = encoded }) },
                .err => |problem| .{ .err = problem },
            };
        }

        /// Construct the submission for this definition.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `payload: *const Payload`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `encoded_payload: []const u8`: Encoded payload owned by the caller until copied.
        /// Returns (`outcomes.Result(types.Submission, keys.Error)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn submission(comptime self: Self, payload: *const Payload, encoded_payload: []const u8) outcomes.Result(types.Submission, keys.Error) {
            return switch (self.handle(payload)) {
                .ok => |identified| .{ .ok = identified.submission(encoded_payload) },
                .err => |problem| .{ .err = problem },
            };
        }

        /// Invoke the selected typed operation.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `handler: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `select_payload: comptime fn (*const R) *const Payload`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(Success, DomainError, R)`): typed reply from invoking the selected workflow through its proxy.
        pub fn invoke(comptime self: Self, comptime handler: anytype, comptime R: type, comptime select_payload: fn (*const R) *const Payload) effects.Effect(Success, DomainError, R) {
            if (@TypeOf(handler).WorkflowDefinition != Definition or
                @TypeOf(handler).Environment != R)
                @compileError("workflow proxy handler must match its definition and environment");
            _ = self;
            return handler.run(select_payload);
        }
    };
}
