//! Typed entity routing over an existing RPC operation. The original request
//! type, constructor and key extractors stay intact; routing adds an address
//! instead of rebuilding the business request as an untyped envelope.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const types = @import("cluster_types.zig");
const messages = @import("message_store.zig");

/// Specialize a typed cluster client from its definition.
/// Parameters:
/// - `EntityDefinition: comptime type`: Compile-time completed typed entity definition.
/// - `Operation: comptime type`: Compile-time typed operation descriptor defining request, success, and error channels.
/// Returns (`type`): typed client proxy derived from an entity definition and its operation.
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
        /// Typed request payload or request record for this operation.
        pub const Request = Operation.Request;
        /// Value type carried by the successful channel.
        pub const Success = Operation.Success;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = Operation.DomainError;

        entity: EntityDefinition,
        operation: Operation,
        entity_key: *const fn (*const Request) []const u8,
        request_key: *const fn (*const Request) []const u8,

        /// Derive a typed proxy from the completed definition.
        /// Parameters:
        /// - `entity: comptime EntityDefinition`: Completed typed entity definition.
        /// - `operation: comptime Operation`: Compile-time RPC or workflow operation descriptor selected by this API.
        /// - `entity_key: comptime fn (*const Request) []const u8`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `request_key: comptime *const fn (*const Request) []const u8`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): proxy bound to this entity definition, operation, and key selector.
        pub fn derive(comptime entity: EntityDefinition, comptime operation: Operation, comptime entity_key: fn (*const Request) []const u8, comptime request_key: *const fn (*const Request) []const u8) Self {
            return .{ .entity = entity, .operation = operation, .entity_key = entity_key, .request_key = request_key };
        }

        /// Derive a typed proxy using the selected operation.
        /// Parameters:
        /// - `entity: comptime EntityDefinition`: Completed typed entity definition.
        /// - `operation: comptime Operation`: Compile-time RPC or workflow operation descriptor selected by this API.
        /// - `entity_key: comptime fn (*const Request) []const u8`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): proxy reusing the selected operation’s typed request and result contract.
        pub fn deriveUsingOperation(comptime entity: EntityDefinition, comptime operation: Operation, comptime entity_key: fn (*const Request) []const u8) Self {
            if (!@hasField(Operation, "idempotency_key") or operation.idempotency_key == null)
                @compileError("entity proxy requires the operation's explicit idempotency key");
            return derive(entity, operation, entity_key, operation.idempotency_key.?);
        }

        /// Return the routed address of this entity.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `request: *const Request`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(types.Address, types.IdentityError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn address(self: Self, request: *const Request) outcomes.Result(types.Address, types.IdentityError) {
            const result = types.Address{ .group = self.entity.group, .entity_key = self.entity_key(request), .request_key = self.request_key(request) };
            return switch (result.validate()) {
                .ok => .{ .ok = result },
                .err => |problem| .{ .err = problem },
            };
        }

        /// Construct the submission for this definition.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `request: *const Request`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `encoded_request: []const u8`: Borrowed wire request to decode or dispatch.
        /// - `retention_until: deadlines.Deadline`: Deadline after which retained data may be cleaned up.
        /// Returns (`outcomes.Result(messages.Submission, types.IdentityError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn submission(self: Self, request: *const Request, encoded_request: []const u8, retention_until: deadlines.Deadline) outcomes.Result(messages.Submission, types.IdentityError) {
            return switch (self.address(request)) {
                .ok => |located| .{ .ok = .{ .address = located, .definition_version = self.entity.version, .encoded_request = encoded_request, .retention_until = retention_until } },
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
        /// - `select_request: comptime fn (*const R) *const Request`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(Success, DomainError, R)`): typed reply from invoking the selected entity operation through the cluster provider.
        pub fn invoke(comptime self: Self, comptime handler: anytype, comptime R: type, comptime select_request: fn (*const R) *const Request) effects.Effect(Success, DomainError, R) {
            if (@TypeOf(handler).EntityDefinition != EntityDefinition or
                @TypeOf(handler).Environment != R)
                @compileError("entity proxy handler must match its definition and environment");
            _ = self;
            return handler.invoke(select_request);
        }
    };
}
