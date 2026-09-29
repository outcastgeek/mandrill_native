//! Typed RPC-facing Workflow call descriptor. It retains the original
//! request/payload type and operation descriptor while deriving a durable
//! execution identity; transport and hosting remain caller capabilities.
const outcomes = @import("../outcome.zig");
const keys = @import("../primary_key.zig");
const effects = @import("../effect.zig");
const types = @import("workflow_types.zig");
const workflows = @import("workflow.zig");

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
        pub const Payload = Definition.Payload;
        pub const Success = Definition.Success;
        pub const DomainError = Definition.DomainError;

        definition: Definition,
        operation: Operation,
        execution_key: *const fn (*const Payload) []const u8,

        pub fn derive(comptime definition: Definition, comptime operation: Operation, comptime execution_key: fn (*const Payload) []const u8) Self {
            return .{ .definition = definition, .operation = operation, .execution_key = execution_key };
        }

        pub fn handle(comptime self: Self, payload: *const Payload) outcomes.Result(workflows.Handle(Definition), keys.Error) {
            const encoded = self.execution_key(payload);
            return switch (keys.inspectEncoded(encoded)) {
                .ok => .{ .ok = self.definition.handle(.{ .encoded = encoded }) },
                .err => |problem| .{ .err = problem },
            };
        }

        pub fn submission(comptime self: Self, payload: *const Payload, encoded_payload: []const u8) outcomes.Result(types.Submission, keys.Error) {
            return switch (self.handle(payload)) {
                .ok => |identified| .{ .ok = identified.submission(encoded_payload) },
                .err => |problem| .{ .err = problem },
            };
        }

        pub fn invoke(comptime self: Self, comptime handler: anytype, comptime R: type, comptime select_payload: fn (*const R) *const Payload) effects.Effect(Success, DomainError, R) {
            if (@TypeOf(handler).WorkflowDefinition != Definition or
                @TypeOf(handler).Environment != R)
                @compileError("workflow proxy handler must match its definition and environment");
            _ = self;
            return handler.run(select_payload);
        }
    };
}
