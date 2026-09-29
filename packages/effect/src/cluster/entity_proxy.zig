//! Typed entity routing over an existing RPC operation. The original request
//! type, constructor and key extractors stay intact; routing adds an address
//! instead of rebuilding the business request as an untyped envelope.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const types = @import("cluster_types.zig");
const messages = @import("message_store.zig");

pub fn Proxy(comptime EntityDefinition: type, comptime Operation: type) type {
    comptime {
        if (!@hasDecl(EntityDefinition, "Request") or
            !@hasDecl(EntityDefinition, "Success") or
            !@hasDecl(EntityDefinition, "DomainError") or
            !@hasDecl(Operation, "Request") or
            !@hasDecl(Operation, "Success") or
            !@hasDecl(Operation, "DomainError"))
            @compileError("entity proxy requires typed entity and RPC descriptors");
        if (EntityDefinition.Request != Operation.Request or
            EntityDefinition.Success != Operation.Success or
            EntityDefinition.DomainError != Operation.DomainError)
            @compileError("entity proxy must preserve the original RPC request, success and error types");
    }
    return struct {
        const Self = @This();
        pub const Request = Operation.Request;
        pub const Success = Operation.Success;
        pub const DomainError = Operation.DomainError;

        entity: EntityDefinition,
        operation: Operation,
        entity_key: *const fn (*const Request) []const u8,
        request_key: *const fn (*const Request) []const u8,

        pub fn derive(comptime entity: EntityDefinition, comptime operation: Operation, comptime entity_key: fn (*const Request) []const u8, comptime request_key: *const fn (*const Request) []const u8) Self {
            return .{ .entity = entity, .operation = operation, .entity_key = entity_key, .request_key = request_key };
        }

        pub fn deriveUsingOperation(comptime entity: EntityDefinition, comptime operation: Operation, comptime entity_key: fn (*const Request) []const u8) Self {
            if (!@hasField(Operation, "idempotency_key") or operation.idempotency_key == null)
                @compileError("entity proxy requires the operation's explicit idempotency key");
            return derive(entity, operation, entity_key, operation.idempotency_key.?);
        }

        pub fn address(self: Self, request: *const Request) outcomes.Result(types.Address, types.IdentityError) {
            const result = types.Address{ .group = self.entity.group, .entity_key = self.entity_key(request), .request_key = self.request_key(request) };
            return switch (result.validate()) {
                .ok => .{ .ok = result },
                .err => |problem| .{ .err = problem },
            };
        }

        pub fn submission(self: Self, request: *const Request, encoded_request: []const u8, retention_until: deadlines.Deadline) outcomes.Result(messages.Submission, types.IdentityError) {
            return switch (self.address(request)) {
                .ok => |located| .{ .ok = .{ .address = located, .definition_version = self.entity.version, .encoded_request = encoded_request, .retention_until = retention_until } },
                .err => |problem| .{ .err = problem },
            };
        }

        pub fn invoke(comptime self: Self, comptime handler: anytype, comptime R: type, comptime select_request: fn (*const R) *const Request) effects.Effect(Success, DomainError, R) {
            if (@TypeOf(handler).EntityDefinition != EntityDefinition or
                @TypeOf(handler).Environment != R)
                @compileError("entity proxy handler must match its definition and environment");
            _ = self;
            return handler.invoke(select_request);
        }
    };
}
