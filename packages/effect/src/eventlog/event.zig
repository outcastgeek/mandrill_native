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

pub const Address = struct {
    aggregate_key: []const u8,
    event_id: []const u8,

    pub fn valid(self: Address) bool {
        return keys.inspectEncoded(self.aggregate_key) == .ok and
            keys.inspectEncoded(self.event_id) == .ok;
    }
};

pub fn Event(comptime PayloadCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    inline for (.{ PayloadCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    return struct {
        const Self = @This();
        pub const Payload = PayloadCodec.Decoded;
        pub const Success = SuccessCodec.Decoded;
        pub const DomainError = ErrorCodec.Decoded;
        pub const PayloadSchema = PayloadCodec;
        pub const SuccessSchema = SuccessCodec;
        pub const ErrorSchema = ErrorCodec;
        pub const Channel = enum { payload, success, failure };

        tag: []const u8,
        version: u32,
        payload: PayloadCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

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

        pub fn validate(comptime self: Self) void {
            if (comptime self.tag.len == 0 or !std.unicode.utf8ValidateSlice(self.tag))
                @compileError("event tags must be nonempty UTF-8");
            if (comptime self.version == 0) @compileError("event schema versions start after the reserved absent version");
        }

        pub fn handledBy(
            comptime self: Self,
            comptime R: type,
            comptime handler: fn (*const Payload, *const R) effects.Effect(Success, DomainError, R),
        ) Handler(Self, R) {
            self.validate();
            return .{ .definition = self, .handle = handler };
        }

        pub fn encodePayloadDirect(comptime self: Self, value: *const Payload) outcomes.Result(PayloadCodec.Representation, Issue) {
            self.validate();
            if (!@hasDecl(PayloadCodec, "encodeDirect"))
                @compileError("direct event payload encoding requires a pure codec");
            return self.payload.encodeDirect(value);
        }

        pub fn decodePayloadDirect(comptime self: Self, value: *const PayloadCodec.Representation) outcomes.Result(Payload, Issue) {
            self.validate();
            if (!@hasDecl(PayloadCodec, "decodeDirect"))
                @compileError("direct event payload decoding requires a pure codec");
            return self.payload.decodeDirect(value);
        }

        pub fn encodePayload(comptime self: Self, value: *const Payload) effects.Effect(PayloadCodec.Representation, Issue, PayloadCodec.EncodingEnvironment) {
            self.validate();
            if (!@hasDecl(PayloadCodec, "encode"))
                @compileError("effectful event payload encoding requires an effectful codec");
            return self.payload.encode(value);
        }

        pub fn decodePayload(comptime self: Self, value: *const PayloadCodec.Representation) effects.Effect(Payload, Issue, PayloadCodec.DecodingEnvironment) {
            self.validate();
            if (!@hasDecl(PayloadCodec, "decode"))
                @compileError("effectful event payload decoding requires an effectful codec");
            return self.payload.decode(value);
        }
    };
}

pub fn Handler(comptime Definition: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const EventDefinition = Definition;
        pub const Environment = R;
        definition: Definition,
        handle: *const fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, Definition.DomainError, R),

        pub fn invoke(comptime self: Self, comptime select_payload: fn (*const R) *const Definition.Payload) effects.Effect(Definition.Success, Definition.DomainError, R) {
            self.definition.validate();
            const Program = effects.Effect(Definition.Success, Definition.DomainError, R);
            return Program.deferEffect(struct {
                fn run(env: *const R) Program {
                    return self.handle(select_payload(env), env);
                }
            }.run);
        }

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
        pub const Environment = R;

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

pub const ResolveError = union(enum) {
    unknown_event,
    incompatible_version: struct { requested: u32, supported: u32 },
};
