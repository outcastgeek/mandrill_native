//! Typed event definitions. A stable tag and version pair owns its payload,
//! success and domain-error schema relationships at comptime.
const std = @import("std");
const effects = @import("../effect.zig");
const keys = @import("../primary_key.zig");
const outcomes = @import("../outcome.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;
const journal = @import("event_journal.zig");
const log = @import("event_log.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

/// Stable event group and tag identifying a handler operation.
pub const Address = struct {
    aggregate_key: []const u8,
    event_id: []const u8,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Address`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when aggregate key and event ID are both valid encoded identities.
    pub fn valid(self: Address) bool {
        return keys.inspectEncoded(self.aggregate_key) == .ok and
            keys.inspectEncoded(self.event_id) == .ok;
    }
};

/// Specialize a typed event definition with stable identity and payload codec.
/// Parameters:
/// - `PayloadCodec: comptime type`: Compile-time codec type for the payload.
/// - `SuccessCodec: comptime type`: Compile-time codec type for the successful result.
/// - `ErrorCodec: comptime type`: Compile-time codec type for the domain error.
/// Returns (`type`): typed stable event definition with payload, result, and error codecs.
pub fn Event(comptime PayloadCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    inline for (.{ PayloadCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    return struct {
        const Self = @This();
        /// Typed payload carried by this definition.
        pub const Payload = PayloadCodec.Decoded;
        /// Value type carried by the successful channel.
        pub const Success = SuccessCodec.Decoded;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = ErrorCodec.Decoded;
        /// PayloadSchema contract selected by this definition; its concrete type is `PayloadCodec`.
        pub const PayloadSchema = PayloadCodec;
        /// SuccessSchema contract selected by this definition; its concrete type is `SuccessCodec`.
        pub const SuccessSchema = SuccessCodec;
        /// ErrorSchema contract selected by this definition; its concrete type is `ErrorCodec`.
        pub const ErrorSchema = ErrorCodec;
        /// `Channel` cases: `payload`, `success`, `failure`.
        pub const Channel = enum { payload, success, failure };

        tag: []const u8,
        version: u32,
        payload: PayloadCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

        /// Select the schema for this event channel.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `channel: comptime Channel`: Compile-time event channel selected from the definition.
        /// Returns (`switch (channel)`): The schema type selected by the compile-time event channel.
        pub fn schema(comptime self: Self, comptime channel: Channel) switch (channel) {
            .payload => PayloadCodec,
            .success => SuccessCodec,
            .failure => ErrorCodec,
        } {
            self.validate();
            return switch (channel) {
                .payload => self.payload,
                .success => self.success,
                .failure => self.failure,
            };
        }

        /// Construct a statically checked definition.
        /// Parameters:
        /// - `tag: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `payload: comptime PayloadCodec`: Compile-time typed input value; move or clone owning payloads according to
        ///   its type contract.
        /// - `success: comptime SuccessCodec`: Codec or value for the successful result channel.
        /// - `failure: comptime ErrorCodec`: Codec or value for the typed failure channel.
        /// Returns (`Self`): A constructed instance; its provider and allocation ownership follow the containing type.
        pub fn define(
            comptime tag: []const u8,
            comptime version: u32,
            comptime payload: PayloadCodec,
            comptime success: SuccessCodec,
            comptime failure: ErrorCodec,
        ) Self {
            const definition: Self = .{ .tag = tag, .version = version, .payload = payload, .success = success, .failure = failure };
            definition.validate();
            return definition;
        }

        /// Validate the value against its declared contract.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`void`): no value.
        pub fn validate(comptime self: Self) void {
            if (comptime self.tag.len == 0 or !std.unicode.utf8ValidateSlice(self.tag))
                @compileError("event tags must be nonempty UTF-8");
            if (comptime self.version == 0) @compileError("event schema versions start after the reserved absent version");
        }

        /// Attach a typed handler to this definition.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `handler: comptime fn (*const Payload, *const R) effects.Effect(Success, DomainError, R)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`Handler(Self, R)`): A completed typed handler binding its operation to the selected environment.
        pub fn handledBy(
            comptime self: Self,
            comptime R: type,
            comptime handler: fn (*const Payload, *const R) effects.Effect(Success, DomainError, R),
        ) Handler(Self, R) {
            self.validate();
            return .{ .definition = self, .handle = handler };
        }

        /// Encode an event payload immediately.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `value: *const Payload`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(PayloadCodec.Representation, Issue)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn encodePayloadDirect(comptime self: Self, value: *const Payload) outcomes.Result(PayloadCodec.Representation, Issue) {
            self.validate();
            if (!@hasDecl(PayloadCodec, "encodeDirect"))
                @compileError("direct event payload encoding requires a pure codec");
            return self.payload.encodeDirect(value);
        }

        /// Decode an event payload immediately.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `value: *const PayloadCodec.Representation`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(Payload, Issue)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn decodePayloadDirect(comptime self: Self, value: *const PayloadCodec.Representation) outcomes.Result(Payload, Issue) {
            self.validate();
            if (!@hasDecl(PayloadCodec, "decodeDirect"))
                @compileError("direct event payload decoding requires a pure codec");
            return self.payload.decodeDirect(value);
        }

        /// Describe payload encoding as a lazy Effect.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `value: *const Payload`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(PayloadCodec.Representation, Issue, PayloadCodec.EncodingEnvironment)`): owned encoded bytes for this event’s typed payload.
        pub fn encodePayload(comptime self: Self, value: *const Payload) effects.Effect(PayloadCodec.Representation, Issue, PayloadCodec.EncodingEnvironment) {
            self.validate();
            if (!@hasDecl(PayloadCodec, "encode"))
                @compileError("effectful event payload encoding requires an effectful codec");
            return self.payload.encode(value);
        }

        /// Describe payload decoding as a lazy Effect.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `value: *const PayloadCodec.Representation`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(Payload, Issue, PayloadCodec.DecodingEnvironment)`): typed payload decoded through the event’s schema.
        pub fn decodePayload(comptime self: Self, value: *const PayloadCodec.Representation) effects.Effect(Payload, Issue, PayloadCodec.DecodingEnvironment) {
            self.validate();
            if (!@hasDecl(PayloadCodec, "decode"))
                @compileError("effectful event payload decoding requires an effectful codec");
            return self.payload.decode(value);
        }
    };
}

/// Specialize the typed handler signature for eventlog operations.
/// Parameters:
/// - `Definition: comptime type`: Compile-time typed definition used to derive this API.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): handler type matching the event payload, outcome, and environment.
pub fn Handler(comptime Definition: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Completed typed event definition.
        pub const EventDefinition = Definition;
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;
        definition: Definition,
        handle: *const fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, Definition.DomainError, R),

        /// Invoke the selected typed operation.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select_payload: comptime fn (*const R) *const Definition.Payload`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(Definition.Success, Definition.DomainError, R)`): handler result for this event’s typed payload.
        pub fn invoke(comptime self: Self, comptime select_payload: fn (*const R) *const Definition.Payload) effects.Effect(Definition.Success, Definition.DomainError, R) {
            self.definition.validate();
            const Program = effects.Effect(Definition.Success, Definition.DomainError, R);
            return Program.deferEffect(struct {
                fn run(env: *const R) Program {
                    return self.handle(select_payload(env), env);
                }
            }.run);
        }

        /// Append already encoded event bytes to the journal.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `select_payload: comptime fn (*const R) *const Definition.Payload`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_journal: comptime fn (*const R) *const journal.Journal(R, ProviderError)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_event_id: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `encode_payload: comptime fn (*const Definition.PayloadSchema, *const Definition.Payload, *const R) effects.Effect(outcomes.OwnedSlice(u8), Issue, R)`: Compile-time typed function selected at comptime; no runtime
        ///   callback lookup is required.
        /// Returns (`effects.Effect(log.AppendResult(Definition.Success), log.AppendError(Definition.DomainError, ProviderError), R)`): journal acknowledgement after encoding and appending this event.
        pub fn appendEncoded(
            comptime self: Self,
            comptime ProviderError: type,
            comptime select_payload: fn (*const R) *const Definition.Payload,
            comptime select_journal: fn (*const R) *const journal.Journal(R, ProviderError),
            comptime select_event_id: fn (*const R) []const u8,
            comptime encode_payload: fn (*const Definition.PayloadSchema, *const Definition.Payload, *const R) effects.Effect(outcomes.OwnedSlice(u8), Issue, R),
        ) effects.Effect(log.AppendResult(Definition.Success), log.AppendError(Definition.DomainError, ProviderError), R) {
            return log.handleAndAppendEncoded(self, R, ProviderError, select_payload, select_journal, select_event_id, encode_payload);
        }
    };
}

/// Build a closed group of typed event definitions.
/// Parameters:
/// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
///   instantiated.
/// Returns (`type`): closed group of completed event definitions with checked identities.
pub fn Group(comptime handlers: anytype) type {
    const Info = @typeInfo(@TypeOf(handlers));
    if (Info != .@"struct" or !Info.@"struct".is_tuple or handlers.len == 0)
        @compileError("an event group requires a nonempty tuple of completed handlers");
    inline for (handlers, 0..) |handler, index| {
        const H = @TypeOf(handler);
        if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "EventDefinition") or
            !@hasDecl(H, "Environment") or !@hasField(H, "definition") or
            H != Handler(H.EventDefinition, H.Environment))
            @compileError("event groups contain only completed typed handlers");
        handler.definition.validate();
        if (H.Environment != @TypeOf(handlers[0]).Environment)
            @compileError("event group handlers need one environment");
        inline for (handlers, 0..) |other, earlier| {
            if (earlier < index and std.mem.eql(u8, handler.definition.tag, other.definition.tag) and
                handler.definition.version == other.definition.version)
                @compileError("duplicate event identity in group");
        }
    }
    const R = @TypeOf(handlers[0]).Environment;
    return struct {
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;

        /// Resolve a request through the selected capability.
        /// Parameters:
        /// - `tag: []const u8`: Stable, nonempty identity component validated at definition time.
        /// - `version: u32`: Stable version used to reject incompatible encoded or replayed data.
        /// Returns (`outcomes.Result(usize, ResolveError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn resolve(tag: []const u8, version: u32) outcomes.Result(usize, ResolveError) {
            var supported: ?u32 = null;
            inline for (handlers, 0..) |handler, index| {
                if (std.mem.eql(u8, handler.definition.tag, tag)) {
                    if (handler.definition.version == version) return .{ .ok = index };
                    if (supported == null) supported = handler.definition.version;
                }
            }
            if (supported) |value| return .{ .err = .{ .incompatible_version = .{ .requested = version, .supported = value } } };
            return .{ .err = .unknown_event };
        }

        /// Route an incoming request to its typed handler.
        /// Parameters:
        /// - `Response: comptime type`: Response payload type produced by the selected operation.
        /// - `tag: []const u8`: Stable, nonempty identity component validated at definition time.
        /// - `version: u32`: Stable version used to reject incompatible encoded or replayed data.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `encoded: []const u8`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `visitor: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`outcomes.Result(Response, ResolveError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn dispatch(
            comptime Response: type,
            tag: []const u8,
            version: u32,
            env: *const R,
            encoded: []const u8,
            comptime visitor: anytype,
        ) outcomes.Result(Response, ResolveError) {
            const selected = switch (resolve(tag, version)) {
                .ok => |index| index,
                .err => |problem| return .{ .err = problem },
            };
            inline for (handlers, 0..) |handler, index| {
                if (selected == index) {
                    const response: Response = visitor(handler, env, encoded);
                    return .{ .ok = response };
                }
            }
            unreachable;
        }
    };
}

/// Typed failure when an event cannot be resolved to a completed definition.
pub const ResolveError = union(enum) {
    unknown_event,
    incompatible_version: struct { requested: u32, supported: u32 },
};
